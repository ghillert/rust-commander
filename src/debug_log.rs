//! Bounded, process-local application events. Never record user input or credentials.
use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
    time::SystemTime,
};

#[derive(Clone, Debug)]
pub struct Event {
    pub sequence: u64,
    pub time: SystemTime,
    pub category: &'static str,
    pub message: String,
}
#[derive(Default)]
pub struct Buffer {
    sequence: u64,
    events: VecDeque<Event>,
}
impl Buffer {
    pub fn record(&mut self, category: &'static str, message: &str) {
        self.sequence += 1;
        self.events.push_back(Event {
            sequence: self.sequence,
            time: SystemTime::now(),
            category,
            message: message.chars().take(512).collect(),
        });
        while self.events.len() > 2000 {
            self.events.pop_front();
        }
    }
    pub fn clear(&mut self) {
        self.events.clear();
    }
    pub fn snapshot(&self) -> Vec<Event> {
        self.events.iter().cloned().collect()
    }
}
fn buffer() -> &'static Mutex<Buffer> {
    static LOG: OnceLock<Mutex<Buffer>> = OnceLock::new();
    LOG.get_or_init(Default::default)
}
pub fn record(category: &'static str, message: &str) {
    if let Ok(mut log) = buffer().lock() {
        log.record(category, message);
    }
}
pub fn clear() {
    if let Ok(mut log) = buffer().lock() {
        log.clear();
    }
}
pub fn snapshot() -> Vec<Event> {
    buffer()
        .lock()
        .map(|log| log.snapshot())
        .unwrap_or_default()
}
pub fn format(events: &[Event], filter: &str) -> String {
    let filter = filter.to_lowercase();
    events
        .iter()
        .filter_map(|event| {
            let time: chrono::DateTime<chrono::Utc> = event.time.into();
            let line = format!(
                "{} #{:05} [{}] {}",
                time.format("%H:%M:%S%.3f UTC"),
                event.sequence,
                event.category,
                event.message
            );
            line.to_lowercase().contains(&filter).then_some(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
}
