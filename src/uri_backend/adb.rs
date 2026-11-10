//! Read-only file sync through the user's local Android Debug Bridge server.
use super::Transport;
use crate::remote::Connection;
use anyhow::{Context, Result, ensure};
use opendal::{EntryMode, Metadata, raw::oio};
use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    time::{Duration, Instant},
};

const SERVER: &str = "adb://localhost";
const TIMEOUT: Duration = Duration::from_secs(8);
const MAX_CHUNK: usize = 64 * 1024;
const ADDRESS: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 5037);

pub(crate) struct Adb {
    serial: String,
    root: String,
    address: SocketAddr,
}

impl Adb {
    pub(super) fn new(c: &Connection) -> Result<Self> {
        ensure!(
            matches!(c.endpoint.trim_end_matches('/'), "" | SERVER),
            "Android browsing uses the local ADB server on port 5037."
        );
        validate_serial(&c.bucket)?;
        let root = c.root.trim_matches('/');
        crate::remote::key(std::path::Path::new(root), false)?;
        Ok(Self {
            serial: c.bucket.clone(),
            root: root.into(),
            address: ADDRESS,
        })
    }

    fn sync(&self, id: &[u8; 4], path: &str) -> Result<TcpStream> {
        let mut socket = server(self.address)?;
        request(&mut socket, &format!("host:transport:{}", self.serial))?;
        request(&mut socket, "sync:")?;
        let path = [self.root.as_str(), path.trim_matches('/')]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("/");
        let path = format!("/{path}");
        ensure!(
            path.len() <= 1024 && !path.contains('\0'),
            "Invalid Android path."
        );
        socket.write_all(id)?;
        socket.write_all(&(path.len() as u32).to_le_bytes())?;
        socket.write_all(path.as_bytes())?;
        Ok(socket)
    }
}

pub(crate) fn validate_serial(serial: &str) -> Result<()> {
    ensure!(
        !serial.is_empty()
            && serial.len() <= 255
            && serial
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._:-[]%".contains(&b)),
        "Enter a valid ADB device serial."
    );
    Ok(())
}

fn server(address: SocketAddr) -> Result<TcpStream> {
    let socket = TcpStream::connect_timeout(&address, TIMEOUT)
        .context("Local ADB server unavailable. Run adb start-server and authorize your device.")?;
    socket.set_read_timeout(Some(TIMEOUT))?;
    socket.set_write_timeout(Some(TIMEOUT))?;
    Ok(socket)
}

fn receive<const N: usize>(socket: &mut impl Read) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    socket.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn payload(socket: &mut impl Read, len: usize, limit: usize) -> io::Result<Vec<u8>> {
    if len > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Oversized ADB response.",
        ));
    }
    let mut data = vec![0; len];
    socket.read_exact(&mut data)?;
    Ok(data)
}

fn host_payload(socket: &mut impl Read) -> Result<String> {
    let size = usize::from_str_radix(std::str::from_utf8(&receive::<4>(socket)?)?, 16)?;
    Ok(String::from_utf8(payload(socket, size, MAX_CHUNK)?)?)
}

fn request(socket: &mut (impl Read + Write), command: &str) -> Result<()> {
    write!(socket, "{:04x}{command}", command.len())?;
    match &receive::<4>(socket)? {
        b"OKAY" => Ok(()),
        b"FAIL" => anyhow::bail!("ADB: {}", host_payload(socket)?),
        _ => anyhow::bail!("Invalid response from the local ADB server."),
    }
}

pub(crate) fn devices() -> Result<Vec<(String, String)>> {
    devices_at(ADDRESS)
}

fn devices_at(address: SocketAddr) -> Result<Vec<(String, String)>> {
    let mut socket = server(address)?;
    request(&mut socket, "host:devices-l")?;
    Ok(host_payload(&mut socket)?
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let serial = fields.next()?;
            if fields.next()? != "device" || validate_serial(serial).is_err() {
                return None;
            }
            let label = fields
                .find_map(|value| value.strip_prefix("model:"))
                .unwrap_or(serial);
            Some((serial.into(), label.replace('_', " ")))
        })
        .collect())
}

