//! Read-only NFSv3 over TCP. The export is opened directly, without an OS mount.
mod rpc;

use super::Transport;
use crate::remote::Connection;
use anyhow::{Context, Result, ensure};
use opendal::{EntryMode, Metadata, raw::oio};
use rpc::{Decoder, Rpc, Xdr};
use std::{
    io::{self, Cursor, Read},
    net::{SocketAddr, ToSocketAddrs},
    path::Path,
    time::{Duration, Instant},
};

const NFS: u32 = 100003;
const MOUNT: u32 = 100005;
const PORTMAP: u32 = 100000;
const MAX_HANDLE: usize = 64;
const READ_SIZE: usize = 64 * 1024;

pub(super) struct Nfs {
    address: SocketAddr,
    root: Vec<u8>,
    credentials: Vec<u8>,
}

impl Nfs {
    pub(super) fn new(c: &Connection) -> Result<Self> {
        let url = url::Url::parse(&c.endpoint).context("Enter an endpoint such as nfs://server")?;
        ensure!(
            url.scheme() == "nfs" && matches!(url.path(), "" | "/") && url.fragment().is_none(),
            "Enter the server in Endpoint and the exported path in Root folder."
        );
        let host = url.host_str().context("Enter the NFS server hostname")?;
        let host = host.trim_matches(['[', ']']);
        let export = c.root.trim_matches('/');
        crate::remote::key(Path::new(export), false)?;
        ensure!(export.len() < 1024, "NFS export path is too long.");
        let export = format!("/{export}");
        let mut addresses = (host, 111).to_socket_addrs()?;
        let mut result = Err(anyhow::anyhow!("NFS hostname returned no addresses."));
        for address in addresses.by_ref().take(4) {
            result = Self::mount(address, url.port(), &export);
            if result.is_ok() {
                break;
            }
        }
        result.map_err(|error| {
            anyhow::anyhow!("Could not open NFSv3 export {export} on {host}: {error:#}")
        })
    }

    fn mount(rpcbind: SocketAddr, nfs_port: Option<u16>, export: &str) -> Result<Self> {
        let mut mapper = Rpc::connect(rpcbind)?;
        let mount_port = port(&mut mapper, MOUNT, rpcbind.is_ipv6())?;
        let nfs_port = match nfs_port {
            Some(port) => port,
            None => port(&mut mapper, NFS, rpcbind.is_ipv6())?,
        };
        let mut mount = Rpc::connect(SocketAddr::new(rpcbind.ip(), mount_port))?;
        let credentials = identity();
        let mut args = Xdr::default();
        args.opaque(export.as_bytes());
        let reply = mount.call(MOUNT, 3, 1, &credentials, &args)?;
        let mut data = Decoder(&reply);
        status(&mut data)?;
        let root = handle(&mut data)?;
        let count = data.word()?;
        ensure!(count <= 32, "Invalid NFS authentication flavor list.");
        let mut auth_none = false;
        let mut auth_sys = false;
        for _ in 0..count {
            match data.word()? {
                0 => auth_none = true,
                1 => auth_sys = true,
                _ => {}
            }
        }
        ensure!(
            auth_sys || auth_none,
            "The export requires an unsupported NFS authentication flavor."
        );
        Ok(Self {
            address: SocketAddr::new(rpcbind.ip(), nfs_port),
            root,
            credentials: if auth_sys { credentials } else { Vec::new() },
        })
    }

    fn resolve(&self, rpc: &mut Rpc, path: &str) -> Result<(Vec<u8>, Option<Attributes>)> {
        let mut file = self.root.clone();
        let mut attributes = None;
        for part in path.split('/').filter(|s| !s.is_empty()) {
            if let Some(attrs) = &attributes {
                ensure!(
                    matches!(attrs, Attributes { kind: 2, .. }),
                    "NFS parent is not a folder."
                );
            }
            (file, attributes) = self.lookup(rpc, &file, part)?;
        }
        Ok((file, attributes))
    }

    fn lookup(
        &self,
        rpc: &mut Rpc,
        dir: &[u8],
        name: &str,
    ) -> Result<(Vec<u8>, Option<Attributes>)> {
        ensure!(
            name.len() <= 255 && !matches!(name, "." | "..") && !name.contains('\0'),
            "Invalid NFS name."
        );
        let mut args = Xdr::default();
        args.opaque(dir);
        args.opaque(name.as_bytes());
        let reply = rpc.call(NFS, 3, 3, &self.credentials, &args)?;
        let mut data = Decoder(&reply);
        status(&mut data)?;
        Ok((handle(&mut data)?, optional_attributes(&mut data)?))
    }

