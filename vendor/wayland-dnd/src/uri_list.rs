//! `text/uri-list` (RFC 2483) for local files.

use std::path::{Path, PathBuf};

#[cfg(unix)]
fn bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().into_owned().into_bytes()
}

#[cfg(unix)]
fn path(bytes: Vec<u8>) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(bytes).into()
}

#[cfg(not(unix))]
fn path(bytes: Vec<u8>) -> PathBuf {
    String::from_utf8_lossy(&bytes).into_owned().into()
}

/// One `file://` line per absolute path, ended with CRLF, with every byte other than an
/// ASCII letter, a digit or one of `-._~/` percent-encoded. Relative paths are skipped.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn encode(paths: &[PathBuf]) -> Vec<u8> {
    let mut out = Vec::new();
    for path in paths.iter().filter(|path| path.is_absolute()) {
        out.extend_from_slice(b"file://");
        for byte in bytes(path) {
            if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
                out.push(byte);
            } else {
                out.extend_from_slice(format!("%{byte:02X}").as_bytes());
            }
        }
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// Local files only: comments, other schemes and remote hosts are skipped.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn decode(data: &[u8]) -> Vec<PathBuf> {
    data.split(|byte| *byte == b'\n' || *byte == b'\r')
        .filter_map(|line| {
            let rest = line.strip_prefix(b"file://")?;
            let rest = rest.strip_prefix(b"localhost").unwrap_or(rest);
            if !rest.starts_with(b"/") {
                return None;
            }
            let mut decoded = Vec::with_capacity(rest.len());
            let mut index = 0;
            while index < rest.len() {
                let hex = (rest[index] == b'%')
                    .then(|| rest.get(index + 1..index + 3))
                    .flatten()
                    .and_then(|hex| std::str::from_utf8(hex).ok())
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok());
                match hex {
                    Some(byte) => {
                        decoded.push(byte);
                        index += 3;
                    }
                    None => {
                        decoded.push(rest[index]);
                        index += 1;
                    }
                }
            }
            Some(path(decoded))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_spaces_unicode_and_reserved_characters() {
        let paths = vec![
            PathBuf::from("/home/user/Mis documentos/año #1 100%.txt"),
            PathBuf::from("/tmp/a-b_c.~d"),
        ];
        let encoded = encode(&paths);
        assert_eq!(
            String::from_utf8(encoded.clone()).unwrap(),
            "file:///home/user/Mis%20documentos/a%C3%B1o%20%231%20100%25.txt\r\nfile:///tmp/a-b_c.~d\r\n"
        );
        assert_eq!(decode(&encoded), paths);
    }

    #[test]
    fn decode_skips_comments_remote_hosts_other_schemes_and_bad_escapes() {
        let data = b"# comment\r\nhttps://example.com/a\r\nfile://server/share/a\r\nfile://localhost/etc/hosts\nfile:///tmp/50%zz\r\n";
        assert_eq!(
            decode(data),
            vec![PathBuf::from("/etc/hosts"), PathBuf::from("/tmp/50%zz")]
        );
    }

    #[test]
    fn encode_drops_relative_paths() {
        assert!(encode(&[PathBuf::from("relative/file")]).is_empty());
    }
}
