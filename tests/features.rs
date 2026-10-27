use rust_commander::{
    archive,
    archive_tools::{self, ZipEdit},
    filesystem::{self, Location},
    history,
    i18n::{self, Language},
    native_files, oauth,
    operations::{Operation, Task},
    preferences::{self, Column, ColumnSpec, Preferences},
    remote::{self, Connection, Secrets},
    transfer::{self, ConflictPolicy, TransferOptions},
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    (root, source, destination)
}

#[test]
fn undo_move_restores_source_and_removes_verified_destination() {
    let (root, source, destination) = fixture();
    let file = source.join("data.txt");
    fs::write(&file, "content").unwrap();
    let record = history::execute_in(
        Operation::Move {
            sources: vec![file.clone()],
            destination: destination.clone(),
        },
        &mut Task::default(),
        &root.path().join("journal"),
    )
    .unwrap();
    assert!(record.success);
    assert!(!file.exists());
    assert_eq!(record.changes.len(), 2);
    history::undo(&record, &mut Task::default()).unwrap();
    assert_eq!(fs::read_to_string(file).unwrap(), "content");
    assert!(!destination.join("data.txt").exists());
}

#[test]
fn undo_rename_restores_original_name() {
    let (root, source, _) = fixture();
    let file = source.join("before");
    fs::write(&file, "content").unwrap();
    let record = history::execute_in(
        Operation::Rename {
            source: file.clone(),
            name: "after".into(),
        },
        &mut Task::default(),
        &root.path().join("journal"),
    )
    .unwrap();
    assert!(record.success);
    history::undo(&record, &mut Task::default()).unwrap();
    assert!(file.exists());
    assert!(!source.join("after").exists());
}

#[test]
fn undo_replacement_restores_previous_contents() {
    let (root, source, destination) = fixture();
    fs::write(source.join("data"), "new").unwrap();
    fs::write(destination.join("data"), "old").unwrap();
    let record = history::execute_in(
        Operation::Transfer {
            sources: vec![source.join("data")],
            destination: destination.clone(),
            moving: false,
            options: TransferOptions {
                conflicts: ConflictPolicy::Replace,
                merge_folders: false,
            },
        },
        &mut Task::default(),
        &root.path().join("journal"),
    )
    .unwrap();
    assert!(record.success);
    assert_eq!(fs::read_to_string(destination.join("data")).unwrap(), "new");
    history::undo(&record, &mut Task::default()).unwrap();
    assert_eq!(fs::read_to_string(destination.join("data")).unwrap(), "old");
    assert!(source.join("data").exists());
}

#[test]
fn undo_refuses_external_edits_even_with_original_size_and_timestamp() {
    let (root, source, destination) = fixture();
    fs::write(source.join("data"), "original").unwrap();
    let record = history::execute_in(
        Operation::Copy {
            sources: vec![source.join("data")],
            destination: destination.clone(),
        },
        &mut Task::default(),
        &root.path().join("journal"),
    )
    .unwrap();
    let path = destination.join("data");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, "external").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    assert!(history::undo(&record, &mut Task::default()).is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), "external");
}

#[test]
fn undo_refuses_modified_recovery_copy() {
    let (root, source, _) = fixture();
    fs::write(source.join("data"), "original").unwrap();
    let record = history::execute_in(
        Operation::Rename {
            source: source.join("data"),
            name: "renamed".into(),
        },
        &mut Task::default(),
        &root.path().join("journal"),
    )
    .unwrap();
    let backup = record
        .changes
        .iter()
        .find_map(|change| change.backup.as_ref())
        .unwrap();
    fs::write(backup, "tampered").unwrap();
    assert!(history::undo(&record, &mut Task::default()).is_err());
    assert!(source.join("renamed").exists());
    assert!(!source.join("data").exists());
}

