use crate::ui_signal::UiSignal;
use anyhow::Result;
use portable_pty::{CommandBuilder, MasterPty, PtySize};
use std::{
    io::{Read, Write},
    path::Path,
    sync::mpsc::{self, Receiver},
};
pub struct Terminal {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    receiver: Receiver<Vec<u8>>,
    parser: vt100::Parser,
    pub focused: bool,
    size: (u16, u16),
    pub(crate) status: String,
}
impl Terminal {
    pub fn new(path: &Path, ctx: &UiSignal) -> Result<Self> {
        let pair = portable_pty::native_pty_system().openpty(PtySize {
            rows: 12,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let shell = if cfg!(windows) {
            std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into())
        } else {
            std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into())
        };
        let mut command = CommandBuilder::new(shell);
        command.cwd(path);
        command.env("TERM", "xterm-256color");
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let (sender, receiver) = mpsc::sync_channel(64);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut buffer = [0; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if sender.send(buffer[..n].to_vec()).is_err() {
                            break;
                        }
                        ctx.request_repaint();
                    }
                }
            }
        });
        Ok(Self {
            master: pair.master,
            child,
            writer,
            receiver,
            parser: vt100::Parser::new(12, 100, 1000),
            focused: false,
            size: (12, 100),
            status: "Terminal".into(),
        })
    }
    pub(crate) fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }
    pub(crate) fn send(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes)?;
        Ok(())
    }
    pub(crate) fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        if self.size != (rows, cols) {
            self.master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })?;
            self.parser.set_size(rows, cols);
            self.size = (rows, cols);
        }
        Ok(())
    }
    pub fn poll(&mut self) {
        if let Ok(Some(status)) = self.child.try_wait() {
            self.status = format!("Shell exited: {status}");
        }
        for _ in 0..64 {
            match self.receiver.try_recv() {
                Ok(bytes) => self.parser.process(&bytes),
                Err(_) => break,
            }
        }
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}
