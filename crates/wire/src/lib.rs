//! Wire protocol shared between the scanner agent and the client.

use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

pub const MAGIC: &[u8; 4] = b"LDS1";
pub const WIRE_VERSION: u32 = 1;

pub const KIND_FILE: u8 = 0;
pub const KIND_DIR: u8 = 1;
pub const KIND_SYMLINK: u8 = 2;
pub const KIND_OTHER: u8 = 3;

#[derive(Debug, Serialize, Deserialize)]
pub struct Header {
    pub magic: [u8; 4],
    pub version: u32,
    pub root: String,
    pub started_unix: u64,
}

/// A filesystem entry. `parent_id` references the `id` of a previously-emitted
/// directory Entry; the root is emitted first and has `parent_id = 0` (and
/// `id = 0` by convention — the client treats self-reference as "root").
#[derive(Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: u32,
    pub parent_id: u32,
    pub name: String,
    pub size: u64,
    pub mtime: i64,
    pub kind: u8,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Summary {
    pub entries: u64,
    pub bytes: u64,
    pub errors: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Frame {
    Header(Header),
    Entry(Entry),
    Summary(Summary),
}

/// Write one frame as `u32 LE length + postcard payload`.
pub fn write_frame<W: Write>(mut w: W, frame: &Frame) -> io::Result<()> {
    let bytes =
        postcard::to_allocvec(frame).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len = u32::try_from(bytes.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame too large"))?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(&bytes)?;
    Ok(())
}

/// Read one frame. Returns `Ok(None)` on clean EOF before a length prefix.
pub fn read_frame<R: Read>(mut r: R) -> io::Result<Option<Frame>> {
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    let frame =
        postcard::from_bytes(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(Some(frame))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut buf = Vec::new();
        let entry = Frame::Entry(Entry {
            id: 7,
            parent_id: 3,
            name: "foo.txt".into(),
            size: 1234,
            mtime: 1_700_000_000,
            kind: KIND_FILE,
        });
        write_frame(&mut buf, &entry).unwrap();
        let got = read_frame(&buf[..]).unwrap().unwrap();
        match got {
            Frame::Entry(e) => {
                assert_eq!(e.id, 7);
                assert_eq!(e.name, "foo.txt");
            }
            _ => panic!("wrong frame"),
        }
    }
}
