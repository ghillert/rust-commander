use super::{Output, Transport};
use crate::remote::{Connection, Secrets};
use anyhow::{Context, Result, ensure};
use opendal::{EntryMode, Metadata, raw::oio};
use russh::{
    client,
    keys::{
        self, PrivateKeyWithHashAlg,
        agent::{AgentIdentity, client::AgentClient},
    },
};
use russh_sftp::{
    client::{SftpSession, fs::File},
    protocol::{FileAttributes, OpenFlags},
};
use std::{
    io::{Read, SeekFrom, Write},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

struct HostKey {
    config: Arc<crate::ssh::Config>,
    review: Option<crate::ssh::Review>,
}
impl client::Handler for HostKey {
    type Error = anyhow::Error;
    async fn check_server_key(&mut self, key: &keys::PublicKeyOrCertificate) -> Result<bool> {
        ensure!(
            key.certificate().is_none(),
            "SSH host certificates are not supported; this server needs certificate verification with OpenSSH."
        );
        let key = key.public_key();
        if self.config.verify(&key)? {
            return Ok(true);
        }
        let offered = self.config.challenge(&key)?;
        if let Some(review) = &self.review {
            if review.accepted.as_ref() == Some(&offered) {
                return Ok(true);
            }
            *review.offered.lock().unwrap_or_else(|e| e.into_inner()) = Some(offered);
        }
        anyhow::bail!("SSH host fingerprint needs review before authentication.");
    }
}
type Client = Arc<Mutex<client::Handle<HostKey>>>;
pub(super) struct Sftp {
    session: Arc<SftpSession>,
    client: Client,
    root: String,
}
fn wait<T>(future: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    crate::remote::runtime().block_on(async {
        tokio::time::timeout(Duration::from_secs(30), future)
            .await
            .context("SFTP request timed out")?
    })
}
impl Sftp {
    pub fn new(c: &Connection, s: &Secrets) -> Result<Self> {
        wait(async {
            let resolved = Arc::new(crate::ssh::Config::resolve(c)?);
            let username = &resolved.user;
            let config = client::Config {
                inactivity_timeout: None,
                keepalive_interval: Some(Duration::from_secs(30)),
                ..Default::default()
            };
            let mut client = client::connect(
                Arc::new(config),
                (resolved.host.as_str(), resolved.port),
                HostKey {
                    config: resolved.clone(),
                    review: s.ssh_review.clone(),
                },
            )
            .await
            .map_err(|error| anyhow::anyhow!("SSH connection failed: {error:#}"))?;
            let mut authenticated = client.authenticate_none(username).await?.success();
            let mut detail = "No supported credentials were available.".to_owned();
            let mut allowed = Vec::new();
            if resolved.public_key_auth {
                for path in &resolved.identities {
                    if authenticated {
                        break;
                    }
                    if !path.exists() && c.key_file.is_empty() {
                        continue;
                    }
                    let passphrase = if !s.ssh_passphrase.is_empty() {
                        Some(s.ssh_passphrase.as_str())
                    } else if !c.key_file.is_empty() && !s.password.is_empty() {
                        Some(s.password.as_str())
                    } else {
                        None
                    };
                    match keys::load_secret_key(path, passphrase) {
                        Ok(key) => {
                            allowed.push(key.public_key().clone());
                            let hash = client.best_supported_rsa_hash().await?.flatten();
                            authenticated = client
                                .authenticate_publickey(
                                    username,
                                    PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                                )
                                .await?
                                .success();
                            if !authenticated {
                                detail =
                                    "The server rejected the private key for this username.".into();
                            }
                        }
                        Err(_) => {
                            detail = "A private key could not be read or decrypted. Check its path, permissions and key passphrase.".into();
                            if let Ok(public) =
                                std::fs::read_to_string(path.with_extension(format!(
                                    "{}pub",
                                    path.extension()
                                        .map(|e| format!("{}.", e.to_string_lossy()))
                                        .unwrap_or_default()
                                )))
                                && let Ok(public) = keys::PublicKey::from_openssh(&public)
                            {
                                allowed.push(public);
                            }
                        }
                    }
                }
            }
            if !authenticated
                && resolved.public_key_auth
                && resolved.agent.as_deref() != Some("none")
            {
                #[cfg(unix)]
                let agent = if let Some(path) = resolved
                    .agent
                    .as_ref()
                    .filter(|p| p.as_str() != "SSH_AUTH_SOCK")
                {
                    AgentClient::connect_uds(crate::filesystem::resolve_path(
                        path,
                        &crate::filesystem::home(),
                    ))
                    .await
                } else {
                    AgentClient::connect_env().await
                };
                #[cfg(windows)]
                let agent = AgentClient::connect_named_pipe(
                    resolved
                        .agent
                        .as_deref()
                        .filter(|p| *p != "SSH_AUTH_SOCK")
                        .unwrap_or(r"\\.\pipe\openssh-ssh-agent"),
                )
                .await;
                #[cfg(any(unix, windows))]
                if let Ok(mut agent) = agent {
                    let hash = client.best_supported_rsa_hash().await?.flatten();
                    for identity in agent.request_identities().await? {
                        if let AgentIdentity::PublicKey { key, .. } = identity
                            && (!resolved.identities_only
                                || allowed
                                    .iter()
                                    .any(|candidate| candidate.key_data() == key.key_data()))
                            && client
                                .authenticate_publickey_with(username, key, hash, &mut agent)
                                .await?
                                .success()
                        {
                            authenticated = true;
                            break;
                        }
                    }
                }
            }
            if !authenticated
                && resolved.password_auth
                && !s.password.is_empty()
                && c.key_file.is_empty()
            {
                authenticated = client
                    .authenticate_password(username, &s.password)
                    .await?
                    .success();
                if !authenticated {
                    detail = "The server rejected the password, or requires an additional authentication step.".into();
                }
            }
            ensure!(
                authenticated,
                "SFTP authentication failed for {}@{}:{}. {} Check the username, password/key passphrase, IdentityFile and unlocked SSH agent. Keyboard-interactive/MFA is not supported.",
                username,
                resolved.host,
                resolved.port,
                detail
            );
            let channel = client.channel_open_session().await?;
            channel.request_subsystem(true, "sftp").await?;
            let session = Arc::new(SftpSession::new(channel.into_stream()).await?);
            let root = format!("/{}", c.root.trim_matches('/'));
            ensure!(
                session.metadata(&root).await?.is_dir(),
                "SFTP root is not a directory."
            );
            Ok(Self {
                session,
                client: Arc::new(Mutex::new(client)),
                root,
            })
        })
    }
    fn path(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.root.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }
}
fn metadata(meta: FileAttributes) -> Metadata {
    let mut result = Metadata::new(if meta.file_type().is_dir() {
        EntryMode::DIR
    } else {
        EntryMode::FILE
    });
    if let Some(size) = meta.size {
        result.set_content_length(size);
    }
    if let Some(time) = meta.mtime {
        result.set_last_modified(chrono::DateTime::from_timestamp(time as i64, 0).unwrap());
    }
    result
}
impl Transport for Sftp {
    fn stat(&self, path: &str) -> Result<Metadata> {
        wait(async { Ok(metadata(self.session.metadata(self.path(path)).await?)) })
    }
    fn list(&self, path: &str) -> Result<Vec<oio::Entry>> {
        wait(async {
            let mut result = Vec::new();
            let prefix = path.trim_matches('/');
            for entry in self.session.read_dir(self.path(path)).await? {
                let name = entry.file_name();
                crate::filesystem::validate_name(&name)?;
                // Follow symlinks for directory navigation, as the existing SFTP backend does.
                let meta = if entry.file_type().is_symlink() {
                    self.session
                        .metadata(format!(
                            "{}/{}",
                            self.path(path).trim_end_matches('/'),
                            name
                        ))
                        .await?
                } else {
                    entry.metadata()
                };
                let meta = metadata(meta);
                let key = format!(
                    "{}{}{}{}",
                    prefix,
                    if prefix.is_empty() { "" } else { "/" },
                    name,
                    if meta.is_dir() { "/" } else { "" }
                );
                result.push(oio::Entry::new(&key, meta));
            }
            Ok(result)
        })
    }
    fn read(&self, path: &str, offset: u64) -> Result<Box<dyn Read + Send>> {
        wait(async {
            let mut file = self.session.open(self.path(path)).await?;
            file.seek(SeekFrom::Start(offset)).await?;
            Ok(Box::new(SftpFile {
                file,
                _client: self.client.clone(),
            }) as Box<dyn Read + Send>)
        })
    }
    fn write(&self, path: &str, append: bool) -> Result<Box<dyn Output>> {
        wait(async {
            let flags = OpenFlags::CREATE
                | OpenFlags::WRITE
                | if append {
                    OpenFlags::APPEND
                } else {
                    OpenFlags::TRUNCATE
                };
            let file = self.session.open_with_flags(self.path(path), flags).await?;
            Ok(Box::new(SftpFile {
                file,
                _client: self.client.clone(),
            }) as Box<dyn Output>)
        })
    }
    fn mkdir(&self, path: &str) -> Result<()> {
        wait(async {
            let mut prefix = self.root.trim_end_matches('/').to_owned();
            for part in path.trim_matches('/').split('/') {
                prefix.push('/');
                prefix.push_str(part);
                if let Err(error) = self.session.create_dir(&prefix).await {
                    ensure!(
                        self.session
                            .metadata(&prefix)
                            .await
                            .is_ok_and(|m| m.is_dir()),
                        "Cannot create SFTP directory: {error}"
                    );
                }
            }
            Ok(())
        })
    }
    fn delete(&self, path: &str) -> Result<()> {
        wait(async {
            if path.ends_with('/') {
                self.session.remove_dir(self.path(path)).await?;
            } else {
                self.session.remove_file(self.path(path)).await?;
            }
            Ok(())
        })
    }
}
struct SftpFile {
    file: File,
    _client: Client,
}
impl Read for SftpFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        wait(async { Ok(self.file.read(buf).await?) }).map_err(std::io::Error::other)
    }
}
impl Write for SftpFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        wait(async { Ok(self.file.write(buf).await?) }).map_err(std::io::Error::other)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        wait(async {
            self.file.flush().await?;
            Ok(())
        })
        .map_err(std::io::Error::other)
    }
}
impl Output for SftpFile {
    fn finish(&mut self) -> Result<()> {
        wait(async {
            self.file.sync_all().await?;
            self.file.shutdown().await?;
            Ok(())
        })
    }
}
