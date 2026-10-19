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
    path::{Path, PathBuf},
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
    pub oauth_client_id: String,
    pub oauth_tenant: String,
    pub oauth_port: u16,
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
            service: if cfg!(unix) { "sftp" } else { "webdav" }.into(),
            endpoint: String::new(),
            root: "/".into(),
            bucket: String::new(),
            region: "us-east-1".into(),
            username: String::new(),
            key_file: String::new(),
            oauth_client_id: String::new(),
            oauth_tenant: "common".into(),
            oauth_port: 53682,
        }
    }
}

// Secrets are serialized only for the OS credential store, never into settings.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Secrets {
    pub password: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub access_token: String,
    pub refresh_token: String,
    pub oauth_client_secret: String,
    pub expires_at: u64,
}
pub const SERVICES: &[&str] = &[
    #[cfg(unix)]
    "sftp",
    "smb",
    "ftp",
    "webdav",
    "s3",
    "http",
    "dropbox",
    "gdrive",
    "onedrive",
    "gcs",
    "webhdfs",
];

struct Session {
    operator: Option<opendal::blocking::Operator>,
    smb: Option<crate::smb::Share>,
    secrets: Secrets,
    remember: bool,
}
impl Session {
    fn operator(&self) -> Result<&opendal::blocking::Operator> {
        self.operator
            .as_ref()
            .context("This operation requires the SMB adapter")
    }
}
static SESSIONS: OnceLock<RwLock<HashMap<String, Arc<Session>>>> = OnceLock::new();
static PROFILES: OnceLock<RwLock<HashMap<String, Connection>>> = OnceLock::new();
fn sessions() -> &'static RwLock<HashMap<String, Arc<Session>>> {
    SESSIONS.get_or_init(Default::default)
}
fn profiles() -> &'static RwLock<HashMap<String, Connection>> {
    PROFILES.get_or_init(Default::default)
}
pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
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
    #[cfg(not(unix))]
    if connection.service == "sftp" {
        bail!(
            "SFTP is currently supported only on macOS and Linux. Choose another service or mount the server as a local folder."
        );
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

pub(crate) fn clean_error(error: impl std::fmt::Display, secrets: &Secrets) -> anyhow::Error {
    let mut text = error.to_string();
    for secret in [
        &secrets.password,
        &secrets.access_key_id,
        &secrets.secret_access_key,
        &secrets.access_token,
        &secrets.refresh_token,
        &secrets.oauth_client_secret,
    ] {
        if !secret.is_empty() {
            text = text.replace(secret, "[redacted]");
        }
    }
    anyhow::anyhow!(text)
}

pub fn connect(connection: &Connection, secrets: Secrets, remember: bool) -> Result<()> {
    validate(connection)?;
    let (operator, smb) = if connection.service == "smb" {
        let share = crate::smb::Share::new(connection, &secrets)?;
        share.list(Path::new(""))?;
        (None, Some(share))
    } else {
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
        (Some(operator), None)
    };
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
            Arc::new(Session {
                operator,
                smb,
                secrets,
                remember,
            }),
        );
    Ok(())
}

fn session(id: &str) -> Result<Arc<Session>> {
    // Serialize reconnects to avoid spending a rotating refresh token twice.
    static RECONNECT: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let existing = sessions()
        .read()
        .map_err(|_| anyhow::anyhow!("Session lock failed"))?
        .get(id)
        .cloned();
    if let Some(session) = existing
        && (session.secrets.expires_at == 0
            || session.secrets.expires_at > crate::oauth::now() + 60)
    {
        return Ok(session);
    }
    let _reconnect = RECONNECT
        .lock()
        .map_err(|_| anyhow::anyhow!("Reconnect lock failed"))?;
    session_locked(id)
}