#[test]
fn per_file_choices_can_skip_and_replace_in_the_same_batch() {
    let (_root, source, destination) = fixture();
    for name in ["a", "b"] {
        fs::write(source.join(name), "new").unwrap();
        fs::write(destination.join(name), "old").unwrap();
    }
    let overrides = BTreeMap::from([
        (destination.join("a"), ConflictPolicy::Skip),
        (destination.join("b"), ConflictPolicy::Replace),
    ]);
    transfer::transfer_resolved(
        &[source.join("a"), source.join("b")],
        &destination,
        false,
        TransferOptions::default(),
        &overrides,
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(fs::read_to_string(destination.join("a")).unwrap(), "old");
    assert_eq!(fs::read_to_string(destination.join("b")).unwrap(), "new");
}

#[test]
fn conflict_preview_reports_sizes_and_nested_paths() {
    let (_root, source, destination) = fixture();
    for base in [&source, &destination] {
        fs::create_dir(base.join("folder")).unwrap();
    }
    fs::write(source.join("folder/a"), "newer").unwrap();
    fs::write(destination.join("folder/a"), "old").unwrap();
    let conflicts = transfer::conflicts(&[source.join("folder")], &destination, true).unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].source_size, 5);
    assert_eq!(conflicts[0].target_size, 3);
    assert_eq!(conflicts[0].target, destination.join("folder/a"));
}

