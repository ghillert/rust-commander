//! Browser-based authorization code flow with PKCE and a loopback callback.
use crate::remote::{Connection, Secrets};
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub struct Provider {
    pub authorization: String,
    pub token: String,
    pub scopes: &'static str,
}
pub fn provider(connection: &Connection) -> Result<Provider> {
    Ok(match connection.service.as_str() {
        "gdrive" => Provider {
            authorization: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token: "https://oauth2.googleapis.com/token".into(),
            scopes: "https://www.googleapis.com/auth/drive",
        },
        "dropbox" => Provider {
            authorization: "https://www.dropbox.com/oauth2/authorize".into(),
            token: "https://api.dropboxapi.com/oauth2/token".into(),
            scopes: "files.metadata.read files.metadata.write files.content.read files.content.write",
        },
        "onedrive" => {
            let tenant = if connection.oauth_tenant.is_empty() {
                "common"
            } else {
                &connection.oauth_tenant
            };
            if !tenant
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
            {
                bail!("Invalid Microsoft tenant identifier.");
            }
            let base = format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0");
            Provider {
                authorization: format!("{base}/authorize"),
                token: format!("{base}/token"),
                scopes: "Files.ReadWrite offline_access",
            }
        }
        _ => bail!("Browser sign-in is supported for Google Drive, Dropbox and OneDrive."),
    })
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn nonce() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn authorization_url(
    connection: &Connection,
    provider: &Provider,
    redirect: &str,
    state: &str,
    verifier: &str,
) -> Result<url::Url> {
    if connection.oauth_client_id.trim().is_empty() {
        bail!("Enter the OAuth client ID from your provider's desktop application registration.");
    }
    let mut url = url::Url::parse(&provider.authorization)?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", connection.oauth_client_id.as_str()),
        ("redirect_uri", redirect),
        ("response_type", "code"),
        ("scope", provider.scopes),
        ("state", state),
        ("code_challenge", &challenge(verifier)),
        ("code_challenge_method", "S256"),
    ]);
    if connection.service == "gdrive" {
        url.query_pairs_mut()
            .extend_pairs([("access_type", "offline"), ("prompt", "consent")]);
    }
    if connection.service == "dropbox" {
        url.query_pairs_mut()
            .append_pair("token_access_type", "offline");
    }
    Ok(url)
}

