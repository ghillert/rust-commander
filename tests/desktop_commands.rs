use rust_commander::{
    archive, debug_log, drag_drop::Selection, filesystem::Location, mail, operations::Task,
    preferences::Preferences, remote, support,
};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

fn settings() -> mail::Settings {
    mail::Settings {
        host: "smtp.example.test".into(),
        port: 587,
        sender: "Sender <sender@example.test>".into(),
        username: "sender".into(),
        ..Default::default()
    }
}
fn file() -> mail::File {
    mail::File {
        name: "café/report.bin".into(),
        bytes: vec![0, 1, 2, 128, 255],
    }
}

#[test]
fn mime_message_contains_reviewed_recipients_and_binary_attachments() {
    use base64::Engine;
    let file = file();
    let message = mail::message(
        &settings(),
        "a@example.test; b@example.test",
        "Report – café",
        "Hello 🙂",
        std::slice::from_ref(&file),
    )
    .unwrap();
    let bytes = message.formatted();
    let mime = String::from_utf8(bytes).unwrap();
    assert!(mime.contains("a@example.test") && mime.contains("b@example.test"));
    assert!(mime.contains("multipart/mixed") && mime.contains("application/octet-stream"));
    assert!(mime.contains("Content-Disposition: attachment"));
    assert!(mime.contains(&base64::engine::general_purpose::STANDARD.encode(file.bytes)));
    assert!(!mime.contains("filename=\"café/report.bin\""));
}
#[test]
fn email_validation_rejects_injection_empty_recipients_attachments_and_oversize() {
    let config = settings();
    for recipient in [
        "",
        "not an address",
        "a@example.test\r\nBcc: leak@example.test",
    ] {
        assert!(mail::message(&config, recipient, "Subject", "body", &[file()]).is_err());
    }
    assert!(
        mail::message(
            &config,
            "a@example.test",
            "Subject\r\nBcc: leak@example.test",
            "body",
            &[file()]
        )
        .is_err()
    );
    assert!(mail::message(&config, "a@example.test", "Subject", "body", &[]).is_err());
    assert!(
        mail::message(
            &config,
            "a@example.test",
            "Subject",
            "body",
            &[mail::File {
                name: "bad\nname".into(),
                bytes: vec![]
            }]
        )
        .is_err()
    );
    assert!(
        mail::message(
            &config,
            "a@example.test",
            "Subject",
            "body",
            &[mail::File {
                name: "big.bin".into(),
                bytes: vec![0; mail::ATTACHMENT_LIMIT + 1]
            }]
        )
        .is_err()
    );
    for host in [
        "https://smtp.test",
        "smtp.test:587",
        "user@smtp.test",
        "bad host",
        "",
    ] {
        let mut config = settings();
        config.host = host.into();
        assert!(config.validate().is_err());
    }
}
#[test]
fn smtp_settings_and_legacy_preferences_never_serialize_a_password() {
    let mut preferences: Preferences = serde_json::from_str("{}").unwrap();
    preferences.mail=serde_json::from_value(serde_json::json!({"host":"smtp.example.test","port":587,"sender":"sender@example.test","username":"sender","password":"never-save-this"})).unwrap();
    let saved = serde_json::to_string(&preferences).unwrap();
    assert!(!saved.contains("never-save-this") && !saved.contains("password"));
    assert_eq!(
        serde_json::from_str::<Preferences>(&saved)
            .unwrap()
            .mail
            .host,
        "smtp.example.test"
    );
}
#[test]
fn email_preparation_flattens_directories_without_changing_sources_and_obeys_limits() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("folder");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("a.txt"), b"a").unwrap();
    fs::write(folder.join("b.bin"), [0, 1, 255]).unwrap();
    let selection = Selection::local(std::slice::from_ref(&folder)).unwrap();
    let prepared = mail::prepare(&selection, &Task::default()).unwrap();
    assert_eq!(
        prepared
            .files
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        vec!["folder/a.txt", "folder/b.bin"]
    );
    assert_eq!(prepared.files[1].bytes, [0, 1, 255]);
    assert_eq!(fs::read(folder.join("a.txt")).unwrap(), b"a");
    let task = Task::new(Arc::new(AtomicBool::new(true)), |_| {});
    assert!(mail::prepare(&selection, &task).is_err());
    let big = root.path().join("big.bin");
    fs::File::create(&big)
        .unwrap()
        .set_len((mail::ATTACHMENT_LIMIT + 1) as u64)
        .unwrap();
    let mut selection = Selection::local(&[big]).unwrap();
    selection.entries[0].size_known = false;
    assert!(mail::prepare(&selection, &Task::default()).is_err());
    #[cfg(unix)]
    {
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&folder, &link).unwrap();
        assert!(mail::prepare(&Selection::local(&[link]).unwrap(), &Task::default()).is_err());
    }
}
#[test]
fn remote_and_archive_attachment_bytes_are_prepared_for_review() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("a.txt");
    fs::write(&file, b"attachment").unwrap();
    let archive = root.path().join("files.zip");
    archive::pack(&[file], &archive, &mut Task::default()).unwrap();
    let archive_location = Location::Archive {
        file: archive,
        prefix: "".into(),
    };
    let entries = rust_commander::filesystem::list(&archive_location).unwrap();
    let prepared = mail::prepare(
        &Selection {
            from: archive_location,
            entries,
        },
        &Task::default(),
    )
    .unwrap();
    assert_eq!(prepared.files[0].bytes, b"attachment");
    let connection = remote::Connection {
        service: "fs".into(),
        root: root.path().display().to_string(),
        ..Default::default()
    };
    remote::connect(&connection, Default::default(), false).unwrap();
    let entries = remote::list(&connection.id, Path::new(""))
        .unwrap()
        .into_iter()
        .filter(|e| e.name == "a.txt")
        .collect();
    let prepared = mail::prepare(
        &Selection {
            from: Location::Remote {
                connection: connection.id,
                path: "".into(),
            },
            entries,
        },
        &Task::default(),
    )
    .unwrap();
    assert_eq!(prepared.files[0].bytes, b"attachment");
}
#[test]
fn smtp_refuses_downgrade_without_sending_credentials_or_message_data() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        stream.write_all(b"220 localhost test SMTP\r\n").unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(line.starts_with("EHLO "), "{line}");
        stream
            .write_all(b"250 localhost (no STARTTLS)\r\n")
            .unwrap();
        let mut tail = String::new();
        let _ = reader.read_to_string(&mut tail);
        format!("{line}{tail}")
    });
    let mut config = settings();
    config.host = "127.0.0.1".into();
    config.port = port;
    let message = mail::message(
        &config,
        "a@example.test",
        "private subject",
        "private body",
        &[file()],
    )
    .unwrap();
    let error = mail::send(&config, "private password".into(), message)
        .unwrap_err()
        .to_string();
    let received = worker.join().unwrap();
    for forbidden in [
        "AUTH ",
        "MAIL FROM",
        "RCPT TO",
        "DATA",
        "private password",
        "private body",
    ] {
        assert!(!received.contains(forbidden), "{received}");
    }
    assert!(!error.contains("private password") && !error.contains("private body"));
}
#[test]
fn console_buffer_is_bounded_filterable_and_clearable() {
    let mut buffer = debug_log::Buffer::default();
    for i in 0..2100 {
        buffer.record("command", &format!("Command {i}"));
    }
    let events = buffer.snapshot();
    assert_eq!(events.len(), 2000);
    assert_eq!(events[0].sequence, 101);
    let filtered = debug_log::format(&events, "command 2099");
    assert!(filtered.contains("Command 2099"));
    assert_eq!(filtered.lines().count(), 1);
    buffer.clear();
    assert!(buffer.snapshot().is_empty());
    buffer.record("operation", "Completed");
    assert_eq!(buffer.snapshot()[0].sequence, 2101);
}
#[test]
fn support_links_accept_only_https_without_embedded_credentials() {
    for link in [
        support::DOCUMENTATION,
        support::COMMUNITY,
        support::REPOSITORY,
        support::DONATION,
        "https://example.test/donate?project=rust-commander",
    ] {
        assert!(support::web_link(link).is_ok());
    }
    for link in [
        "javascript:alert(1)",
        "file:///tmp/report",
        "http://example.test",
        "https://user:password@example.test",
        "",
    ] {
        assert!(support::web_link(link).is_err());
    }
}
