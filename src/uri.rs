//! Location input is parsed before attempting any filesystem or network access.
use crate::{
    filesystem::{self, Location},
    remote::{Connection, FtpTls, Secrets},
};
use anyhow::{Context, Result, bail, ensure};
use std::path::{Path, PathBuf};
use url::Url;

pub enum Input {
    Location(Location),
    Remote(Box<Target>),
}
pub struct Target {
    pub connection: Connection,
    pub path: PathBuf,
    pub secrets: Secrets,
}

fn decoded(text: &str) -> Result<String> {
    Ok(percent_encoding::percent_decode_str(text)
        .decode_utf8()
        .context("URI components must contain valid UTF-8")?
        .into_owned())
}

// Remote names need not be valid local filenames (for example, '?' on Windows).
// Still reject separators and traversal before joining a decoded URI segment.
fn validate_remote_segment(name: &str) -> Result<()> {
    ensure!(
        !name.trim().is_empty() && !name.contains(['/', '\\', '\0']) && name != "." && name != "..",
        "Invalid remote path component."
    );
    crate::remote::key(Path::new(name), false)?;
    Ok(())
}

pub fn parse(text: &str, base: &Location) -> Result<Input> {
    let text = text.trim();
    ensure!(!text.is_empty(), "Enter a folder path or URI.");
    #[cfg(windows)]
    if (text.as_bytes().get(1) == Some(&b':') && text.as_bytes()[0].is_ascii_alphabetic())
        || text.starts_with(r"\\")
    {
        return Ok(Input::Location(filesystem::parse_location(text, base)?));
    }
    if text == "~" || text.starts_with("~/") {
        return Ok(Input::Location(filesystem::parse_location(text, base)?));
    }
    if text == base.label() {
        return Ok(Input::Location(base.clone()));
    }
    let scheme = text.split_once(':').map(|(s, _)| s).filter(|s| {
        s.len() > 1
            && s.bytes().next().is_some_and(|b| b.is_ascii_alphabetic())
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"+.-".contains(&b))
    });
    let Some(scheme) = scheme else {
        if let Location::Remote { connection, path } = base {
            let mut target = if text.starts_with('/') {
                PathBuf::new()
            } else {
                path.clone()
            };
            for part in text.split('/') {
                match part {
                    "" | "." => {}
                    ".." => {
                        target.pop();
                    }
                    name => {
                        validate_remote_segment(name)?;
                        target.push(name);
                    }
                }
            }
            crate::remote::key(&target, false)?;
            return Ok(Input::Location(Location::Remote {
                connection: connection.clone(),
                path: target,
            }));
        }
        return Ok(Input::Location(filesystem::parse_location(text, base)?));
    };
    let scheme = scheme.to_ascii_lowercase();
    if scheme == "remote" {
        return Ok(Input::Location(filesystem::parse_location(text, base)?));
    }
    if scheme == "data" {
        bail!("The data: scheme is not supported. Save the embedded content to a file first.");
    }
    ensure!(
        matches!(
            scheme.as_str(),
            "file" | "http" | "https" | "ftp" | "ftps" | "ftpes" | "sftp" | "smb" | "s3" | "gs"
        ),
        "Unsupported URI scheme: {scheme}."
    );
    // Do not attach the original input to errors: it may include passwords or signed queries.
    let mut url = Url::parse(text).map_err(|_| anyhow::anyhow!("Invalid {scheme} URI."))?;
    ensure!(
        url.password().is_none() || !url.username().is_empty(),
        "A password requires a username."
    );
    if scheme == "file" {
        ensure!(
            url.username().is_empty() && url.password().is_none() && url.query().is_none(),
            "File URIs cannot contain credentials or query strings."
        );
        let path = url.to_file_path().map_err(|_| anyhow::anyhow!("This file URI is not a local path on this platform. Use smb:// for a network share."))?;
        return Ok(Input::Location(
            if path.is_file() && crate::archive::format(&path).is_some() {
                Location::Archive {
                    file: path,
                    prefix: PathBuf::new(),
                }
            } else {
                Location::Local(path)
            },
        ));
    }
    ensure!(
        url.host_str().is_some(),
        "The URI needs a host or bucket name."
    );
    ensure!(
        matches!(scheme.as_str(), "http" | "https") || url.query().is_none(),
        "This protocol does not support URI query strings."
    );
    let username = decoded(url.username())?;
    let password = decoded(url.password().unwrap_or_default())?;
    let request_query = url.query().unwrap_or_default().to_owned();
    url.set_username("")
        .map_err(|_| anyhow::anyhow!("Invalid URI authority"))?;
    url.set_password(None)
        .map_err(|_| anyhow::anyhow!("Invalid URI authority"))?;
    url.set_query(None);
    url.set_fragment(None);
    let mut path = PathBuf::new();
    for segment in url
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
    {
        let segment = decoded(segment)?;
        validate_remote_segment(&segment)?;
        path.push(segment);
    }
    crate::remote::key(&path, false)?;
    let mut connection = Connection {
        name: url.host_str().unwrap().into(),
        listed: false,
        username,
        query_required: !request_query.is_empty(),
        ..Default::default()
    };
    connection.service = match scheme.as_str() {
        "http" | "https" => "http",
        "ftps" | "ftpes" => "ftp",
        "gs" => "gcs",
        s => s,
    }
    .into();
    if matches!(scheme.as_str(), "s3" | "gs") {
        ensure!(
            url.port().is_none() && connection.username.is_empty() && password.is_empty(),
            "Cloud URIs use a bucket name; configure endpoints and credentials in Connections."
        );
        connection.bucket = url.host_str().unwrap().into();
        connection.name = connection.bucket.clone();
    } else {
        if scheme == "smb"
            && let Some(share) = path.components().next()
        {
            connection.bucket = share.as_os_str().to_string_lossy().into_owned();
            path = path.components().skip(1).collect();
        }
        connection.ftp_tls = match scheme.as_str() {
            "ftp" => FtpTls::Plain,
            "ftps" => FtpTls::Implicit,
            "ftpes" => FtpTls::Explicit,
            _ => FtpTls::Automatic,
        };
        let port = url.port().unwrap_or(match scheme.as_str() {
            "sftp" => 22,
            "smb" => 445,
            "ftps" => 990,
            "ftp" | "ftpes" => 21,
            "https" => 443,
            _ => 80,
        });
        // Backend endpoint schemes differ from the user-facing SFTP/FTP TLS schemes.
        let endpoint_scheme = match scheme.as_str() {
            "sftp" => "ssh",
            "ftpes" => "ftps",
            s => s,
        };
        let host = url.host().unwrap().to_string();
        connection.endpoint = format!("{endpoint_scheme}://{host}:{port}");
    }
    let request_path = crate::remote::display_path(&path);
    Ok(Input::Remote(Box::new(Target {
        connection,
        path,
        secrets: Secrets {
            password,
            request_query,
            request_path,
            ..Default::default()
        },
    })))
}

