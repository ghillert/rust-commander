//! Connections and file operations for remote storage.
//!
//! A [`Connection`] describes a service and its root. [`connect`] opens a
//! session using [`Secrets`]. Pass the connection ID and a path relative to
//! that root to [`list`], [`read`], and [`mkdir`]. An empty path denotes the root.
//!
//! [`transfer`] copies or moves entries into unused names. To review collisions,
//! use [`conflicts_with_merge`] and supply the decisions to [`transfer_resolved`].

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

/// Identifies a container on a local Docker engine for read-only browsing.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct DockerTarget {
    /// Local Docker socket or Windows named-pipe endpoint.
    pub endpoint: String,
    /// Full 64-character hexadecimal container ID.
    pub container: String,
}

/// A serializable storage profile. Locations refer to it by [`id`](Self::id),
/// and [`connect`] accepts its credentials separately.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Connection {
    /// Stable key shared by saved locations, live sessions, and stored credentials.
    pub id: String,
    /// Display name used in the sidebar and at the connection root.
    pub name: String,
    /// Backend identifier, such as `sftp`, `webdav`, or `s3`.
    pub service: String,
    /// Server or service address in the format expected by the backend.
    pub endpoint: String,
    /// Directory or object prefix exposed as the root of the connection.
    pub root: String,
    /// Bucket, SMB share, or device identifier, depending on the service.
    pub bucket: String,
    /// Storage region for services that use one.
    pub region: String,
    /// Login name supplied to the service.
    pub username: String,
    /// Path to a private key for SSH authentication.
    pub key_file: String,
    /// Public OAuth client identifier for this profile.
    pub oauth_client_id: String,
    /// Microsoft tenant used for OneDrive authorization.
    pub oauth_tenant: String,
    /// Local loopback port used to receive the OAuth redirect.
    pub oauth_port: u16,
    /// Named connections are shown in the sidebar; URI descriptors restore tabs only.
    pub listed: bool,
    /// Path within the connection to open when the profile is selected.
    pub initial_path: PathBuf,
    /// Parent server profile for a share opened from the Network list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_parent: Option<String>,
    /// Engine and container identity for a Docker location.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docker: Option<DockerTarget>,
    /// Requested FTP transport security, resolved by [`Connection::tls_mode`].
    pub ftp_tls: FtpTls,
    /// Whether opening the HTTP resource requires a session-supplied query string.
    pub query_required: bool,
    /// Native SFTP preference recorded in the profile. The `sftp` service uses the native backend.
    pub native_sftp: bool,
    /// Optional SSH known-hosts file used to verify server keys.
    pub known_hosts_file: String,
    /// Whether to consult SSH configuration when resolving the host.
    pub ssh_use_config: bool,
    /// Optional SSH configuration file to use for host lookup.
    pub ssh_config_file: String,
    /// Optional DER-encoded CA certificate trusted by the FTPS connection.
    pub tls_ca_file: String,
    /// Profile supplying credentials for this navigation, such as the parent of a network share.
    #[serde(skip)]
    #[doc(hidden)]
    pub credentials_from: String,
}
impl Default for Connection {
    fn default() -> Self {
        Self {
            // Wall-clock timestamps can repeat, especially when connections are
            // created concurrently. IDs key both saved profiles and live sessions.
            id: format!("{:032x}", rand::random::<u128>()),
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
            listed: true,
            initial_path: PathBuf::new(),
            network_parent: None,
            docker: None,
            ftp_tls: FtpTls::Automatic,
            query_required: false,
            native_sftp: false,
            known_hosts_file: String::new(),
            ssh_use_config: true,
            ssh_config_file: String::new(),
            tls_ca_file: String::new(),
            credentials_from: String::new(),
        }
    }
}

/// How an FTP connection establishes transport security.
#[derive(Clone, Copy, Default, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum FtpTls {
    /// Choose the mode from the endpoint as described by [`Connection::tls_mode`].
    #[default]
    Automatic,
    /// Use unencrypted FTP.
    Plain,
    /// Start an FTP connection and negotiate TLS before login.
    Explicit,
    /// Establish TLS as soon as the connection opens.
    Implicit,
}
impl Connection {
    /// Returns whether the application offers writes for this service type.
    /// The server's permissions still apply to each operation.
    pub fn writable(&self) -> bool {
        !matches!(self.service.as_str(), "http" | "adb" | "nfs" | "docker")
    }

    pub(crate) fn uses_local_identity(&self) -> bool {
        matches!(self.service.as_str(), "adb" | "nfs" | "docker")
    }

