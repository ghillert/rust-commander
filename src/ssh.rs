//! OpenSSH configuration resolution and explicit host-key review. No credentials
//! are sent before verification; reviewed keys are scoped to the exact endpoint.
use anyhow::{Context, Result, bail, ensure};
use russh::keys;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Challenge {
    pub host: String,
    pub port: u16,
    pub lookup: String,
    pub algorithm: String,
    pub fingerprint: String,
    pub public_key: String,
    pub save_to: Option<PathBuf>,
}
#[derive(Clone, Default)]
pub struct Review {
    pub offered: Arc<Mutex<Option<Challenge>>>,
    pub accepted: Option<Challenge>,
}
#[derive(Debug)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub identities: Vec<PathBuf>,
    pub known_hosts: Vec<PathBuf>,
    pub save_to: Option<PathBuf>,
    pub lookup: String,
    pub identities_only: bool,
    pub agent: Option<String>,
    pub password_auth: bool,
    pub public_key_auth: bool,
}
fn local_user() -> String {
    std::env::var(if cfg!(windows) { "USERNAME" } else { "USER" }).unwrap_or_default()
}
fn expand(value: &str, host: &str, alias: &str, port: u16, user: &str) -> Result<PathBuf> {
    let home = crate::filesystem::home();
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        out.push_str(
            &match chars.next().context("Incomplete SSH configuration token")? {
                '%' => "%".into(),
                'd' => home.to_string_lossy().into_owned(),
                'h' => host.into(),
                'n' => alias.into(),
                'p' => port.to_string(),
                'r' => user.into(),
                'u' => local_user(),
                _ => bail!("Unsupported SSH path token; use an explicit path in this connection"),
            },
        );
    }
    ensure!(
        !out.contains("${"),
        "Use an explicit path for environment substitutions in SSH configuration"
    );
    #[cfg(windows)]
    let out = if let Some(suffix) = out.strip_prefix("__PROGRAMDATA__") {
        let root = std::env::var("PROGRAMDATA")
            .context("OpenSSH system known-hosts path requires PROGRAMDATA")?;
        format!("{root}{suffix}")
    } else {
        out
    };
    let base = std::env::current_dir()?;
    Ok(crate::filesystem::resolve_path(&out, &base))
}
/// OpenSSH itself evaluates Host/Match/Include and first-value precedence, rather
/// than maintaining a second, incompatible configuration parser. ssh -G does
/// not establish a connection. Configuration is the user's trusted local config.
fn effective(c: &crate::remote::Connection, alias: &str, port: Option<u16>) -> Result<String> {
    let program = if cfg!(windows) {
        PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into()))
            .join("System32/OpenSSH/ssh.exe")
    } else {
        PathBuf::from("/usr/bin/ssh")
    };
    if !program.is_file() {
        ensure!(
            c.ssh_config_file.is_empty() && !crate::filesystem::home().join(".ssh/config").exists(),
            "Install the OpenSSH client to use SSH configuration, or disable SSH configuration for this connection."
        );
        return Ok(String::new());
    }
    let mut command = Command::new(program);
    command
        .arg("-G")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if !c.ssh_config_file.is_empty() {
        command.arg("-F").arg(crate::filesystem::resolve_path(
            &c.ssh_config_file,
            &crate::filesystem::home(),
        ));
    }
    if let Some(port) = port {
        command.arg("-p").arg(port.to_string());
    }
    if !c.username.is_empty() {
        command.arg("-l").arg(&c.username);
    }
    command.arg("--").arg(alias);
    let mut child = command
        .spawn()
        .context("Cannot evaluate OpenSSH configuration")?;
    let stdout = child.stdout.take().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = send.send(result);
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait()? {
            Some(status) => {
                ensure!(
                    status.success(),
                    "OpenSSH configuration could not be evaluated. Check the configuration syntax."
                );
                break;
            }
            None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                bail!("OpenSSH configuration evaluation timed out");
            }
        }
    }
    let bytes = receive
        .recv_timeout(Duration::from_secs(1))
        .context("OpenSSH configuration output timed out")??;
    ensure!(
        bytes.len() <= 1024 * 1024,
        "OpenSSH configuration output is too large"
    );
    String::from_utf8(bytes).context("OpenSSH configuration output is not UTF-8")
}
fn words(text: &str) -> Result<Vec<String>> {
    let mut values = Vec::new();
    let mut value = String::new();
    let mut quote = None;
    for ch in text.chars() {
        match ch {
            '\'' | '"' if quote == Some(ch) => quote = None,
            '\'' | '"' if quote.is_none() => quote = Some(ch),
            ch if ch.is_whitespace() && quote.is_none() => {
                if !value.is_empty() {
                    values.push(std::mem::take(&mut value));
                }
            }
            _ => value.push(ch),
        }
    }
    ensure!(quote.is_none(), "Invalid quoted SSH configuration path");
    if !value.is_empty() {
        values.push(value);
    }
    Ok(values)
}
impl Config {
    pub fn resolve(c: &crate::remote::Connection) -> Result<Self> {
        let url = url::Url::parse(&c.endpoint).context("Invalid SFTP endpoint")?;
        let alias = url
            .host_str()
            .context("SFTP needs a hostname or SSH alias")?
            .trim_matches(['[', ']']);
        ensure!(
            !alias.starts_with('-') && !alias.chars().any(char::is_whitespace),
            "Invalid SSH host alias"
        );
        let text = if c.ssh_use_config {
            effective(c, alias, url.port())?
        } else {
            String::new()
        };
        Self::from_effective(c, alias, url.port(), &text)
    }
    fn from_effective(
        c: &crate::remote::Connection,
        alias: &str,
        port: Option<u16>,
        text: &str,
    ) -> Result<Self> {
        let mut values = std::collections::HashMap::<&str, Vec<&str>>::new();
        for line in text.lines() {
            if let Some((key, value)) = line.split_once(' ') {
                values.entry(key).or_default().push(value);
            }
        }
        let first = |key| values.get(key).and_then(|v| v.first()).copied();
        for key in ["proxycommand", "proxyjump", "certificatefile"] {
            ensure!(
                first(key).is_none_or(|v| v == "none"),
                "SSH {key} is configured but not supported by the SFTP transport; use a direct connection or an explicit local tunnel."
            );
        }
        let host = first("hostname").unwrap_or(alias).to_owned();
        let port = port
            .or(first("port").map(str::parse).transpose()?)
            .unwrap_or(22);
        let user = if c.username.is_empty() {
            first("user").map(str::to_owned).unwrap_or_else(local_user)
        } else {
            c.username.clone()
        };
        ensure!(
            !user.is_empty(),
            "Enter an SFTP username or set User in SSH configuration"
        );
        let path = |s: &str| expand(s, &host, alias, port, &user);
        let identities = if !c.key_file.is_empty() {
            vec![crate::filesystem::resolve_path(
                &c.key_file,
                &crate::filesystem::home(),
            )]
        } else {
            values
                .get("identityfile")
                .into_iter()
                .flatten()
                .filter(|v| **v != "none")
                .map(|v| path(v))
                .collect::<Result<Vec<_>>>()?
        };
        let files = |key| -> Result<Vec<PathBuf>> {
            let raw = first(key).unwrap_or("");
            let paths = words(raw)?;
            // ssh -G leaves custom path lists ambiguous when a name has
            // spaces. Recognize the conventional hostfile/hostfile2 pair on
            // either path separator, including Windows drive/system paths.
            let conventional_pair = paths.len() == 2 && paths[1] == format!("{}2", paths[0]) && {
                let normalized = paths[0].replace('\\', "/");
                normalized.ends_with("/known_hosts") || normalized.ends_with("/ssh_known_hosts")
            };
            ensure!(
                paths.len() <= 1 || conventional_pair || raw.starts_with(['\'', '"']),
                "OpenSSH returned an ambiguous {key} path list. Set an explicit Known hosts file or disable SSH configuration for this connection."
            );
            paths
                .iter()
                .filter(|v| *v != "none")
                .map(|v| path(v))
                .collect()
        };
        let mut known_hosts = if !c.known_hosts_file.is_empty() {
            vec![crate::filesystem::resolve_path(
                &c.known_hosts_file,
                &crate::filesystem::home(),
            )]
        } else if first("userknownhostsfile").is_some() {
            files("userknownhostsfile")?
        } else {
            vec![crate::filesystem::home().join(".ssh/known_hosts")]
        };
        let save_to = known_hosts.first().cloned();
        known_hosts.extend(files("globalknownhostsfile")?);
        let lookup = if let Some(alias) = first("hostkeyalias").filter(|v| *v != "none") {
            alias.to_owned()
        } else if port == 22 {
            host.clone()
        } else {
            format!("[{host}]:{port}")
        };
        let agent = first("identityagent")
            .map(|agent| -> Result<String> {
                if matches!(agent, "none" | "SSH_AUTH_SOCK") {
                    return Ok(agent.into());
                }
                if let Some(variable) = agent.strip_prefix('$') {
                    ensure!(
                        variable
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_'),
                        "Invalid SSH agent environment variable"
                    );
                    return std::env::var(variable)
                        .context("Configured SSH agent environment variable is unavailable");
                }
                Ok(path(agent)?.to_string_lossy().into_owned())
            })
            .transpose()?;
        Ok(Self {
            host,
            port,
            user,
            identities,
            known_hosts,
            save_to,
            lookup,
            identities_only: first("identitiesonly") == Some("yes"),
            agent,
            password_auth: first("passwordauthentication") != Some("no"),
            public_key_auth: first("pubkeyauthentication") != Some("no"),
        })
    }
}
fn wildcard(pattern: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t, mut star, mut retry) = (0, 0, None, 0);
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p].eq_ignore_ascii_case(&text[t])) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            p += 1;
            retry = t;
        } else if let Some(s) = star {
            retry += 1;
            t = retry;
            p = s + 1;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}