/// A canonical, credential-free address. Legacy/unregistered profiles remain usable.
pub fn address(id: &str, path: &Path) -> String {
    let legacy = || format!("remote://{id}/{}", crate::remote::display_path(path));
    let Some(c) = crate::remote::profile(id) else {
        return legacy();
    };
    let mut url = match c.service.as_str() {
        "s3" | "gcs" => Url::parse(&format!(
            "{}://{}/",
            if c.service == "s3" { "s3" } else { "gs" },
            c.bucket
        )),
        "sftp" => Url::parse(&c.endpoint.replace("ssh://", "sftp://")),
        "ftp" => Url::parse(&c.endpoint).and_then(|u| {
            let scheme = match c.tls_mode() {
                FtpTls::Plain => "ftp",
                FtpTls::Implicit => "ftps",
                _ => "ftpes",
            };
            Url::parse(&format!(
                "{scheme}://{}:{}/",
                u.host().map(|h| h.to_string()).unwrap_or_default(),
                u.port().unwrap_or(if c.tls_mode() == FtpTls::Implicit {
                    990
                } else {
                    21
                })
            ))
        }),
        "http" | "smb" => Url::parse(&c.endpoint),
        _ => return legacy(),
    };
    let Ok(ref mut url) = url else {
        return legacy();
    };
    let username =
        percent_encoding::utf8_percent_encode(&c.username, percent_encoding::NON_ALPHANUMERIC)
            .to_string();
    let _ = url.set_username(&username);
    let _ = url.set_password(None);
    let mut full = PathBuf::new();
    if c.service == "smb" && !c.bucket.is_empty() {
        full.push(&c.bucket);
    }
    full.push(mount_prefix(&c));
    full.push(path);
    if let Ok(mut segments) = url.path_segments_mut() {
        segments.clear();
        for name in full.iter() {
            segments.push(&name.to_string_lossy());
        }
    }
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

pub fn matches_profile(target: &Target, c: &Connection) -> bool {
    let t = &target.connection;
    if c.query_required {
        return false;
    }
    if t.service != c.service || (!t.username.is_empty() && t.username != c.username) {
        return false;
    }
    if !target.path.starts_with(mount_prefix(c)) {
        return false;
    }
    if matches!(c.service.as_str(), "s3" | "gcs") {
        return c.bucket == t.bucket;
    }
    if c.service == "smb" && c.bucket != t.bucket {
        return false;
    }
    if c.service == "ftp" && c.tls_mode() != t.tls_mode() {
        return false;
    }
    let endpoint = |c: &Connection| {
        Url::parse(&c.endpoint).ok().map(|u| {
            (
                if c.service == "http" {
                    u.scheme().to_owned()
                } else {
                    c.service.clone()
                },
                u.host_str().unwrap_or_default().to_ascii_lowercase(),
                u.port_or_known_default()
                    .unwrap_or(match c.service.as_str() {
                        "sftp" => 22,
                        "ftp" if c.tls_mode() == FtpTls::Implicit => 990,
                        "ftp" => 21,
                        _ => 445,
                    }),
            )
        })
    };
    endpoint(t) == endpoint(c)
}

fn mount_prefix(c: &Connection) -> PathBuf {
    let mut prefix = PathBuf::new();
    if c.service == "http"
        && let Ok(url) = Url::parse(&c.endpoint)
    {
        for part in url
            .path_segments()
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
        {
            if let Ok(part) = decoded(part) {
                prefix.push(part);
            }
        }
    }
    prefix.push(c.root.trim_matches('/'));
    prefix
}
pub(crate) fn relative_to_profile(path: &Path, c: &Connection) -> Result<PathBuf> {
    Ok(path
        .strip_prefix(mount_prefix(c))
        .context("Address is outside this connection's root")?
        .into())
}

pub fn protocol(location: &Location) -> String {
    match location {
        Location::Local(_) => "FILE".into(),
        Location::Archive { .. } => "ARCHIVE".into(),
        Location::Remote { connection, path } => address(connection, path)
            .split(':')
            .next()
            .unwrap_or("REMOTE")
            .to_ascii_uppercase(),
    }
}
