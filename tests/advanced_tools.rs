use rust_commander::{
    comparison::{self, Mode, Options},
    desktop::{self, SearchOptions},
    filesystem::Location,
    folder_tools,
    operations::Task,
    path_rules::PathRules,
    remote::{self, Connection, Secrets},
    search,
    sync::{self, Choice},
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

fn mount(root: &Path) -> Location {
    let profile = Connection {
        id: format!("advanced-{}", rand::random::<u64>()),
        name: "Disposable storage".into(),
        service: "fs".into(),
        root: root.display().to_string(),
        ..Default::default()
    };
    remote::connect(&profile, Secrets::default(), false).unwrap();
    Location::Remote {
        connection: profile.id,
        path: PathBuf::new(),
    }
}
fn run(location: &Location, options: &SearchOptions) -> rust_commander::desktop::SearchResult {
    search::search(location, options, &mut Task::default()).unwrap()
}
#[test]
fn large_literal_search_streams_local_and_remote_without_a_file_size_limit() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("large.log");
    let mut file = fs::File::create(&path).unwrap();
    for _ in 0..321 {
        file.write_all(&[b'x'; 65_536]).unwrap();
    }
    // The Unicode match straddles a read boundary and is case-insensitive.
    file.write_all(&[b'x'; 65_534]).unwrap();
    file.write_all("Äpfel DONE".as_bytes()).unwrap();
    drop(file);
    for location in [Location::Local(root.path().into()), mount(root.path())] {
        let mut options = SearchOptions {
            root: rust_commander::directory_tree::path(&location).into(),
            content: "äPFEL done".into(),
            ..Default::default()
        };
        options.filters.content_case_sensitive = false;
        let result = run(&location, &options);
        assert_eq!(result.paths.len(), 1);
        assert!(result.errors.is_empty());
        options.filters.content_case_sensitive = true;
        assert!(run(&location, &options).paths.is_empty());
    }
}
#[test]
fn search_combines_regex_case_size_date_and_pruned_directory_rules() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("nested/cache")).unwrap();
    fs::write(root.path().join("nested/Notes.TXT"), "Error: 429\n").unwrap();
    fs::write(root.path().join("nested/other.TXT"), "Error: 12\n").unwrap();
    fs::write(root.path().join("nested/cache/hidden.TXT"), "Error: 429\n").unwrap();
    let modified = search::parse_date("2026-10-23T12:00:00Z").unwrap().unwrap();
    filetime::set_file_mtime(
        root.path().join("nested/Notes.TXT"),
        filetime::FileTime::from_system_time(modified),
    )
    .unwrap();
    let location = Location::Local(root.path().into());
    let mut options = SearchOptions {
        root: root.path().into(),
        pattern: "*.txt".into(),
        content: r"^error: \d{3}$".into(),
        ..Default::default()
    };
    options.filters.content_regex = true;
    options.filters.content_case_sensitive = false;
    options.filters.exclude = "cache;*.bin".into();
    options.filters.min_size = Some(10);
    options.filters.max_size = Some(20);
    options.filters.modified_after = search::parse_date("2026-10-23T00:00:00Z").unwrap();
    options.filters.modified_before = search::parse_date("2026-10-24T00:00:00Z").unwrap();
    assert_eq!(
        run(&location, &options).paths,
        vec![root.path().join("nested/Notes.TXT")]
    );
    options.filters.name_case_sensitive = true;
    assert!(run(&location, &options).paths.is_empty());
    options.pattern = "nested/*.TXT".into();
    assert_eq!(run(&location, &options).paths.len(), 1);
}
#[test]
fn regex_long_line_limits_are_reported_and_literal_search_still_finds_the_tail() {
    let root = tempfile::tempdir().unwrap();
    let mut bytes = vec![b'a'; 9 * 1024 * 1024];
    bytes.extend_from_slice(b"needle");
    fs::write(root.path().join("long"), bytes).unwrap();
    let location = Location::Local(root.path().into());
    let mut options = SearchOptions {
        root: root.path().into(),
        content: "needle".into(),
        ..Default::default()
    };
    options.filters.content_regex = true;
    let result = run(&location, &options);
    assert!(result.paths.is_empty());
    assert_eq!(result.errors.len(), 1);
    assert!(result.errors[0].contains("8 MiB"));
    options.filters.content_regex = false;
    assert_eq!(run(&location, &options).paths.len(), 1);
}
#[test]
fn invalid_search_options_fail_before_scanning_and_streams_can_be_cancelled() {
    let root = tempfile::tempdir().unwrap();
    let mut options = SearchOptions {
        root: root.path().into(),
        content: "[".into(),
        ..Default::default()
    };
    options.filters.content_regex = true;
    assert!(search::validate(&options).is_err());
    options.filters.content_regex = false;
    options.filters.min_size = Some(20);
    options.filters.max_size = Some(10);
    assert!(search::validate(&options).is_err());
    assert!(search::parse_date("not a date").is_err());
    options.filters = Default::default();
    fs::write(root.path().join("large"), vec![b'x'; 2 * 1024 * 1024]).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let mut task = Task::new(cancel, move |p| {
        if p.bytes >= 65_536 {
            signal.store(true, Ordering::Relaxed);
        }
    })
    .report_every(Duration::ZERO);
    assert!(desktop::search(&options, &mut task).is_err());
}
#[test]
fn quick_comparison_is_explicitly_unverified_even_with_identical_metadata() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a");
    let b = root.path().join("b");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    fs::write(a.join("file.txt"), b"AAAA").unwrap();
    fs::write(b.join("file.txt"), b"BBBB").unwrap();
    let modified = filetime::FileTime::from_unix_time(1_700_000_000, 0);
    for folder in [&a, &b] {
        filetime::set_file_mtime(folder.join("file.txt"), modified).unwrap();
        fs::write(folder.join("skip.bin"), "ignore").unwrap();
    }
    let left = Location::Local(a);
    let right = mount(&b);
    let options = Options {
        rules: PathRules {
            include: "*.txt".into(),
            ..Default::default()
        },
        mode: Mode::Metadata,
    };
    let result = comparison::compare_with_options(
        &left,
        &Location::Local(b.clone()),
        &options,
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].status, "Metadata match (unverified)");
    let remote_result =
        comparison::compare_with_options(&left, &right, &options, &mut Task::default()).unwrap();
    assert!(remote_result[0].status.contains("unverified"));
    let result = comparison::compare_with_options(
        &left,
        &right,
        &Options {
            mode: Mode::Content,
            ..options
        },
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(result[0].status, "Different");
}
#[test]
fn filtered_sync_never_copies_excluded_descendants_of_a_missing_folder() {
    for (remote_source, remote_target) in
        [(false, false), (false, true), (true, false), (true, true)]
    {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a");
        let b = root.path().join("b");
        fs::create_dir_all(a.join("nested/cache")).unwrap();
        fs::create_dir(&b).unwrap();
        fs::write(a.join("nested/keep.txt"), "copy").unwrap();
        fs::write(a.join("nested/private.bin"), "secret").unwrap();
        fs::write(a.join("nested/cache/no.txt"), "secret").unwrap();
        let left = if remote_source {
            mount(&a)
        } else {
            Location::Local(a)
        };
        let right = if remote_target {
            mount(&b)
        } else {
            Location::Local(b.clone())
        };
        let rules = PathRules {
            include: "*.txt".into(),
            exclude: "cache".into(),
            ..Default::default()
        };
        let plan =
            sync::preview_locations_with_rules(&left, &right, 1, &rules, &Task::default()).unwrap();
        assert!(plan.items.iter().any(|i| i.shallow_directory));
        let restored = serde_json::from_slice(&serde_json::to_vec(&plan).unwrap()).unwrap();
        sync::apply(&restored, &mut Task::default()).unwrap();
        assert_eq!(fs::read(b.join("nested/keep.txt")).unwrap(), b"copy");
        assert!(!b.join("nested/private.bin").exists());
        assert!(!b.join("nested/cache").exists());
        let directory = plan.items.iter().find(|i| i.shallow_directory).unwrap();
        assert!(!plan.allows(directory, Choice::DeleteLeft));
    }
}
#[test]
fn filtered_sync_rejects_type_conflicts_and_stale_plans_without_touching_exclusions() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a");
    let b = root.path().join("b");
    fs::create_dir_all(a.join("folder")).unwrap();
    fs::create_dir(&b).unwrap();
    fs::write(a.join("folder/a.txt"), "new").unwrap();
    fs::write(b.join("folder"), "preserve").unwrap();
    let rules = PathRules {
        include: "*.txt".into(),
        ..Default::default()
    };
    assert!(sync::preview_with_rules(&a, &b, 1, &rules, &Task::default()).is_err());
    assert_eq!(fs::read(b.join("folder")).unwrap(), b"preserve");
    fs::remove_file(b.join("folder")).unwrap();
    let plan = sync::preview_with_rules(&a, &b, 1, &rules, &Task::default()).unwrap();
    fs::write(a.join("folder/a.txt"), "external").unwrap();
    assert!(sync::apply(&plan, &mut Task::default()).is_err());
    assert!(!b.join("folder").exists());
}
#[test]
fn text_diff_aligns_insertions_replacements_and_line_endings_without_writing() {
    let rows = folder_tools::align(
        "alpha\nbeta\ngamma\n",
        "alpha\ninsert\nbeta\nGAMMA\n",
        &Task::default(),
    )
    .unwrap();
    assert_eq!(rows.len(), 4);
    assert!(!rows[0].changed);
    assert!(rows[1].left.is_empty());
    assert!(rows[1].right.contains("insert"));
    assert!(!rows[2].changed);
    assert!(rows[3].left.contains("gamma"));
    assert!(rows[3].right.contains("GAMMA"));
    assert!(folder_tools::align("a\n", "a", &Task::default()).unwrap()[0].changed);
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a");
    let b = root.path().join("b");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    fs::write(a.join("text"), "hello\n").unwrap();
    fs::write(b.join("text"), "hello\nworld\n").unwrap();
    let result = folder_tools::text_diff(
        &Location::Local(a.clone()),
        &mount(&b),
        Path::new("text"),
        &Task::default(),
    )
    .unwrap();
    assert_eq!(result.rows.len(), 2);
    assert_eq!(fs::read(a.join("text")).unwrap(), b"hello\n");
}
#[test]
fn folder_presets_round_trip_with_old_settings_defaults_and_shared_rules() {
    let mut settings: rust_commander::settings::Settings = serde_json::from_str("{}").unwrap();
    assert!(settings.preferences.folder_tool_presets.is_empty());
    settings
        .preferences
        .folder_tool_presets
        .push(comparison::Preset {
            name: "Source files".into(),
            options: Options {
                rules: PathRules {
                    include: "*.rs;*.toml".into(),
                    exclude: "target;.git".into(),
                    case_sensitive: true,
                },
                mode: Mode::Metadata,
            },
        });
    let saved = serde_json::to_string(&settings).unwrap();
    let restored: rust_commander::settings::Settings = serde_json::from_str(&saved).unwrap();
    let rules = restored.preferences.folder_tool_presets[0]
        .options
        .rules
        .compile()
        .unwrap();
    assert!(rules.included(Path::new("src/lib.rs")));
    assert!(!rules.included(Path::new("target/generated.rs")));
    assert!(!rules.included(Path::new("src/Main.RS")));
}
