use std::sync::mpsc::{Receiver, TryRecvError};

/// A disconnected worker is a completed failure, rather than a permanent busy
/// state. This also handles panics in third-party file parsers and adapters.
pub fn poll<T>(pending: &mut Option<Receiver<Result<T, String>>>) -> Option<Result<T, String>> {
    let result = match pending.as_ref()?.try_recv() {
        Ok(result) => result,
        Err(TryRecvError::Empty) => return None,
        Err(TryRecvError::Disconnected) => Err(
            "The background task stopped unexpectedly. The operation did not complete; review any output before retrying.".into(),
        ),
    };
    *pending = None;
    Some(result)
}
