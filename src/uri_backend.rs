//! OpenDAL adapters for URI transports. Blocking socket I/O runs on blocking workers;
//! file transfers remain streamed and use the same guarded operations as other services.
mod ftp;
mod http;
mod sftp;
use crate::remote::{Connection, Secrets};
use opendal::{raw::*, *};
use std::{
    fmt,
    io::{Read, Write},
    sync::{Arc, Mutex},
};

pub(super) trait Output: Write + Send {
    fn finish(&mut self) -> anyhow::Result<()>;
}
pub(super) trait Transport: Send + Sync + 'static {
    fn stat(&self, path: &str) -> anyhow::Result<Metadata>;
    fn list(&self, path: &str) -> anyhow::Result<Vec<oio::Entry>>;
    fn read(&self, path: &str, offset: u64) -> anyhow::Result<Box<dyn Read + Send>>;
    fn write(&self, _path: &str, _append: bool) -> anyhow::Result<Box<dyn Output>> {
        anyhow::bail!("This location is read-only.")
    }
    fn mkdir(&self, _path: &str) -> anyhow::Result<()> {
        anyhow::bail!("This location is read-only.")
    }
    fn delete(&self, _path: &str) -> anyhow::Result<()> {
        anyhow::bail!("This location is read-only.")
    }
}

pub fn operator(c: &Connection, s: &Secrets) -> anyhow::Result<Option<Operator>> {
    let (transport, scheme, writable): (Arc<dyn Transport>, Scheme, bool) = match c.service.as_str()
    {
        "http" => (Arc::new(http::Http::new(c, s)?), Scheme::Http, false),
        "ftp" => (Arc::new(ftp::Ftp::new(c, s)?), Scheme::Ftp, true),
        "sftp"
            if cfg!(windows)
                || c.native_sftp
                || !c.known_hosts_file.is_empty()
                || (!s.password.is_empty() && c.key_file.is_empty()) =>
        {
            (
                Arc::new(sftp::Sftp::new(c, s)?),
                Scheme::Custom("rust-sftp"),
                true,
            )
        }
        _ => return Ok(None),
    };
    let info = Arc::new(AccessorInfo::default());
    info.set_scheme(scheme.into())
        .set_root("/")
        .set_native_capability(Capability {
            stat: true,
            read: true,
            list: true,
            write: writable,
            write_can_multi: writable,
            write_can_append: writable,
            create_dir: writable,
            delete: writable,
            shared: true,
            ..Default::default()
        });
    Ok(Some(
        OperatorBuilder::new(Backend { transport, info }).finish(),
    ))
}

#[derive(Clone)]
struct Backend {
    transport: Arc<dyn Transport>,
    info: Arc<AccessorInfo>,
}
impl fmt::Debug for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UriBackend")
    }
}

fn error(e: impl fmt::Display) -> Error {
    let message = e.to_string();
    let kind = if message.contains("not found") || message.contains("No such file") {
        ErrorKind::NotFound
    } else if message.contains("Permission denied") {
        ErrorKind::PermissionDenied
    } else {
        ErrorKind::Unexpected
    };
    Error::new(kind, message)
}
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(error)?
        .map_err(error)
}
impl Access for Backend {
    type Reader = StreamRead;
    type Writer = StreamWrite;
    type Lister = Listing;
    type Deleter = oio::OneShotDeleter<Self>;
    fn info(&self) -> Arc<AccessorInfo> {
        self.info.clone()
    }
    async fn stat(&self, path: &str, _: OpStat) -> Result<RpStat> {
        let t = self.transport.clone();
        let path = path.to_owned();
        Ok(RpStat::new(blocking(move || t.stat(&path)).await?))
    }
    async fn list(&self, path: &str, _: OpList) -> Result<(RpList, Self::Lister)> {
        let t = self.transport.clone();
        let path = path.to_owned();
        Ok((
            RpList::default(),
            Listing(blocking(move || t.list(&path)).await?.into_iter()),
        ))
    }
    async fn read(&self, path: &str, args: OpRead) -> Result<(RpRead, Self::Reader)> {
        let t = self.transport.clone();
        let path = path.to_owned();
        let range = args.range();
        let reader = blocking(move || t.read(&path, range.offset())).await?;
        Ok((
            RpRead::new(),
            StreamRead {
                reader: Arc::new(Mutex::new(reader)),
                remaining: range.size(),
            },
        ))
    }
    async fn write(&self, path: &str, args: OpWrite) -> Result<(RpWrite, Self::Writer)> {
        let t = self.transport.clone();
        let path = path.to_owned();
        let writer = blocking(move || t.write(&path, args.append())).await?;
        Ok((
            RpWrite::default(),
            StreamWrite {
                writer: Arc::new(Mutex::new(Some(writer))),
                size: 0,
            },
        ))
    }
    async fn create_dir(&self, path: &str, _: OpCreateDir) -> Result<RpCreateDir> {
        let t = self.transport.clone();
        let path = path.to_owned();
        blocking(move || t.mkdir(&path)).await?;
        Ok(RpCreateDir::default())
    }
    async fn delete(&self) -> Result<(RpDelete, Self::Deleter)> {
        Ok((RpDelete::default(), oio::OneShotDeleter::new(self.clone())))
    }
}
impl oio::OneShotDelete for Backend {
    async fn delete_once(&self, path: String, _: OpDelete) -> Result<()> {
        let t = self.transport.clone();
        blocking(move || t.delete(&path)).await
    }
}
struct Listing(std::vec::IntoIter<oio::Entry>);
impl oio::List for Listing {
    async fn next(&mut self) -> Result<Option<oio::Entry>> {
        Ok(self.0.next())
    }
}
struct StreamRead {
    reader: Arc<Mutex<Box<dyn Read + Send>>>,
    remaining: Option<u64>,
}
impl oio::Read for StreamRead {
    async fn read(&mut self) -> Result<Buffer> {
        let length = self.remaining.unwrap_or(64 * 1024).min(64 * 1024) as usize;
        if length == 0 {
            return Ok(Buffer::new());
        }
        let reader = self.reader.clone();
        let bytes = blocking(move || {
            let mut data = vec![0; length];
            let n = reader
                .lock()
                .map_err(|_| anyhow::anyhow!("Reader lock failed"))?
                .read(&mut data)?;
            data.truncate(n);
            Ok(data)
        })
        .await?;
        if let Some(remaining) = &mut self.remaining {
            *remaining -= bytes.len() as u64;
        }
        Ok(Buffer::from(bytes))
    }
}
struct StreamWrite {
    writer: Arc<Mutex<Option<Box<dyn Output>>>>,
    size: u64,
}
impl oio::Write for StreamWrite {
    async fn write(&mut self, data: Buffer) -> Result<()> {
        let writer = self.writer.clone();
        self.size += data.len() as u64;
        blocking(move || {
            writer
                .lock()
                .map_err(|_| anyhow::anyhow!("Writer lock failed"))?
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("Writer closed"))?
                .write_all(&data.to_vec())?;
            Ok(())
        })
        .await
    }
    async fn close(&mut self) -> Result<Metadata> {
        let writer = self.writer.clone();
        blocking(move || {
            let mut writer = writer
                .lock()
                .map_err(|_| anyhow::anyhow!("Writer lock failed"))?
                .take()
                .ok_or_else(|| anyhow::anyhow!("Writer closed"))?;
            writer.finish()
        })
        .await?;
        Ok(Metadata::new(EntryMode::FILE).with_content_length(self.size))
    }
    async fn abort(&mut self) -> Result<()> {
        self.writer.lock().map_err(error)?.take();
        Ok(())
    }
}