fn callback_url(target: &str, expected_state: &str) -> Result<url::Url> {
    let url = url::Url::parse(&format!("http://127.0.0.1{target}"))?;
    if url.path() != "/callback" {
        bail!("Unexpected OAuth callback path.");
    }
    let pairs = url.query_pairs().into_owned().collect::<Vec<_>>();
    let value = |name: &str| -> Result<&str> {
        let values = pairs
            .iter()
            .filter(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
            .collect::<Vec<_>>();
        if values.len() != 1 {
            bail!("Missing or duplicate OAuth callback field.");
        }
        Ok(values[0])
    };
    let state = value("state")?;
    let difference = state
        .as_bytes()
        .iter()
        .zip(expected_state.as_bytes())
        .fold(0u8, |different, (a, b)| different | (a ^ b));
    if state.len() != expected_state.len() || difference != 0 {
        bail!("OAuth state validation failed.");
    }
    Ok(url)
}

pub fn parse_callback(target: &str, expected_state: &str) -> Result<String> {
    let url = callback_url(target, expected_state)?;
    let pairs = url.query_pairs().into_owned().collect::<Vec<_>>();
    if pairs.iter().any(|(key, _)| key == "error") {
        bail!("Authorization was declined or failed in the browser.");
    }
    let codes = pairs
        .iter()
        .filter(|(key, _)| key == "code")
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>();
    if codes.len() != 1 {
        bail!("Missing or duplicate OAuth callback field.");
    }
    let code = codes[0];
    if code.is_empty() {
        bail!("The OAuth callback did not include an authorization code.");
    }
    Ok(code.into())
}

fn respond(stream: &mut TcpStream, status: &str, message: &str) {
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>Rust Commander</title><h1>Rust Commander</h1><p>{message}</p>"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

pub fn authorize(
    connection: &Connection,
    mut secrets: Secrets,
    cancel: Arc<AtomicBool>,
) -> Result<Secrets> {
    let provider = provider(connection)?;
    let listener = TcpListener::bind(("127.0.0.1", connection.oauth_port))
        .context("Cannot listen for OAuth callback; choose an unused callback port")?;
    listener.set_nonblocking(true)?;
    let host = if connection.service == "onedrive" {
        "localhost"
    } else {
        "127.0.0.1"
    };
    let redirect = format!("http://{host}:{}/callback", listener.local_addr()?.port());
    let state = nonce();
    let verifier = nonce();
    let url = authorization_url(connection, &provider, &redirect, &state, &verifier)?;
    open::that(url.as_str()).context("Could not open your browser")?;
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        if cancel.load(Ordering::Relaxed) {
            bail!("Browser sign-in cancelled.");
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                let request = match read_request(&mut stream) {
                    Ok(request) => request,
                    Err(_) => continue,
                };
                let mut line = request.lines().next().unwrap_or("").split_whitespace();
                if line.next() != Some("GET") {
                    respond(
                        &mut stream,
                        "405 Method Not Allowed",
                        "Use your browser to complete sign-in.",
                    );
                    continue;
                }
                let target = line.next().unwrap_or("");
                match parse_callback(target, &state) {
                    Ok(code) => {
                        let mut form = vec![
                            ("grant_type", "authorization_code"),
                            ("client_id", connection.oauth_client_id.as_str()),
                            ("redirect_uri", redirect.as_str()),
                            ("code", code.as_str()),
                            ("code_verifier", verifier.as_str()),
                        ];
                        if !secrets.oauth_client_secret.is_empty() {
                            form.push(("client_secret", &secrets.oauth_client_secret));
                        }
                        let tokens = exchange(&provider.token, &form);
                        match tokens {
                            Ok(value) => {
                                apply_tokens(&mut secrets, &value)?;
                                respond(
                                    &mut stream,
                                    "200 OK",
                                    "Sign-in completed. You can return to Rust Commander.",
                                );
                                return Ok(secrets);
                            }
                            Err(error) => {
                                respond(
                                    &mut stream,
                                    "400 Bad Request",
                                    "Sign-in could not be completed. Return to Rust Commander for details.",
                                );
                                return Err(error);
                            }
                        }
                    }
                    Err(error) => {
                        if let Ok(url) = callback_url(target, &state)
                            && url.query_pairs().any(|(key, _)| key == "error")
                        {
                            respond(
                                &mut stream,
                                "400 Bad Request",
                                "Sign-in was declined. You can return to Rust Commander.",
                            );
                            return Err(error);
                        }
                        respond(
                            &mut stream,
                            "400 Bad Request",
                            "This sign-in response is invalid. Use the original sign-in tab or start again.",
                        );
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(error) => return Err(error.into()),
        }
    }
    bail!("Browser sign-in timed out after three minutes.")
}

fn exchange(endpoint: &str, form: &[(&str, &str)]) -> Result<Value> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(25))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let response = client
        .post(endpoint)
        .form(form)
        .send()
        .context("OAuth token request failed")?;
    if !response.status().is_success() {
        bail!(
            "OAuth token request was rejected (HTTP {}). Check the client registration and redirect URI.",
            response.status()
        );
    }
    let mut bytes = Vec::new();
    response.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        bail!("OAuth token response exceeds the size limit.");
    }
    serde_json::from_slice(&bytes).context("Invalid OAuth token response")
}

fn apply_tokens(secrets: &mut Secrets, value: &Value) -> Result<()> {
    secrets.access_token = value
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .context("Provider did not return an access token")?
        .into();
    if let Some(refresh) = value.get("refresh_token").and_then(Value::as_str) {
        secrets.refresh_token = refresh.into();
    }
    let expires = value
        .get("expires_in")
        .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
        .unwrap_or(3600);
    secrets.expires_at = now().saturating_add(expires.min(365 * 24 * 3600));
    Ok(())
}

