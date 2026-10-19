//! Single-request providers need a complete Buffer, not repeated writer calls.
//! Keep its bytes in a private file mapping instead of an unbounded heap Vec.
use crate::operations::Task;
use anyhow::Result;
use std::{fs::File, io::Read, path::Path};

struct Upload {
    map: memmap2::Mmap,
    // Retain the handle until after the mapping is dropped, including Windows.
    _file: File,
}
impl AsRef<[u8]> for Upload {
    fn as_ref(&self) -> &[u8] {
        &self.map
    }
}
pub(crate) fn buffer(
    input: &mut dyn Read,
    path: &Path,
    task: &mut Task,
) -> Result<opendal::Buffer> {
    let mut file = tempfile::tempfile()?;
    let size = task.stream(input, &mut file, path)?;
    task.check()?;
    if size == 0 {
        return Ok(opendal::Buffer::new());
    }
    file.sync_all()?;
    // SAFETY: this is a fresh anonymous/private temporary file. No path or
    // writable handle escapes this function. After mapping we never mutate or
    // truncate it; Bytes owns both the mapping and file until upload completes.
    let map = unsafe { memmap2::Mmap::map(&file)? };
    Ok(bytes::Bytes::from_owner(Upload { map, _file: file }).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Cursor,
        sync::{Arc, atomic::AtomicBool},
    };
    #[test]
    fn single_request_uploads_keep_large_and_empty_content_exact() {
        let content = vec![0x63; 1024 * 1024 + 317];
        let buffer = buffer(
            &mut Cursor::new(&content),
            Path::new("fixture"),
            &mut Task::default(),
        )
        .unwrap();
        assert_eq!(buffer.to_bytes().as_ref(), content);
        assert!(
            super::buffer(
                &mut Cursor::new([]),
                Path::new("empty"),
                &mut Task::default()
            )
            .unwrap()
            .is_empty()
        );
    }
    #[test]
    fn cancelled_staging_does_not_create_a_publishable_buffer() {
        let mut task = Task::new(Arc::new(AtomicBool::new(true)), |_| {});
        assert!(
            buffer(
                &mut Cursor::new(b"content"),
                Path::new("fixture"),
                &mut task
            )
            .is_err()
        );
    }
}
