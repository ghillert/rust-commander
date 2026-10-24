//! Cooperative transfer control and bounded retries of checkpointed operations.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    any::Any,
    fmt,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Zero means unlimited. This caps streamed processing, including staging.
    pub kib_per_second: u32,
    pub retries: u8,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            kib_per_second: 0,
            retries: 3,
        }
    }
}

#[derive(Default)]
struct State {
    pause: bool,
    acknowledged: bool,
    settings: Settings,
    deadline: Option<Instant>,
    retry: Option<(u8, Duration)>,
}
pub struct Control {
    state: Mutex<State>,
    changed: Condvar,
}
impl Control {
    pub fn new(settings: Settings) -> Self {
        Self {
            state: Mutex::new(State {
                settings,
                ..State::default()
            }),
            changed: Condvar::new(),
        }
    }
    pub fn toggle_pause(&self) {
        let mut state = self.state.lock().unwrap();
        state.pause = !state.pause;
        state.acknowledged = false;
        state.deadline = None;
        self.changed.notify_all();
    }
    pub fn settings(&self) -> Settings {
        self.state.lock().unwrap().settings
    }
    pub fn update(&self, settings: Settings) {
        let mut state = self.state.lock().unwrap();
        state.settings = settings;
        state.deadline = None;
        self.changed.notify_all();
    }
    pub fn paused(&self) -> bool {
        self.state.lock().unwrap().pause
    }
    pub fn status(&self) -> String {
        let state = self.state.lock().unwrap();
        if state.pause {
            if state.acknowledged {
                "Paused"
            } else {
                "Pausing…"
            }
            .into()
        } else if let Some((attempt, delay)) = state.retry {
            format!("Retry {attempt} after {}s", delay.as_secs())
        } else {
            String::new()
        }
    }
    pub fn chunk_size(&self) -> usize {
        match self.settings().kib_per_second {
            0 => 65_536,
            limit => ((u64::from(limit) * 1024 / 10).clamp(1, 65_536)) as usize,
        }
    }
    pub fn check(&self, cancel: &AtomicBool) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        loop {
            cancelled(cancel)?;
            if !state.pause {
                return Ok(());
            }
            state.acknowledged = true;
            state = self
                .changed
                .wait_timeout(state, Duration::from_millis(25))
                .unwrap()
                .0;
        }
    }
    /// Wait without accumulating credit across pause or a live limit change.
    pub fn pace(&self, bytes: u64, cancel: &AtomicBool) -> Result<()> {
        self.check(cancel)?;
        let mut state = self.state.lock().unwrap();
        let limit = state.settings.kib_per_second;
        if limit == 0 || bytes == 0 {
            return Ok(());
        }
        let now = Instant::now();
        let duration = Duration::from_secs_f64(bytes as f64 / (f64::from(limit) * 1024.0));
        let deadline = state.deadline.unwrap_or(now).max(now) + duration;
        state.deadline = Some(deadline);
        loop {
            cancelled(cancel)?;
            if state.pause {
                drop(state);
                return self.check(cancel);
            }
            if state.deadline != Some(deadline) || Instant::now() >= deadline {
                return Ok(());
            }
            state = self
                .changed
                .wait_timeout(
                    state,
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(25)),
                )
                .unwrap()
                .0;
        }
    }
    pub(crate) fn backoff(&self, attempt: u8, delay: Duration, cancel: &AtomicBool) -> Result<()> {
        self.state.lock().unwrap().retry = Some((attempt, delay));
        let result = (|| {
            let mut remaining = delay;
            while !remaining.is_zero() {
                self.check(cancel)?;
                if attempt > self.settings().retries.min(8) {
                    return Ok(());
                }
                let start = Instant::now();
                let state = self.state.lock().unwrap();
                if state.pause {
                    continue;
                }
                let _guard = self
                    .changed
                    .wait_timeout(state, remaining.min(Duration::from_millis(25)))
                    .unwrap();
                remaining = remaining.saturating_sub(start.elapsed());
            }
            cancelled(cancel)
        })();
        self.state.lock().unwrap().retry = None;
        result
    }
}
fn cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        bail!("Operation cancelled. Completed items were kept.");
    }
    Ok(())
}

/// Retain only a retry classification when sanitizing provider errors. The
/// original error may contain credentials and must never be a source chain.
#[derive(Debug)]
pub(crate) struct SanitizedError {
    pub message: String,
    pub temporary: bool,
}
impl fmt::Display for SanitizedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for SanitizedError {}