    fn getattr(&self, rpc: &mut Rpc, file: &[u8]) -> Result<Attributes> {
        let mut args = Xdr::default();
        args.opaque(file);
        let reply = rpc.call(NFS, 3, 1, &self.credentials, &args)?;
        let mut data = Decoder(&reply);
        status(&mut data)?;
        attributes(&mut data)
    }
}

impl Transport for Nfs {
    fn stat(&self, path: &str) -> Result<Metadata> {
        let mut rpc = Rpc::connect(self.address)?;
        let (file, attrs) = self.resolve(&mut rpc, path)?;
        match attrs {
            Some(attrs) => attrs,
            None => self.getattr(&mut rpc, &file)?,
        }
        .metadata()
        .context("NFS browsing supports only regular files and folders.")
    }

    fn list(&self, path: &str) -> Result<Vec<oio::Entry>> {
        let mut rpc = Rpc::connect(self.address)?;
        let (dir, attrs) = self.resolve(&mut rpc, path)?;
        ensure!(
            attrs.is_none_or(|attrs| attrs.kind == 2),
            "This NFS path is not a folder."
        );
        let mut cookie = 0;
        let mut verifier = [0; 8];
        let mut entries = Vec::new();
        let mut count = 0;
        let mut plus = true;
        let deadline = Instant::now() + Duration::from_secs(24);
        loop {
            ensure!(Instant::now() < deadline, "NFS folder listing timed out.");
            let mut args = Xdr::default();
            args.opaque(&dir);
            args.wide(cookie);
            args.0.extend(verifier);
            args.word(32 * 1024);
            if plus {
                args.word(128 * 1024);
            }
            let procedure = if plus { 17 } else { 16 };
            let reply = rpc.call(NFS, 3, procedure, &self.credentials, &args)?;
            if plus && reply.starts_with(&10004_u32.to_be_bytes()) {
                // Some servers implement READDIR but return NFS3ERR_NOTSUPP for READDIRPLUS.
                plus = false;
                cookie = 0;
                verifier = [0; 8];
                entries.clear();
                count = 0;
                continue;
            }
            let mut data = Decoder(&reply);
            status(&mut data)?;
            optional_attributes(&mut data)?;
            verifier.copy_from_slice(data.take(8)?);
            let previous = cookie;
            while data.boolean()? {
                ensure!(Instant::now() < deadline, "NFS folder listing timed out.");
                count += 1;
                ensure!(
                    count <= 100_000,
                    "NFS folder exceeds the supported listing size."
                );
                data.wide()?; // File ID
                let name = std::str::from_utf8(data.opaque(255)?).ok();
                cookie = data.wide()?;
                let attrs = if plus {
                    optional_attributes(&mut data)?
                } else {
                    None
                };
                let file = if plus && data.boolean()? {
                    Some(handle(&mut data)?)
                } else {
                    None
                };
                let Some(name) = name.filter(|name| !matches!(*name, "." | "..")) else {
                    continue;
                };
                let attrs = match (attrs, file) {
                    (Some(attrs), _) => attrs,
                    (None, Some(file)) => self.getattr(&mut rpc, &file)?,
                    (None, None) => {
                        let (file, attrs) = self.lookup(&mut rpc, &dir, name)?;
                        match attrs {
                            Some(attrs) => attrs,
                            None => self.getattr(&mut rpc, &file)?,
                        }
                    }
                };
                if let Some(entry) = super::child_key(path, name, attrs.metadata()) {
                    entries.push(entry);
                }
            }
            if data.boolean()? {
                return Ok(entries);
            }
            ensure!(cookie != previous, "NFS listing did not advance.");
        }
    }

    fn read(&self, path: &str, offset: u64) -> Result<Box<dyn Read + Send>> {
        let mut rpc = Rpc::connect(self.address)?;
        let (file, attrs) = self.resolve(&mut rpc, path)?;
        let attrs = match attrs {
            Some(attrs) => attrs,
            None => self.getattr(&mut rpc, &file)?,
        };
        ensure!(attrs.kind == 1, "Select a regular NFS file.");
        Ok(Box::new(Reader {
            rpc,
            file,
            offset,
            credentials: self.credentials.clone(),
            buffer: Cursor::new(Vec::new()),
            eof: offset >= attrs.size,
        }))
    }
}

struct Reader {
    rpc: Rpc,
    file: Vec<u8>,
    offset: u64,
    credentials: Vec<u8>,
    buffer: Cursor<Vec<u8>>,
    eof: bool,
}

