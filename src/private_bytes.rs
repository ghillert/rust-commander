//! Immutable file-backed snapshots for large viewers and editor save-back.
//! A fresh anonymous file is sealed before mapping; mutable user files are never
//! memory-mapped, so external truncation cannot invalidate a mapped document.
use anyhow::{Result, ensure};
use std::{fs::File, io::Read, path::Path, sync::Arc};
#[derive(Debug)]
pub struct Bytes {
    map: Option<memmap2::Mmap>,
    _file: File,
}
impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        self.map.as_deref().unwrap_or(&[])
    }
}
impl Bytes {
    pub fn capture(
        input: &mut dyn Read,
        limit: u64,
        path: &Path,
        task: &mut crate::operations::Task,
    ) -> Result<Arc<Self>> {
        let mut file = tempfile::tempfile()?;
        let count = task.stream(&mut input.take(limit.saturating_add(1)), &mut file, path)?;
        ensure!(
            count <= limit,
            "File exceeds the {} MiB workflow limit",
            limit / (1024 * 1024)
        );
        file.sync_all()?;
        let map = if count == 0 {
            None
        } else {
            // SAFETY: the file is anonymous/private; neither its path nor a writable
            // handle escapes. All further access is through this read-only mapping.
            Some(unsafe { memmap2::Mmap::map(&file)? })
        };
        Ok(Arc::new(Self { map, _file: file }))
    }
}
