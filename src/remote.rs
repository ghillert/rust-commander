use crate::{
    filesystem::{self, Entry, Location},
    operations::{self, Task},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::Read,
    path::Path,
    sync::{Arc, OnceLock, RwLock},
};

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub service: String,
    pub endpoint: String,
    pub root: String,
    pub bucket: String,
    pub region: String,
    pub username: String,
    pub key_file: String,
}
impl Default for Connection {
    fn default() -> Self {
        Self {
            id: format!(
                "{:x}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ),
            name: "New connection".into(),
            service: "sftp".into(),
            endpoint: String::new(),
            root: "/".into(),
            bucket: String::new(),
            region: "us-east-1".into(),
            username: String::new(),
            key_file: String::new(),
        }
    }
}

// Secrets are serialized only for the OS credential store, never into settings.
#[derive(Default, Serialize, Deserialize)]
pub struct Secrets {
    pub password: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub access_token: String,
}
pub const SERVICES: &[&str] = &[
    "sftp", "ftp", "webdav", "s3", "http", "dropbox", "gdrive", "onedrive", "gcs", "webhdfs",
];

struct Session {
    operator: opendal::blocking::Operator,
    secrets: Secrets,
}
static SESSIONS: OnceLock<RwLock<HashMap<String, Arc<Session>>>> = OnceLock::new();
static PROFILES: OnceLock<RwLock<HashMap<String, Connection>>> = OnceLock::new();
fn sessions() -> &'static RwLock<HashMap<String, Arc<Session>>> {
    SESSIONS.get_or_init(Default::default)
}
fn profiles() -> &'static RwLock<HashMap<String, Connection>> {
    PROFILES.get_or_init(Default::default)
}
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| tokio::runtime::Runtime::new().expect("Cannot create storage runtime"))
}

pub fn register_profiles(connections: &[Connection]) {
    if let Ok(mut map) = profiles().write() {
        for c in connections {
            map.insert(c.id.clone(), c.clone());
        }
    }
}

pub fn connection_name(id: &str) -> String {
    profiles()
        .read()
        .ok()
        .and_then(|map| map.get(id).map(|c| c.name.clone()))
        .unwrap_or_else(|| id.to_owned())
}

pub fn validate(connection: &Connection) -> Result<()> {
    if connection.name.trim().is_empty() || connection.id.is_empty() {
        bail!("Enter a connection name.");
    }
    if !SERVICES.contains(&connection.service.as_str()) && connection.service != "fs" {
        bail!("Unsupported storage service.");
    }
    if connection.service != "fs" && connection.endpoint.contains("://") {
        let url = url::Url::parse(&connection.endpoint)?;
        if !url.username().is_empty() || url.password().is_some() || url.query().is_some() {
            bail!(
                "Use the credential fields rather than embedding credentials in the endpoint URL."
            );
        }
    }
    Ok(())
}

fn options(connection: &Connection, secrets: &Secrets) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for (key, value) in [
        ("root", &connection.root),
        ("endpoint", &connection.endpoint),
        ("bucket", &connection.bucket),
        ("region", &connection.region),
        ("key", &connection.key_file),
        ("password", &secrets.password),
        ("access_key_id", &secrets.access_key_id),
        ("secret_access_key", &secrets.secret_access_key),
        ("access_token", &secrets.access_token),
    ] {
        if !value.is_empty() {
            map.insert(key.into(), value.clone());
        }
    }
    let username_key = match connection.service.as_str() {
        "sftp" | "ftp" => "user",
        "webhdfs" => "user_name",
        _ => "username",
    };
    if !connection.username.is_empty() {
        map.insert(username_key.into(), connection.username.clone());
    }
    if connection.service == "sftp" {
        map.insert("known_hosts_strategy".into(), "Strict".into());
    }
    if connection.service == "gcs" && !secrets.access_token.is_empty() {
        map.insert("credential".into(), secrets.access_token.clone());
    }
    map
}