    /// Returns the configured FTP mode, resolving [`FtpTls::Automatic`] to
    /// [`FtpTls::Plain`] for a `ftp://` endpoint and [`FtpTls::Explicit`] otherwise.
    pub fn tls_mode(&self) -> FtpTls {
        match self.ftp_tls {
            FtpTls::Automatic if self.endpoint.starts_with("ftp://") => FtpTls::Plain,
            FtpTls::Automatic => FtpTls::Explicit,
            mode => mode,
        }
    }
}

/// Authentication material for a session. [`save_credentials`] serializes the
/// persistable fields to the OS credential store. Request-specific fields stay in memory.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Secrets {
    /// Login password for password-based authentication.
    pub password: String,
    /// Passphrase used to unlock an encrypted SSH private key.
    pub ssh_passphrase: String,
    /// Access key identifier for object storage.
    pub access_key_id: String,
    /// Secret paired with [`Secrets::access_key_id`].
    pub secret_access_key: String,
    /// Access token used for authenticated cloud requests.
    pub access_token: String,
    /// Token used to renew expiring access tokens.
    pub refresh_token: String,
    /// Client secret for OAuth providers that require one.
    pub oauth_client_secret: String,
    /// Access-token expiry in Unix seconds, or zero when no expiry was supplied.
    pub expires_at: u64,
    /// URL queries may contain signed credentials. Never serialize them, even to keyring.
    #[serde(skip)]
    pub request_query: String,
    /// HTTP resource path to which [`Secrets::request_query`] belongs.
    #[serde(skip)]
    pub request_path: String,
    /// Host-key review state for the current SSH connection attempt.
    #[serde(skip)]
    pub ssh_review: Option<crate::ssh::Review>,
}
/// Storage service identifiers offered by the connection editor.
pub const SERVICES: &[&str] = &[
    "sftp", "smb", "ftp", "webdav", "nfs", "adb", "s3", "http", "dropbox", "gdrive", "onedrive",
    "gcs", "webhdfs",
];

