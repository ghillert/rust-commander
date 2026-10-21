use super::{Output, Transport};
use crate::remote::{Connection, FtpTls, Secrets};
use anyhow::{Context, Result, ensure};
use opendal::{EntryMode, Metadata, raw::oio};
use std::{
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::Arc,
    time::Duration,
};
use suppaftp::{RustlsConnector, RustlsFtpStream, Status, list::File, types::FileType};

pub(super) struct Ftp {
    connection: Connection,
    password: String,
}
impl Ftp {
    pub fn new(c: &Connection, s: &Secrets) -> Result<Self> {
        Ok(Self {
            connection: c.clone(),
            password: s.password.clone(),
        })
    }
    fn connect(&self) -> Result<RustlsFtpStream> {
        let c = &self.connection;
        let url = url::Url::parse(&c.endpoint).context("Invalid FTP endpoint")?;
        let host = url
            .host_str()
            .context("FTP needs a host")?
            .trim_matches(['[', ']']);
        let port = url.port().unwrap_or(if c.tls_mode() == FtpTls::Implicit {
            990
        } else {
            21
        });
        let addresses = (host, port).to_socket_addrs()?.collect::<Vec<_>>();
        ensure!(!addresses.is_empty(), "FTP host has no address");
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        if !c.tls_ca_file.is_empty() {
            roots.add(rustls::pki_types::CertificateDer::from(std::fs::read(
                &c.tls_ca_file,
            )?))?;
        }
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots)
        .with_no_client_auth();
        let connector = RustlsConnector::from(Arc::new(config));
        let mut ftp = if c.tls_mode() == FtpTls::Implicit {
            let mut ftp =
                RustlsFtpStream::connect_secure_implicit(addresses.as_slice(), connector, host)?;
            // SuppaFTP's legacy implicit constructor doesn't negotiate private data protection.
            ftp.custom_command("PBSZ 0", &[Status::CommandOk])?;
            ftp.custom_command("PROT P", &[Status::CommandOk])?;
            ftp
        } else {
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            let socket = addresses
                .iter()
                .find_map(|address| {
                    TcpStream::connect_timeout(
                        address,
                        deadline
                            .saturating_duration_since(std::time::Instant::now())
                            .max(Duration::from_millis(1)),
                    )
                    .ok()
                })
                .context("Cannot connect to FTP host within 30 seconds")?;
            socket.set_read_timeout(Some(Duration::from_secs(30)))?;
            socket.set_write_timeout(Some(Duration::from_secs(30)))?;
            let ftp = RustlsFtpStream::connect_with_stream(socket)?;
            if c.tls_mode() == FtpTls::Plain {
                ftp
            } else {
                ftp.into_secure(connector, host)?
            }
        };
        ftp.get_ref()
            .set_read_timeout(Some(Duration::from_secs(30)))?;
        ftp.get_ref()
            .set_write_timeout(Some(Duration::from_secs(30)))?;
        ftp = ftp.passive_stream_builder(|address| {
            let stream = TcpStream::connect_timeout(&address, Duration::from_secs(30))
                .map_err(suppaftp::FtpError::ConnectionError)?;
            stream
                .set_read_timeout(Some(Duration::from_secs(30)))
                .map_err(suppaftp::FtpError::ConnectionError)?;
            stream
                .set_write_timeout(Some(Duration::from_secs(30)))
                .map_err(suppaftp::FtpError::ConnectionError)?;
            Ok(stream)
        });
        let anonymous = c.username.is_empty() || c.username == "anonymous";
        ftp.login(
            if anonymous { "anonymous" } else { &c.username },
            if anonymous && self.password.is_empty() {
                "rust-commander@localhost"
            } else {
                &self.password
            },
        )?;
        ftp.transfer_type(FileType::Binary)?;
        // CWD validates the configured root. It must never create a missing root.
        ftp.cwd(if c.root.is_empty() { "/" } else { &c.root })?;
        Ok(ftp)
    }
    fn files(&self, path: &str) -> Result<Vec<File>> {
        let mut ftp = self.connect()?;
        let directory =
            (!path.trim_matches('/').is_empty()).then_some(path.trim_start_matches('/'));
        let files = match ftp.mlsd(directory) {
            Ok(lines) => lines
                .into_iter()
                .filter(|l| {
                    !l.to_ascii_lowercase().starts_with("type=cdir;")
                        && !l.to_ascii_lowercase().starts_with("type=pdir;")
                })
                .map(|l| File::from_mlsx_line(&l).map_err(anyhow::Error::from))
                .collect::<Result<Vec<_>>>()?,
            Err(_) => ftp
                .list(directory)?
                .into_iter()
                .map(|l| l.parse::<File>().map_err(anyhow::Error::from))
                .collect::<Result<Vec<_>>>()?,
        };
        Ok(files)
    }
}
fn metadata(file: &File) -> Metadata {
    let mut meta = Metadata::new(if file.is_directory() {
        EntryMode::DIR
    } else {
        EntryMode::FILE
    });
    meta.set_content_length(file.size() as u64);
    meta.set_last_modified(file.modified().into());
    meta
}
impl Transport for Ftp {
    fn stat(&self, path: &str) -> Result<Metadata> {
        if path.trim_matches('/').is_empty() {
            self.connect()?;
            return Ok(Metadata::new(EntryMode::DIR));
        }
        let path = path.trim_matches('/');
        let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
        self.files(parent)?
            .into_iter()
            .find(|f| f.name() == name)
            .map(|f| metadata(&f))
            .context("FTP file not found")
    }
    fn list(&self, path: &str) -> Result<Vec<oio::Entry>> {
        let prefix = path.trim_matches('/');
        self.files(prefix)?
            .into_iter()
            .filter(|f| f.name() != "." && f.name() != "..")
            .map(|f| {
                crate::filesystem::validate_name(f.name())?;
                let key = format!(
                    "{}{}{}{}",
                    prefix,
                    if prefix.is_empty() { "" } else { "/" },
                    f.name(),
                    if f.is_directory() { "/" } else { "" }
                );
                Ok(oio::Entry::new(&key, metadata(&f)))
            })
            .collect()
    }
    fn read(&self, path: &str, offset: u64) -> Result<Box<dyn Read + Send>> {
        let mut ftp = self.connect()?;
        if offset > 0 {
            ftp.resume_transfer(usize::try_from(offset)?)?;
        }
        let data = ftp.retr_as_stream(path)?;
        Ok(Box::new(FtpRead {
            ftp,
            data: Some(Box::new(data)),
        }))
    }
    fn write(&self, path: &str, append: bool) -> Result<Box<dyn Output>> {
        let mut ftp = self.connect()?;
        let data = if append {
            ftp.append_with_stream(path)?
        } else {
            ftp.put_with_stream(path)?
        };
        let socket = data.get_ref().try_clone()?;
        socket.set_read_timeout(Some(Duration::from_secs(30)))?;
        socket.set_write_timeout(Some(Duration::from_secs(30)))?;
        Ok(Box::new(FtpWrite {
            ftp,
            data: Some(Box::new(data)),
            socket,
        }))
    }
    fn mkdir(&self, path: &str) -> Result<()> {
        let mut ftp = self.connect()?;
        let mut prefix = String::new();
        for part in path.trim_matches('/').split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            if let Err(error) = ftp.mkdir(&prefix) {
                ensure!(
                    ftp.cwd(&prefix).is_ok(),
                    "Cannot create FTP directory: {error}"
                );
                ftp.cwd(if self.connection.root.is_empty() {
                    "/"
                } else {
                    &self.connection.root
                })?;
            }
        }
        Ok(())
    }
    fn delete(&self, path: &str) -> Result<()> {
        let mut ftp = self.connect()?;
        if path.ends_with('/') {
            ftp.rmdir(path)?;
        } else {
            ftp.rm(path)?;
        }
        Ok(())
    }
}
struct FtpRead {
    ftp: RustlsFtpStream,
    data: Option<Box<dyn Read + Send>>,
}
impl Read for FtpRead {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let Some(data) = self.data.as_mut() else {
            return Ok(0);
        };
        let n = data.read(buf)?;
        if n == 0 && !buf.is_empty() {
            self.ftp
                .finalize_retr_stream(self.data.take().unwrap())
                .map_err(std::io::Error::other)?;
        }
        Ok(n)
    }
}
struct FtpWrite {
    ftp: RustlsFtpStream,
    data: Option<Box<dyn Write + Send>>,
    socket: TcpStream,
}
impl Write for FtpWrite {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.data
            .as_mut()
            .ok_or_else(|| std::io::Error::other("FTP writer closed"))?
            .write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(data) = &mut self.data {
            data.flush()?;
        }
        Ok(())
    }
}
impl Output for FtpWrite {
    fn finish(&mut self) -> Result<()> {
        if let Some(mut data) = self.data.take() {
            data.flush()?;
            // Keep the socket alive while Rustls sends close_notify, then complete the
            // data-channel shutdown before waiting for the control-channel completion.
            drop(data);
            self.socket.shutdown(std::net::Shutdown::Write)?;
            let mut buffer = [0; 4096];
            loop {
                match self.socket.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => break,
                    Err(e) => return Err(e.into()),
                }
            }
            self.ftp.finalize_put_stream(std::io::sink())?;
        }
        Ok(())
    }
}
