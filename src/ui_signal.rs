//! Thread-safe notifications from file-manager workers to the desktop event loop.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// Shared notifications from background workers to the event loop.
///
/// Clones share repaint requests, queued clipboard text, and close requests.
#[derive(Clone, Default)]
pub struct UiSignal {
    dirty: Arc<AtomicBool>,
    deadline: Arc<Mutex<Option<std::time::Instant>>>,
    clipboard: Arc<Mutex<Vec<String>>>,
    close: Arc<AtomicBool>,
    terminal_output: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl UiSignal {
    /// Marks the UI dirty so the event loop can repaint after a worker update.
    pub fn request_repaint(&self) {
        self.dirty.store(true, Ordering::Release);
    }
    /// Schedules a repaint after `delay`. When several requests are pending, the earliest
    /// deadline wins.
    pub fn request_repaint_after(&self, delay: std::time::Duration) {
        let when = std::time::Instant::now() + delay;
        let mut deadline = self.deadline.lock().unwrap_or_else(|e| e.into_inner());
        if deadline.is_none_or(|previous| when < previous) {
            *deadline = Some(when);
        }
    }
    pub(crate) fn take_repaint(&self) -> bool {
        let mut deadline = self.deadline.lock().unwrap_or_else(|e| e.into_inner());
        let due = deadline
            .take_if(|when| *when <= std::time::Instant::now())
            .is_some();
        self.dirty.swap(false, Ordering::Acquire) || due
    }
    pub(crate) fn on_terminal_output(&mut self, notify: impl Fn() + Send + Sync + 'static) {
        self.terminal_output = Some(Arc::new(notify));
    }
    pub(crate) fn terminal_output_ready(&self) {
        if let Some(notify) = &self.terminal_output {
            notify();
        }
    }
    /// Queues text for the UI thread to place on the clipboard and requests a repaint.
    pub fn copy_text(&self, text: String) {
        self.clipboard
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(text);
        self.request_repaint();
    }
    /// Signals the UI thread to close the window and requests a repaint so it can process
    /// the request.
    pub fn request_close(&self) {
        self.close.store(true, Ordering::Relaxed);
        self.request_repaint();
    }
    pub(crate) fn take_events(&self) -> (Vec<String>, bool) {
        let clipboard =
            std::mem::take(&mut *self.clipboard.lock().unwrap_or_else(|e| e.into_inner()));
        (clipboard, self.close.swap(false, Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn draining_ui_events_preserves_worker_wakeups_and_the_earliest_deadline() {
        let signal = UiSignal::default();
        assert!(!signal.take_repaint());
        signal.request_repaint_after(std::time::Duration::from_secs(60));
        assert!(!signal.take_repaint());
        signal.copy_text("ready".into());
        assert_eq!(signal.take_events().0, ["ready"]);
        assert!(signal.take_repaint());
        assert!(!signal.take_repaint());
        signal.request_repaint_after(std::time::Duration::ZERO);
        signal.request_repaint_after(std::time::Duration::from_secs(1));
        assert!(signal.take_repaint());
        assert!(!signal.take_repaint());
    }
}