struct Session {
    operator: Option<opendal::blocking::Operator>,
    http_streaming: Option<opendal::Operator>,
    smb: Option<crate::smb::Share>,
    secrets: Secrets,
    remember: std::sync::atomic::AtomicBool,
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

/// Adds profiles to the in-memory registry, replacing entries with matching IDs.
/// Call [`connect`] to open a session with explicit credentials.
///
/// # Examples
///
/// Registering a share's profile gives its root a parent in the Network browser.
///
/// ```
/// use rust_commander::filesystem::Location;
/// use rust_commander::remote::{self, Connection};
///
/// let share = Connection {
///     service: "smb".into(),
///     network_parent: Some("office-server".into()),
///     ..Connection::default()
/// };
/// remote::register_profiles(std::slice::from_ref(&share));
/// let root = Location::Remote { connection: share.id, path: Default::default() };
/// let server = Location::Network { server: Some("office-server".into()) };
/// assert_eq!(root.parent(), Some(server.clone()));
/// assert_eq!(server.parent(), Some(Location::Network { server: None }));
/// ```
pub fn register_profiles(connections: &[Connection]) {
    if let Ok(mut map) = profiles().write() {
        for c in connections {
            map.insert(c.id.clone(), c.clone());
        }
    }
}

/// Returns the registered display name, falling back to `id` if the profile is unavailable.
pub fn connection_name(id: &str) -> String {
    profiles()
        .read()
        .ok()
        .and_then(|map| map.get(id).map(|c| c.name.clone()))
        .unwrap_or_else(|| id.to_owned())
}

/// Returns a copy of the registered profile, or `None` when it is unavailable.
pub fn profile(id: &str) -> Option<Connection> {
    profiles().read().ok()?.get(id).cloned()
}

pub(crate) fn navigation_credentials(
    id: &str,
    mut provided: Secrets,
    stored: bool,
) -> Result<Secrets> {
    let saved = sessions()
        .read()
        .ok()
        .and_then(|m| m.get(id).map(|s| s.secrets.clone()))
        .or_else(|| {
            if stored {
                load_credentials(id).ok()
            } else {
                None
            }
        });
    if let Some(saved) = saved {
        if provided.request_query.is_empty() && provided.request_path == saved.request_path {
            provided.request_query = saved.request_query;
        }
        if provided.password.is_empty() {
            provided.password = saved.password;
        }
        if provided.ssh_passphrase.is_empty() {
            provided.ssh_passphrase = saved.ssh_passphrase;
        }
        if provided.access_key_id.is_empty() {
            provided.access_key_id = saved.access_key_id;
        }
        if provided.secret_access_key.is_empty() {
            provided.secret_access_key = saved.secret_access_key;
        }
        if provided.access_token.is_empty() {
            provided.access_token = saved.access_token;
        }
        if provided.refresh_token.is_empty() {
            provided.refresh_token = saved.refresh_token;
        }
        if provided.oauth_client_secret.is_empty() {
            provided.oauth_client_secret = saved.oauth_client_secret;
        }
        provided.expires_at = saved.expires_at;
    }
    Ok(provided)
}

/// Checks a profile's identifying fields and service-specific addressing before connection.
///
/// # Errors
///
/// Rejects missing names or IDs, unsupported services, invalid Docker targets,
/// and endpoint URLs with embedded credentials or a query string.
pub fn validate(connection: &Connection) -> Result<()> {
    if connection.name.trim().is_empty() || connection.id.is_empty() {
        bail!("Enter a connection name.");
    }
    if connection.service == "docker" {
        return crate::uri_backend::docker::validate_target(
            connection
                .docker
                .as_ref()
                .context("Missing Docker container target")?,
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

pub(crate) fn clean_error(
    error: impl std::fmt::Display + 'static,
    secrets: &Secrets,
) -> anyhow::Error {
    let mut text = error.to_string();
    for secret in [
        &secrets.password,
        &secrets.ssh_passphrase,
        &secrets.access_key_id,
        &secrets.secret_access_key,
        &secrets.access_token,
        &secrets.refresh_token,
        &secrets.oauth_client_secret,
        &secrets.request_query,
    ] {
        if !secret.is_empty() {
            text = text.replace(secret, "[redacted]");
        }
    }
    crate::transfer_control::SanitizedError {
        message: text,
        temporary: crate::transfer_control::temporary_any(&error),
    }
    .into()
}

/// Opens a storage session and registers it under [`Connection::id`].
///
/// Services that support listing or inspecting the root are probed during
/// connection. HTTP access is checked when the requested resource is opened.
/// Set `remember` to store credentials for services that use them.
///
/// # Errors
///
/// Fails if validation, backend setup, the root probe, or credential storage
/// fails. Docker targets are opened through the device browser.
pub fn connect(connection: &Connection, secrets: Secrets, remember: bool) -> Result<()> {
    validate(connection)?;
    anyhow::ensure!(
        connection.service != "docker",
        "Open Docker containers from Devices."
    );
    let mut http_streaming = None;
    let (operator, smb) = if connection.service == "smb" {
        let share = crate::smb::Share::new(connection, &secrets)?;
        share.list(Path::new(""))?;
        (None, Some(share))
    } else {
        let _guard = runtime().enter();
        let operator = match crate::uri_backend::operator(connection, &secrets)
            .map_err(|e| clean_error(e, &secrets))?
        {
            Some(operator) => operator,
            None => opendal::Operator::via_iter(&connection.service, options(connection, &secrets))
                .map_err(|e| clean_error(e, &secrets))?,
        };
        let operator = operator.layer(
            opendal::layers::TimeoutLayer::new()
                .with_timeout(std::time::Duration::from_secs(30))
                .with_io_timeout(std::time::Duration::from_secs(30)),
        );
        if connection.service == "http" {
            http_streaming = Some(operator.clone());
        }
        let operator = opendal::blocking::Operator::new(operator)?;
        // Validate the credentials against the service, before storing them.
        if connection.service == "http" {
            // HTTP validates the requested resource, rather than an unrelated server root.
        } else if operator.info().full_capability().list {
            operator.list("").map_err(|e| clean_error(e, &secrets))?;
        } else if operator.info().full_capability().stat {
            operator.stat("").map_err(|e| clean_error(e, &secrets))?;
        }
        (Some(operator), None)
    };
    if remember && !connection.uses_local_identity() {
        save_credentials(&connection.id, &secrets)?;
    }
    profiles()
        .write()
        .map_err(|_| anyhow::anyhow!("Connection lock failed"))?
        .insert(connection.id.clone(), connection.clone());
    store_session(
        &connection.id,
        Session {
            operator,
            http_streaming,
            smb,
            secrets,
            remember: std::sync::atomic::AtomicBool::new(remember),
        },
    )
}

pub(crate) fn keep_server_credentials(id: &str, secrets: Secrets) -> Result<()> {
    store_session(
        id,
        Session {
            operator: None,
            http_streaming: None,
            smb: None,
            secrets,
            remember: std::sync::atomic::AtomicBool::new(false),
        },
    )
}

fn store_session(id: &str, session: Session) -> Result<()> {
    sessions()
        .write()
        .map_err(|_| anyhow::anyhow!("Session lock failed"))?
        .insert(id.into(), Arc::new(session));
    Ok(())
}

fn session(id: &str) -> Result<Arc<Session>> {
    if let Some(profile) = profile(id)
        && profile.service == "docker"
    {
        validate(&profile)?;
        let _runtime = runtime().enter();
        let secrets = Secrets::default();
        let operator =
            crate::uri_backend::operator(&profile, &secrets)?.context("Missing Docker backend")?;
        return Ok(Arc::new(Session {
            operator: Some(opendal::blocking::Operator::new(operator)?),
            http_streaming: None,
            smb: None,
            secrets,
            remember: Default::default(),
        }));
    }
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
            let remember = session.remember.load(std::sync::atomic::Ordering::Relaxed);
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
    let stored = if connection.uses_local_identity() {
        Err(keyring::Error::NoEntry.into())
    } else {
        load_credentials(id)
    };
    let remember = stored.is_ok();
    let mut secrets = stored.or_else(|error| {
        if matches!(error.downcast_ref::<keyring::Error>(),Some(keyring::Error::NoEntry)) {Ok(Secrets::default())} else {Err(error)}
    }).context(
        "Reconnect to this service in Connections; credentials are not available for this session",
    )?;
    if secrets.expires_at > 0 && secrets.expires_at <= crate::oauth::now() + 60 {
        secrets = crate::oauth::refresh(&connection, &secrets)?;
    }
    connect(&connection, secrets, remember)?;
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

/// Renames a remote entry within its parent using a default [`Task`].
/// See [`rename_with_task`] for conflict and failure behavior.
pub fn rename(id: &str, source: &Path, name: &str) -> Result<()> {
    rename_with_task(id, source, name, &mut Task::default())
}

/// Renames an entry by copying it to an unused sibling name, rechecking the
/// source snapshot, and deleting the source. Renaming an entry to itself succeeds.
///
/// # Errors
///
/// Rejects an invalid name, a missing source, or an occupied destination.
/// A source change detected after copying leaves both versions available.
/// Cancellation or a backend error may leave a partially completed rename.
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

/// Writes the persistable fields of [`Secrets`] to the OS credential store under `id`.
pub fn save_credentials(id: &str, secrets: &Secrets) -> Result<()> {
    keyring::Entry::new(CREDENTIAL_SERVICE, id)?.set_password(&serde_json::to_string(secrets)?)?;
    Ok(())
}
/// Reads credentials for `id`, trying the legacy service name if the current entry is missing.
///
/// # Errors
///
/// Returns an error if neither entry exists or the stored credentials cannot be read or decoded.
pub fn load_credentials(id: &str) -> Result<Secrets> {
    read_stored_credentials(
        &keyring::Entry::new(CREDENTIAL_SERVICE, id)?,
        &keyring::Entry::new(LEGACY_CREDENTIAL_SERVICE, id)?,
    )
}

pub(crate) fn remember_session(id: &str) -> Result<()> {
    let session = session(id)?;
    save_credentials(id, &session.secrets)?;
    session
        .remember
        .store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
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

/// Removes current and legacy credentials for `id`, then evicts its live session.
/// A missing credential entry is accepted, so the call can be repeated.
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

/// Joins normal path components with `/` for display. Non-UTF-8 names are
/// rendered lossily and root, prefix, and parent components are omitted.
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

/// Lists the children of a directory relative to the connection root.
/// For services without directory listings, a nonempty path requests metadata
/// for that resource and returns it as a single entry.
///
/// # Errors
///
/// Fails for an invalid path, a session or backend error, or a root listing
/// on a service that cannot enumerate children.
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
            size_known: true,
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
        .filter(|e| {
            !e.metadata().is_dir()
                || e.path().trim_end_matches('/') != directory.trim_end_matches('/')
        })
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
                size_known: metadata.user_metadata().is_none_or(|m| {
                    m.get("rust-commander-size-known")
                        .is_none_or(|v| v != "false")
                }),
                size: metadata.content_length(),
                modified: metadata.last_modified().map(std::time::SystemTime::from),
            })
        })
        .collect()
}

// OpenDAL's seekable std reader stats the object to get an end offset.
// HTTP can omit Content-Length, so consume its raw, timeout-layered stream to
// EOF instead. The shared accessor retains authenticated/query-bearing links.
struct HttpStream {
    reader: opendal::raw::oio::Reader,
    chunk: std::io::Cursor<Vec<u8>>,
}
impl Read for HttpStream {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        if self.chunk.position() == self.chunk.get_ref().len() as u64 {
            let bytes = runtime()
                .block_on(self.reader.read_dyn())
                .map_err(std::io::Error::other)?;
            self.chunk = std::io::Cursor::new(bytes.to_vec());
        }
        self.chunk.read(output)
    }
}
fn http_stream(session: &Session, path: &Path, offset: u64) -> Result<Option<Box<dyn Read>>> {
    use opendal::raw::Access;
    let Some(operator) = &session.http_streaming else {
        return Ok(None);
    };
    let (_, reader) = runtime()
        .block_on(operator.inner().read(
            &key(path, false)?,
            opendal::raw::OpRead::new().with_range(opendal::raw::BytesRange::new(offset, None)),
        ))
        .map_err(|e| clean_error(e, &session.secrets))?;
    Ok(Some(Box::new(HttpStream {
        reader,
        chunk: Default::default(),
    })))
}

