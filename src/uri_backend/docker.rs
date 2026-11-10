//! Read-only browsing of Linux containers via a pinned local Docker endpoint.
mod process;
mod snapshot;

use crate::{filesystem::Entry, remote::DockerTarget};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use opendal::{EntryMode, Metadata, raw::oio};
use process::{Cancel, capture, cli};
use serde::Deserialize;
use snapshot::Snapshot;
use std::{
    collections::HashMap,
    io::{self, Read},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Duration,
};

const MAX_LIST: usize = 8 * 1024 * 1024;
const MAX_CHILDREN: usize = 50_000;

#[derive(Clone, Copy)]
struct Item {
    directory: bool,
    size: u64,
    modified: Option<i64>,
}

impl Item {
    fn directory() -> Self {
        Self {
            directory: true,
            size: 0,
            modified: None,
        }
    }

    fn metadata(self) -> Metadata {
        let mut meta = Metadata::new(if self.directory {
            EntryMode::DIR
        } else {
            EntryMode::FILE
        });
        meta.set_content_length(self.size);
        if let Some(time) = self
            .modified
            .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        {
            meta.set_last_modified(time);
        }
        meta
    }
}

fn validate_endpoint(endpoint: &str) -> Result<()> {
    let unix = endpoint
        .strip_prefix("unix://")
        .is_some_and(|path| path.starts_with('/') && path.len() > 1);
    let pipe = endpoint.to_ascii_lowercase();
    let local_pipe = cfg!(windows)
        && ["npipe:////./pipe/", "npipe://./pipe/"]
            .iter()
            .any(|prefix| {
                pipe.strip_prefix(prefix)
                    .is_some_and(|name| !name.is_empty() && !name.contains(['/', '\\']))
            });
    ensure!(
        (unix || local_pipe) && !endpoint.contains(['\0', '\n', '\r']) && endpoint.len() <= 4096,
        "Docker browsing requires a local Unix socket or Windows named pipe. Choose a local Docker context."
    );
    Ok(())
}

pub(crate) fn validate_target(target: &DockerTarget) -> Result<()> {
    validate_endpoint(&target.endpoint)?;
    ensure!(
        target.container.len() == 64 && target.container.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid full Docker container ID."
    );
    Ok(())
}

fn host_command(endpoint: &str) -> Command {
    let mut command = cli();
    command
        .env_remove("DOCKER_CONTEXT")
        .env_remove("DOCKER_HOST")
        .args(["--host", endpoint]);
    command
}

fn output_text(command: &mut Command, cancel: &Cancel) -> Result<String> {
    Ok(String::from_utf8(capture(command, 16 * 1024, cancel)?)?
        .trim()
        .to_owned())
}

fn endpoint(cancel: &Cancel) -> Result<String> {
    let context = std::env::var("DOCKER_CONTEXT")
        .ok()
        .filter(|s| !s.is_empty());
    if context.is_none()
        && let Some(host) = std::env::var_os("DOCKER_HOST").filter(|s| !s.is_empty())
    {
        let host = host
            .into_string()
            .map_err(|_| anyhow::anyhow!("Invalid Docker host."))?;
        validate_endpoint(&host)?;
        return Ok(host);
    }
    let context = match context {
        Some(context) => context,
        None => output_text(cli().args(["context", "show"]), cancel)?,
    };
    ensure!(!context.is_empty(), "Docker returned no active context.");
    let endpoint = output_text(
        cli().env_remove("DOCKER_HOST").args([
            "context",
            "inspect",
            "--format",
            "{{.Endpoints.docker.Host}}",
            "--",
            &context,
        ]),
        cancel,
    )?;
    validate_endpoint(&endpoint)?;
    Ok(endpoint)
}

