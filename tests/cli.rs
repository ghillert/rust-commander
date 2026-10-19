use std::process::Command;
fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rust-commander"))
}
#[test]
fn help_and_version_do_not_require_a_display_or_configuration() {
    let work = tempfile::tempdir().unwrap();
    let config = work.path().join("config");
    for flag in ["--help", "-h", "--version"] {
        let output = binary()
            .arg(flag)
            .env("RUST_COMMANDER_CONFIG_DIR", &config)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("Rust Commander"));
        assert!(
            !config.exists(),
            "CLI information must not initialize user settings"
        );
    }
}
#[test]
fn excessive_starting_paths_fail_before_opening_the_desktop() {
    let output = binary().args(["one", "two", "three"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("at most two"));
}