fn session_locked(id: &str) -> Result<Arc<Session>> {
    let existing = sessions()
        .read()
        .map_err(|_| anyhow::anyhow!("Session lock failed"))?
        .get(id)
        .cloned();
    if let Some(session) = existing {
        let session = session.clone();
        if session.secrets.expires_at > 0 && session.secrets.expires_at <= crate::oauth::now() + 60
        {
            let connection = profiles()
                .read()
                .map_err(|_| anyhow::anyhow!("Connection lock failed"))?
                .get(id)
                .cloned()
                .context("Connection profile missing")?;
            let secrets = crate::oauth::refresh(&connection, &session.secrets)?;
            let remember = session.remember;
            // Release the read guard before reconnecting and replacing the session.
            drop(session);
            // Refresh outside the session map lock.
            return refreshed_session(&connection, secrets, remember);
        }
        return Ok(session);
    }
    let connection = profiles()
        .read()
        .map_err(|_| anyhow::anyhow!("Connection lock failed"))?
        .get(id)
        .cloned()
        .context("Connection profile is missing. Open Connections to reconnect.")?;
    validate(&connection)?;
    let mut secrets = load_credentials(id).context(
        "Reconnect to this service in Connections; credentials are not available for this session",
    )?;
    if secrets.expires_at > 0 && secrets.expires_at <= crate::oauth::now() + 60 {
        secrets = crate::oauth::refresh(&connection, &secrets)?;
    }
    connect(&connection, secrets, true)?;
    sessions()
        .read()
        .map_err(|_| anyhow::anyhow!("Session lock failed"))?
        .get(id)
        .cloned()
        .context("Connection failed")
}

fn refreshed_session(
    connection: &Connection,
    secrets: Secrets,
    remember: bool,
) -> Result<Arc<Session>> {
    connect(connection, secrets, remember)?;
    sessions()
        .read()
        .map_err(|_| anyhow::anyhow!("Session lock failed"))?
        .get(&connection.id)
        .cloned()
        .context("Cloud reconnection failed")
}

pub fn rename(id: &str, source: &Path, name: &str) -> Result<()> {
    rename_with_task(id, source, name, &mut Task::default())
}

