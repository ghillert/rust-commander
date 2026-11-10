//! The TCP record layer and bounded XDR primitives used by the NFSv3 client.
use anyhow::{Context, Result, ensure};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(8);
const MAX_RECORD: usize = 512 * 1024;

#[derive(Default)]
pub(super) struct Xdr(pub Vec<u8>);

impl Xdr {
    pub fn word(&mut self, value: u32) {
        self.0.extend(value.to_be_bytes());
    }

    pub fn wide(&mut self, value: u64) {
        self.0.extend(value.to_be_bytes());
    }

    pub fn opaque(&mut self, value: &[u8]) {
        self.word(value.len() as u32);
        self.0.extend(value);
        self.0.resize(self.0.len().next_multiple_of(4), 0);
    }
}

pub(super) struct Decoder<'a>(pub &'a [u8]);

impl<'a> Decoder<'a> {
    pub fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let (field, remaining) = self
            .0
            .split_at_checked(len)
            .context("Truncated RPC reply")?;
        self.0 = remaining;
        Ok(field)
    }

    pub fn word(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into()?))
    }

    pub fn wide(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into()?))
    }

    pub fn boolean(&mut self) -> Result<bool> {
        match self.word()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => anyhow::bail!("Invalid RPC boolean"),
        }
    }

    pub fn opaque(&mut self, max: usize) -> Result<&'a [u8]> {
        let size = self.word()? as usize;
        ensure!(size <= max, "Oversized RPC field");
        Ok(&self.take(size.next_multiple_of(4))?[..size])
    }
}

pub(super) struct Rpc {
    socket: TcpStream,
    xid: u32,
}

impl Rpc {
    pub fn connect(address: SocketAddr) -> Result<Self> {
        let socket = TcpStream::connect_timeout(&address, TIMEOUT)?;
        socket.set_write_timeout(Some(TIMEOUT))?;
        Ok(Self {
            socket,
            xid: rand::random(),
        })
    }

    pub fn call(
        &mut self,
        program: u32,
        version: u32,
        procedure: u32,
        credentials: &[u8],
        args: &Xdr,
    ) -> Result<Vec<u8>> {
        self.xid = self.xid.wrapping_add(1);
        let mut call = Xdr::default();
        for word in [self.xid, 0, 2, program, version, procedure] {
            call.word(word);
        }
        call.word(u32::from(!credentials.is_empty())); // AUTH_NONE or AUTH_SYS
        call.opaque(credentials);
        call.word(0); // AUTH_NONE verifier
        call.opaque(&[]);
        call.0.extend(&args.0);
        self.socket
            .write_all(&(0x8000_0000 | call.0.len() as u32).to_be_bytes())?;
        self.socket.write_all(&call.0)?;

        let deadline = Instant::now() + TIMEOUT;
        let mut reply = Vec::new();
        for _ in 0..1024 {
            let mut header = [0; 4];
            self.receive(&mut header, deadline)?;
            let header = u32::from_be_bytes(header);
            let size = (header & 0x7fff_ffff) as usize;
            let start = reply.len();
            ensure!(size <= MAX_RECORD - start, "Oversized RPC record");
            reply.resize(start + size, 0);
            self.receive(&mut reply[start..], deadline)?;
            if header & 0x8000_0000 != 0 {
                let mut data = Decoder(&reply);
                ensure!(
                    data.word()? == self.xid && data.word()? == 1,
                    "Invalid RPC reply"
                );
                ensure!(data.word()? == 0, "RPC server rejected the request");
                data.word()?; // Verifier flavor
                data.opaque(400)?;
                let status = data.word()?;
                ensure!(status == 0, "RPC procedure rejected ({status})");
                return Ok(data.0.to_vec());
            }
        }
        anyhow::bail!("RPC record has too many fragments")
    }

    fn receive(&mut self, mut data: &mut [u8], deadline: Instant) -> Result<()> {
        while !data.is_empty() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            ensure!(!remaining.is_zero(), "RPC response timed out");
            self.socket.set_read_timeout(Some(remaining))?;
            match self.socket.read(data) {
                Ok(0) => anyhow::bail!("Connection closed during RPC reply"),
                Ok(count) => data = &mut data[count..],
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}