impl Reader {
    fn fill(&mut self) -> Result<()> {
        let mut args = Xdr::default();
        args.opaque(&self.file);
        args.wide(self.offset);
        args.word(READ_SIZE as u32);
        let reply = self.rpc.call(NFS, 3, 6, &self.credentials, &args)?;
        let mut data = Decoder(&reply);
        status(&mut data)?;
        optional_attributes(&mut data)?;
        let count = data.word()? as usize;
        self.eof = data.boolean()?;
        let bytes = data.opaque(READ_SIZE)?;
        ensure!(
            bytes.len() == count && (count != 0 || self.eof),
            "Invalid NFS read length."
        );
        self.offset = self
            .offset
            .checked_add(count as u64)
            .context("NFS read offset overflow")?;
        self.buffer = Cursor::new(bytes.to_vec());
        Ok(())
    }
}

impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !buf.is_empty()
            && self.buffer.position() == self.buffer.get_ref().len() as u64
            && !self.eof
        {
            self.fill().map_err(io::Error::other)?;
        }
        self.buffer.read(buf)
    }
}

fn identity() -> Vec<u8> {
    #[cfg(unix)]
    let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
    #[cfg(not(unix))]
    let (uid, gid) = (65534, 65534);
    let mut auth = Xdr::default();
    auth.word(rand::random());
    auth.opaque(b"rust-commander");
    auth.word(uid);
    auth.word(gid);
    auth.word(0); // Supplemental groups
    auth.0
}

fn port(rpc: &mut Rpc, program: u32, ipv6: bool) -> Result<u16> {
    let mut args = Xdr::default();
    args.word(program);
    args.word(3);
    let port = if ipv6 {
        // rpcbind v4 GETADDR returns a universal address ending in two port bytes.
        args.opaque(b"tcp6");
        args.opaque(&[]);
        args.opaque(&[]);
        let reply = rpc.call(PORTMAP, 4, 3, &[], &args)?;
        let address = std::str::from_utf8(Decoder(&reply).opaque(128)?)?;
        if address.is_empty() {
            0
        } else {
            let (host_and_high, low) = address
                .rsplit_once('.')
                .context("Invalid rpcbind address")?;
            let (_, high) = host_and_high
                .rsplit_once('.')
                .context("Invalid rpcbind address")?;
            u16::from_be_bytes([high.parse()?, low.parse()?])
        }
    } else {
        args.word(6); // TCP
        args.word(0);
        let reply = rpc.call(PORTMAP, 2, 3, &[], &args)?;
        u16::try_from(Decoder(&reply).word()?)?
    };
    ensure!(port != 0, "NFSv3 RPC program {program} is not registered.");
    Ok(port)
}

fn status(data: &mut Decoder<'_>) -> Result<()> {
    match data.word()? {
        0 => Ok(()),
        2 => anyhow::bail!("NFS file or export not found."),
        1 | 13 => anyhow::bail!("Permission denied by the NFS server."),
        20 => anyhow::bail!("This NFS path is not a folder."),
        code => anyhow::bail!("NFS server returned status {code}."),
    }
}

fn handle(data: &mut Decoder<'_>) -> Result<Vec<u8>> {
    let handle = data.opaque(MAX_HANDLE)?;
    ensure!(!handle.is_empty(), "NFS returned an empty file handle.");
    Ok(handle.to_vec())
}

struct Attributes {
    kind: u32,
    size: u64,
    modified: Option<chrono::DateTime<chrono::Utc>>,
}

impl Attributes {
    fn metadata(self) -> Option<Metadata> {
        let mode = match self.kind {
            1 => EntryMode::FILE,
            2 => EntryMode::DIR,
            _ => return None,
        };
        let mut meta = Metadata::new(mode).with_content_length(self.size);
        if let Some(modified) = self.modified {
            meta.set_last_modified(modified);
        }
        Some(meta)
    }
}

fn attributes(data: &mut Decoder<'_>) -> Result<Attributes> {
    let kind = data.word()?;
    data.take(16)?; // Mode, link count, UID, GID
    let size = data.wide()?;
    data.take(40)?; // Used space, device ID, filesystem ID, file ID, atime
    let modified = chrono::DateTime::from_timestamp(i64::from(data.word()?), data.word()?);
    data.take(8)?; // ctime
    Ok(Attributes {
        kind,
        size,
        modified,
    })
}

fn optional_attributes(data: &mut Decoder<'_>) -> Result<Option<Attributes>> {
    data.boolean()?.then(|| attributes(data)).transpose()
}

#[cfg(test)]
mod tests;