fn sync_error(socket: &mut impl Read) -> io::Result<io::Error> {
    let len = u32::from_le_bytes(receive::<4>(socket)?) as usize;
    let data = payload(socket, len, MAX_CHUNK)?;
    Ok(io::Error::other(format!(
        "ADB: {}",
        String::from_utf8_lossy(&data)
    )))
}

fn metadata(record: &[u8; 68]) -> Result<Option<Metadata>> {
    let error = u32::from_le_bytes(record[0..4].try_into()?);
    match error {
        0 => {}
        2 => anyhow::bail!("Android file not found."),
        1 | 13 => anyhow::bail!("Permission denied by the Android device."),
        code => anyhow::bail!("Android file metadata error ({code})."),
    }
    let mode = match u32::from_le_bytes(record[20..24].try_into()?) & 0o170000 {
        0o040000 => EntryMode::DIR,
        0o100000 => EntryMode::FILE,
        _ => return Ok(None),
    };
    let size = u64::from_le_bytes(record[36..44].try_into()?);
    let mtime = i64::from_le_bytes(record[52..60].try_into()?);
    let mut result = Metadata::new(mode).with_content_length(size);
    if let Some(modified) = chrono::DateTime::from_timestamp(mtime, 0) {
        result.set_last_modified(modified);
    }
    Ok(Some(result))
}