#[test]
fn zip_edit_add_rename_and_remove_preserves_other_entries() {
    let (root, source, destination) = fixture();
    fs::write(source.join("first"), "one").unwrap();
    let zip = destination.join("editable.zip");
    archive::pack(&[source.join("first")], &zip, &mut Task::default()).unwrap();
    fs::write(root.path().join("second"), "two").unwrap();
    archive_tools::edit_zip(
        &zip,
        &ZipEdit::Add {
            sources: vec![root.path().join("second")],
            prefix: "nested".into(),
        },
        &mut Task::default(),
    )
    .unwrap();
    archive_tools::edit_zip(
        &zip,
        &ZipEdit::Rename {
            source: "first".into(),
            target: "renamed".into(),
        },
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(
        archive::read_bytes(&zip, Path::new("renamed"), 100).unwrap(),
        b"one"
    );
    assert_eq!(
        archive::read_bytes(&zip, Path::new("nested/second"), 100).unwrap(),
        b"two"
    );
    archive_tools::edit_zip(
        &zip,
        &ZipEdit::Remove(vec!["nested".into()]),
        &mut Task::default(),
    )
    .unwrap();
    assert!(archive::read_bytes(&zip, Path::new("nested/second"), 100).is_err());
    assert_eq!(archive::list(&zip, Path::new("")).unwrap().len(), 1);
}

#[test]
fn invalid_zip_edits_leave_original_archive_unchanged() {
    let (_root, source, destination) = fixture();
    fs::write(source.join("first"), "one").unwrap();
    fs::write(source.join("second"), "two").unwrap();
    let zip = destination.join("editable.zip");
    archive::pack(
        &[source.join("first"), source.join("second")],
        &zip,
        &mut Task::default(),
    )
    .unwrap();
    let original = fs::read(&zip).unwrap();
    for edit in [
        ZipEdit::Rename {
            source: "first".into(),
            target: "../escaped".into(),
        },
        ZipEdit::Rename {
            source: "first".into(),
            target: "second".into(),
        },
        ZipEdit::Add {
            sources: vec![destination.clone()],
            prefix: PathBuf::new(),
        },
    ] {
        assert!(archive_tools::edit_zip(&zip, &edit, &mut Task::default()).is_err());
        assert_eq!(fs::read(&zip).unwrap(), original);
    }
}

#[test]
fn encrypted_zip_and_sevenz_round_trip_and_reject_wrong_passwords() {
    let (_root, source, destination) = fixture();
    fs::write(source.join("secret.txt"), "confidential").unwrap();
    for extension in ["zip", "7z"] {
        let file = destination.join(format!("secret.{extension}"));
        archive::pack_with_password(
            &[source.join("secret.txt")],
            &file,
            "correct-password",
            &mut Task::default(),
        )
        .unwrap();
        assert!(archive::list(&file, Path::new("")).is_err());
        assert!(archive::unlock(&file, "wrong-password").is_err());
        archive::unlock(&file, "correct-password").unwrap();
        let extracted = destination.join(format!("extracted-{extension}"));
        archive::extract(&file, &[], Path::new(""), &extracted, &mut Task::default()).unwrap();
        assert_eq!(
            fs::read_to_string(extracted.join("secret.txt")).unwrap(),
            "confidential"
        );
    }
}

#[test]
fn zip_edit_does_not_add_plaintext_files_to_encrypted_archive() {
    let (_root, source, destination) = fixture();
    fs::write(source.join("secret"), "secret").unwrap();
    let file = destination.join("encrypted.zip");
    archive::pack_with_password(
        &[source.join("secret")],
        &file,
        "password",
        &mut Task::default(),
    )
    .unwrap();
    let original = fs::read(&file).unwrap();
    assert!(
        archive_tools::edit_zip(
            &file,
            &ZipEdit::Add {
                sources: vec![source.join("secret")],
                prefix: "extra".into()
            },
            &mut Task::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(file).unwrap(), original);
}

#[test]
fn nested_archive_navigation_returns_to_the_containing_archive() {
    let (_root, source, destination) = fixture();
    fs::write(source.join("data"), "nested content").unwrap();
    let inner = destination.join("inner.zip");
    archive::pack(&[source.join("data")], &inner, &mut Task::default()).unwrap();
    let outer = source.join("outer.zip");
    archive::pack(&[inner], &outer, &mut Task::default()).unwrap();
    let location = Location::Archive {
        file: outer.clone(),
        prefix: PathBuf::new(),
    };
    let materialized = archive_tools::materialize(&location, Path::new("inner.zip")).unwrap();
    assert_eq!(
        archive::read_bytes(&materialized, Path::new("data"), 100).unwrap(),
        b"nested content"
    );
    assert_eq!(
        Location::Archive {
            file: materialized,
            prefix: PathBuf::new()
        }
        .parent(),
        Some(location)
    );
}

fn remote_fixture(root: &Path) -> Connection {
    let connection = Connection {
        service: "fs".into(),
        root: root.display().to_string(),
        ..Default::default()
    };
    remote::connect(&connection, Secrets::default(), false).unwrap();
    connection
}

#[test]
fn remote_rename_preserves_contents_and_refuses_existing_targets() {
    let (_root, source, _) = fixture();
    fs::write(source.join("first"), "content").unwrap();
    fs::write(source.join("existing"), "other").unwrap();
    let connection = remote_fixture(&source);
    assert!(remote::rename(&connection.id, Path::new("first"), "existing").is_err());
    remote::rename(&connection.id, Path::new("first"), "renamed").unwrap();
    assert_eq!(
        fs::read_to_string(source.join("renamed")).unwrap(),
        "content"
    );
    assert!(!source.join("first").exists());
}

#[test]
fn remote_conflict_policies_keep_both_skip_and_retain_replacement_backup() {
    let (_root, source, destination) = fixture();
    fs::write(source.join("file.txt"), "new").unwrap();
    fs::write(destination.join("file.txt"), "old").unwrap();
    let connection = remote_fixture(&destination);
    let from = Location::Local(source.clone());
    let to = Location::Remote {
        connection: connection.id.clone(),
        path: PathBuf::new(),
    };
    let entries = vec![filesystem::entry_from_path(&source.join("file.txt")).unwrap()];
    for policy in [
        ConflictPolicy::Skip,
        ConflictPolicy::Rename,
        ConflictPolicy::Replace,
    ] {
        remote::transfer_resolved(
            &from,
            &entries,
            &to,
            false,
            TransferOptions {
                conflicts: policy,
                merge_folders: false,
            },
            &BTreeMap::new(),
            &mut Task::default(),
        )
        .unwrap();
        if policy == ConflictPolicy::Skip {
            assert_eq!(
                fs::read_to_string(destination.join("file.txt")).unwrap(),
                "old"
            );
        }
    }
    assert_eq!(
        fs::read_to_string(destination.join("file (1).txt")).unwrap(),
        "new"
    );
    assert_eq!(
        fs::read_to_string(destination.join("file.txt")).unwrap(),
        "new"
    );
    assert!(fs::read_dir(destination).unwrap().flatten().any(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .starts_with(".rust-commander-replaced-")
            && fs::read_to_string(entry.path()).unwrap() == "old"
    }));
}

#[test]
fn remote_archives_materialize_and_save_edits_with_recovery() {
    let (_root, source, destination) = fixture();
    fs::write(source.join("data"), "remote archive").unwrap();
    archive::pack(
        &[source.join("data")],
        &destination.join("remote.zip"),
        &mut Task::default(),
    )
    .unwrap();
    let connection = remote_fixture(&destination);
    let location = Location::Remote {
        connection: connection.id,
        path: PathBuf::new(),
    };
    let materialized = archive_tools::materialize(&location, Path::new("remote.zip")).unwrap();
    assert_eq!(
        archive::read_bytes(&materialized, Path::new("data"), 100).unwrap(),
        b"remote archive"
    );
    let original = fs::read(destination.join("remote.zip")).unwrap();
    archive_tools::edit_zip(
        &materialized,
        &ZipEdit::Remove(vec!["data".into()]),
        &mut Task::default(),
    )
    .unwrap();
    assert!(
        archive::list(&destination.join("remote.zip"), Path::new(""))
            .unwrap()
            .is_empty()
    );
    let backup = fs::read_dir(&destination)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".rust-commander-edit-")
        })
        .unwrap();
    assert_eq!(fs::read(backup).unwrap(), original);
}

