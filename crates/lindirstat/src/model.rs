use lindirstat_wire::{Entry, KIND_DIR};
use std::collections::HashMap;

/// In-memory tree built incrementally from the scanner's wire stream.
#[derive(Default)]
pub struct Tree {
    pub entries: Vec<Entry>,
    pub subtree: Vec<u64>,
    pub id_to_index: HashMap<u32, usize>,
    pub children: HashMap<u32, Vec<usize>>,
}

impl Tree {
    pub fn push(&mut self, e: Entry) {
        let i = self.entries.len();
        self.id_to_index.insert(e.id, i);
        let parent_id = e.parent_id;
        let size = e.size;

        self.subtree.push(size);
        if e.id != parent_id {
            self.children.entry(parent_id).or_default().push(i);
        }
        self.entries.push(e);

        // Accumulate size into every ancestor.
        let mut cur = i;
        loop {
            let pid = self.entries[cur].parent_id;
            let Some(&pi) = self.id_to_index.get(&pid) else {
                break;
            };
            if pi == cur {
                break;
            }
            self.subtree[pi] += size;
            cur = pi;
        }
    }

    pub fn extend<I: IntoIterator<Item = Entry>>(&mut self, it: I) {
        for e in it {
            self.push(e);
        }
    }

    pub fn root_idx(&self) -> Option<usize> {
        self.entries.iter().position(|e| e.id == e.parent_id)
    }

    pub fn is_dir(&self, i: usize) -> bool {
        self.entries[i].kind == KIND_DIR
    }

    /// Child indices of a directory, or an empty slice if none.
    pub fn children_of(&self, i: usize) -> &[usize] {
        let id = self.entries[i].id;
        self.children.get(&id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn path_of(&self, i: usize) -> String {
        let mut parts = vec![self.entries[i].name.clone()];
        let mut cur_idx = i;
        loop {
            let pid = self.entries[cur_idx].parent_id;
            let Some(&pi) = self.id_to_index.get(&pid) else {
                break;
            };
            if pi == cur_idx {
                break;
            }
            parts.push(self.entries[pi].name.clone());
            cur_idx = pi;
        }
        parts.reverse();
        // Root name usually already contains slashes; join and collapse runs.
        let joined = parts.join("/");
        let mut out = String::with_capacity(joined.len());
        let mut prev_slash = false;
        for ch in joined.chars() {
            if ch == '/' {
                if !prev_slash {
                    out.push('/');
                }
                prev_slash = true;
            } else {
                out.push(ch);
                prev_slash = false;
            }
        }
        out
    }
}
