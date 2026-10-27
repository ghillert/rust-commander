//! Explicit release checks and shareable diagnostics without local paths or credentials.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{io::Read, time::Duration};

pub const REPOSITORY: &str = "https://github.com/ghillert/rust-commander";
pub const DOCUMENTATION: &str = "https://github.com/ghillert/rust-commander/blob/main/README.md";
pub const COMMUNITY: &str = "https://github.com/ghillert/rust-commander/issues";
/// Repository .github/FUNDING.yml names this GitHub Sponsors account.
pub const DONATION: &str = "https://github.com/sponsors/ghillert";

pub fn web_link(value: &str) -> Result<url::Url> {
    let url = url::Url::parse(value)?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none(),
        "Use an HTTPS website address without credentials"
    );
    Ok(url)
}
const LATEST: &str = "https://api.github.com/repos/ghillert/rust-commander/releases/latest";
const RESPONSE_LIMIT: u64 = 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub html_url: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}
#[derive(Clone, Debug)]
pub struct Update {
    pub release: Option<Release>,
    pub newer: bool,
}
fn version(text: &str) -> Result<[u64; 3]> {
    let text = text.strip_prefix('v').unwrap_or(text);
    let core = text.split(['-', '+']).next().unwrap_or_default();
    let parts = core
        .split('.')
        .map(str::parse)
        .collect::<Result<Vec<u64>, _>>()?;
    ensure!(
        parts.len() == 3,
        "The release has an unsupported version number"
    );
    Ok([parts[0], parts[1], parts[2]])
}
pub fn parse_release(bytes: &[u8], current: &str) -> Result<Update> {
    ensure!(
        bytes.len() as u64 <= RESPONSE_LIMIT,
        "Release response exceeds 1 MiB"
    );
    let release: Release = serde_json::from_slice(bytes).context("Invalid release information")?;
    ensure!(
        !release.draft && !release.prerelease,
        "The server did not return a stable release"
    );
    ensure!(
        release.tag_name.len() <= 128 && release.name.len() <= 512,
        "Release information is too long"
    );
    let url = url::Url::parse(&release.html_url)?;
    ensure!(
        url.scheme() == "https"
            && url.host_str() == Some("github.com")
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none()
            && url
                .path()
                .starts_with("/ghillert/rust-commander/releases/tag/")
            && url.query().is_none()
            && url.fragment().is_none(),
        "Unexpected release page"
    );
    let latest = version(&release.tag_name)?;
    let installed = version(current)?;
    let newer = latest > installed
        || (latest == installed && current.split('+').next().unwrap_or(current).contains('-'));
    Ok(Update {
        release: Some(release),
        newer,
    })
}
pub fn check_updates() -> Result<Update> {
    check_at(LATEST, env!("CARGO_PKG_VERSION"))
}
#[doc(hidden)]
pub fn check_at(endpoint: &str, current: &str) -> Result<Update> {
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("Rust-Commander/", env!("CARGO_PKG_VERSION")))
        .build()?
        .get(endpoint)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .context("Could not reach GitHub; check your connection and try again")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Update {
            release: None,
            newer: false,
        });
    }
    if response.status() == reqwest::StatusCode::FORBIDDEN
        || response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        anyhow::bail!("GitHub refused the request or its rate limit was reached. Try again later.");
    }
    let mut bytes = Vec::new();
    response
        .error_for_status()?
        .take(RESPONSE_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    parse_release(&bytes, current)
}

pub fn diagnostics() -> String {
    format!(
        "Rust Commander {}\nOS: {}\nArchitecture: {}\nBuild: {}\nUI: Slint 1.18.1\nUpdates: manual checks of GitHub stable releases\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        if cfg!(debug_assertions) {
            "development"
        } else {
            "release"
        }
    )
}
pub fn bug_report_url() -> String {
    let mut url = url::Url::parse(&format!("{REPOSITORY}/issues/new")).unwrap();
    let body = format!(
        "### What happened?\n\nDescribe the problem here.\n\n### Steps to reproduce\n\n1. \n\n### Expected behavior\n\n\n### Diagnostics\n\n```text\n{}```\n\nReview any additional logs or screenshots for private information before submitting.\n",
        diagnostics()
    );
    url.query_pairs_mut()
        .append_pair("title", "Bug: ")
        .append_pair("body", &body);
    url.to_string()
}