#[test]
fn oauth_pkce_matches_rfc7636_example() {
    assert_eq!(
        oauth::challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}

#[test]
fn oauth_urls_use_pkce_and_offline_access_for_supported_clouds() {
    for service in ["gdrive", "dropbox", "onedrive"] {
        let connection = Connection {
            service: service.into(),
            oauth_client_id: "public-client-id".into(),
            ..Default::default()
        };
        let provider = oauth::provider(&connection).unwrap();
        let url = oauth::authorization_url(
            &connection,
            &provider,
            "http://127.0.0.1:53682/callback",
            "nonce",
            "verifier",
        )
        .unwrap();
        let query = url.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(query.get("code_challenge_method").unwrap(), "S256");
        assert_eq!(query.get("state").unwrap(), "nonce");
        assert!(!url.as_str().contains("client_secret"));
    }
}

#[test]
fn oauth_callback_rejects_state_mismatch_duplicates_and_wrong_paths() {
    assert_eq!(
        oauth::parse_callback("/callback?state=nonce&code=authorization-code", "nonce").unwrap(),
        "authorization-code"
    );
    for target in [
        "/callback?state=wrong&code=code",
        "/callback?state=nonce&state=nonce&code=code",
        "/other?state=nonce&code=code",
        "/callback?state=nonce&error=access_denied",
        "/callback?state=nonce&code=",
    ] {
        assert!(oauth::parse_callback(target, "nonce").is_err());
    }
}

#[test]
fn oauth_secrets_are_compatible_with_old_keychain_json_and_not_profiles() {
    let old: Secrets = serde_json::from_str(r#"{"access_token":"old-token"}"#).unwrap();
    assert_eq!(old.access_token, "old-token");
    assert!(old.refresh_token.is_empty());
    let profile = serde_json::to_string(&Connection::default()).unwrap();
    assert!(!profile.contains("refresh_token"));
    assert!(!profile.contains("oauth_client_secret"));
}

#[test]
fn clipboard_file_urls_decode_spaces_unicode_and_reject_remote_hosts() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("two words ü.txt");
    let uri = url::Url::from_file_path(&path).unwrap().to_string();
    assert_eq!(
        native_files::decode_paths(&[uri.clone(), uri]).unwrap(),
        vec![path]
    );
    assert!(native_files::decode_paths(&["file://untrusted-host/shared/file".into()]).is_err());
    assert!(native_files::decode_paths(&["relative/file".into()]).is_err());
}

#[test]
fn custom_columns_validate_and_survive_settings_round_trip() {
    let mut preferences = Preferences::default();
    preferences.columns.push(ColumnSpec {
        kind: Column::Extension,
        width: 120.0,
    });
    preferences::validate(&preferences).unwrap();
    let restored: Preferences =
        serde_json::from_str(&serde_json::to_string(&preferences).unwrap()).unwrap();
    assert_eq!(restored.columns.last().unwrap().kind, Column::Extension);
    preferences.columns.push(ColumnSpec {
        kind: Column::Extension,
        width: 100.0,
    });
    assert!(preferences::validate(&preferences).is_err());
}

#[test]
fn translated_labels_and_preferences_round_trip() {
    assert_eq!(i18n::translate(Language::German, "File"), "Datei");
    assert_eq!(i18n::translate(Language::Spanish, "Copy  F5"), "Copiar  F5");
    assert_eq!(
        i18n::translate(Language::German, "/Users/example"),
        "/Users/example"
    );
    let preferences = Preferences {
        language: Language::Spanish,
        ..Default::default()
    };
    let restored: Preferences =
        serde_json::from_str(&serde_json::to_string(&preferences).unwrap()).unwrap();
    assert_eq!(restored.language, Language::Spanish);
}

#[test]
fn history_recovers_completed_worker_records_without_an_index_and_keeps_undo_state() {
    let (root, source, _) = fixture();
    let journal = root.path().join("journal");
    let record = history::execute_in(
        Operation::Rename {
            source: source.join("missing"),
            name: "after".into(),
        },
        &mut Task::default(),
        &journal,
    )
    .unwrap();
    assert!(!record.success);
    let mut recovered = history::load_in(&journal).unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].id, record.id);
    recovered[0].undone = true;
    history::save_in(&recovered, &journal).unwrap();
    fs::write(journal.join("index.json"), "interrupted-index").unwrap();
    let recovered = history::load_in(&journal).unwrap();
    assert!(recovered[0].undone);
    history::save_in(&recovered, &journal).unwrap();
    assert!(fs::read_dir(&journal).unwrap().flatten().any(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .starts_with("index.invalid-")
    }));
}

