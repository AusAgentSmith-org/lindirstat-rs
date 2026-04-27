use anyhow::{Context, Result};
use clap::Parser;
use jwalk::WalkDirGeneric;
use lindirstat_wire::{
    write_frame, Entry, Frame, Header, Summary, KIND_DIR, KIND_FILE, KIND_OTHER, KIND_SYMLINK,
    MAGIC, WIRE_VERSION,
};
use std::collections::HashMap;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const BUILD_HASH: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser, Debug)]
#[command(name = "scanner", version, about = "lindirstat scanner agent")]
struct Args {
    /// Root path to scan.
    path: Option<PathBuf>,

    /// Stay on one filesystem (don't cross mount points).
    #[arg(long)]
    one_filesystem: bool,

    /// Print wire version + build hash and exit.
    #[arg(long)]
    wire_version: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.wire_version {
        println!("wire={WIRE_VERSION} build={BUILD_HASH}");
        return Ok(());
    }

    let root = args.path.context("missing <path> argument")?;
    let root = root
        .canonicalize()
        .with_context(|| format!("canonicalize {}", root.display()))?;

    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);

    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    write_frame(
        &mut out,
        &Frame::Header(Header {
            magic: *MAGIC,
            version: WIRE_VERSION,
            root: root.display().to_string(),
            started_unix: started,
        }),
    )?;

    let start = Instant::now();
    let errors = AtomicU64::new(0);
    let mut entries: u64 = 0;
    let mut bytes: u64 = 0;

    // Map absolute path -> id for parent lookups. The root is id 0.
    let mut ids: HashMap<PathBuf, u32> = HashMap::new();
    ids.insert(root.clone(), 0);

    // Root entry.
    let root_md = std::fs::metadata(&root).ok();
    write_frame(
        &mut out,
        &Frame::Entry(Entry {
            id: 0,
            parent_id: 0,
            name: root.display().to_string(),
            size: root_md.as_ref().map(|m| m.len()).unwrap_or(0),
            mtime: root_md.as_ref().and_then(mtime_secs).unwrap_or(0),
            kind: KIND_DIR,
        }),
    )?;
    entries += 1;

    let mut next_id: u32 = 1;

    #[cfg(unix)]
    let root_dev = {
        use std::os::unix::fs::MetadataExt;
        root_md.as_ref().map(|m| m.dev()).unwrap_or(0)
    };

    let one_fs = args.one_filesystem;
    let walk = WalkDirGeneric::<((), ())>::new(&root)
        .skip_hidden(false)
        .follow_links(false)
        .process_read_dir(move |_depth, _path, _state, children| {
            #[cfg(unix)]
            if one_fs {
                use std::os::unix::fs::MetadataExt;
                children.iter_mut().for_each(|entry_result| {
                    if let Ok(entry) = entry_result {
                        if entry.file_type.is_dir() {
                            if let Ok(md) = std::fs::symlink_metadata(entry.path()) {
                                if md.dev() != root_dev {
                                    entry.read_children_path = None;
                                }
                            }
                        }
                    }
                });
            }
            #[cfg(not(unix))]
            let _ = one_fs;
        });

    for dent in walk {
        let dent = match dent {
            Ok(d) => d,
            Err(e) => {
                eprintln!("walk error: {e}");
                errors.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };

        let path = dent.path();
        if path == root {
            continue;
        }

        let md = match dent.metadata() {
            Ok(m) => m,
            Err(e) => {
                eprintln!("metadata error for {}: {}", path.display(), e);
                errors.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };

        let parent = path.parent().unwrap_or(&root).to_path_buf();
        let parent_id = *ids.get(&parent).unwrap_or(&0);

        let kind = if md.is_dir() {
            KIND_DIR
        } else if md.is_file() {
            KIND_FILE
        } else if md.file_type().is_symlink() {
            KIND_SYMLINK
        } else {
            KIND_OTHER
        };

        let id = next_id;
        next_id += 1;
        if kind == KIND_DIR {
            ids.insert(path.clone(), id);
        }

        let name = dent.file_name().to_string_lossy().into_owned();
        let size = md.len();
        bytes += size;
        entries += 1;

        write_frame(
            &mut out,
            &Frame::Entry(Entry {
                id,
                parent_id,
                name,
                size,
                mtime: mtime_secs(&md).unwrap_or(0),
                kind,
            }),
        )?;
    }

    write_frame(
        &mut out,
        &Frame::Summary(Summary {
            entries,
            bytes,
            errors: errors.load(Ordering::Relaxed),
            elapsed_ms: start.elapsed().as_millis() as u64,
        }),
    )?;
    out.flush()?;
    Ok(())
}

fn mtime_secs(md: &std::fs::Metadata) -> Option<i64> {
    md.modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}