fn hashed(pattern: &str, host: &str) -> bool {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use sha1::{Digest, Sha1};
    let parts = pattern.split('|').collect::<Vec<_>>();
    if parts.len() != 4 || parts[1] != "1" {
        return false;
    }
    let (Ok(mut key), Ok(expected)) = (STANDARD.decode(parts[2]), STANDARD.decode(parts[3])) else {
        return false;
    };
    if key.len() > 64 {
        key = Sha1::digest(&key).to_vec();
    }
    key.resize(64, 0);
    let mut inner = Sha1::new();
    inner.update(key.iter().map(|v| v ^ 0x36).collect::<Vec<_>>());
    inner.update(host.as_bytes());
    let mut outer = Sha1::new();
    outer.update(key.iter().map(|v| v ^ 0x5c).collect::<Vec<_>>());
    outer.update(inner.finalize());
    outer.finalize().as_slice() == expected
}
fn matches(patterns: &str, host: &str) -> bool {
    let mut matched = false;
    for pattern in patterns.split(',') {
        let (negative, pattern) = pattern
            .strip_prefix('!')
            .map_or((false, pattern), |p| (true, p));
        let yes = if pattern.starts_with('|') {
            hashed(pattern, host)
        } else {
            wildcard(pattern.as_bytes(), host.as_bytes())
        };
        if yes && negative {
            return false;
        }
        matched |= yes;
    }
    matched
}
/// Return unknown only when no key is recorded. Changed keys, revocations,
/// unreadable stores and host-certificate policy never become an accept dialog.
fn verify_text(text: &str, host: &str, key: &keys::PublicKey) -> Result<Option<bool>> {
    let mut recorded = false;
    let mut accepted = false;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(mut pattern) = parts.next() else {
            continue;
        };
        if pattern.starts_with('#') {
            continue;
        }
        let marker = if pattern.starts_with('@') {
            let marker = pattern;
            pattern = parts.next().context("Invalid known_hosts marker")?;
            Some(marker)
        } else {
            None
        };
        if !matches(pattern, host) {
            continue;
        }
        let algorithm = parts.next().context("Invalid known_hosts entry")?;
        let encoded = parts.next().context("Invalid known_hosts key")?;
        let stored = keys::parse_public_key_base64(encoded)
            .context("Cannot parse a recorded SSH host key")?;
        ensure!(
            stored.algorithm().as_str() == algorithm,
            "Invalid known_hosts key algorithm"
        );
        match marker {
            Some("@revoked") => ensure!(
                &stored != key,
                "SSH host key is revoked. Contact the server administrator."
            ),
            Some("@cert-authority") => bail!(
                "SSH host certificate authority verification is not supported; use a reviewed direct-host profile."
            ),
            Some(_) => bail!("Unsupported known_hosts marker"),
            None => {
                recorded = true;
                accepted |= &stored == key;
            }
        }
    }
    Ok(recorded.then_some(accepted))
}
impl Config {
    pub fn verify(&self, key: &keys::PublicKey) -> Result<bool> {
        let mut recorded = false;
        let mut accepted = false;
        for path in &self.known_hosts {
            let text = match File::open(path) {
                Ok(file) => {
                    let mut bytes = Vec::new();
                    file.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
                    ensure!(
                        bytes.len() <= 4 * 1024 * 1024,
                        "SSH known_hosts file is too large"
                    );
                    String::from_utf8(bytes).context("SSH known_hosts is not UTF-8")?
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    return Err(e)
                        .context("Cannot read SSH known_hosts; fix file access before connecting");
                }
            };
            if let Some(found) = verify_text(&text, &self.lookup, key)? {
                recorded = true;
                accepted |= found;
            }
        }
        ensure!(
            !recorded || accepted,
            "SSH HOST KEY CHANGED. The server does not match its recorded key. Connection refused; verify the change with the administrator before updating known_hosts."
        );
        Ok(accepted)
    }
    pub fn challenge(&self, key: &keys::PublicKey) -> Result<Challenge> {
        Ok(Challenge {
            host: self.host.clone(),
            port: self.port,
            lookup: self.lookup.clone(),
            algorithm: key.algorithm().to_string(),
            fingerprint: key.fingerprint(keys::HashAlg::Sha256).to_string(),
            public_key: key.to_openssh()?,
            save_to: self.save_to.clone(),
        })
    }
}
impl Challenge {
    pub fn remember(&self) -> Result<()> {
        let path = self
            .save_to
            .as_ref()
            .context("SSH configuration disables saving host keys")?;
        let key = keys::PublicKey::from_openssh(&self.public_key)?;
        ensure!(
            key.fingerprint(keys::HashAlg::Sha256).to_string() == self.fingerprint,
            "Host review no longer matches the key"
        );
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut options = OpenOptions::new();
        options.read(true).append(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        file.lock()?;
        let mut text = String::new();
        (&file)
            .take(4 * 1024 * 1024 + 1)
            .read_to_string(&mut text)?;
        ensure!(
            text.len() <= 4 * 1024 * 1024,
            "SSH known_hosts file is too large"
        );
        match verify_text(&text, &self.lookup, &key)? {
            Some(true) => return Ok(()),
            Some(false) => bail!("SSH host key changed while reviewing it. Nothing was saved."),
            None => {}
        }
        let line = format!(
            "{}{} {}\n",
            if text.is_empty() || text.ends_with('\n') {
                ""
            } else {
                "\n"
            },
            self.lookup,
            key.to_openssh()?
        );
        file.write_all(line.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ";
    #[test]
    fn review_is_exact_revocations_and_changed_keys_never_become_unknown() {
        let key = keys::PublicKey::from_openssh(KEY).unwrap();
        assert_eq!(verify_text("", "host", &key).unwrap(), None);
        assert_eq!(
            verify_text(&format!("host {KEY}"), "host", &key).unwrap(),
            Some(true)
        );
        assert!(verify_text(&format!("@revoked host {KEY}"), "host", &key).is_err());
        assert!(matches("*.example,!blocked.example", "ok.example"));
        assert!(!matches("*.example,!blocked.example", "blocked.example"));
    }
    #[test]
    fn resolved_aliases_keep_explicit_fields_and_known_hosts_with_spaces() {
        let c = crate::remote::Connection {
            username: "explicit".into(),
            ..Default::default()
        };
        let resolved=Config::from_effective(&c,"alias",Some(2222),"hostname real.example\nuser config\nport 22\nidentityfile ~/.ssh/%r-%h\nuserknownhostsfile \"/tmp/keys with spaces\"\nidentitiesonly yes\n").unwrap();
        assert_eq!(
            (
                resolved.host.as_str(),
                resolved.port,
                resolved.user.as_str()
            ),
            ("real.example", 2222, "explicit")
        );
        assert_eq!(
            resolved.known_hosts[0],
            PathBuf::from("/tmp/keys with spaces")
        );
        assert!(resolved.identities_only);
        assert!(resolved.identities[0].ends_with("explicit-real.example"));
    }
    #[test]
    fn conventional_windows_hostfile_pairs_are_not_treated_as_ambiguous() {
        let c = crate::remote::Connection {
            username: "fixture".into(),
            ..Default::default()
        };
        let resolved=Config::from_effective(&c,"host",None,r"userknownhostsfile C:/Users/fixture/.ssh/known_hosts C:/Users/fixture/.ssh/known_hosts2").unwrap();
        assert_eq!(resolved.known_hosts.len(), 2);
        assert!(
            Config::from_effective(
                &c,
                "host",
                None,
                "userknownhostsfile /tmp/hosts with spaces"
            )
            .is_err()
        );
    }
    #[test]
    fn explicit_paths_keep_percent_tokens_literal() {
        let c = crate::remote::Connection {
            username: "fixture".into(),
            key_file: "/tmp/100% ready key".into(),
            known_hosts_file: "/tmp/hosts %h".into(),
            ..Default::default()
        };
        let resolved = Config::from_effective(&c, "host", None, "").unwrap();
        assert_eq!(resolved.identities[0], PathBuf::from("/tmp/100% ready key"));
        assert_eq!(resolved.known_hosts[0], PathBuf::from("/tmp/hosts %h"));
    }
    #[test]
    fn remembering_a_review_does_not_replace_an_existing_key() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("known_hosts");
        let c = Config::from_effective(
            &crate::remote::Connection {
                username: "fixture".into(),
                ..Default::default()
            },
            "host",
            None,
            "",
        )
        .unwrap();
        let mut challenge = c
            .challenge(&keys::PublicKey::from_openssh(KEY).unwrap())
            .unwrap();
        challenge.save_to = Some(path.clone());
        challenge.remember().unwrap();
        let before = std::fs::read(&path).unwrap();
        challenge.remember().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}