#[test]
fn cancelled_operations_are_persisted_without_mutating_files() {
    let (root, source, destination) = fixture();
    fs::write(source.join("data"), "keep").unwrap();
    let mut task = Task::new(
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        |_| {},
    );
    let journal = root.path().join("journal");
    let record = history::execute_in(
        Operation::Copy {
            sources: vec![source.join("data")],
            destination: destination.clone(),
        },
        &mut task,
        &journal,
    )
    .unwrap();
    assert!(record.cancelled);
    assert!(!record.success);
    assert!(!destination.join("data").exists());
    assert!(history::load_in(&journal).unwrap()[0].cancelled);
    let queued = history::cancelled("Copy");
    assert!(queued.cancelled && queued.changes.is_empty());
}

#[test]
fn undo_merged_folder_restores_old_tree_and_removes_new_children() {
    let (root, source, destination) = fixture();
    for base in [&source, &destination] {
        fs::create_dir(base.join("folder")).unwrap();
    }
    fs::write(source.join("folder/data"), "new").unwrap();
    fs::write(source.join("folder/added"), "added").unwrap();
    fs::write(destination.join("folder/data"), "old").unwrap();
    fs::write(destination.join("folder/kept"), "kept").unwrap();
    let record = history::execute_in(
        Operation::Transfer {
            sources: vec![source.join("folder")],
            destination: destination.clone(),
            moving: false,
            options: TransferOptions {
                conflicts: ConflictPolicy::Replace,
                merge_folders: true,
            },
        },
        &mut Task::default(),
        &root.path().join("journal"),
    )
    .unwrap();
    assert!(record.success, "{}", record.result);
    history::undo(&record, &mut Task::default()).unwrap();
    assert_eq!(
        fs::read_to_string(destination.join("folder/data")).unwrap(),
        "old"
    );
    assert_eq!(
        fs::read_to_string(destination.join("folder/kept")).unwrap(),
        "kept"
    );
    assert!(!destination.join("folder/added").exists());
}

