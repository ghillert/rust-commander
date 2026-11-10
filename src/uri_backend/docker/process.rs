use anyhow::{Context, Result, ensure};
use std::{
    io::{self, Cursor, Read},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    time::{Duration, Instant},
};

pub(super) type Cancel = Arc<AtomicBool>;

pub(super) fn check(cancel: &Cancel) -> Result<()> {
    ensure!(!cancel.load(Ordering::Relaxed), "Docker request cancelled.");
    Ok(())
}

pub(super) fn cli() -> Command {
    #[cfg(target_os = "macos")]
    {
        use std::path::PathBuf;
        // A Finder launch may omit the directories used by Docker's CLI installers.
        let from_path = std::env::var_os("PATH")
            .into_iter()
            .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>());
        if let Some(path) = from_path
            .chain([
                crate::filesystem::home().join(".docker/bin"),
                PathBuf::from("/usr/local/bin"),
                PathBuf::from("/opt/homebrew/bin"),
            ])
            .map(|path| path.join("docker"))
            .find(|path| {
                use std::os::unix::fs::PermissionsExt;
                path.is_absolute()
                    && path
                        .metadata()
                        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            })
        {
            return Command::new(path);
        }
    }
    Command::new("docker")
}

#[derive(Debug)]
pub(super) struct Failure {
    pub(super) code: Option<i32>,
    pub(super) message: String,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.message.is_empty() {
            write!(f, "Docker command failed ({:?})", self.code)
        } else {
            f.write_str(&self.message)
        }
    }
}

impl std::error::Error for Failure {}

pub(super) struct Pipe {
    child: Child,
    stdout: Receiver<io::Result<Vec<u8>>>,
    stderr: Receiver<String>,
    chunk: Cursor<Vec<u8>>,
    until: Instant,
    cancel: Cancel,
    finished: bool,
}

impl Pipe {
    pub(super) fn start(command: &mut Command, timeout: Duration, cancel: &Cancel) -> Result<Self> {
        check(cancel)?;
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("LC_ALL", "C")
            .spawn()
            .context(
                "Could not start the Docker CLI. Install Docker and make its CLI available.",
            )?;
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let (out, out_rx) = mpsc::sync_channel(2);
        let (err, err_rx) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut buffer = [0; 64 * 1024];
            loop {
                match stdout.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        if out.send(Ok(buffer[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = out.send(Err(e));
                        break;
                    }
                }
            }
        });
        std::thread::spawn(move || {
            let mut buffer = [0; 4096];
            let mut bytes = Vec::new();
            while let Ok(n @ 1..) = stderr.read(&mut buffer) {
                bytes.extend_from_slice(&buffer[..n.min(8192 - bytes.len())]);
            }
            let message = String::from_utf8_lossy(&bytes).trim().to_owned();
            let _ = err.send(message);
        });
        Ok(Self {
            child,
            stdout: out_rx,
            stderr: err_rx,
            chunk: Cursor::new(Vec::new()),
            until: Instant::now() + timeout,
            cancel: cancel.clone(),
            finished: false,
        })
    }

    fn remaining(&self) -> io::Result<Duration> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::other("Docker request cancelled."));
        }
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Docker request timed out.",
            ));
        }
        Ok(left.min(Duration::from_millis(40)))
    }

    fn finish(&mut self) -> io::Result<()> {
        loop {
            let left = self.remaining()?;
            if let Some(status) = self.child.try_wait()? {
                self.finished = true;
                if !status.success() {
                    return Err(io::Error::other(Failure {
                        code: status.code(),
                        message: self
                            .stderr
                            .recv_timeout(Duration::from_millis(200))
                            .unwrap_or_default(),
                    }));
                }
                return Ok(());
            }
            std::thread::sleep(left);
        }
    }
}

impl Read for Pipe {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.finished || buffer.is_empty() {
            return Ok(0);
        }
        loop {
            let left = self.remaining()?;
            let count = self.chunk.read(buffer)?;
            if count != 0 {
                return Ok(count);
            }
            match self.stdout.recv_timeout(left) {
                Ok(chunk) => self.chunk = Cursor::new(chunk?),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.finish()?;
                    return Ok(0);
                }
            }
        }
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub(super) fn capture(command: &mut Command, limit: usize, cancel: &Cancel) -> Result<Vec<u8>> {
    let pipe = Pipe::start(command, Duration::from_secs(15), cancel)?;
    let mut bytes = Vec::new();
    pipe.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "Docker response exceeds the supported size."
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_cli_has_a_docker_specific_error() {
        let work = tempfile::tempdir().unwrap();
        let error = Pipe::start(
            &mut Command::new(work.path().join("missing-docker")),
            Duration::from_secs(1),
            &Default::default(),
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("Could not start the Docker CLI"));
    }

    #[cfg(unix)]
    #[test]
    fn cancelling_a_blocked_reader_stops_the_child() {
        let cancel = Cancel::default();
        let mut pipe = Pipe::start(
            Command::new("/bin/sleep").arg("30"),
            Duration::from_secs(5),
            &cancel,
        )
        .unwrap();
        let id = pipe.child.id();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            cancel.store(true, Ordering::Relaxed);
        });
        let error = pipe.read(&mut [0; 1]).unwrap_err();
        assert!(error.to_string().contains("cancelled"));
        drop(pipe);
        // SAFETY: Signal zero only tests whether the spawned child still exists.
        assert_eq!(unsafe { libc::kill(id as libc::pid_t, 0) }, -1);
    }
}