fn ensure_linux(endpoint: &str, cancel: &Cancel) -> Result<()> {
    let os = output_text(
        host_command(endpoint).args(["info", "--format", "{{.OSType}}"]),
        cancel,
    )
    .context("Cannot connect to the local Docker engine")?;
    ensure!(
        os == "linux",
        "Docker browsing supports Linux containers. Switch the engine to Linux containers."
    );
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ListedContainer {
    #[serde(rename = "ID")]
    id: String,
    names: String,
    state: String,
}

pub(crate) fn list_containers(cancel: &Cancel) -> Result<Vec<Entry>> {
    let endpoint = endpoint(cancel)?;
    ensure_linux(&endpoint, cancel)?;
    let rows = capture(
        host_command(&endpoint).args([
            "container",
            "ls",
            "--all",
            "--no-trunc",
            "--format",
            "{{json .}}",
        ]),
        MAX_LIST,
        cancel,
    )?;
    std::str::from_utf8(&rows)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let row: ListedContainer =
                serde_json::from_str(line).context("Invalid Docker container list")?;
            let target = DockerTarget {
                endpoint: endpoint.clone(),
                container: row.id,
            };
            validate_target(&target)?;
            Ok(Entry {
                path: row_key(&target),
                name: format!("{} ({})", row.names, row.state),
                is_dir: true,
                is_symlink: false,
                size: 0,
                size_known: false,
                modified: None,
            })
        })
        .collect()
}

pub(crate) fn target_from_entry(path: &Path) -> Result<DockerTarget> {
    let encoded = path.to_str().context("Invalid Docker container row")?;
    ensure!(encoded.len() <= 8192, "Invalid Docker container row.");
    let target = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded)?)?;
    validate_target(&target)?;
    Ok(target)
}

pub(crate) fn row_key(target: &DockerTarget) -> PathBuf {
    // A DockerTarget contains only strings.
    PathBuf::from(
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(target).expect("Serialize Docker target")),
    )
}

pub(super) fn valid_key(path: &str) -> bool {
    path.len() <= 4096
        && !path.contains(['\\', ':', '\0'])
        && path
            .split('/')
            .all(|p| !p.is_empty() && !matches!(p, "." | ".."))
        && !["proc", "sys", "dev"].iter().any(|root| inside(path, root))
}

pub(super) fn inside(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|tail| tail.starts_with('/'))
}

fn checked_key(path: &str) -> Result<&str> {
    let path = path.trim_end_matches('/');
    ensure!(
        path.is_empty() || valid_key(path),
        "Invalid or unsupported Docker path."
    );
    Ok(path)
}

#[derive(Clone)]
enum Mode {
    Live,
    Snapshot(Arc<Snapshot>),
}

struct Inspect {
    state: State,
    mounts: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct State {
    running: bool,
    paused: bool,
    restarting: bool,
}

pub(crate) struct Container {
    target: DockerTarget,
    mode: Mutex<Option<Mode>>,
    prepare: Mutex<()>,
}

impl std::fmt::Debug for Container {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DockerContainer")
    }
}

pub(crate) fn attach(target: &DockerTarget) -> Arc<Container> {
    static CONTAINERS: OnceLock<Mutex<HashMap<DockerTarget, Weak<Container>>>> = OnceLock::new();
    let mut containers = CONTAINERS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = containers.get(target).and_then(Weak::upgrade) {
        return existing;
    }
    containers.retain(|_, container| container.strong_count() != 0);
    let container = Arc::new(Container {
        target: target.clone(),
        mode: Mutex::new(None),
        prepare: Mutex::new(()),
    });
    containers.insert(target.clone(), Arc::downgrade(&container));
    container
}

impl Container {
    fn command(&self) -> Command {
        let mut command = host_command(&self.target.endpoint);
        command.arg("container");
        command
    }

    fn exec(&self) -> Command {
        let mut command = self.command();
        command.args(["exec", "--env", "LC_ALL=C", &self.target.container]);
        command
    }