#[test]
fn zip_edits_can_be_undone_to_the_exact_original_archive() {
    let (root, source, destination) = fixture();
    fs::write(source.join("data"), "content").unwrap();
    let file = destination.join("data.zip");
    archive::pack(&[source.join("data")], &file, &mut Task::default()).unwrap();
    let before = fs::read(&file).unwrap();
    let record = history::execute_in(
        Operation::ArchiveEdit {
            archive: file.clone(),
            edit: ZipEdit::Rename {
                source: "data".into(),
                target: "renamed".into(),
            },
        },
        &mut Task::default(),
        &root.path().join("journal"),
    )
    .unwrap();
    assert!(record.success, "{}", record.result);
    history::undo(&record, &mut Task::default()).unwrap();
    assert_eq!(fs::read(&file).unwrap(), before);
}

#[test]
fn remote_directory_rename_preserves_children() {
    let (_root, source, _) = fixture();
    fs::create_dir(source.join("before")).unwrap();
    fs::write(source.join("before/data"), "content").unwrap();
    let connection = remote_fixture(&source);
    let entry = remote::list(&connection.id, Path::new(""))
        .unwrap()
        .into_iter()
        .find(|entry| entry.is_dir)
        .unwrap();
    remote::rename(&connection.id, &entry.path, "after").unwrap();
    assert_eq!(
        fs::read_to_string(source.join("after/data")).unwrap(),
        "content"
    );
    assert!(!source.join("before").exists());
}

#[test]
fn language_catalogs_cover_help_and_common_error_contexts() {
    for language in [Language::German, Language::Spanish] {
        for text in [
            "Browse the archive and select entries first.",
            "Cancelled",
            "This operation has no pending undo.",
            "Find files",
            "Keep working",
        ] {
            assert_ne!(i18n::translate(language, text), text);
        }
        let error = i18n::translate(language, "Destination already exists: /some/user/file.txt");
        assert_ne!(error, "Destination already exists: /some/user/file.txt");
        assert!(error.ends_with("/some/user/file.txt"));
    }
    let de: BTreeMap<String, String> =
        serde_json::from_str(include_str!("../locales/de.json")).unwrap();
    let es: BTreeMap<String, String> =
        serde_json::from_str(include_str!("../locales/es.json")).unwrap();
    assert_eq!(de.keys().collect::<Vec<_>>(), es.keys().collect::<Vec<_>>());
}

#[test]
fn consecutive_undo_restores_directory_metadata_for_older_records() {
    let (root, source, destination) = fixture();
    fs::create_dir(source.join("folder")).unwrap();
    fs::write(source.join("folder/data"), "content").unwrap();
    let journal = root.path().join("journal");
    let first = history::execute_in(
        Operation::Copy {
            sources: vec![source.join("folder")],
            destination: destination.clone(),
        },
        &mut Task::default(),
        &journal,
    )
    .unwrap();
    let second = history::execute_in(
        Operation::Rename {
            source: destination.join("folder"),
            name: "renamed".into(),
        },
        &mut Task::default(),
        &journal,
    )
    .unwrap();
    assert!(first.success && second.success);
    history::undo(&second, &mut Task::default()).unwrap();
    history::undo(&first, &mut Task::default()).unwrap();
    assert!(!destination.join("folder").exists());
    assert_eq!(
        fs::read_to_string(source.join("folder/data")).unwrap(),
        "content"
    );
}