/// Opens a reader at the beginning of a file under the connection root.
/// Errors can occur while opening the file or later as the reader consumes it.
pub fn read(id: &str, path: &Path) -> Result<Box<dyn Read>> {
    let session = session(id)?;
    if let Some(reader) = http_stream(&session, path, 0)? {
        return Ok(reader);
    }
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

/// Reads a complete remote file into memory. Choose a `limit` below `u64::MAX`
/// to allow a one-byte check for excess data. Use [`read`] for streaming.
///
/// # Errors
///
/// Returns an error if the file exceeds `limit` bytes or if reading fails.
pub fn read_bytes(id: &str, path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    read(id, path)?.take(limit + 1).read_to_end(&mut data)?;
    if data.len() as u64 > limit {
        bail!("Remote file exceeds the viewer size limit.");
    }
    Ok(data)
}

/// Creates a directory at `path` relative to the connection root.
/// The service determines whether missing parent directories can be created.
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

/// Permanently deletes selected remote entries, removing directory contents first.
/// A selected path that is already absent is treated as complete.
///
/// # Errors
///
/// Refuses to delete the connection root or an entry whose directory/file type
/// changed. A backend error or cancellation can leave part of the selection deleted.
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

/// Copies entries into an existing local or remote directory, or moves them
/// when `moving` is true. Top-level names must be unused and distinct.
/// Use [`transfer_resolved`] to choose how to handle collisions.
///
/// For a move involving a remote endpoint, all copies finish and the source
/// snapshots are checked again before removal begins. Local-to-local transfers
/// use the [local file operations](crate::operations).
///
/// # Errors
///
/// Fails for an empty selection, occupied targets, or archive locations.
/// Remote transfers reject entries identified as symlinks. Completed copies
/// can remain after a source change, cancellation, or I/O failure.
///
/// # Panics
///
/// Passing a network or container discovery list as a file endpoint can panic.
/// Open a concrete local or remote directory first.
pub fn transfer(
    from: &Location,
    entries: &[Entry],
    to: &Location,
    moving: bool,
    task: &mut Task,
) -> Result<()> {
    let _docker = from
        .docker_target()
        .as_ref()
        .map(crate::uri_backend::docker::attach);
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
    if task.needs_total() {
        task.plan_total(operations::totals::entries_total(from, entries, task)?);
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
                if !entry.is_dir
                    && profile(connection).is_some_and(|c| c.service == "http")
                    && (metadata.user_metadata().is_some_and(|m| {
                        m.get("rust-commander-size-known")
                            .is_some_and(|v| v == "false")
                    }) || (metadata.etag().is_none() && metadata.last_modified().is_none()))
                {
                    use sha2::{Digest, Sha256};
                    let mut input = read(connection, &entry.path)?;
                    let mut hash = Sha256::new();
                    let mut length = 0u64;
                    let mut buffer = [0; 65536];
                    loop {
                        task.check()?;
                        let n = input.read(&mut buffer)?;
                        if n == 0 {
                            break;
                        }
                        length += n as u64;
                        hash.update(&buffer[..n]);
                    }
                    result.push(format!(
                        "{}|{length}|{:x}",
                        display_path(&entry.path),
                        hash.finalize()
                    ));
                }
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
                size_known: metadata.user_metadata().is_none_or(|m| {
                    m.get("rust-commander-size-known")
                        .is_none_or(|v| v != "false")
                }),
                size: metadata.content_length(),
                modified: metadata.last_modified().map(Into::into),
            }))
        }
        _ => bail!("Extract archives to a local folder first."),
    }
}

