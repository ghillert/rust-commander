//! Bounded, read-only pages from local/remote objects or a private archive snapshot.
use super::*;
use anyhow::ensure;
use std::{
    io::{Seek, SeekFrom},
    path::PathBuf,
    sync::Arc,
};
#[derive(Debug)]
pub struct Source {
    pub location: Location,
    pub path: PathBuf,
    pub total: u64,
    stamp: String,
    local: Option<same_file::Handle>,
    archive: Option<Arc<crate::private_bytes::Bytes>>,
    _docker: Option<Arc<crate::uri_backend::docker::Container>>,
}
fn local_stamp(path: &Path) -> Result<String> {
    let m = std::fs::metadata(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(format!(
            "{}:{}:{}:{:?}",
            m.dev(),
            m.ino(),
            m.len(),
            m.modified()?
        ))
    }
    #[cfg(not(unix))]
    Ok(format!(
        "{}:{:?}:{:?}",
        m.len(),
        m.modified()?,
        m.created().ok()
    ))
}
impl Source {
    pub fn open(location: Location, path: PathBuf) -> Result<Arc<Self>> {
        let docker = location
            .docker_target()
            .as_ref()
            .map(crate::uri_backend::docker::attach);
        let (total, stamp, archive) = match &location {
            Location::Network { .. } | Location::Containers => {
                anyhow::bail!("Open a share or export to view files.")
            }
            Location::Local(_) => (std::fs::metadata(&path)?.len(), local_stamp(&path)?, None),
            Location::Remote { connection, .. } => {
                let (size, stamp) = crate::remote::viewer_stamp(connection, &path)?;
                (size, stamp, None)
            }
            Location::Archive { file, .. } => {
                let before = local_stamp(file)?;
                let bytes = crate::archive::snapshot_member(
                    file,
                    &path,
                    PAGE_CACHE_LIMIT,
                    &mut crate::operations::Task::default(),
                )?;
                ensure!(
                    local_stamp(file)? == before,
                    "Archive changed while preparing its viewer snapshot"
                );
                (bytes.as_ref().as_ref().len() as u64, before, Some(bytes))
            }
        };
        let local = if matches!(location, Location::Local(_)) {
            Some(same_file::Handle::from_path(&path)?)
        } else {
            None
        };
        Ok(Arc::new(Self {
            location,
            path,
            total,
            stamp,
            local,
            archive,
            _docker: docker,
        }))
    }
    fn verify(&self) -> Result<()> {
        if let Some(expected) = &self.local {
            ensure!(
                &same_file::Handle::from_path(&self.path)? == expected,
                "Viewer source was replaced; reopen it"
            );
        }
        let current = match &self.location {
            Location::Network { .. } | Location::Containers => {
                anyhow::bail!("Open a share or export to view files.")
            }
            Location::Local(_) => local_stamp(&self.path)?,
            Location::Remote { connection, .. } => {
                crate::remote::viewer_stamp(connection, &self.path)?.1
            }
            Location::Archive { .. } => return Ok(()),
        };
        ensure!(
            current == self.stamp,
            "Viewer source changed. Reopen it before navigating more pages."
        );
        Ok(())
    }
    pub fn reader_at(&self, offset: u64) -> Result<Box<dyn Read>> {
        ensure!(
            offset <= self.total,
            "Offset is beyond the end of this file"
        );
        self.verify()?;
        Ok(match &self.location {
            Location::Network { .. } | Location::Containers => {
                anyhow::bail!("Open a share or export to view files.")
            }
            Location::Local(_) => {
                let mut file = File::open(&self.path)?;
                ensure!(
                    self.local.as_ref() == Some(&same_file::Handle::from_file(file.try_clone()?)?),
                    "Viewer source was replaced; reopen it"
                );
                file.seek(SeekFrom::Start(offset))?;
                Box::new(file)
            }
            Location::Remote { connection, .. } => {
                crate::remote::read_range(connection, &self.path, offset)?
            }
            Location::Archive { .. } => Box::new(SnapshotReader {
                bytes: self.archive.clone().unwrap(),
                offset: offset as usize,
            }),
        })
    }
    pub fn bytes(&self, offset: u64, count: u64) -> Result<Vec<u8>> {
        ensure!(
            count <= crate::text_document::PAGE_SIZE + 4,
            "Page request exceeds the viewer buffer limit"
        );
        let mut bytes = Vec::new();
        self.reader_at(offset)?
            .take(count)
            .read_to_end(&mut bytes)?;
        self.verify()?;
        Ok(bytes)
    }
    pub fn page(
        self: &Arc<Self>,
        offset: u64,
        encoding: crate::text_document::Encoding,
    ) -> Result<crate::text_document::Page> {
        let mut offset = offset.min(self.total);
        if matches!(
            encoding,
            crate::text_document::Encoding::Utf16Le | crate::text_document::Encoding::Utf16Be
        ) {
            offset -= offset % 2;
        }
        let bytes = self.bytes(offset, crate::text_document::PAGE_SIZE + 4)?;
        let mut page = crate::text_document::Page::from_bytes(bytes, offset, self.total, encoding)?;
        page.source = Some(self.clone());
        Ok(page)
    }
    pub fn hex(&self, offset: u64) -> Result<String> {
        Ok(super::hex_dump_at(&self.bytes(offset, 65536)?, offset))
    }
}
struct SnapshotReader {
    bytes: Arc<crate::private_bytes::Bytes>,
    offset: usize,
}
impl Read for SnapshotReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let bytes = self.bytes.as_ref().as_ref();
        let count = out.len().min(bytes.len().saturating_sub(self.offset));
        out[..count].copy_from_slice(&bytes[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }
}