#[test]
#[ignore = "Requires an interactive desktop clipboard; preserves current content in memory"]
fn native_clipboard_copy_cut_and_external_change_round_trip() {
    use clipboard_rs::{Clipboard, ClipboardContent, ClipboardContext};
    let context = ClipboardContext::new().unwrap();
    // A format snapshot represents one item. Preserve an existing multi-file selection intact.
    let old_files = context.get_files().ok();
    assert!(
        old_files.as_ref().is_none_or(|files| files.len() <= 1),
        "Keep the current multiple-file selection intact; rerun with a single clipboard item."
    );
    let formats = context.available_formats().unwrap();
    let saved = formats
        .iter()
        .map(|format| {
            ClipboardContent::Other(
                match format.as_str() {
                    "NSStringPboardType" => "public.utf8-plain-text".into(),
                    "NSRTFPboardType" => "public.rtf".into(),
                    "NSHTMLPboardType" => "public.html".into(),
                    _ => format.clone(),
                },
                context
                    .get_buffer(format)
                    .expect("Read every format before modifying clipboard"),
            )
        })
        .collect::<Vec<_>>();
    struct Restore {
        context: ClipboardContext,
        saved: Vec<ClipboardContent>,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            self.context
                .set(std::mem::take(&mut self.saved))
                .expect("Restore clipboard");
        }
    }
    let _restore = Restore { context, saved };
    let root = tempfile::tempdir().unwrap();
    let paths = vec![
        root.path().join("spaces ä.txt"),
        root.path().join("second.txt"),
    ];
    for path in &paths {
        fs::write(path, "test").unwrap();
    }
    let mut files = native_files::NativeFiles::default();
    files.copy(&paths, false).unwrap();
    assert_eq!(files.paste().unwrap(), (paths.clone(), false));
    assert!(files.owns_selection());
    files.copy(&paths, true).unwrap();
    assert_eq!(files.paste().unwrap(), (paths.clone(), true));
    let external = ClipboardContext::new().unwrap();
    #[cfg(target_os = "linux")]
    for format in [
        "x-special/gnome-copied-files",
        "x-special/nautilus-clipboard",
    ] {
        assert!(external.get_buffer(format).unwrap().starts_with(b"cut\n"));
    }
    external
        .set_files(
            paths
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
        )
        .unwrap();
    assert!(!files.paste().unwrap().1);
    assert!(!files.owns_selection());
    files.mark_remote().unwrap();
    assert!(files.owns_selection());
    external.set_text("clipboard test".into()).unwrap();
    assert!(!files.owns_selection());
}

#[test]
fn zip_rename_rejects_file_as_parent_without_changing_original() {
    let (_root, source, destination) = fixture();
    fs::write(source.join("data"), "content").unwrap();
    fs::write(source.join("parent"), "other").unwrap();
    let file = destination.join("data.zip");
    archive::pack(
        &[source.join("data"), source.join("parent")],
        &file,
        &mut Task::default(),
    )
    .unwrap();
    let before = fs::read(&file).unwrap();
    assert!(
        archive_tools::edit_zip(
            &file,
            &ZipEdit::Rename {
                source: "data".into(),
                target: "parent/data".into()
            },
            &mut Task::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(file).unwrap(), before);
}

#[test]
fn legacy_shortcuts_receive_new_actions_without_stealing_custom_bindings() {
    let mut preferences: Preferences =
        serde_json::from_str(r#"{"shortcuts":{"Copy":"Ctrl+Z","Move":"F6"}}"#).unwrap();
    preferences.upgrade();
    assert_eq!(preferences.shortcuts["Copy"], "Ctrl+Z");
    assert_eq!(preferences.shortcuts["Undo"], "");
    assert_eq!(preferences.shortcuts["Queue"], "Command+J");
    preferences::validate(&preferences).unwrap();
    preferences
        .shortcuts
        .insert("Undo".into(), "Command+Shift+Z".into());
    preferences.upgrade();
    assert_eq!(preferences.shortcuts["Undo"], "Command+Shift+Z");
}