pub fn rename_with_task(id: &str, source: &Path, name: &str, task: &mut Task) -> Result<()> {
    crate::filesystem::validate_name(name)?;
    let parent = source.parent().unwrap_or(Path::new(""));
    let entry = list(id, parent)?
        .into_iter()
        .find(|entry| entry.path == source)
        .context("Remote source no longer exists")?;
    let location = Location::Remote {
        connection: id.into(),
        path: parent.into(),
    };
    let target = parent.join(name);
    if target == source {
        return Ok(());
    }
    ensure_absent(&location, &target)?;
    let before = snapshot(&location, std::slice::from_ref(&entry), task)?;
    copy_entry(&location, &entry, &location, &target, task)?;
    if snapshot(&location, std::slice::from_ref(&entry), task)? != before {
        bail!("Remote source changed during rename. Both copies were kept.");
    }
    delete(id, &[entry], task)
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

pub(crate) fn key(path: &Path, directory: bool) -> Result<String> {
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
    if let Some(smb) = &session.smb {
        return smb.list(path);
    }
    let directory = key(path, true)?;
    if !session.operator()?.info().full_capability().list {
        if path.as_os_str().is_empty() {
            bail!(
                "This service does not provide directory listings. Enter a full file path after remote://{id}/ to view or download it."
            );
        }
        let metadata = session
            .operator()?
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
        .operator()?
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
    if let Some(smb) = &session.smb {
        return smb.reader(path);
    }
    let reader = session
        .operator()?
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
    if let Some(smb) = &session.smb {
        return smb.mkdir(path);
    }
    session
        .operator()?
        .create_dir(&key(path, true)?)
        .map_err(|e| clean_error(e, &session.secrets))?;
    Ok(())
}

pub fn delete(id: &str, entries: &[Entry], task: &mut Task) -> Result<()> {
    fn remove(
        id: &str,
        session: &Session,
        entry: &Entry,
        task: &mut Task,
        visited: &mut std::collections::BTreeSet<PathBuf>,
        depth: usize,
    ) -> Result<()> {
        task.check()?;
        anyhow::ensure!(
            depth < 64 && visited.len() < 100_000,
            "Remote deletion exceeds the tree limit"
        );
        anyhow::ensure!(
            !entry.path.as_os_str().is_empty(),
            "Cannot delete a connection root"
        );
        let location = Location::Remote {
            connection: id.into(),
            path: PathBuf::new(),
        };
        let Some(current) = entry_at(&location, &entry.path)? else {
            return Ok(());
        };
        anyhow::ensure!(
            visited.insert(entry.path.clone()),
            "Remote deletion encountered a repeated path"
        );
        anyhow::ensure!(
            current.is_dir == entry.is_dir,
            "Remote deletion target type changed"
        );
        if let Some(smb) = &session.smb {
            smb.delete(&current, task)?;
        } else {
            // Recursive bulk deletion may issue RMD before descendants. FTP can
            // silently treat its 550 'directory not empty' as 'already absent'.
            // Visit children first and verify every reported deletion instead.
            if current.is_dir && !current.is_symlink {
                for child in list(id, &current.path)? {
                    anyhow::ensure!(
                        child.path.parent() == Some(current.path.as_path()),
                        "Server returned a child outside the selected folder"
                    );
                    remove(id, session, &child, task, visited, depth + 1)?;
                }
            }
            task.tick(&current.path, 0)?;
            session
                .operator()?
                .delete(&key(&current.path, current.is_dir)?)
                .map_err(|e| clean_error(e, &session.secrets))?;
        }
        anyhow::ensure!(
            entry_at(&location, &current.path)?.is_none(),
            "Server did not remove {}; completed deletions were kept",
            current.path.display()
        );
        Ok(())
    }
    let session = session(id)?;
    let mut visited = std::collections::BTreeSet::new();
    for entry in entries {
        remove(id, &session, entry, task, &mut visited, 0)?;
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

pub(crate) fn snapshot(location: &Location, entries: &[Entry], task: &Task) -> Result<Vec<String>> {
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
                if let Some(smb) = &session.smb {
                    let current = smb
                        .stat(&entry.path)?
                        .context("Remote source disappeared")?;
                    let mut version = format!(
                        "{}|{}|{:?}",
                        entry.path.display(),
                        current.size,
                        current.modified
                    );
                    if !current.is_dir {
                        use sha2::{Digest, Sha256};
                        let mut hash = Sha256::new();
                        let mut read = smb.reader(&entry.path)?;
                        let mut buffer = [0; 65536];
                        loop {
                            task.check()?;
                            let n = read.read(&mut buffer)?;
                            if n == 0 {
                                break;
                            }
                            hash.update(&buffer[..n]);
                        }
                        version.push_str(&format!("|{:x}", hash.finalize()));
                    }
                    result.push(version);
                    if current.is_dir {
                        stack.extend(smb.list(&entry.path)?);
                    }
                    continue;
                }
                let metadata = session
                    .operator()?
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

pub(crate) fn ensure_absent(location: &Location, path: &Path) -> Result<()> {
    match location {
        Location::Local(_) => {
            if operations::exists(path) {
                bail!("Destination already exists: {}", path.display());
            }
        }
        Location::Remote { connection, .. } => {
            let session = session(connection)?;
            if let Some(smb) = &session.smb {
                if smb.stat(path)?.is_some() {
                    bail!("Remote destination already exists: {}", path.display());
                }
                return Ok(());
            }
            match session.operator()?.stat(&key(path, false)?) {
                Ok(_) => bail!("Remote destination already exists: {}", path.display()),
                Err(error) if error.kind() == opendal::ErrorKind::NotFound => {}
                Err(error) => return Err(clean_error(error, &session.secrets)),
            }
        }
        _ => bail!("Archive destination is read-only"),
    }
    Ok(())
}

pub(crate) fn copy_entry(
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
                if let Some(smb) = &session.smb {
                    return smb.write(target, &mut input, task, true);
                }
                let options = opendal::options::WriteOptions {
                    if_not_exists: session
                        .operator()?
                        .info()
                        .full_capability()
                        .write_with_if_not_exists,
                    ..Default::default()
                };
                if !session.operator()?.info().full_capability().write_can_multi {
                    let bytes = crate::remote_upload::buffer(&mut input, &entry.path, task)?;
                    task.check()?;
                    session
                        .operator()?
                        .write_options(&key(target, false)?, bytes, options)
                        .map_err(|e| clean_error(e, &session.secrets))?;
                    task.check()?;
                    return Ok(());
                }
                let mut writer = session
                    .operator()?
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

pub(crate) fn entry_at(location: &Location, path: &Path) -> Result<Option<Entry>> {
    match location {
        Location::Local(_) => {
            if operations::exists(path) {
                Ok(Some(crate::filesystem::entry_from_path(path)?))
            } else {
                Ok(None)
            }
        }
        Location::Remote { connection, .. } => {
            let session = session(connection)?;
            if let Some(smb) = &session.smb {
                return smb.stat(path);
            }
            let metadata = match session.operator()?.stat(&key(path, false)?) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == opendal::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(clean_error(error, &session.secrets)),
            };
            Ok(Some(Entry {
                path: path.into(),
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                is_dir: metadata.is_dir(),
                is_symlink: false,
                size: metadata.content_length(),
                modified: metadata.last_modified().map(Into::into),
            }))
        }
        _ => bail!("Extract archives to a local folder first."),
    }
}

pub fn conflicts(
    from: &Location,
    entries: &[Entry],
    to: &Location,
) -> Result<Vec<crate::transfer::Conflict>> {
    conflicts_with_merge(from, entries, to, false)
}
pub fn conflicts_with_merge(
    from: &Location,
    entries: &[Entry],
    to: &Location,
    merge: bool,
) -> Result<Vec<crate::transfer::Conflict>> {
    fn visit(
        from: &Location,
        entry: &Entry,
        to: &Location,
        target: &Path,
        merge: bool,
        depth: usize,
        out: &mut Vec<crate::transfer::Conflict>,
    ) -> Result<()> {
        anyhow::ensure!(
            depth < 64 && out.len() < 5000,
            "Too many nested conflicts; select fewer files"
        );
        if let Some(existing) = entry_at(to, target)? {
            if merge && entry.is_dir && existing.is_dir && !entry.is_symlink && !existing.is_symlink
            {
                for child in filesystem::list(&crate::directory_tree::at(from, &entry.path))? {
                    visit(
                        from,
                        &child,
                        to,
                        &target.join(&child.name),
                        merge,
                        depth + 1,
                        out,
                    )?;
                }
            } else {
                out.push(crate::transfer::Conflict {
                    source: entry.path.clone(),
                    target: target.into(),
                    source_size: entry.size,
                    target_size: existing.size,
                });
            }
        }
        Ok(())
    }
    let base = crate::directory_tree::path(to);
    let mut out = Vec::new();
    for entry in entries {
        visit(from, entry, to, &base.join(&entry.name), merge, 0, &mut out)?;
    }
    Ok(out)
}
struct RemotePlan {
    entry: Entry,
    target: PathBuf,
    existing: Option<Entry>,
}
struct RemotePlanner<'a> {
    from: &'a Location,
    to: &'a Location,
    options: crate::transfer::TransferOptions,
    overrides: &'a std::collections::BTreeMap<PathBuf, crate::transfer::ConflictPolicy>,
    reserved: std::collections::BTreeSet<PathBuf>,
    plans: Vec<RemotePlan>,
    complete: Vec<bool>,
    skipped: usize,
    visited: usize,
}
impl RemotePlanner<'_> {
    fn visit(
        &mut self,
        entry: &Entry,
        mut target: PathBuf,
        root: usize,
        depth: usize,
        task: &Task,
    ) -> Result<()> {
        use crate::transfer::ConflictPolicy;
        task.check()?;
        self.visited += 1;
        anyhow::ensure!(
            self.visited <= 100_000 && depth < 64,
            "Remote transfer exceeds the tree limit"
        );
        anyhow::ensure!(!entry.is_symlink, "Remote transfers do not follow links");
        let existing = entry_at(self.to, &target)?;
        let policy = self
            .overrides
            .get(&target)
            .copied()
            .unwrap_or(self.options.conflicts);
        if let Some(existing) = &existing
            && self.options.merge_folders
            && entry.is_dir
            && existing.is_dir
            && !existing.is_symlink
        {
            for child in filesystem::list(&crate::directory_tree::at(self.from, &entry.path))? {
                self.visit(&child, target.join(&child.name), root, depth + 1, task)?;
            }
            return Ok(());
        }
        if existing.is_some() || self.reserved.contains(&target) {
            match policy {
                ConflictPolicy::Refuse => bail!("Destination already exists: {}", target.display()),
                ConflictPolicy::Skip => {
                    self.complete[root] = false;
                    self.skipped += 1;
                    return Ok(());
                }
                ConflictPolicy::Replace => anyhow::ensure!(
                    !self.reserved.contains(&target),
                    "Duplicate transfer target"
                ),
                ConflictPolicy::Rename => {
                    let original = target.clone();
                    let parent = target.parent().context("Missing target parent")?;
                    let stem = original.file_stem().unwrap_or_default().to_string_lossy();
                    let extension = original
                        .extension()
                        .map(|e| format!(".{}", e.to_string_lossy()))
                        .unwrap_or_default();
                    let mut found = false;
                    for n in 1..=100_000 {
                        let candidate = parent.join(format!("{stem} ({n}){extension}"));
                        if !self.reserved.contains(&candidate)
                            && entry_at(self.to, &candidate)?.is_none()
                        {
                            target = candidate;
                            found = true;
                            break;
                        }
                    }
                    anyhow::ensure!(found, "Cannot find an unused target name");
                }
            }
        }
        self.reserved.insert(target.clone());
        self.plans.push(RemotePlan {
            entry: entry.clone(),
            target,
            existing: if policy == ConflictPolicy::Replace {
                existing
            } else {
                None
            },
        });
        Ok(())
    }
}
pub fn transfer_resolved(
    from: &Location,
    entries: &[Entry],
    to: &Location,
    moving: bool,
    options: crate::transfer::TransferOptions,
    overrides: &std::collections::BTreeMap<PathBuf, crate::transfer::ConflictPolicy>,
    task: &mut Task,
) -> Result<String> {
    if let (Location::Local(_), Location::Local(destination)) = (from, to) {
        return crate::transfer::transfer_resolved(
            &entries.iter().map(|e| e.path.clone()).collect::<Vec<_>>(),
            destination,
            moving,
            options,
            overrides,
            task,
        );
    }
    anyhow::ensure!(!entries.is_empty(), "Select files to transfer");
    anyhow::ensure!(
        !matches!(from, Location::Archive { .. }) && !matches!(to, Location::Archive { .. }),
        "Extract archive entries first"
    );
    let base = crate::directory_tree::path(to);
    let mut planner = RemotePlanner {
        from,
        to,
        options,
        overrides,
        reserved: Default::default(),
        plans: Vec::new(),
        complete: vec![true; entries.len()],
        skipped: 0,
        visited: 0,
    };
    for (root, entry) in entries.iter().enumerate() {
        let target = base.join(&entry.name);
        let same = matches!((from,to),(Location::Remote{connection:a,..},Location::Remote{connection:b,..}) if a==b);
        anyhow::ensure!(
            !(same && (target == entry.path || (entry.is_dir && target.starts_with(&entry.path)))),
            "Cannot transfer an item into itself"
        );
        planner.visit(entry, target, root, 0, task)?;
    }
    let before = if moving {
        Some(snapshot(from, entries, task)?)
    } else {
        None
    };
    let mut backups = Vec::new();
    for plan in &planner.plans {
        task.check()?;
        if let Some(existing) = &plan.existing {
            let backup = plan
                .target
                .parent()
                .context("Missing target parent")?
                .join(format!(
                    ".rust-commander-replaced-{:016x}-{}",
                    rand::random::<u64>(),
                    existing.name
                ));
            let stamp = snapshot(to, std::slice::from_ref(existing), task)?;
            ensure_absent(to, &backup)?;
            copy_entry(to, existing, to, &backup, task)?;
            backups.push(backup.clone());
            anyhow::ensure!(
                snapshot(to, std::slice::from_ref(existing), task)? == stamp,
                "Destination changed; recovery retained at {}",
                backup.display()
            );
            remove_entry(to, existing, task)?;
        }
        if matches!(to, Location::Local(_)) {
            task.capture(&plan.target)?;
        }
        if let Err(e) = copy_entry(from, &plan.entry, to, &plan.target, task) {
            bail!("Transfer failed: {e:#}. Recovery copies: {backups:?}");
        }
    }
    if moving {
        anyhow::ensure!(
            Some(snapshot(from, entries, task)?) == before,
            "Sources changed. Originals and completed copies were kept"
        );
        for (entry, complete) in entries.iter().zip(&planner.complete) {
            if *complete {
                remove_entry(from, entry, task)?;
            }
        }
    }
    Ok(format!(
        "Transferred {} item(s); skipped {} conflict(s). Source folders with skipped items remain intact. Recovery copies: {backups:?}",
        planner.plans.len(),
        planner.skipped
    ))
}
pub(crate) fn remove_entry(location: &Location, entry: &Entry, task: &mut Task) -> Result<()> {
    task.check()?;
    match location {
        Location::Local(_) => {
            task.capture(&entry.path)?;
            operations::remove(&entry.path)
        }
        Location::Remote { connection, .. } => {
            delete(connection, std::slice::from_ref(entry), task)
        }
        _ => bail!("Archive entries are read-only"),
    }
}

/// Guarded remote editor save. Keep a server-side recovery copy before uploading.
pub fn save_text(
    id: &str,
    path: &Path,
    original: &[u8],
    text: &str,
    task: &mut Task,
) -> Result<String> {
    save_bytes(id, path, original, text.as_bytes(), task)
}
pub fn save_bytes(
    id: &str,
    path: &Path,
    original: &[u8],
    bytes: &[u8],
    task: &mut Task,
) -> Result<String> {
    if original.len() > 2 * 1024 * 1024 || bytes.len() > 2 * 1024 * 1024 {
        bail!("Remote text editing is limited to 2 MiB");
    }
    task.check()?;
    if read_bytes(id, path, 2 * 1024 * 1024)? != original {
        bail!("Remote file changed since it was opened. Reload before saving.");
    }
    let session = session(id)?;
    let backup = path.with_file_name(format!(
        ".rust-commander-edit-{:016x}.bak",
        rand::random::<u64>()
    ));
    if let Some(smb) = &session.smb {
        smb.write(&backup, &mut std::io::Cursor::new(original), task, true)?;
        if read_bytes(id, path, 2 * 1024 * 1024)? != original {
            bail!(
                "Remote file changed before upload. Recovery copy: {}",
                backup.display()
            );
        }
        smb.write(path, &mut std::io::Cursor::new(bytes), task, false)?;
    } else {
        let operator = session.operator()?;
        let metadata = operator.stat(&key(path, false)?)?;
        operator
            .write_options(
                &key(&backup, false)?,
                original.to_vec(),
                opendal::options::WriteOptions {
                    if_not_exists: operator.info().full_capability().write_with_if_not_exists,
                    ..Default::default()
                },
            )
            .map_err(|e| clean_error(e, &session.secrets))?;
        if read_bytes(id, path, 2 * 1024 * 1024)? != original {
            bail!(
                "Remote file changed before upload. Recovery copy: {}",
                backup.display()
            );
        }
        let options = opendal::options::WriteOptions {
            if_match: if operator.info().full_capability().write_with_if_match {
                metadata.etag().map(str::to_owned)
            } else {
                None
            },
            ..Default::default()
        };
        operator
            .write_options(&key(path, false)?, bytes.to_vec(), options)
            .map_err(|e| clean_error(e, &session.secrets))?;
    }
    if read_bytes(id, path, 2 * 1024 * 1024)? != bytes {
        bail!(
            "Uploaded content did not verify. Recovery copy: {}",
            backup.display()
        );
    }
    Ok(format!(
        "Saved remote text. Recovery copy: {}. Services without conditional writes cannot make the external-edit check atomic.",
        backup.display()
    ))
}
pub fn search(
    id: &str,
    options: &crate::desktop::SearchOptions,
    task: &mut Task,
) -> Result<crate::desktop::SearchResult> {
    let matcher = globset::GlobBuilder::new(&options.pattern)
        .case_insensitive(true)
        .build()?
        .compile_matcher();
    let mut result = crate::desktop::SearchResult {
        paths: Vec::new(),
        errors: Vec::new(),
        truncated: false,
    };
    let mut stack = vec![(options.root.clone(), 0)];
    let mut visited = std::collections::BTreeSet::new();
    let mut count = 0;
    while let Some((folder, depth)) = stack.pop() {
        task.check()?;
        if !visited.insert(folder.clone()) {
            continue;
        }
        let entries = match list(id, &folder) {
            Ok(e) => e,
            Err(e) => {
                if result.errors.len() < 1000 {
                    result.errors.push(format!("{}: {e:#}", folder.display()));
                }
                continue;
            }
        };
        for entry in entries {
            task.check()?;
            count += 1;
            if count > 100_000 || depth > 256 {
                result.truncated = true;
                return Ok(result);
            }
            if !options.hidden && entry.name.starts_with('.') {
                continue;
            }
            if entry.is_dir {
                stack.push((entry.path.clone(), depth + 1));
            }
            if !matcher.is_match(&entry.name)
                && !matcher.is_match(
                    entry
                        .path
                        .strip_prefix(&options.root)
                        .unwrap_or(&entry.path),
                )
            {
                continue;
            }
            if !options.content.is_empty() {
                if entry.is_dir {
                    continue;
                }
                match read_bytes(id, &entry.path, 2 * 1024 * 1024) {
                    Ok(bytes) if String::from_utf8_lossy(&bytes).contains(&options.content) => {}
                    Ok(_) => continue,
                    Err(e) => {
                        if result.errors.len() < 1000 {
                            result
                                .errors
                                .push(format!("{}: {e:#}", entry.path.display()));
                        }
                        continue;
                    }
                }
            }
            result.paths.push(entry.path);
            if result.paths.len() >= options.max_results {
                result.truncated = true;
                return Ok(result);
            }
        }
    }
    Ok(result)
}

/// Start a new ranged reader. Backends without native range support may emulate it.
pub fn read_range(id: &str, path: &Path, start: u64) -> Result<Box<dyn Read>> {
    let session = session(id)?;
    if let Some(smb) = &session.smb {
        return smb.reader_at(path, start);
    }
    let reader = session
        .operator()?
        .reader(&key(path, false)?)
        .map_err(|e| clean_error(e, &session.secrets))?;
    Ok(Box::new(
        reader
            .into_std_read(start..)
            .map_err(|e| clean_error(e, &session.secrets))?,
    ))
}
pub(crate) fn upload_checkpoint(
    id: &str,
    target: &Path,
    source: &Path,
    offset: u64,
    task: &mut Task,
) -> Result<()> {
    use std::io::{Seek, SeekFrom};
    let session = session(id)?;
    let mut input = File::open(source)?;
    if let Some(smb) = &session.smb {
        if offset > 0
            && let Some(entry) = smb.stat(target)?
        {
            smb.delete(&entry, task)?;
        }
        return smb.write(target, &mut input, task, true);
    }
    let op = session.operator()?;
    let append = offset > 0 && op.info().full_capability().write_can_append;
    if offset > 0 && !append {
        op.delete(&key(target, false)?)
            .map_err(|e| clean_error(e, &session.secrets))?;
    }
    if append {
        input.seek(SeekFrom::Start(offset))?;
    }
    let options = opendal::options::WriteOptions {
        append,
        if_not_exists: !append && op.info().full_capability().write_with_if_not_exists,
        ..Default::default()
    };
    if !op.info().full_capability().write_can_multi {
        let bytes = crate::remote_upload::buffer(&mut input, source, task)?;
        task.check()?;
        op.write_options(&key(target, false)?, bytes, options)
            .map_err(|e| clean_error(e, &session.secrets))?;
        task.check()?;
        return Ok(());
    }
    let mut writer = op
        .writer_options(&key(target, false)?, options)
        .map_err(|e| clean_error(e, &session.secrets))?
        .into_std_write();
    task.stream(&mut input, &mut writer, source)?;
    writer
        .close()
        .map_err(|e| clean_error(e, &session.secrets))?;
    Ok(())
}
pub(crate) fn publish_checkpoint(
    id: &str,
    stage: &Path,
    target: &Path,
    task: &mut Task,
) -> Result<()> {
    let location = Location::Remote {
        connection: id.into(),
        path: PathBuf::new(),
    };
    ensure_absent(&location, target)?;
    task.check()?;
    let session = session(id)?;
    if let Some(smb) = &session.smb {
        return smb.publish_checkpoint(stage, target);
    }
    if session.operator()?.info().scheme() == opendal::Scheme::Fs {
        let info = session.operator()?.info();
        let root = PathBuf::from(info.root());
        crate::desktop::rename_exclusive(
            &root.join(filesystem::safe_relative(stage)?),
            &root.join(filesystem::safe_relative(target)?),
        )?;
        return Ok(());
    }
    // Publish through exclusive-create where the provider supports it. A plain
    // provider rename can overwrite a destination created by another client.
    let entry = entry_at(&location, stage)?.context("Transfer stage disappeared")?;
    copy_entry(&location, &entry, &location, target, task)?;
    remove_entry(&location, &entry, task)
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
