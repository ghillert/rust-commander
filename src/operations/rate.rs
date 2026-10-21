//! A bounded recent byte rate; persisted progress never includes timing state.
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const WINDOW: Duration = Duration::from_secs(2);
const SAMPLE_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, Default)]
pub(super) struct TransferRate {
    samples: VecDeque<(Instant, u64)>,
    transferred: u64,
    last_byte: Option<Instant>,
}

impl TransferRate {
    pub fn record(&mut self, bytes: u64, now: Instant) {
        // Reset the initial baseline during setup. Previously resumed bytes are
        // assigned to Progress.bytes directly, so they never enter this count.
        if self.samples.is_empty() || (self.transferred == 0 && bytes == 0) {
            self.samples.clear();
            self.samples.push_back((now, 0));
        }
        self.transferred = self.transferred.saturating_add(bytes);
        if bytes > 0 {
            self.last_byte = Some(now);
        }
        while self.samples.len() > 1 && now.saturating_duration_since(self.samples[1].0) >= WINDOW {
            self.samples.pop_front();
        }
        if self
            .samples
            .back()
            .is_none_or(|(at, _)| now.saturating_duration_since(*at) >= SAMPLE_INTERVAL)
        {
            self.samples.push_back((now, self.transferred));
        }
    }

    pub fn per_second(&self, now: Instant) -> Option<u64> {
        let last_byte = self.last_byte?;
        if now.saturating_duration_since(last_byte) >= WINDOW {
            return Some(0);
        }
        // Retain the last sample before the window to avoid noisy jumps as
        // samples expire. Sampling is capped at 10 Hz, independent of chunk size.
        let &(at, before) = self
            .samples
            .iter()
            .rev()
            .find(|(at, _)| now.saturating_duration_since(*at) >= WINDOW)
            .or_else(|| self.samples.front())?;
        let elapsed = now
            .saturating_duration_since(at)
            .min(WINDOW)
            .max(SAMPLE_INTERVAL);
        Some(
            ((self.transferred.saturating_sub(before) as u128 * 1_000_000_000) / elapsed.as_nanos())
                .min(u64::MAX as u128) as u64,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_recent_rate_and_decays_to_zero_when_stalled() {
        let start = Instant::now();
        let mut rate = TransferRate::default();
        assert_eq!(rate.per_second(start), None);
        rate.record(0, start);
        for second in 1..=4 {
            rate.record(1024 * 1024, start + Duration::from_secs(second));
        }
        assert_eq!(
            rate.per_second(start + Duration::from_secs(4)),
            Some(1024 * 1024)
        );
        assert_eq!(
            rate.per_second(start + Duration::from_secs(5)),
            Some(512 * 1024)
        );
        assert_eq!(rate.per_second(start + Duration::from_secs(6)), Some(0));
        rate.record(256 * 1024, start + Duration::from_secs(7));
        assert_eq!(
            rate.per_second(start + Duration::from_secs(7)),
            Some(128 * 1024)
        );
    }

    #[test]
    fn checkpoint_offsets_are_not_counted_and_timing_is_not_persisted() {
        let start = Instant::now();
        let mut progress = crate::operations::Progress {
            bytes: 1024 * 1024 * 1024, // Restored checkpoint offset.
            ..Default::default()
        };
        progress.rate.record(0, start);
        progress.rate.record(4096, start + Duration::from_secs(1));
        assert_eq!(
            progress.rate.per_second(start + Duration::from_secs(1)),
            Some(4096)
        );
        let json = serde_json::to_value(&progress).unwrap();
        assert!(json.get("rate").is_none());
        let restored: crate::operations::Progress = serde_json::from_value(json).unwrap();
        assert_eq!(restored.bytes, progress.bytes);
        assert_eq!(restored.bytes_per_second(), None);
    }

    #[test]
    fn frequent_chunks_keep_a_bounded_sample_window() {
        let start = Instant::now();
        let mut rate = TransferRate::default();
        rate.record(0, start);
        for millisecond in 1..=10_000 {
            rate.record(1024, start + Duration::from_millis(millisecond));
        }
        assert!(rate.samples.len() <= 22);
        assert_eq!(
            rate.per_second(start + Duration::from_secs(10)),
            Some(1_024_000)
        );
    }

    #[test]
    fn initial_values_do_not_produce_infinite_rates_or_count_setup() {
        let start = Instant::now();
        let mut rate = TransferRate::default();
        rate.record(0, start);
        rate.record(0, start + Duration::from_secs(60));
        rate.record(1024, start + Duration::from_secs(61));
        assert_eq!(rate.per_second(start + Duration::from_secs(61)), Some(1024));
        let mut instant = TransferRate::default();
        instant.record(1, start);
        assert_eq!(instant.per_second(start), Some(10));
    }
}