pub(crate) fn temporary_any(error: &dyn Any) -> bool {
    if let Some(e) = error.downcast_ref::<anyhow::Error>() {
        return temporary(e);
    }
    if let Some(e) = error.downcast_ref::<opendal::Error>() {
        return e.is_temporary();
    }
    if let Some(e) = error.downcast_ref::<std::io::Error>() {
        return temporary_io(e);
    }
    if let Some(e) = error.downcast_ref::<reqwest::Error>() {
        return temporary_http(e);
    }
    false
}
fn temporary_io(error: &std::io::Error) -> bool {
    use std::io::ErrorKind::*;
    matches!(
        error.kind(),
        TimedOut
            | ConnectionReset
            | ConnectionAborted
            | ConnectionRefused
            | BrokenPipe
            | Interrupted
            | WouldBlock
            | NotConnected
            | UnexpectedEof
    ) || error.get_ref().is_some_and(|source| {
        source
            .downcast_ref::<opendal::Error>()
            .is_some_and(|e| e.is_temporary())
            || source
                .downcast_ref::<SanitizedError>()
                .is_some_and(|e| e.temporary)
            || source
                .downcast_ref::<reqwest::Error>()
                .is_some_and(temporary_http)
    })
}
fn temporary_http(error: &reqwest::Error) -> bool {
    error.is_timeout()
        || error
            .status()
            .is_some_and(|s| s.as_u16() == 429 || s.is_server_error())
}
pub(crate) fn temporary(error: &anyhow::Error) -> bool {
    error.chain().any(|source| {
        source
            .downcast_ref::<SanitizedError>()
            .is_some_and(|e| e.temporary)
            || source
                .downcast_ref::<opendal::Error>()
                .is_some_and(|e| e.is_temporary())
            || source
                .downcast_ref::<std::io::Error>()
                .is_some_and(temporary_io)
            || source
                .downcast_ref::<reqwest::Error>()
                .is_some_and(temporary_http)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, mpsc};
    #[test]
    fn pause_acknowledges_and_cancel_wakes() {
        let control = Arc::new(Control::new(Settings::default()));
        control.toggle_pause();
        let cancel = Arc::new(AtomicBool::new(false));
        let (send, receive) = mpsc::channel();
        let (c, stop) = (control.clone(), cancel.clone());
        let worker = std::thread::spawn(move || send.send(c.check(&stop).is_err()).unwrap());
        assert!(receive.recv_timeout(Duration::from_millis(60)).is_err());
        assert_eq!(control.status(), "Paused");
        cancel.store(true, Ordering::Relaxed);
        assert!(receive.recv_timeout(Duration::from_secs(1)).unwrap());
        worker.join().unwrap();
    }
    #[test]
    fn live_limit_change_releases_wait_and_respects_cap() {
        let control = Arc::new(Control::new(Settings {
            kib_per_second: 1,
            retries: 0,
        }));
        assert_eq!(control.chunk_size(), 102);
        let start = Instant::now();
        control.pace(102, &AtomicBool::new(false)).unwrap();
        assert!(start.elapsed() >= Duration::from_millis(90));
        let (send, receive) = mpsc::channel();
        let c = control.clone();
        let worker = std::thread::spawn(move || {
            send.send(c.pace(1024, &AtomicBool::new(false)).is_ok())
                .unwrap()
        });
        assert!(receive.recv_timeout(Duration::from_millis(60)).is_err());
        control.update(Settings::default());
        assert!(receive.recv_timeout(Duration::from_secs(1)).unwrap());
        worker.join().unwrap();
    }
    #[test]
    fn only_typed_transient_errors_retry() {
        assert!(temporary(
            &std::io::Error::from(std::io::ErrorKind::TimedOut).into()
        ));
        assert!(!temporary(
            &std::io::Error::from(std::io::ErrorKind::PermissionDenied).into()
        ));
        assert!(!temporary(&anyhow::anyhow!(
            "connection reset; Permission denied"
        )));
        assert!(temporary(
            &opendal::Error::new(opendal::ErrorKind::Unexpected, "redacted")
                .set_temporary()
                .into()
        ));
    }
    #[test]
    fn sanitized_provider_error_keeps_classification_without_its_secret_source() {
        let secrets = crate::remote::Secrets {
            password: "private-password".into(),
            ..Default::default()
        };
        let original =
            opendal::Error::new(opendal::ErrorKind::Unexpected, "private-password").set_temporary();
        let error = crate::remote::clean_error(original, &secrets);
        assert!(temporary(&error));
        assert!(!format!("{error:#}").contains("private-password"));
        assert_eq!(error.chain().count(), 1);
    }
    #[test]
    fn retries_are_bounded_reset_progress_and_cancel_backoff() {
        let mut task = crate::operations::Task::default();
        task.control = Some(std::sync::Arc::new(Control::new(Settings {
            kib_per_second: 0,
            retries: 1,
        })));
        let mut calls = 0;
        let result: Result<()> = task.retry_checkpoint(|task| {
            calls += 1;
            if calls == 2 {
                assert_eq!(task.progress.bytes, 0);
                assert_eq!(task.progress.total_bytes, None);
            }
            task.plan_total(Some(100));
            task.tick(std::path::Path::new("partial"), 50)?;
            Err(std::io::Error::from(std::io::ErrorKind::TimedOut).into())
        });
        assert!(result.is_err());
        assert_eq!(calls, 2);
        calls = 0;
        let result: Result<()> = task.retry_checkpoint(|_| {
            calls += 1;
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).into())
        });
        assert!(result.is_err());
        assert_eq!(calls, 1);
        let control = task.control.clone().unwrap();
        let cancel = task.cancel.clone();
        let (send, receive) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result: Result<()> = task.retry_checkpoint(|_| {
                Err(std::io::Error::from(std::io::ErrorKind::TimedOut).into())
            });
            send.send(result.is_err()).unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(1);
        while control.status().is_empty() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(!control.status().is_empty());
        cancel.store(true, Ordering::Relaxed);
        assert!(receive.recv_timeout(Duration::from_secs(1)).unwrap());
        worker.join().unwrap();
    }
}
