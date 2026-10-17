use mucommander_rust::remote::{self, Connection, Secrets};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

struct Server {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if worker_stop.load(Ordering::Relaxed) {
                    break;
                }
                let mut stream = stream.unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") && request.len() < 65536 {
                    if stream.read(&mut byte).unwrap_or(0) == 0 {
                        break;
                    }
                    request.push(byte[0]);
                }
                let header = String::from_utf8_lossy(&request);
                let length = header
                    .lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        if key.eq_ignore_ascii_case("content-length") {
                            value.trim().parse::<usize>().ok()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0);
                if length > 0 && length < 1024 * 1024 {
                    let mut body = vec![0; length];
                    let _ = stream.read_exact(&mut body);
                }
                let method = header.split_whitespace().next().unwrap_or("");
                let path = header.split_whitespace().nth(1).unwrap_or("/");
                let (status, body, kind) = if method == "PROPFIND" {
                    let response = |name: &str, directory: bool| {
                        format!(
                            "<d:response><d:href>{name}</d:href><d:propstat><d:prop><d:resourcetype>{}</d:resourcetype><d:getcontentlength>{}</d:getcontentlength><d:getlastmodified>Fri, 16 Oct 2026 00:00:00 GMT</d:getlastmodified></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>",
                            if directory { "<d:collection/>" } else { "" },
                            if directory { 0 } else { 5 }
                        )
                    };
                    let entries = if path.ends_with("hello.txt") {
                        response("/hello.txt", false)
                    } else {
                        response("/", true) + &response("/hello.txt", false)
                    };
                    (
                        "207 Multi-Status",
                        format!(
                            "<?xml version=\"1.0\"?><d:multistatus xmlns:d=\"DAV:\">{entries}</d:multistatus>"
                        ),
                        "application/xml",
                    )
                } else if method == "GET" {
                    ("200 OK", "hello".into(), "application/octet-stream")
                } else {
                    ("404 Not Found", String::new(), "text/plain")
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: {kind}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        Self {
            address,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[test]
fn webdav_lists_and_reads_over_real_http() {
    let server = Server::new();
    let connection = Connection {
        service: "webdav".into(),
        endpoint: format!("http://{}", server.address),
        ..Default::default()
    };
    remote::connect(&connection, Secrets::default(), false).unwrap();
    let entries = remote::list(&connection.id, Path::new("")).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "hello.txt");
    assert_eq!(
        remote::read_bytes(&connection.id, Path::new("hello.txt"), 32).unwrap(),
        b"hello"
    );
}