fn clean_error(error: impl std::fmt::Display, secrets: &Secrets) -> anyhow::Error {
    let mut text = error.to_string();
    for secret in [
        &secrets.password,
        &secrets.access_key_id,
        &secrets.secret_access_key,
        &secrets.access_token,
    ] {
        if !secret.is_empty() {
            text = text.replace(secret, "[redacted]");
        }
    }
    anyhow::anyhow!(text)
}

pub fn connect(connection: &Connection, secrets: Secrets, remember: bool) -> Result<()> {
    validate(connection)?;
    let _guard = runtime().enter();
    let scheme = connection.service.parse::<opendal::Scheme>()?;
    let operator = opendal::Operator::via_iter(scheme, options(connection, &secrets))
        .map_err(|e| clean_error(e, &secrets))?;
    let operator = operator.layer(
        opendal::layers::TimeoutLayer::new()
            .with_timeout(std::time::Duration::from_secs(30))
            .with_io_timeout(std::time::Duration::from_secs(30)),
    );
    let operator = opendal::blocking::Operator::new(operator)?;
    // Validate the credentials against the service, before storing them.
    if operator.info().full_capability().list {
        operator.list("").map_err(|e| clean_error(e, &secrets))?;
    } else if operator.info().full_capability().stat {
        operator.stat("").map_err(|e| clean_error(e, &secrets))?;
    }
    if remember {
        save_credentials(&connection.id, &secrets)?;
    }
    profiles()
        .write()
        .map_err(|_| anyhow::anyhow!("Connection lock failed"))?
        .insert(connection.id.clone(), connection.clone());
    sessions()
        .write()
        .map_err(|_| anyhow::anyhow!("Session lock failed"))?
        .insert(
            connection.id.clone(),
            Arc::new(Session { operator, secrets }),
        );
    Ok(())
}

fn session(id: &str) -> Result<Arc<Session>> {
    if let Some(session) = sessions()
        .read()
        .map_err(|_| anyhow::anyhow!("Session lock failed"))?
        .get(id)
    {
        return Ok(session.clone());
    }
    let connection = profiles()
        .read()
        .map_err(|_| anyhow::anyhow!("Connection lock failed"))?
        .get(id)
        .cloned()
        .context("Connection profile is missing. Open Connections to reconnect.")?;
    let secrets = load_credentials(id).context(
        "Reconnect to this service in Connections; credentials are not available for this session",
    )?;
    connect(&connection, secrets, false)?;
    sessions()
        .read()
        .map_err(|_| anyhow::anyhow!("Session lock failed"))?
        .get(id)
        .cloned()
        .context("Connection failed")
}

const CREDENTIAL_SERVICE: &str = "rust-commander";
// Retain access to credentials saved by the earlier muCommander Rust preview.
const LEGACY_CREDENTIAL_SERVICE: &str = "mucommander-rust";

pub fn save_credentials(id: &str, secrets: &Secrets) -> Result<()> {
    keyring::Entry::new(CREDENTIAL_SERVICE, id)?.set_password(&serde_json::to_string(secrets)?)?;
    Ok(())
}
pub fn load_credentials(id: &str) -> Result<Secrets> {
    read_stored_credentials(
        &keyring::Entry::new(CREDENTIAL_SERVICE, id)?,
        &keyring::Entry::new(LEGACY_CREDENTIAL_SERVICE, id)?,
    )
}

fn read_stored_credentials(current: &keyring::Entry, legacy: &keyring::Entry) -> Result<Secrets> {
    let contents = match current.get_password() {
        Ok(password) => password,
        Err(keyring::Error::NoEntry) => legacy.get_password()?,
        Err(error) => return Err(error.into()),
    };
    Ok(serde_json::from_str(&contents)?)
}

