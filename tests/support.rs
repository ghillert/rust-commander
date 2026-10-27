use rust_commander::{filesystem, navigation, support};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
};

fn release(version: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"tag_name":version,"name":"Release","html_url":format!("{}/releases/tag/{version}",support::REPOSITORY),"draft":false,"prerelease":false})).unwrap()
}
#[test]
fn stable_release_comparison_and_untrusted_metadata_are_checked() {
    assert!(
        support::parse_release(&release("v0.8.0"), "0.7.0")
            .unwrap()
            .newer
    );
    assert!(
        support::parse_release(&release("v0.7.0"), "0.7.0-rc.1")
            .unwrap()
            .newer
    );
    assert!(
        !support::parse_release(&release("v0.7.0"), "0.7.0")
            .unwrap()
            .newer
    );
    assert!(
        !support::parse_release(&release("v0.6.0"), "0.7.0")
            .unwrap()
            .newer
    );
    assert!(
        !support::parse_release(&release("v0.7.0"), "0.7.0+build.42")
            .unwrap()
            .newer
    );
    for patch in [
        serde_json::json!({"html_url":"https://example.test/download"}),
        serde_json::json!({"prerelease":true}),
        serde_json::json!({"draft":true}),
        serde_json::json!({"tag_name":"latest"}),
    ] {
        let mut value: serde_json::Value = serde_json::from_slice(&release("v0.8.0")).unwrap();
        for (key, value_patch) in patch.as_object().unwrap() {
            value[key] = value_patch.clone();
        }
        assert!(support::parse_release(&serde_json::to_vec(&value).unwrap(), "0.7.0").is_err());
    }
    assert!(support::parse_release(&vec![b' '; 1024 * 1024 + 1], "0.7.0").is_err());
}
fn response(status: &str, body: Vec<u8>) -> (anyhow::Result<support::Update>, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/latest", listener.local_addr().unwrap());
    let status = status.to_owned();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") && socket.read(&mut byte).unwrap() > 0 {
            request.push(byte[0]);
        }
        let headers = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = socket.write_all(headers.as_bytes());
        let _ = socket.write_all(&body);
        String::from_utf8(request).unwrap()
    });
    let result = support::check_at(&endpoint, "0.7.0");
    (result, server.join().unwrap())
}
#[test]
fn update_requests_handle_no_release_rate_limits_errors_and_bounded_bodies() {
    let (update, request) = response("200 OK", release("v0.8.0"));
    assert!(update.unwrap().newer);
    assert!(request.starts_with("GET /latest "));
    assert!(
        request
            .to_lowercase()
            .contains("user-agent: rust-commander/")
    );
    assert!(!request.to_lowercase().contains("authorization:"));
    assert!(
        response("404 Not Found", Vec::new())
            .0
            .unwrap()
            .release
            .is_none()
    );
    for status in [
        "403 Forbidden",
        "429 Too Many Requests",
        "500 Internal Server Error",
        "302 Found",
    ] {
        assert!(response(status, Vec::new()).0.is_err(), "{status}");
    }
    assert!(response("200 OK", vec![b' '; 1024 * 1024 + 1]).0.is_err());
}
#[test]
fn diagnostics_and_bug_report_include_only_the_reviewable_platform_summary() {
    let summary = support::diagnostics();
    assert!(summary.contains(env!("CARGO_PKG_VERSION")));
    assert!(summary.contains(std::env::consts::OS));
    assert!(!summary.contains(&filesystem::home().display().to_string()));
    assert!(!summary.contains("password"));
    let url = url::Url::parse(&support::bug_report_url()).unwrap();
    assert_eq!(url.host_str(), Some("github.com"));
    assert_eq!(url.path(), "/ghillert/rust-commander/issues/new");
    let body = url
        .query_pairs()
        .find(|(key, _)| key == "body")
        .unwrap()
        .1
        .into_owned();
    assert!(body.contains(&summary));
    assert!(body.contains("Steps to reproduce"));
    assert!(!body.contains(&filesystem::home().display().to_string()));
}
#[test]
fn copying_base_names_preserves_directory_names_hidden_files_and_unicode() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("folder.with.dots")).unwrap();
    for (name, expected) in [
        ("café.tar.gz", "café.tar"),
        (".profile", ".profile"),
        ("plain", "plain"),
        ("file.txt", "file"),
    ] {
        let path = root.path().join(name);
        fs::write(&path, b"fixture").unwrap();
        assert_eq!(
            navigation::base_name(&filesystem::entry_from_path(&path).unwrap()),
            expected
        );
    }
    assert_eq!(
        navigation::base_name(
            &filesystem::entry_from_path(&root.path().join("folder.with.dots")).unwrap()
        ),
        "folder.with.dots"
    );
    let preferences: rust_commander::preferences::Preferences = serde_json::from_str("{}").unwrap();
    assert!(preferences.parent_entry);
    for index in 1..=10 {
        assert_eq!(
            preferences.shortcuts[&format!("RecallWindow{index}")],
            format!("Command+Alt+{}", index % 10)
        );
    }
}