/// Lists occupied top-level destination names for a selection. A pair of
/// matching directories counts as one conflict. See [`conflicts_with_merge`]
/// for a preview of conflicts within those directories.
pub fn conflicts(
    from: &Location,
    entries: &[Entry],
    to: &Location,
) -> Result<Vec<crate::transfer::Conflict>> {
    conflicts_with_merge(from, entries, to, false)
}
/// Previews collisions between `entries` and existing paths at `to`.
///
/// With `merge` enabled, descends into matching directories when both entries
/// are identified as directories and neither is a symlink. Each
/// [`target`](crate::transfer::Conflict::target) can be passed as a decision
/// key to [`transfer_resolved`]. A preview does not reserve those paths.
///
/// # Errors
///
/// Fails if an entry cannot be inspected or traversal reaches the depth or
/// conflict-count limit.
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
/// Copies or moves a selection using per-destination conflict decisions.
///
/// Use the destination paths from [`conflicts_with_merge`] as `overrides` keys.
/// For a remote destination, each key is relative to the connection root and
/// includes the destination directory. The result summarizes completed work,
/// skipped conflicts, and the paths of any replacement backups.
///
/// When a remote endpoint participates and `merge_folders` is enabled, matching
/// directories are merged. Each child uses its own override or the batch policy.
/// Replacing an entry copies the existing destination to a sibling and registers
/// it for [recovery](crate::recovery) before removing it and copying the incoming entry.
/// Local-to-local requests use [`crate::transfer::transfer_resolved`].
///
/// For a remote move, removal starts after copying and rechecking the source
/// selection. A top-level folder with a skipped child stays intact, including
/// children that were successfully copied to the destination.
///
/// # Errors
///
/// Planning rejects unresolved conflicts and invalid selections. An error or
/// cancellation during execution can leave completed copies, earlier source
/// removals, or a replacement backup with an incomplete or missing destination.
///
/// # Panics
///
/// Passing a network or container discovery list as a file source can panic.
/// Supply the local or remote location that owns the selected entries.
///
/// # Examples
///
/// Move a file and merge a folder in the same batch. Skipping one conflict
/// keeps the source folder intact. The filesystem-backed connection uses a
/// temporary directory as the remote root.
///
/// ```
/// use rust_commander::filesystem::{self, Location};
/// use rust_commander::operations::Task;
/// use rust_commander::remote::{self, Connection, Secrets};
/// use rust_commander::transfer::{ConflictPolicy, TransferOptions};
/// use std::{collections::BTreeMap, fs};
///
/// let local = tempfile::tempdir()?;
/// let storage = tempfile::tempdir()?;
/// let incoming = local.path().join("reports");
/// let existing = storage.path().join("reports");
/// let ready = local.path().join("ready.txt");
/// fs::create_dir(&incoming)?;
/// fs::create_dir(&existing)?;
/// fs::write(&ready, "ready to move")?;
/// fs::write(incoming.join("summary.txt"), "incoming summary")?;
/// fs::write(incoming.join("details.txt"), "new details")?;
/// fs::write(existing.join("summary.txt"), "existing summary")?;
///
/// let profile = Connection {
///     service: "fs".into(),
///     root: storage.path().display().to_string(),
///     ..Connection::default()
/// };
/// remote::connect(&profile, Secrets::default(), false)?;
/// let from = Location::Local(local.path().into());
/// let to = Location::Remote {
///     connection: profile.id,
///     path: Default::default(),
/// };
/// let entries = filesystem::list(&from)?;
/// let options = TransferOptions {
///     merge_folders: true,
///     ..TransferOptions::default()
/// };
/// let collisions = remote::conflicts_with_merge(&from, &entries, &to, options.merge_folders)?;
/// assert_eq!(collisions.len(), 1);
/// let decisions = BTreeMap::from([(collisions[0].target.clone(), ConflictPolicy::Skip)]);
///
/// let mut task = Task::default();
/// remote::transfer_resolved(&from, &entries, &to, true, options, &decisions, &mut task)?;
/// assert_eq!(fs::read_to_string(storage.path().join("ready.txt"))?, "ready to move");
/// assert!(!ready.exists());
/// assert_eq!(fs::read_to_string(existing.join("summary.txt"))?, "existing summary");
/// assert_eq!(fs::read_to_string(existing.join("details.txt"))?, "new details");
/// assert_eq!(fs::read_to_string(incoming.join("summary.txt"))?, "incoming summary");
/// assert!(incoming.join("details.txt").is_file());
/// # Ok::<(), anyhow::Error>(())
/// ```
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
    let _docker = from
        .docker_target()
        .as_ref()
        .map(crate::uri_backend::docker::attach);
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
    if task.needs_total() {
        let mut total = Some(0u64);
        for plan in &planner.plans {
            total = operations::totals::add(
                total,
                operations::totals::entries_total(from, std::slice::from_ref(&plan.entry), task)?,
            );
            if let Some(existing) = &plan.existing {
                total = operations::totals::add(
                    total,
                    operations::totals::entries_total(to, std::slice::from_ref(existing), task)?,
                );
            }
        }
        task.plan_total(total);
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
            crate::recovery::record_backup(to, &backup, &plan.target, task)?;
            task.note_replacement(crate::history::ReplacementRecovery::Manager);
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
/// Remove exactly one checkpoint node. Never recurse into a newly added child.
pub(crate) fn remove_checkpoint_leaf(
    location: &Location,
    entry: &Entry,
    task: &Task,
) -> Result<()> {
    task.check()?;
    match location {
        Location::Local(_) => {
            if entry.is_dir {
                fs::remove_dir(&entry.path)?;
            } else {
                fs::remove_file(&entry.path)?;
            }
        }
        Location::Remote { connection, .. } => {
            let session = session(connection)?;
            if let Some(smb) = &session.smb {
                smb.delete_leaf(entry)?;
            } else {
                session
                    .operator()?
                    .delete(&key(&entry.path, entry.is_dir)?)
                    .map_err(|e| clean_error(e, &session.secrets))?;
            }
        }
        _ => bail!("Archive entries are read-only"),
    }
    anyhow::ensure!(
        entry_at(location, &entry.path)?.is_none(),
        "Checkpoint replacement was not removed"
    );
    Ok(())
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

/// Saves edited UTF-8 text using the bytes originally read from the remote file.
/// See [`save_bytes`] for recovery and concurrent-edit checks.
pub fn save_text(
    id: &str,
    path: &Path,
    original: &[u8],
    text: &str,
    task: &mut Task,
) -> Result<String> {
    save_bytes(id, path, original, text.as_bytes(), task)
}
/// Saves a remote editor buffer after comparing the current file with `original`.
///
/// Before uploading, this verifies and registers a sibling backup of the
/// original and checks the target again. Where supported, the upload also
/// uses the provider's ETag condition. A final read checks the uploaded bytes.
/// On success the returned message includes the recovery path.
///
/// # Errors
///
/// Fails if either buffer exceeds [`crate::viewer::EDIT_LIMIT`], the original
/// has changed, or a backup, upload, or verification step fails. A failure
/// after the upload begins can leave a changed target and a retained backup.
pub fn save_bytes(
    id: &str,
    path: &Path,
    original: &[u8],
    bytes: &[u8],
    task: &mut Task,
) -> Result<String> {
    save_bytes_bounded(id, path, original, bytes, crate::viewer::EDIT_LIMIT, task)
}
pub(crate) fn save_bytes_bounded(
    id: &str,
    path: &Path,
    original: &[u8],
    bytes: &[u8],
    limit: u64,
    task: &mut Task,
) -> Result<String> {
    anyhow::ensure!(
        original.len() as u64 <= limit && bytes.len() as u64 <= limit,
        "Remote save exceeds the {} MiB limit",
        limit / (1024 * 1024)
    );
    task.check()?;
    if !crate::operations::matches_reader(&mut *read(id, path)?, original, task)? {
        bail!("Remote file changed since it was opened. Reload before saving.");
    }
    let session = session(id)?;
    let backup = path.with_file_name(format!(
        ".rust-commander-edit-{:016x}.bak",
        rand::random::<u64>()
    ));
    if let Some(smb) = &session.smb {
        smb.write(&backup, &mut std::io::Cursor::new(original), task, true)?;
        anyhow::ensure!(
            crate::operations::matches_reader(&mut *read(id, &backup)?, original, task)?,
            "Remote recovery copy did not verify; source preserved at {}",
            path.display()
        );
        crate::recovery::record_backup(
            &Location::Remote {
                connection: id.into(),
                path: path.parent().unwrap_or(Path::new("")).into(),
            },
            &backup,
            path,
            task,
        )?;
        if !crate::operations::matches_reader(&mut *read(id, path)?, original, task)? {
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
                crate::remote_upload::buffer(&mut std::io::Cursor::new(original), path, task)?,
                opendal::options::WriteOptions {
                    if_not_exists: operator.info().full_capability().write_with_if_not_exists,
                    ..Default::default()
                },
            )
            .map_err(|e| clean_error(e, &session.secrets))?;
        anyhow::ensure!(
            crate::operations::matches_reader(&mut *read(id, &backup)?, original, task)?,
            "Remote recovery copy did not verify; source preserved at {}",
            path.display()
        );
        crate::recovery::record_backup(
            &Location::Remote {
                connection: id.into(),
                path: path.parent().unwrap_or(Path::new("")).into(),
            },
            &backup,
            path,
            task,
        )?;
        if !crate::operations::matches_reader(&mut *read(id, path)?, original, task)? {
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
            .write_options(
                &key(path, false)?,
                crate::remote_upload::buffer(&mut std::io::Cursor::new(bytes), path, task)?,
                options,
            )
            .map_err(|e| clean_error(e, &session.secrets))?;
    }
    if !crate::operations::matches_reader(&mut *read(id, path)?, bytes, task)? {
        bail!(
            "Uploaded content did not verify. Recovery copy: {}",
            backup.display()
        );
    }
    Ok(format!(
        "Saved remote file. Recovery copy: {}. Services without conditional writes cannot make the external-edit check atomic.",
        backup.display()
    ))
}
/// Searches a remote connection starting at the
/// [configured root](crate::desktop::SearchOptions::root).
/// The result includes matching paths, per-entry errors, and whether a search
/// limit was reached. Invalid search options and cancellation return an error.
pub fn search(
    id: &str,
    options: &crate::desktop::SearchOptions,
    task: &mut Task,
) -> Result<crate::desktop::SearchResult> {
    crate::search::search(
        &Location::Remote {
            connection: id.into(),
            path: options.root.clone(),
        },
        options,
        task,
    )
}

/// Start a new ranged reader. Backends without native range support may emulate it.
pub fn read_range(id: &str, path: &Path, start: u64) -> Result<Box<dyn Read>> {
    let session = session(id)?;
    if let Some(reader) = http_stream(&session, path, start)? {
        return Ok(reader);
    }
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
        task.advance_progress(offset);
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
pub(crate) fn checkpoint_passes(location: &Location) -> Result<u64> {
    if let Location::Remote { connection, .. } = location {
        let session = session(connection)?;
        if session.smb.is_none()
            && session.operator()?.info().scheme() != opendal::services::FS_SCHEME
        {
            return Ok(3);
        }
    }
    Ok(2)
}

/// Resolve filesystem-backed locations for overlap checks, including aliases
/// reached through a saved filesystem connection.
pub(crate) fn checkpoint_local_path(location: &Location, path: &Path) -> Result<Option<PathBuf>> {
    match location {
        Location::Local(_) => Ok(Some(path.into())),
        Location::Remote { connection, .. } => {
            let session = session(connection)?;
            if session.smb.is_none()
                && session.operator()?.info().scheme() == opendal::services::FS_SCHEME
            {
                let root = PathBuf::from(session.operator()?.info().root());
                Ok(Some(if path.as_os_str().is_empty() {
                    root
                } else {
                    root.join(filesystem::portable_relative(path)?)
                }))
            } else {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
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
    if session.operator()?.info().scheme() == opendal::services::FS_SCHEME {
        let info = session.operator()?.info();
        let root = PathBuf::from(info.root());
        crate::desktop::rename_exclusive(
            &root.join(filesystem::portable_relative(stage)?),
            &root.join(filesystem::portable_relative(target)?),
        )?;
        return Ok(());
    }
    // Publish through exclusive-create where the provider supports it. A plain
    // provider rename can overwrite a destination created by another client.
    let entry = entry_at(&location, stage)?.context("Transfer stage disappeared")?;
    copy_entry(&location, &entry, &location, target, task)?;
    remove_entry(&location, &entry, task)
}

/// Metadata-only token for read-only ranged viewers; no file content is hashed.
pub(crate) fn viewer_stamp(id: &str, path: &Path) -> Result<(u64, String)> {
    let session = session(id)?;
    if let Some(smb) = &session.smb {
        let entry = smb.stat(path)?.context("Remote file no longer exists")?;
        return Ok((entry.size, format!("{}:{:?}", entry.size, entry.modified)));
    }
    let metadata = session
        .operator()?
        .stat(&key(path, false)?)
        .map_err(|e| clean_error(e, &session.secrets))?;
    anyhow::ensure!(metadata.is_file(), "Select a remote regular file");
    anyhow::ensure!(
        metadata
            .user_metadata()
            .and_then(|m| m.get("rust-commander-size-known"))
            .is_none_or(|v| v != "false"),
        "Server does not provide a length for ranged viewing"
    );
    Ok((
        metadata.content_length(),
        format!(
            "{}:{:?}:{:?}",
            metadata.content_length(),
            metadata.etag(),
            metadata.last_modified()
        ),
    ))
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