    fn inspect(&self, cancel: &Cancel) -> Result<Inspect> {
        validate_target(&self.target)?;
        let bytes = capture(
            self.command().args([
                "inspect",
                "--format",
                "{{println (json .State)}}{{range .Mounts}}{{println (json .Destination)}}{{end}}",
                &self.target.container,
            ]),
            MAX_LIST,
            cancel,
        )
        .context("Cannot inspect this Docker container")?;
        let mut lines = std::str::from_utf8(&bytes)?.lines();
        let state = serde_json::from_str(lines.next().context("Missing Docker container state")?)?;
        let mounts = lines
            .filter(|line| !line.is_empty())
            .map(serde_json::from_str)
            .collect::<serde_json::Result<Vec<String>>>()?;
        Ok(Inspect { state, mounts })
    }

    fn current(&self) -> Option<Mode> {
        self.mode.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(crate) fn status(&self) -> Option<String> {
        self.current().map(|mode| match mode {
            Mode::Live => crate::i18n::text("Live").into_owned(),
            Mode::Snapshot(snapshot) => format!(
                "{} · {}",
                crate::i18n::text("Snapshot"),
                snapshot
                    .captured
                    .with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M:%S")
            ),
        })
    }

    fn mode(&self, refresh: bool, cancel: &Cancel) -> Result<Mode> {
        process::check(cancel)?;
        if !refresh && let Some(mode) = self.current() {
            return Ok(mode);
        }
        let previous = self.current();
        let _preparation = loop {
            process::check(cancel)?;
            match self.prepare.try_lock() {
                Ok(guard) => break guard,
                Err(std::sync::TryLockError::WouldBlock) => {
                    std::thread::sleep(Duration::from_millis(40))
                }
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    bail!("Docker browsing worker stopped.")
                }
            }
        };
        if let Some(current) = self.current()
            && (!refresh
                || match (&previous, &current) {
                    (Some(Mode::Snapshot(a)), Mode::Snapshot(b)) => !Arc::ptr_eq(a, b),
                    (Some(Mode::Live), Mode::Live) => false,
                    _ => true,
                })
        {
            return Ok(current);
        }
        let inspect = self.inspect(cancel)?;
        let mode = if inspect.state.running
            && !inspect.state.paused
            && !inspect.state.restarting
            && self.supports_live(cancel)?
        {
            Mode::Live
        } else {
            let mounts: Vec<_> = inspect
                .mounts
                .into_iter()
                .map(|mount| mount.trim_matches('/').to_owned())
                .filter(|path| !path.is_empty())
                .collect();
            Mode::Snapshot(Arc::new(Snapshot::export(
                self.command().args(["export", &self.target.container]),
                &mounts,
                cancel,
            )?))
        };
        process::check(cancel)?;
        *self.mode.lock().unwrap_or_else(|e| e.into_inner()) = Some(mode.clone());
        Ok(mode)
    }

    fn supports_live(&self, cancel: &Cancel) -> Result<bool> {
        for (args, expected) in [
            (
                &["find", "/", "-maxdepth", "0", "-print0"][..],
                b"/\0".as_slice(),
            ),
            (&["stat", "-c", "%f %s %Y", "--", "/"][..], b"".as_slice()),
            (&["cat", "--", "/dev/null"][..], b"".as_slice()),
        ] {
            match capture(self.exec().args(args), 4096, cancel) {
                Ok(bytes) if args[0] == "stat" => {
                    if !parse_stat(&bytes).is_ok_and(|item| item.is_some_and(|item| item.directory))
                    {
                        return Ok(false);
                    }
                }
                Ok(bytes) if bytes == expected => {}
                Ok(_) => return Ok(false),
                Err(e) if missing_tools(&e) || inactive(&e) => return Ok(false),
                Err(e) => return Err(e),
            }
        }
        Ok(true)
    }

    fn live_stat(&self, path: &str, cancel: &Cancel) -> Result<Item> {
        let path = format!("/{path}");
        let bytes = capture(
            self.exec().args(["stat", "-c", "%f %s %Y", "--", &path]),
            1024,
            cancel,
        )?;
        parse_stat(&bytes)?.context("Unsupported Docker file type.")
    }