fn forget_stored_credentials(entries: &[keyring::Entry]) -> Result<()> {
    for entry in entries {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub fn forget_credentials(id: &str) -> Result<()> {
    forget_stored_credentials(&[
        keyring::Entry::new(CREDENTIAL_SERVICE, id)?,
        keyring::Entry::new(LEGACY_CREDENTIAL_SERVICE, id)?,
    ])?;
    sessions()
        .write()
        .map_err(|_| anyhow::anyhow!("Session lock failed"))?
        .remove(id);
    Ok(())
}

pub fn display_path(path: &Path) -> String {
    path.components()
        .filter_map(|part| match part {
            std::path::Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn key(path: &Path, directory: bool) -> Result<String> {
    if path.as_os_str().is_empty() {
        return Ok(String::new());
    }
    let mut segments = Vec::new();
    for part in path.components() {
        let std::path::Component::Normal(name) = part else {
            bail!("Remote paths must stay inside their configured root.");
        };
        let name = name.to_str().context("Remote paths must be UTF-8")?;
        if name.contains(['\\', ':', '\0']) {
            bail!("Invalid remote path component.");
        }
        segments.push(name);
    }
    let mut text = segments.join("/");
    if directory {
        text.push('/');
    }
    Ok(text)
}

pub fn list(id: &str, path: &Path) -> Result<Vec<Entry>> {
    let session = session(id)?;
    let directory = key(path, true)?;
    if !session.operator.info().full_capability().list {
        if path.as_os_str().is_empty() {
            bail!(
                "This service does not provide directory listings. Enter a full file path after remote://{id}/ to view or download it."
            );
        }
        let metadata = session
            .operator
            .stat(&key(path, false)?)
            .map_err(|e| clean_error(e, &session.secrets))?;
        return Ok(vec![Entry {
            path: path.to_owned(),
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            is_dir: metadata.is_dir(),
            is_symlink: false,
            size: metadata.content_length(),
            modified: metadata.last_modified().map(std::time::SystemTime::from),
        }]);
    }
    let entries = session
        .operator
        .list(&directory)
        .map_err(|e| clean_error(e, &session.secrets))?;
    entries
        .into_iter()
        .filter(|e| e.path().trim_end_matches('/') != directory.trim_end_matches('/'))
        .map(|entry| {
            let path = filesystem::safe_relative(Path::new(entry.path().trim_end_matches('/')))?;
            let metadata = entry.metadata();
            Ok(Entry {
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                path,
                is_dir: metadata.is_dir(),
                is_symlink: false,
                size: metadata.content_length(),
                modified: metadata.last_modified().map(std::time::SystemTime::from),
            })
        })
        .collect()
}

pub fn read(id: &str, path: &Path) -> Result<Box<dyn Read>> {
    let session = session(id)?;
    let reader = session
        .operator
        .reader(&key(path, false)?)
        .map_err(|e| clean_error(e, &session.secrets))?;
    Ok(Box::new(
        reader
            .into_std_read(..)
            .map_err(|e| clean_error(e, &session.secrets))?,
    ))
}

pub fn read_bytes(id: &str, path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    read(id, path)?.take(limit + 1).read_to_end(&mut data)?;
    if data.len() as u64 > limit {
        bail!("Remote file exceeds the viewer size limit.");
    }
    Ok(data)
}

pub fn mkdir(id: &str, path: &Path) -> Result<()> {
    let session = session(id)?;
    session
        .operator
        .create_dir(&key(path, true)?)
        .map_err(|e| clean_error(e, &session.secrets))?;
    Ok(())
}

pub fn delete(id: &str, entries: &[Entry], task: &mut Task) -> Result<()> {
    let session = session(id)?;
    for entry in entries {
        task.tick(&entry.path, 0)?;
        session
            .operator
            .remove_all(&key(&entry.path, entry.is_dir)?)
            .map_err(|e| clean_error(e, &session.secrets))?;
    }
    Ok(())
}

pub fn transfer(
    from: &Location,
    entries: &[Entry],
    to: &Location,
    moving: bool,
    task: &mut Task,
) -> Result<()> {
    if entries.is_empty() {
        bail!("Select at least one item.");
    }
    if let (Location::Local(_), Location::Local(destination)) = (from, to) {
        let sources = entries.iter().map(|e| e.path.clone()).collect();
        let operation = if moving {
            operations::Operation::Move {
                sources,
                destination: destination.clone(),
            }
        } else {
            operations::Operation::Copy {
                sources,
                destination: destination.clone(),
            }
        };
        operations::run(operation, task)?;
        return Ok(());
    }
    if matches!(from, Location::Archive { .. }) || matches!(to, Location::Archive { .. }) {
        bail!("Extract archive entries to a local folder first.");
    }
    let before = if moving {
        Some(snapshot(from, entries, task)?)
    } else {
        None
    };
    // Preflight all top-level conflicts before transferring the batch.
    let mut targets = Vec::new();
    for entry in entries {
        let name = entry
            .path
            .file_name()
            .context("Cannot transfer a filesystem root")?;
        let target = match to {
            Location::Local(path) | Location::Remote { path, .. } => path.join(name),
            _ => unreachable!(),
        };
        if targets.contains(&target) {
            bail!("The selection contains duplicate names.");
        }
        ensure_absent(to, &target)?;
        if let (Location::Remote { connection: a, .. }, Location::Remote { connection: b, .. }) =
            (from, to)
            && a == b
            && (target == entry.path || target.starts_with(&entry.path))
        {
            bail!("Cannot transfer a folder into itself.");
        }
        if let (Location::Local(_), Location::Local(directory)) = (from, to) {
            operations::preflight(std::slice::from_ref(&entry.path), directory)?;
        }
        targets.push(target);
    }
    for (entry, target) in entries.iter().zip(&targets) {
        copy_entry(from, entry, to, target, task)?;
    }
    task.check()?;
    if moving {
        if before != Some(snapshot(from, entries, task)?) {
            bail!(
                "The source changed during transfer. All originals and completed copies were kept."
            );
        }
        match from {
            Location::Remote { connection, .. } => delete(connection, entries, task)?,
            Location::Local(_) => {
                for entry in entries {
                    if entry.is_dir && !entry.is_symlink {
                        fs::remove_dir_all(&entry.path)?;
                    } else {
                        fs::remove_file(&entry.path)?;
                    }
                }
            }
            _ => unreachable!(),
        }
    }
    Ok(())
}

fn snapshot(location: &Location, entries: &[Entry], task: &Task) -> Result<Vec<String>> {
    let mut result = Vec::new();
    match location {
        Location::Local(_) => {
            for entry in entries {
                result.push(format!(
                    "{:x?}",
                    operations::source_stamp(&entry.path, task)?
                ));
            }
        }
        Location::Remote { connection, .. } => {
            let session = session(connection)?;
            let mut stack = entries.to_vec();
            while let Some(entry) = stack.pop() {
                task.check()?;
                let metadata = session
                    .operator
                    .stat(&key(&entry.path, entry.is_dir)?)
                    .map_err(|e| clean_error(e, &session.secrets))?;
                result.push(format!(
                    "{}|{}|{:?}|{:?}",
                    display_path(&entry.path),
                    metadata.content_length(),
                    metadata.last_modified(),
                    metadata.etag()
                ));
                if entry.is_dir {
                    stack.extend(list(connection, &entry.path)?);
                }
            }
            result.sort();
        }
        _ => bail!("Archive source is read-only"),
    }
    Ok(result)
}

fn ensure_absent(location: &Location, path: &Path) -> Result<()> {
    match location {
        Location::Local(_) => {
            if operations::exists(path) {
                bail!("Destination already exists: {}", path.display());
            }
        }
        Location::Remote { connection, .. } => {
            let session = session(connection)?;
            match session.operator.stat(&key(path, false)?) {
                Ok(_) => bail!("Remote destination already exists: {}", path.display()),
                Err(error) if error.kind() == opendal::ErrorKind::NotFound => {}
                Err(error) => return Err(clean_error(error, &session.secrets)),
            }
        }
        _ => bail!("Archive destination is read-only"),
    }
    Ok(())
}

fn copy_entry(
    from: &Location,
    entry: &Entry,
    to: &Location,
    target: &Path,
    task: &mut Task,
) -> Result<()> {
    task.tick(&entry.path, 0)?;
    ensure_absent(to, target)?;
    if entry.is_symlink {
        bail!("Remote transfers do not follow symlinks. Transfer the target explicitly.");
    }
    if entry.is_dir {
        match to {
            Location::Local(_) => fs::create_dir(target)?,
            Location::Remote { connection, .. } => mkdir(connection, target)?,
            _ => unreachable!(),
        }
        let child_location = match from {
            Location::Local(_) => Location::Local(entry.path.clone()),
            Location::Remote { connection, .. } => Location::Remote {
                connection: connection.clone(),
                path: entry.path.clone(),
            },
            _ => unreachable!(),
        };
        for child in filesystem::list(&child_location)? {
            copy_entry(
                from,
                &child,
                to,
                &target.join(child.path.file_name().context("Missing filename")?),
                task,
            )?;
        }
    } else {
        let mut input: Box<dyn Read> = match from {
            Location::Local(_) => Box::new(File::open(&entry.path)?),
            Location::Remote { connection, .. } => read(connection, &entry.path)?,
            _ => unreachable!(),
        };
        match to {
            Location::Local(_) => {
                let mut output = tempfile::NamedTempFile::new_in(
                    target.parent().context("Missing destination folder")?,
                )?;
                task.stream(&mut input, &mut output, &entry.path)?;
                output.as_file().sync_all()?;
                output.persist_noclobber(target).map_err(|e| e.error)?;
            }
            Location::Remote { connection, .. } => {
                let session = session(connection)?;
                let options = opendal::options::WriteOptions {
                    if_not_exists: session
                        .operator
                        .info()
                        .full_capability()
                        .write_with_if_not_exists,
                    ..Default::default()
                };
                let mut writer = session
                    .operator
                    .writer_options(&key(target, false)?, options)
                    .map_err(|e| clean_error(e, &session.secrets))?
                    .into_std_write();
                task.stream(&mut input, &mut writer, &entry.path)?;
                writer
                    .close()
                    .map_err(|e| clean_error(e, &session.secrets))?;
            }
            _ => unreachable!(),
        }
    }
    Ok(())
}

#[cfg(test)]
mod credential_compatibility_tests {
    use super::*;

    fn entry(token: Option<&str>) -> keyring::Entry {
        let entry =
            keyring::Entry::new_with_credential(Box::new(keyring::mock::MockCredential::default()));
        if let Some(token) = token {
            entry
                .set_password(
                    &serde_json::to_string(&Secrets {
                        access_token: token.into(),
                        ..Default::default()
                    })
                    .unwrap(),
                )
                .unwrap();
        }
        entry
    }

    #[test]
    fn current_credentials_take_precedence_over_legacy_credentials() {
        let current = entry(Some("current-token"));
        let legacy = entry(Some("legacy-token"));
        assert_eq!(
            read_stored_credentials(&current, &legacy)
                .unwrap()
                .access_token,
            "current-token"
        );
    }

    #[test]
    fn missing_current_credentials_fall_back_without_modifying_legacy_credentials() {
        let current = entry(None);
        let legacy = entry(Some("legacy-token"));
        assert_eq!(
            read_stored_credentials(&current, &legacy)
                .unwrap()
                .access_token,
            "legacy-token"
        );
        assert!(matches!(
            current.get_password(),
            Err(keyring::Error::NoEntry)
        ));
        assert!(legacy.get_password().is_ok());
    }

    #[test]
    fn credential_lookup_errors_do_not_fall_back_to_older_secrets() {
        let current = entry(Some("current-token"));
        let legacy = entry(Some("legacy-token"));
        current
            .get_credential()
            .downcast_ref::<keyring::mock::MockCredential>()
            .unwrap()
            .set_error(keyring::Error::Invalid(
                "mock lookup".into(),
                "access refused".into(),
            ));
        assert!(read_stored_credentials(&current, &legacy).is_err());
    }

    #[test]
    fn invalid_current_credentials_are_reported_without_falling_back() {
        let current = entry(None);
        current.set_password("invalid JSON").unwrap();
        let legacy = entry(Some("legacy-token"));
        assert!(read_stored_credentials(&current, &legacy).is_err());
    }

    #[test]
    fn forgetting_credentials_removes_both_namespaces() {
        let entries = [entry(Some("current-token")), entry(Some("legacy-token"))];
        forget_stored_credentials(&entries).unwrap();
        for entry in &entries {
            assert!(matches!(entry.get_password(), Err(keyring::Error::NoEntry)));
        }
        forget_stored_credentials(&entries).unwrap();
    }
}
