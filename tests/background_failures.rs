use mucommander_rust::background;
use std::sync::mpsc;

#[test]
fn idle_worker_is_pending_and_completion_is_delivered_once() {
    let (sender, receiver) = mpsc::channel();
    let mut pending = Some(receiver);
    assert!(background::poll::<usize>(&mut pending).is_none());
    assert!(pending.is_some());
    sender.send(Ok(42)).unwrap();
    assert_eq!(background::poll(&mut pending), Some(Ok(42)));
    assert!(pending.is_none());
    assert!(background::poll(&mut pending).is_none());
}

#[test]
fn panicked_worker_reports_failure_and_clears_busy_state() {
    let (sender, receiver) = mpsc::channel::<Result<(), String>>();
    let mut pending = Some(receiver);
    let worker = std::thread::spawn(move || {
        let _sender = sender;
        panic!("parser failed");
    });
    assert!(worker.join().is_err());
    assert!(
        background::poll(&mut pending)
            .unwrap()
            .unwrap_err()
            .contains("stopped unexpectedly")
    );
    assert!(pending.is_none());
}