    fn live_list(&self, path: &str, cancel: &Cancel) -> Result<Vec<(String, Item)>> {
        ensure!(
            self.live_stat(path, cancel)?.directory,
            "Docker path is not a folder."
        );
        let full = format!("/{path}");
        let prefix = if path.is_empty() {
            "/".into()
        } else {
            format!("{full}/")
        };
        let bytes = capture(
            self.exec()
                .args(["find", &full, "-mindepth", "1", "-maxdepth", "1", "-print0"]),
            MAX_LIST,
            cancel,
        )?;
        ensure!(
            bytes.is_empty() || bytes.ends_with(b"\0"),
            "Invalid Docker directory response."
        );
        let names = bytes
            .split(|&b| b == 0)
            .filter_map(|raw| {
                let path = std::str::from_utf8(raw).ok()?;
                let name = path.strip_prefix(&prefix)?;
                (!name.contains('/') && valid_key(path.trim_start_matches('/')))
                    .then_some((name, path))
            })
            .collect::<Vec<_>>();
        ensure!(
            names.len() <= MAX_CHILDREN,
            "Docker directory has too many entries."
        );
        let mut items = Vec::new();
        let mut rest = names.as_slice();
        while !rest.is_empty() {
            // Keep argument lists below Windows' command-line limit even with long names.
            let mut length = 0;
            let count = rest
                .iter()
                .take(128)
                .take_while(|(_, p)| {
                    length += p.len() + 4;
                    length <= 16 * 1024
                })
                .count();
            let (batch, remaining) = rest.split_at(count);
            rest = remaining;
            let bytes = capture(
                self.exec()
                    .args(["stat", "-c", "%f %s %Y", "--"])
                    .args(batch.iter().map(|(_, path)| path)),
                64 * 1024,
                cancel,
            )?;
            let lines = std::str::from_utf8(&bytes)?.lines().collect::<Vec<_>>();
            ensure!(
                lines.len() == batch.len(),
                "Incomplete Docker directory metadata. Refresh to try again."
            );
            for ((name, _), line) in batch.iter().zip(lines) {
                if let Some(item) = parse_stat(line.as_bytes())? {
                    items.push(((*name).to_owned(), item));
                }
            }
        }
        Ok(items)
    }

    fn listing(&self, path: &str, refresh: bool, cancel: &Cancel) -> Result<Vec<(String, Item)>> {
        match self.mode(refresh, cancel)? {
            Mode::Live => match self.live_list(path, cancel) {
                Err(error) if inactive(&error) => match self.mode(true, cancel)? {
                    Mode::Live => Err(error),
                    Mode::Snapshot(snapshot) => snapshot.list(path),
                },
                result => result,
            },
            Mode::Snapshot(snapshot) => snapshot.list(path),
        }
    }

    pub(crate) fn entries(
        &self,
        path: &Path,
        refresh: bool,
        cancel: &Cancel,
    ) -> Result<Vec<Entry>> {
        let key = crate::remote::key(path, false)?;
        Ok(self
            .listing(checked_key(&key)?, refresh, cancel)?
            .into_iter()
            .map(|(name, item)| Entry {
                path: path.join(&name),
                name,
                is_dir: item.directory,
                is_symlink: false,
                size: item.size,
                size_known: !item.directory,
                modified: item
                    .modified
                    .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
                    .map(Into::into),
            })
            .collect())
    }
}

fn command_failure(error: &anyhow::Error) -> Option<&process::Failure> {
    error.downcast_ref::<io::Error>()?.get_ref()?.downcast_ref()
}

fn inactive(error: &anyhow::Error) -> bool {
    command_failure(error).is_some_and(|failure| {
        failure.message.starts_with("Error response from daemon:")
            && ["is not running", "is paused", "is restarting"]
                .iter()
                .any(|text| failure.message.contains(text))
    })
}

fn missing_tools(error: &anyhow::Error) -> bool {
    command_failure(error).is_some_and(|failure| {
        let message = failure.message.to_ascii_lowercase();
        if message.contains("permission denied") || message.contains("operation not permitted") {
            return false;
        }
        matches!(failure.code, Some(126 | 127))
            || [
                "executable file not found",
                "unrecognized option",
                "unknown option",
                "invalid option",
                "unrecognized:",
                "unknown predicate",
            ]
            .iter()
            .any(|text| message.contains(text))
    })
}

