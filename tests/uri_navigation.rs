use rust_commander::{
    filesystem::Location,
    remote::{self, Connection, FtpTls, Secrets},
    uri::{self, Input},
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};
fn target(text: &str) -> Box<uri::Target> {
    match uri::parse(text, &Location::Local(std::env::temp_dir())).unwrap() {
        Input::Remote(t) => t,
        _ => panic!("Expected remote URI"),
    }
}
#[test]
fn schemes_and_authorities() {
    for (uri, service, port, tls) in [
        (
            "https://sftp.kew.org/pub/data-repositories/WCVP/",
            "http",
            443,
            FtpTls::Automatic,
        ),
        ("http://example.test/data/", "http", 80, FtpTls::Automatic),
        (
            "sftp://user@[::1]:2222/caf%C3%A9/a%20b",
            "sftp",
            2222,
            FtpTls::Automatic,
        ),
        ("ftp://example.test/pub/", "ftp", 21, FtpTls::Plain),
        ("ftps://example.test/pub/", "ftp", 990, FtpTls::Implicit),
        (
            "ftpes://example.test:2121/pub/",
            "ftp",
            2121,
            FtpTls::Explicit,
        ),
        (
            "smb://example.test/share/folder/",
            "smb",
            445,
            FtpTls::Automatic,
        ),
    ] {
        let t = target(uri);
        assert_eq!(t.connection.service, service);
        assert!(!t.connection.listed);
        assert_eq!(
            url::Url::parse(&t.connection.endpoint)
                .unwrap()
                .port_or_known_default(),
            Some(port)
        );
        assert_eq!(t.connection.ftp_tls, tls);
        remote::register_profiles(std::slice::from_ref(&t.connection));
        let display = uri::address(&t.connection.id, &t.path);
        let roundtrip = target(&display);
        assert_eq!(roundtrip.path, t.path);
        assert_eq!(roundtrip.connection.service, service);
    }
    let smb = target("smb://nas/share/a/b");
    assert_eq!(smb.connection.bucket, "share");
    assert_eq!(smb.path, Path::new("a/b"));
    for scheme in ["s3", "gs"] {
        let t = target(&format!("{scheme}://bucket/path/to/file"));
        assert_eq!(t.connection.bucket, "bucket");
        assert_eq!(t.path, Path::new("path/to/file"));
    }
}
#[test]
fn secrets_never_persist_or_appear_in_addresses() {
    let t = target("https://alice:p%40ssword@example.test/a.pdf?X-Amz-Signature=private#fragment");
    assert_eq!(t.secrets.password, "p@ssword");
    assert_eq!(t.secrets.request_query, "X-Amz-Signature=private");
    remote::register_profiles(std::slice::from_ref(&t.connection));
    let address = uri::address(&t.connection.id, &t.path);
    assert!(!address.contains("private"));
    assert!(!address.contains("p%40"));
    assert!(!address.contains('#'));
    let profile = serde_json::to_string(&t.connection).unwrap();
    assert!(!profile.contains("private"));
    assert!(!profile.contains("p@ssword"));
    let serialized = serde_json::to_string(&t.secrets).unwrap();
    assert!(!serialized.contains("private"));
    assert!(!serialized.contains("request_query"));
    let error = uri::parse(
        "https://alice:very-secret@[broken/path?token=secret",
        &Location::Local("/".into()),
    )
    .err()
    .unwrap()
    .to_string();
    assert!(!error.contains("very-secret"));
    assert!(!error.contains("token"));
}
#[test]
fn local_legacy_unsupported_and_encoded_paths() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("hello world.txt");
    std::fs::write(&file, b"hello").unwrap();
    assert!(
        matches!(uri::parse(url::Url::from_file_path(&file).unwrap().as_str(),&Location::Local(root.path().into())).unwrap(),Input::Location(Location::Local(p)) if p==file)
    );
    assert!(
        matches!(uri::parse("relative",&Location::Local(root.path().into())).unwrap(),Input::Location(Location::Local(p)) if p==root.path().join("relative"))
    );
    assert!(
        matches!(uri::parse("remote://legacy/a%20b",&Location::Local(root.path().into())).unwrap(),Input::Location(Location::Remote{connection,path}) if connection=="legacy" && path==Path::new("a b"))
    );
    for text in [
        "data:text/plain,hello",
        "unsupported://host/a",
        "https:///",
        "sftp://host/a%2fb",
        "ftp://host/a?token=secret",
    ] {
        assert!(
            uri::parse(text, &Location::Local(root.path().into())).is_err(),
            "{text}"
        );
    }
    for name in [
        "space name.txt",
        "café.txt",
        "100% done.pdf",
        "a#b?.txt",
        "literal%20name.txt",
    ] {
        let t = target("https://example.test/");
        remote::register_profiles(std::slice::from_ref(&t.connection));
        let address = uri::address(&t.connection.id, Path::new(name));
        assert_eq!(target(&address).path, Path::new(name), "{address}");
    }
}
#[test]
fn legacy_preferences_and_profile_defaults() {
    let p: rust_commander::preferences::Preferences = serde_json::from_str("{}").unwrap();
    assert_eq!(
        p.location_bar,
        rust_commander::preferences::LocationBar::Address
    );
    let c: Connection =
        serde_json::from_str(r#"{"endpoint":"ftps://example.test","service":"ftp"}"#).unwrap();
    assert!(c.listed);
    assert_eq!(c.tls_mode(), FtpTls::Explicit);
    let mut updated = p;
    updated.location_bar = rust_commander::preferences::LocationBar::Breadcrumbs;
    let restored: rust_commander::preferences::Preferences =
        serde_json::from_str(&serde_json::to_string(&updated).unwrap()).unwrap();
    assert_eq!(restored.location_bar, updated.location_bar);
}

#[test]
fn matching_profiles_preserve_protocol_and_root() {
    let t = target("https://example.test/base/root/child");
    let c = Connection {
        service: "http".into(),
        endpoint: "https://example.test/base/".into(),
        root: "/root".into(),
        ..Default::default()
    };
    assert!(uri::matches_profile(&t, &c));
    remote::register_profiles(std::slice::from_ref(&c));
    assert_eq!(
        uri::address(&c.id, Path::new("child")),
        "https://example.test/base/root/child"
    );
    let insecure = Connection {
        endpoint: "http://example.test:443/base/".into(),
        ..c.clone()
    };
    assert!(!uri::matches_profile(&t, &insecure));
    assert!(!uri::matches_profile(
        &target("https://example.test/elsewhere"),
        &c
    ));
    let ftp = Connection {
        service: "ftp".into(),
        endpoint: "ftp://example.test:990".into(),
        ftp_tls: FtpTls::Implicit,
        ..Default::default()
    };
    remote::register_profiles(std::slice::from_ref(&ftp));
    assert!(uri::address(&ftp.id, Path::new("a")).starts_with("ftps://"));
}

#[cfg(windows)]
#[test]
fn windows_drive_and_unc_paths_remain_local_from_remote_panes() {
    let remote = Location::Remote {
        connection: "legacy".into(),
        path: "dir".into(),
    };
    for text in [r"C:\Users\Commander", r"\\server\share\folder"] {
        assert!(matches!(
            uri::parse(text, &remote).unwrap(),
            Input::Location(Location::Local(_))
        ));
    }
}

struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<String>>>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let worker_stop = stop.clone();
        let worker_requests = requests.clone();
        let thread = thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(std::time::Duration::from_millis(2));
                    continue;
                };
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut b = [0; 1];
                while stream.read(&mut b).unwrap_or(0) > 0 {
                    request.push(b[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&request);
                worker_requests.lock().unwrap().push(request.to_string());
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let head = request.starts_with("HEAD ");
                let (status,kind,body,extra)=match path {
                "/dir"=>("301 Moved Permanently","text/plain",String::new(),"Location: /dir/\r\n"),
                "/dir/"=>("200 OK","text/html","<title>Index of /dir/</title><a href='../'>Parent</a><a href='a%20b.pdf?sig=private'>PDF</a><a href='sub/'>Folder</a>".into(),""),
                "/dir/sub"|"/dir/sub/"=>("200 OK","text/html","<h1>Directory listing for /dir/sub/</h1>".into(),""),
                "/dir/a%20b.pdf?sig=private"|"/direct.bin"=>("200 OK","application/octet-stream","0123456789".into(),""),
                "/plain"=>("200 OK","text/html","<title>Welcome</title><a href='login'>login</a>".into(),""),
                "/large"=>("200 OK","text/html",String::new(),"Content-Length: 9000000\r\n"),
                "/auth"=>("401 Unauthorized","text/plain",String::new(),""),
                _=>("404 Not Found","text/plain",String::new(),""),
            };
                let length = if path == "/large" {
                    String::new()
                } else {
                    format!("Content-Length: {}\r\n", body.len())
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\n{length}{extra}Connection: close\r\n\r\n{}",
                    if head { "" } else { &body }
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Self {
            url,
            stop,
            requests,
            thread: Some(thread),
        }
    }
    fn connection(&self) -> Connection {
        Connection {
            service: "http".into(),
            endpoint: self.url.clone(),
            listed: false,
            ..Default::default()
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}
#[test]
fn http_index_download_direct_file_range_and_errors() {
    let server = Server::new();
    let c = server.connection();
    remote::connect(&c, Secrets::default(), false).unwrap();
    let entries = remote::list(&c.id, Path::new("dir")).unwrap();
    assert_eq!(entries.len(), 2);
    let pdf = entries.iter().find(|e| e.name == "a b.pdf").unwrap();
    assert!(!pdf.size_known);
    assert!(!pdf.is_dir);
    assert_eq!(
        remote::read_bytes(&c.id, &pdf.path, 100).unwrap(),
        b"0123456789"
    );
    let mut range = remote::read_range(&c.id, &pdf.path, 4).unwrap();
    let mut tail = Vec::new();
    range.read_to_end(&mut tail).unwrap();
    assert_eq!(tail, b"456789");
    assert_eq!(
        remote::list(&c.id, Path::new("direct.bin")).unwrap().len(),
        1
    );
    let directory = tempfile::tempdir().unwrap();
    remote::transfer(
        &Location::Remote {
            connection: c.id.clone(),
            path: "direct.bin".into(),
        },
        &remote::list(&c.id, Path::new("direct.bin")).unwrap(),
        &Location::Local(directory.path().into()),
        false,
        &mut rust_commander::operations::Task::default(),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(directory.path().join("direct.bin")).unwrap(),
        b"0123456789"
    );
    assert!(
        remote::list(&c.id, Path::new("plain"))
            .unwrap_err()
            .to_string()
            .contains("not a directory listing")
    );
    assert!(
        remote::list(&c.id, Path::new("large"))
            .unwrap_err()
            .to_string()
            .contains("8 MiB")
    );
    assert!(
        remote::list(&c.id, Path::new("auth"))
            .unwrap_err()
            .to_string()
            .contains("authentication")
    );
    assert!(
        remote::list(&c.id, Path::new("missing"))
            .unwrap_err()
            .to_string()
            .contains("not found")
    );
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.starts_with("GET ") || r.starts_with("HEAD "))
    );
}

#[test]
#[ignore = "Live read-only acceptance check; requires network access"]
fn kew_https_directory_smoke() {
    let t = target("https://sftp.kew.org/pub/data-repositories/WCVP/");
    remote::connect(&t.connection, t.secrets, false).unwrap();
    let entries = remote::list(&t.connection.id, &t.path).unwrap();
    assert!(entries.iter().any(|e| e.name == "Archive" && e.is_dir));
    assert!(
        entries
            .iter()
            .any(|e| e.name == "wcvp_dwca.zip" && !e.is_dir)
    );
}
