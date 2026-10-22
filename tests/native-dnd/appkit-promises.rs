#[cfg(target_os = "macos")]
fn main() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSFilePromiseProviderDelegate;
    use objc2_foundation::{NSError, NSString, NSURL};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let root = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let files = drag::PromisedFiles {
        internal_token: None,
        entries: vec![drag::PromiseEntry {
            path: "café.txt".into(),
            directory: false,
            size: Some(14),
        }],
        list: Arc::new(|| unreachable!()),
        read: Arc::new(|_| unreachable!()),
        write: Arc::new(move |index, path| {
            assert_eq!(index, 0);
            counter.fetch_add(1, Ordering::Relaxed);
            std::fs::write(path, b"deferred bytes").map_err(|e| e.to_string())
        }),
    };
    let provider = drag::file_promise_provider(&files, 0, false);
    let delegate = provider.delegate().unwrap();
    let marker = MainThreadMarker::new().unwrap();
    assert_eq!(
        delegate
            .filePromiseProvider_fileNameForType(
                &provider,
                &NSString::from_str("public.data"),
                marker
            )
            .to_string(),
        "café.txt"
    );
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "querying a native name must not fetch content"
    );
    let target = root.path().join("delivered.txt");
    let url = NSURL::fileURLWithPath(&NSString::from_str(target.to_str().unwrap()));
    let completed = Arc::new(AtomicUsize::new(0));
    let finished = completed.clone();
    let completion = block2::RcBlock::new(move |error: *mut NSError| {
        assert!(error.is_null());
        finished.fetch_add(1, Ordering::Relaxed);
    });
    delegate.filePromiseProvider_writePromiseToURL_completionHandler(&provider, &url, &completion);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(completed.load(Ordering::Relaxed), 1);
    assert_eq!(std::fs::read(target).unwrap(), b"deferred bytes");
    drop(provider);
    drop(delegate);
    let abandoned = drag::file_promise_provider(&files, 0, false);
    drop(abandoned);
    assert_eq!(
        calls.load(Ordering::Relaxed),
        1,
        "abandoning an offer must never fetch content"
    );
    println!(
        "AppKit native provider: metadata-only hover, deferred write, completion and cancellation passed"
    );
}
#[cfg(not(target_os = "macos"))]
fn main() {}