fn parse_stat(bytes: &[u8]) -> Result<Option<Item>> {
    let mut fields = std::str::from_utf8(bytes)?.split_ascii_whitespace();
    let mode = u32::from_str_radix(fields.next().context("Missing Docker file mode")?, 16)?;
    let size = fields.next().context("Missing Docker file size")?.parse()?;
    let modified = fields.next().context("Missing Docker file time")?.parse()?;
    ensure!(fields.next().is_none(), "Invalid Docker file metadata.");
    Ok(match mode & 0o170000 {
        0o040000 => Some(Item {
            directory: true,
            size: 0,
            modified: Some(modified),
        }),
        0o100000 => Some(Item {
            directory: false,
            size,
            modified: Some(modified),
        }),
        _ => None,
    })
}

impl super::Transport for Container {
    fn stat(&self, path: &str) -> Result<Metadata> {
        let path = checked_key(path)?;
        let cancel = Cancel::default();
        Ok(match self.mode(false, &cancel)? {
            Mode::Live => self.live_stat(path, &cancel)?,
            Mode::Snapshot(snapshot) => snapshot.stat(path)?,
        }
        .metadata())
    }

    fn list(&self, path: &str) -> Result<Vec<oio::Entry>> {
        let path = checked_key(path)?;
        Ok(self
            .listing(path, false, &Default::default())?
            .into_iter()
            .filter_map(|(name, item)| super::child_key(path, &name, Some(item.metadata())))
            .collect())
    }

    fn read(&self, path: &str, offset: u64) -> Result<Box<dyn Read + Send>> {
        let path = checked_key(path)?;
        let cancel = Cancel::default();
        match self.mode(false, &cancel)? {
            Mode::Live => {
                let item = self.live_stat(path, &cancel)?;
                ensure!(
                    !item.directory && offset <= item.size,
                    "Invalid Docker file range."
                );
                let mut pipe = process::Pipe::start(
                    self.exec().args(["cat", "--", &format!("/{path}")]),
                    Duration::from_secs(120),
                    &cancel,
                )?;
                let skipped = io::copy(&mut (&mut pipe).take(offset), &mut io::sink())?;
                ensure!(skipped == offset, "Docker file changed while reading.");
                Ok(Box::new(pipe))
            }
            Mode::Snapshot(snapshot) => snapshot.read(path, offset),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_command_keeps_its_endpoint_when_context_changes() {
        let command = host_command("unix:///tmp/pinned-engine.sock");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["--host", "unix:///tmp/pinned-engine.sock"]
        );
        for key in ["DOCKER_CONTEXT", "DOCKER_HOST"] {
            assert!(
                command
                    .get_envs()
                    .any(|(name, value)| name == key && value.is_none())
            );
        }
    }

    #[test]
    fn docker_permission_errors_do_not_trigger_snapshot_fallback() {
        let error = anyhow::Error::new(io::Error::other(process::Failure {
            code: Some(126),
            message: "find: /path is not running: Permission denied".into(),
        }));
        assert!(!missing_tools(&error));
        assert!(!inactive(&error));
    }

    #[test]
    fn docker_paths_and_endpoints_are_local_and_bounded() {
        assert!(validate_endpoint("unix:///tmp/docker.sock").is_ok());
        assert!(validate_endpoint("unix://other/var/run/docker.sock").is_err());
        assert!(validate_endpoint("tcp://localhost:2375").is_err());
        assert!(validate_endpoint("ssh://example.org").is_err());
        assert!(valid_key("folder/file with spaces"));
        assert!(!valid_key("folder/../etc"));
        assert!(!valid_key("proc/self"));
        assert!(parse_stat(b"81a4 5 123\n").unwrap().is_some());
        assert!(parse_stat(b"a1ff 5 123\n").unwrap().is_none());
    }
}