pub fn refresh(connection: &Connection, secrets: &Secrets) -> Result<Secrets> {
    if secrets.refresh_token.is_empty() {
        bail!("The cloud session expired. Sign in again from Connections.");
    }
    let provider = provider(connection)?;
    let mut form = vec![
        ("grant_type", "refresh_token"),
        ("client_id", connection.oauth_client_id.as_str()),
        ("refresh_token", secrets.refresh_token.as_str()),
    ];
    if !secrets.oauth_client_secret.is_empty() {
        form.push(("client_secret", &secrets.oauth_client_secret));
    }
    let value = exchange(&provider.token, &form)?;
    let mut updated = secrets.clone();
    apply_tokens(&mut updated, &value)?;
    Ok(updated)
}

fn read_request(stream: &mut TcpStream) -> Result<String> {
    let mut buffer = Vec::new();
    let mut chunk = [0; 1024];
    while !buffer.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            bail!("Incomplete OAuth callback request.");
        }
        buffer.extend_from_slice(&chunk[..count]);
        if buffer.len() > 8192 {
            bail!("OAuth callback request exceeds the size limit.");
        }
    }
    Ok(String::from_utf8(buffer)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn token_server(status: &str, body: String) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let status = status.to_string();
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                header.push(byte[0]);
            }
            let header = String::from_utf8(header).unwrap();
            let size = header
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            let mut form = vec![0; size];
            stream.read_exact(&mut form).unwrap();
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            String::from_utf8(form).unwrap()
        });
        (endpoint, thread)
    }
    #[test]
    fn token_exchange_encodes_pkce_and_keeps_refresh_token_until_rotated() {
        let (endpoint, thread) = token_server(
            "200 OK",
            r#"{"access_token":"new-token","expires_in":"120"}"#.into(),
        );
        let value = exchange(
            &endpoint,
            &[
                ("grant_type", "authorization_code"),
                ("code_verifier", "a+b/code"),
                ("code", "private-code"),
            ],
        )
        .unwrap();
        let form = thread.join().unwrap();
        let fields = url::form_urlencoded::parse(form.as_bytes())
            .into_owned()
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(fields["code_verifier"], "a+b/code");
        let mut secrets = Secrets {
            refresh_token: "old-refresh".into(),
            ..Default::default()
        };
        apply_tokens(&mut secrets, &value).unwrap();
        assert_eq!(secrets.access_token, "new-token");
        assert_eq!(secrets.refresh_token, "old-refresh");
        assert!(secrets.expires_at >= now() + 119);
        apply_tokens(&mut secrets, &serde_json::json!({"access_token":"rotated-access","refresh_token":"rotated-refresh","expires_in":60})).unwrap();
        assert_eq!(secrets.refresh_token, "rotated-refresh");
    }
    #[test]
    fn token_errors_do_not_expose_provider_response_or_secrets() {
        let (endpoint, thread) = token_server(
            "400 Bad Request",
            r#"{"error_description":"private-code secret-value"}"#.into(),
        );
        let error = exchange(&endpoint, &[("client_secret", "secret-value")])
            .unwrap_err()
            .to_string();
        thread.join().unwrap();
        assert!(error.contains("400"));
        assert!(!error.contains("secret-value"));
        assert!(!error.contains("private-code"));
    }
    #[test]
    fn callback_reader_handles_fragmented_http_and_rejects_oversized_input() {
        for oversized in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let thread = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                read_request(&mut stream)
            });
            let mut client = TcpStream::connect(address).unwrap();
            if oversized {
                client.write_all(&vec![b'x'; 9000]).unwrap();
                assert!(thread.join().unwrap().is_err());
            } else {
                client
                    .write_all(b"GET /callback?state=ok&code=sample HTTP/1.1\r\n")
                    .unwrap();
                std::thread::sleep(Duration::from_millis(10));
                client.write_all(b"Host: localhost\r\n\r\n").unwrap();
                assert!(thread.join().unwrap().unwrap().contains("code=sample"));
            }
        }
    }
}
