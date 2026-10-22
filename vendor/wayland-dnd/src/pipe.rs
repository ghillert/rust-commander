//! Bounded, deadline-limited file-list pipes. Never block the Wayland event queue.
use std::{
    io::{self, Read, Write},
    os::fd::AsRawFd,
    time::{Duration, Instant},
};
const LIMIT: usize = 16 * 1024 * 1024;
fn nonblocking(fd: i32) -> io::Result<()> {
    // This descriptor is owned by one transfer thread and closed when it returns.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
fn wait(fd: i32, event: i16, deadline: Instant) -> io::Result<()> {
    loop {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            return Err(io::ErrorKind::TimedOut.into());
        };
        let mut descriptor = libc::pollfd {
            fd,
            events: event,
            revents: 0,
        };
        let timeout = i32::try_from(left.as_millis()).unwrap_or(i32::MAX).max(1);
        let result = unsafe { libc::poll(&mut descriptor, 1, timeout) };
        if result > 0 {
            return Ok(());
        }
        if result == 0 {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}
pub fn receive(mut pipe: impl Read + AsRawFd) -> io::Result<Vec<u8>> {
    nonblocking(pipe.as_raw_fd())?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut data = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        wait(pipe.as_raw_fd(), libc::POLLIN, deadline)?;
        match pipe.read(&mut buffer) {
            Ok(0) => return Ok(data),
            Ok(n) => {
                if data.len() + n > LIMIT {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "File list exceeds 16 MiB",
                    ));
                }
                data.extend_from_slice(&buffer[..n]);
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(e) => return Err(e),
        }
    }
}
pub fn send(mut pipe: impl Write + AsRawFd, data: &[u8]) -> io::Result<()> {
    if data.len() > LIMIT {
        return Err(io::ErrorKind::InvalidData.into());
    }
    nonblocking(pipe.as_raw_fd())?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut offset = 0;
    while offset < data.len() {
        wait(pipe.as_raw_fd(), libc::POLLOUT, deadline)?;
        match pipe.write(&data[offset..]) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => offset += n,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    #[test]
    fn large_file_lists_stream_without_blocking_the_worker() {
        let (source, target) = UnixStream::pair().unwrap();
        let data = vec![b'x'; 128 * 1024];
        let expected = data.clone();
        let sender = std::thread::spawn(move || send(source, &data));
        assert_eq!(receive(target).unwrap(), expected);
        sender.join().unwrap().unwrap();
    }
    #[test]
    fn oversize_lists_and_closed_receivers_are_rejected() {
        let (source, target) = UnixStream::pair().unwrap();
        assert!(send(source, &vec![0; LIMIT + 1]).is_err());
        drop(target);
        let (source, target) = UnixStream::pair().unwrap();
        drop(target);
        assert!(send(source, b"x").is_err());
    }
}