impl Transport for Adb {
    fn stat(&self, path: &str) -> Result<Metadata> {
        let mut socket = self.sync(b"LST2", path)?;
        match &receive::<4>(&mut socket)? {
            b"LST2" => metadata(&receive::<68>(&mut socket)?)?
                .context("ADB browsing supports only regular files and folders."),
            b"FAIL" => Err(sync_error(&mut socket)?.into()),
            _ => anyhow::bail!("The Android device did not return file-sync v2 metadata."),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<oio::Entry>> {
        ensure!(
            self.stat(path)?.is_dir(),
            "This Android path is not a folder."
        );
        let mut socket = self.sync(b"LIS2", path)?;
        let mut entries = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(24);
        for _ in 0..100_000 {
            ensure!(
                Instant::now() < deadline,
                "Android folder listing timed out."
            );
            match &receive::<4>(&mut socket)? {
                b"DONE" => return Ok(entries),
                b"DNT2" => {
                    let record = receive::<68>(&mut socket)?;
                    let len = u32::from_le_bytes(receive::<4>(&mut socket)?) as usize;
                    let name = payload(&mut socket, len, 1024)?;
                    if let Ok(name) = std::str::from_utf8(&name)
                        && let Some(key) =
                            super::child_key(path, name, metadata(&record).ok().flatten())
                    {
                        entries.push(key);
                    }
                }
                b"FAIL" => return Err(sync_error(&mut socket)?.into()),
                _ => anyhow::bail!("The Android device did not return a file-sync v2 listing."),
            }
        }
        anyhow::bail!("Android folder exceeds the supported listing size.")
    }

    fn read(&self, path: &str, offset: u64) -> Result<Box<dyn Read + Send>> {
        ensure!(self.stat(path)?.is_file(), "Select a regular Android file.");
        let mut reader = Reader {
            socket: self.sync(b"RECV", path)?,
            remaining: 0,
            done: false,
        };
        let deadline = Instant::now() + Duration::from_secs(24);
        let mut remaining = offset;
        let mut buffer = [0; MAX_CHUNK];
        while remaining > 0 {
            ensure!(
                Instant::now() < deadline,
                "Android file offset took too long to reach."
            );
            let limit = remaining.min(MAX_CHUNK as u64) as usize;
            let read = reader.read(&mut buffer[..limit])?;
            if read == 0 {
                break;
            }
            remaining -= read as u64;
        }
        Ok(Box::new(reader))
    }
}

struct Reader {
    socket: TcpStream,
    remaining: usize,
    done: bool,
}

impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            match &receive::<4>(&mut self.socket)? {
                b"DONE" => {
                    receive::<4>(&mut self.socket)?;
                    self.done = true;
                    return Ok(0);
                }
                b"FAIL" => return Err(sync_error(&mut self.socket)?),
                b"DATA" => {
                    let size = u32::from_le_bytes(receive::<4>(&mut self.socket)?) as usize;
                    if !(1..=MAX_CHUNK).contains(&size) {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "Invalid ADB chunk size.",
                        ));
                    }
                    self.remaining = size;
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Invalid ADB file data.",
                    ));
                }
            }
        }
        let size = self.remaining.min(buf.len());
        self.socket.read_exact(&mut buf[..size])?;
        self.remaining -= size;
        Ok(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn host_command(socket: &mut TcpStream, expected: &str) {
        assert_eq!(host_payload(socket).unwrap(), expected);
        socket.write_all(b"OKAY").unwrap();
    }

    fn sync_command(listener: &TcpListener, id: &[u8; 4], path: &str) -> TcpStream {
        let (mut socket, _) = listener.accept().unwrap();
        socket.set_read_timeout(Some(TIMEOUT)).unwrap();
        host_command(&mut socket, "host:transport:emulator-5554");
        host_command(&mut socket, "sync:");
        assert_eq!(&receive::<4>(&mut socket).unwrap(), id);
        let len = u32::from_le_bytes(receive::<4>(&mut socket).unwrap()) as usize;
        assert_eq!(payload(&mut socket, len, 1024).unwrap(), path.as_bytes());
        socket
    }

    fn record(mode: u32, size: u64) -> [u8; 68] {
        let mut record = [0; 68];
        record[20..24].copy_from_slice(&mode.to_le_bytes());
        record[36..44].copy_from_slice(&size.to_le_bytes());
        record
    }

    #[test]
    fn discovers_authorized_devices_and_browses_sync_v2() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let fixture = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            host_command(&mut socket, "host:devices-l");
            let devices = "emulator-5554 device model:Test_Phone\nother unauthorized\n";
            write!(socket, "{:04x}{devices}", devices.len()).unwrap();
            drop(socket);

            let mut socket = sync_command(&listener, b"LST2", "/storage");
            socket.write_all(b"LST2").unwrap();
            socket.write_all(&record(0o040755, 0)).unwrap();
            let mut socket = sync_command(&listener, b"LIS2", "/storage");
            for (name, mode) in [("file.txt", 0o100644), ("..", 0o040755), ("link", 0o120777)] {
                socket.write_all(b"DNT2").unwrap();
                socket.write_all(&record(mode, 0x1_0000_0005)).unwrap();
                socket
                    .write_all(&(name.len() as u32).to_le_bytes())
                    .unwrap();
                socket.write_all(name.as_bytes()).unwrap();
            }
            socket.write_all(b"DONE").unwrap();

            let mut socket = sync_command(&listener, b"LST2", "/storage/file.txt");
            socket.write_all(b"LST2").unwrap();
            socket.write_all(&record(0o100644, 5)).unwrap();
            let mut socket = sync_command(&listener, b"RECV", "/storage/file.txt");
            socket
                .write_all(b"DATA\x02\0\0\0abDATA\x03\0\0\0cdeDONE\0\0\0\0")
                .unwrap();
        });
        assert_eq!(
            devices_at(address).unwrap(),
            [("emulator-5554".into(), "Test Phone".into())]
        );
        let device = Adb {
            serial: "emulator-5554".into(),
            root: "storage".into(),
            address,
        };
        let entries = device.list("/").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path(), "file.txt");
        assert_eq!(entries[0].mode(), EntryMode::FILE);
        assert_eq!(
            metadata(&record(0o100644, 0x1_0000_0005))
                .unwrap()
                .unwrap()
                .content_length(),
            0x1_0000_0005
        );
        let mut read = device.read("file.txt", 1).unwrap();
        let mut bytes = Vec::new();
        read.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"bcde");
        fixture.join().unwrap();
    }
}
