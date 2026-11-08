//! Thread-safe notifications from file-manager workers to the desktop event loop.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Default)]
pub struct UiSignal {
    dirty: Arc<AtomicBool>,
    clipboard: Arc<Mutex<Vec<String>>>,
    close: Arc<AtomicBool>,
    terminal_output: Option<Arc<dyn Fn() + Send + Sync>>,
}
impl UiSignal {
    pub fn request_repaint(&self) {
        self.dirty.store(true, Ordering::Relaxed);
    }
    pub fn request_repaint_after(&self, _delay: std::time::Duration) {
        self.request_repaint();
    }
    pub(crate) fn on_terminal_output(&mut self, notify: impl Fn() + Send + Sync + 'static) {
        self.terminal_output = Some(Arc::new(notify));
    }
    pub(crate) fn terminal_output_ready(&self) {
        if let Some(notify) = &self.terminal_output {
            notify();
        }
    }
    pub fn copy_text(&self, text: String) {
        self.clipboard
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(text);
        self.request_repaint();
    }
    pub fn request_close(&self) {
        self.close.store(true, Ordering::Relaxed);
        self.request_repaint();
    }
    pub(crate) fn take_events(&self) -> (Vec<String>, bool) {
        self.dirty.store(false, Ordering::Relaxed);
        let clipboard =
            std::mem::take(&mut *self.clipboard.lock().unwrap_or_else(|e| e.into_inner()));
        (clipboard, self.close.swap(false, Ordering::Relaxed))
    }
}
