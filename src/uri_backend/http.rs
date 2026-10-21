use super::Transport;
use crate::remote::{Connection, Secrets};
use anyhow::{Context, Result, bail, ensure};
use opendal::{EntryMode, Metadata, raw::oio};
use reqwest::{
    Method, StatusCode,
    blocking::{Client, Response},
    header,
};
use scraper::{Html, Selector};
use std::{
    collections::{BTreeMap, HashMap},
    io::Read,
    sync::Mutex,
    time::Duration,
};
use url::Url;
const INDEX_LIMIT: u64 = 8 * 1024 * 1024;

pub(super) struct Http {
    client: Client,
    base: Url,
    username: String,
    password: String,
    query: String,
    query_path: String,
    links: Mutex<HashMap<String, Url>>,
}
impl Http {
    pub fn new(c: &Connection, s: &Secrets) -> Result<Self> {
        ensure!(
            !c.query_required || !s.request_query.is_empty(),
            "This address needs its original query string. Paste the complete URI again; query strings are never saved."
        );
        let mut base = Url::parse(&c.endpoint).context("Invalid HTTP endpoint")?;
        ensure!(
            matches!(base.scheme(), "http" | "https"),
            "HTTP endpoints must use http:// or https://."
        );
        let prefix = format!(
            "{}/{}/",
            base.path().trim_end_matches('/'),
            c.root.trim_matches('/')
        );
        base.set_path(&prefix.replace("//", "/"));
        base.set_query(None);
        base.set_fragment(None);
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("Rust-Commander/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            client,
            base,
            username: c.username.clone(),
            password: s.password.clone(),
            query: s.request_query.clone(),
            query_path: s.request_path.clone(),
            links: Default::default(),
        })
    }
    fn url(&self, path: &str) -> Result<Url> {
        let key = path.trim_matches('/');
        if let Some(url) = self
            .links
            .lock()
            .map_err(|_| anyhow::anyhow!("HTTP link cache lock failed"))?
            .get(key)
        {
            return Ok(url.clone());
        }
        let mut url = self.base.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| anyhow::anyhow!("Invalid HTTP endpoint"))?;
            segments.pop_if_empty();
            for part in path
                .trim_start_matches('/')
                .split('/')
                .filter(|p| !p.is_empty())
            {
                segments.push(part);
            }
            if path.ends_with('/') {
                segments.push("");
            }
        }
        if key == self.query_path.trim_matches('/') && !self.query.is_empty() {
            url.set_query(Some(&self.query));
        }
        Ok(url)
    }
    fn request(&self, method: Method, path: &str, offset: Option<u64>) -> Result<Response> {
        let mut url = self.url(path)?;
        for _ in 0..10 {
            let mut request = self
                .client
                .request(method.clone(), url.clone())
                .header(header::ACCEPT_ENCODING, "identity");
            if url.origin() == self.base.origin() && !self.username.is_empty() {
                request = request.basic_auth(&self.username, Some(&self.password));
            }
            if let Some(offset) = offset {
                request = request.header(header::RANGE, format!("bytes={offset}-"));
            }
            let response = request
                .send()
                .map_err(|e| anyhow::anyhow!(e.without_url()))?;
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|h| h.to_str().ok())
                    .context("HTTP redirect has no valid destination")?;
                let next = url
                    .join(location)
                    .map_err(|_| anyhow::anyhow!("Invalid HTTP redirect"))?;
                ensure!(
                    matches!(next.scheme(), "http" | "https")
                        && next.username().is_empty()
                        && next.password().is_none(),
                    "Unsupported HTTP redirect destination."
                );
                ensure!(
                    !(url.scheme() == "https" && next.scheme() == "http"),
                    "The server redirected HTTPS to unencrypted HTTP."
                );
                url = next;
                continue;
            }
            if response.status() == StatusCode::NOT_FOUND {
                bail!("HTTP file or directory not found (404).");
            }
            if response.status() == StatusCode::UNAUTHORIZED {
                bail!("HTTP authentication required (401). Enter credentials to continue.");
            }
            ensure!(
                response.status().is_success(),
                "HTTP request failed ({}).",
                response.status().as_u16()
            );
            return Ok(response);
        }
        bail!("Too many HTTP redirects.")
    }
}
fn metadata(response: &Response) -> Metadata {
    let mut meta = Metadata::new(EntryMode::FILE);
    if let Some(length) = response
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
    {
        meta.set_content_length(length);
    } else {
        meta = meta.with_user_metadata(HashMap::from([(
            "rust-commander-size-known".into(),
            "false".into(),
        )]));
    }
    if let Some(modified) = response
        .headers()
        .get(header::LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| chrono::DateTime::parse_from_rfc2822(v).ok())
    {
        meta.set_last_modified(modified.to_utc());
    }
    if let Some(etag) = response
        .headers()
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
    {
        meta.set_etag(etag);
    }
    meta
}
impl Transport for Http {
    fn stat(&self, path: &str) -> Result<Metadata> {
        if path.ends_with('/') || path.is_empty() {
            return Ok(Metadata::new(EntryMode::DIR));
        }
        match self.request(Method::HEAD, path, None) {
            Ok(response) => Ok(metadata(&response)),
            Err(_) => Ok(metadata(&self.request(Method::GET, path, None)?)),
        }
    }
    fn list(&self, path: &str) -> Result<Vec<oio::Entry>> {
        let path = path.trim_matches('/');
        let response = self.request(Method::GET, path, None)?;
        let effective = response.url().clone();
        let html = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.to_ascii_lowercase().starts_with("text/html"));
        if !html {
            let meta = metadata(&response);
            ensure!(
                !path.trim_matches('/').is_empty(),
                "This HTTP address is a file. Paste its complete file URI."
            );
            return Ok(vec![oio::Entry::new(path.trim_matches('/'), meta)]);
        }
        ensure!(
            response.content_length().is_none_or(|n| n <= INDEX_LIMIT),
            "HTTP directory index exceeds the 8 MiB limit."
        );
        let mut bytes = Vec::new();
        response.take(INDEX_LIMIT + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= INDEX_LIMIT,
            "HTTP directory index exceeds the 8 MiB limit."
        );
        let mut batch = HashMap::new();
        let entries = parse_index(&bytes, &effective, path, &mut batch)?;
        ensure!(
            batch.len() < 20_000,
            "HTTP directory index exceeds the 20,000-link limit."
        );
        let mut links = self
            .links
            .lock()
            .map_err(|_| anyhow::anyhow!("HTTP link cache lock failed"))?;
        // Keep all current entries, including signed queries, when evicting old directories.
        if links.len() + batch.len() >= 20_000 {
            links.clear();
        }
        links.extend(batch);
        links.insert(path.trim_matches('/').into(), effective);
        Ok(entries)
    }
    fn read(&self, path: &str, offset: u64) -> Result<Box<dyn Read + Send>> {
        let mut response = self.request(Method::GET, path, (offset > 0).then_some(offset))?;
        if offset > 0 && response.status() != StatusCode::PARTIAL_CONTENT {
            ensure!(
                std::io::copy(&mut response.by_ref().take(offset), &mut std::io::sink())? == offset,
                "HTTP file is shorter than the requested offset."
            );
        } else if offset > 0 {
            let start = response
                .headers()
                .get(header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("bytes "))
                .and_then(|v| v.split('-').next())
                .and_then(|v| v.parse::<u64>().ok());
            ensure!(
                start == Some(offset),
                "HTTP server returned an incorrect byte range."
            );
        }
        Ok(Box::new(response))
    }
}
fn parse_index(
    bytes: &[u8],
    base: &Url,
    path: &str,
    links: &mut HashMap<String, Url>,
) -> Result<Vec<oio::Entry>> {
    let doc = Html::parse_document(&String::from_utf8_lossy(bytes));
    let headings = Selector::parse("title,h1").unwrap();
    let heading = doc
        .select(&headings)
        .flat_map(|e| e.text())
        .collect::<String>()
        .to_ascii_lowercase();
    let caddy = Selector::parse("table.entries, table#files, .file-listing, .listing").unwrap();
    ensure!(
        heading.contains("index of")
            || heading.contains("directory listing")
            || heading.contains("directory of")
            || doc.select(&caddy).next().is_some(),
        "This website is not a directory listing. Open it in your browser, or paste a direct download URI."
    );
    let selector = Selector::parse("a[href]").unwrap();
    let parent = base.path().trim_end_matches('/').to_owned() + "/";
    let prefix = path.trim_matches('/');
    let mut entries = BTreeMap::new();
    for anchor in doc.select(&selector) {
        let href = anchor.value().attr("href").unwrap_or_default();
        if href.starts_with(['?', '#']) {
            continue;
        }
        let Ok(url) = base.join(href) else {
            continue;
        };
        if url.origin() != base.origin() || !url.username().is_empty() || url.password().is_some() {
            continue;
        }
        let Some(child) = url.path().strip_prefix(&parent) else {
            continue;
        };
        let directory = child.ends_with('/');
        let child = child.trim_end_matches('/');
        if child.is_empty() || child.contains('/') {
            continue;
        }
        let Ok(name) = percent_encoding::percent_decode_str(child).decode_utf8() else {
            continue;
        };
        if crate::filesystem::validate_name(&name).is_err() || name.contains(':') {
            continue;
        }
        let key = if prefix.is_empty() {
            name.into_owned()
        } else {
            format!("{prefix}/{name}")
        };
        let mut meta = Metadata::new(if directory {
            EntryMode::DIR
        } else {
            EntryMode::FILE
        });
        meta = meta.with_user_metadata(HashMap::from([(
            "rust-commander-size-known".into(),
            "false".into(),
        )]));
        links.insert(key.clone(), url);
        entries.insert(
            key.clone(),
            oio::Entry::new(&(key + if directory { "/" } else { "" }), meta),
        );
    }
    Ok(entries.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexes_filter_and_decode_children() {
        let html=br#"<title>Index of /dir/</title><a href='../'>Parent</a><a href='?C=N'>sort</a><a href='a%20b.pdf?token=secret'>a</a><a href='folder/'>f</a><a href='https://elsewhere.test/dir/evil'>x</a><a href='deep/nested.txt'>n</a><a href='%2e%2e/'>bad</a><a href='javascript:alert(1)'>bad</a>"#;
        let mut links = HashMap::new();
        let entries = parse_index(
            html,
            &Url::parse("https://example.test/dir/").unwrap(),
            "dir",
            &mut links,
        )
        .unwrap();
        assert_eq!(
            entries.iter().map(|e| e.path()).collect::<Vec<_>>(),
            ["dir/a b.pdf", "dir/folder/"]
        );
        assert_eq!(links["dir/a b.pdf"].query(), Some("token=secret"));
    }
    #[test]
    fn empty_index_and_ordinary_website() {
        let base = Url::parse("https://example.test/").unwrap();
        assert!(
            parse_index(b"<h1>Index of /</h1>", &base, "", &mut HashMap::new())
                .unwrap()
                .is_empty()
        );
        assert!(
            parse_index(
                b"<h1>Welcome</h1><a href='login'>Login</a>",
                &base,
                "",
                &mut HashMap::new()
            )
            .is_err()
        );
    }
}
