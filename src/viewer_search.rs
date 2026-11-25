//! Streaming literal/hex search with bounded memory, including chunk boundaries.
use crate::{
    operations::Task,
    text_document::{Document, Encoding},
};
use anyhow::{Result, ensure};
use std::io::Read;
/// Converts a query to the bytes to search for. Hex mode accepts whitespace between digit
/// pairs. Text mode encodes the query in the selected encoding without a byte-order mark.
///
/// # Errors
///
/// Returns an error for an empty query, a query over 4,096 bytes, invalid hex, or
/// unrepresentable text.
pub fn needle(query: &str, hex: bool, encoding: Encoding) -> Result<Vec<u8>> {
    ensure!(
        !query.is_empty() && query.len() <= 4096,
        "Enter a search of 1–4096 bytes"
    );
    if hex {
        let compact: String = query.chars().filter(|c| !c.is_whitespace()).collect();
        ensure!(
            compact.len().is_multiple_of(2) && compact.is_ascii(),
            "Use pairs of hexadecimal digits"
        );
        let bytes = (0..compact.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&compact[i..i + 2], 16))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ensure!(!bytes.is_empty(), "Enter hexadecimal bytes");
        Ok(bytes)
    } else {
        Document {
            original: Vec::new(),
            text: String::new(),
            encoding,
            original_encoding: encoding,
            bom: false,
            crlf: false,
        }
        .encode(query)
    }
}
/// Returns the byte offset of the first aligned match at or after `start`, measured from
/// the reader's initial position. Matches may cross read boundaries. An alignment of zero
/// or one accepts any offset. End of input yields `None`.
///
/// # Errors
///
/// Returns an error for an empty needle, a failed read, or cancellation.
pub fn find(
    reader: &mut dyn Read,
    needle: &[u8],
    start: u64,
    alignment: u64,
    task: &Task,
) -> Result<Option<u64>> {
    ensure!(!needle.is_empty(), "Enter a search string");
    let mut prefix = vec![0; needle.len()];
    for i in 1..needle.len() {
        let mut j = prefix[i - 1];
        while j > 0 && needle[i] != needle[j] {
            j = prefix[j - 1];
        }
        if needle[i] == needle[j] {
            j += 1;
        }
        prefix[i] = j;
    }
    let (mut offset, mut matched) = (0u64, 0usize);
    let mut buffer = [0; 65536];
    loop {
        task.check()?;
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            return Ok(None);
        }
        for &b in &buffer[..n] {
            if offset >= start {
                while matched > 0 && b != needle[matched] {
                    matched = prefix[matched - 1];
                }
                if b == needle[matched] {
                    matched += 1;
                }
                if matched == needle.len() {
                    let at = offset + 1 - needle.len() as u64;
                    if at.is_multiple_of(alignment.max(1)) {
                        return Ok(Some(at));
                    }
                    matched = prefix[matched - 1];
                }
            }
            offset += 1;
        }
    }
}
/// Parses a byte offset in decimal or with a lowercase `0x` hexadecimal prefix. Surrounding
/// whitespace is ignored.
///
/// # Errors
///
/// Returns an error if the value is invalid or outside the range of `u64`.
pub fn offset(text: &str) -> Result<u64> {
    Ok(if let Some(s) = text.trim().strip_prefix("0x") {
        u64::from_str_radix(s, 16)?
    } else {
        text.trim().parse()?
    })
}
/// Reads up to 64 KiB of a local file as hex rows. The first row begins at the 16-byte
/// boundary at or before `offset` and is labeled with its file offset.
///
/// # Errors
///
/// Returns an error if the offset exceeds the file size or the file cannot be opened,
/// positioned, or read.
pub fn hex_page(path: &std::path::Path, offset: u64) -> Result<String> {
    use std::io::{Seek, SeekFrom};
    let mut input = std::fs::File::open(path)?;
    ensure!(
        offset <= input.metadata()?.len(),
        "Offset exceeds the file size"
    );
    let start = offset - offset % 16;
    input.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    input.take(64 * 1024).read_to_end(&mut bytes)?;
    let text = crate::viewer::hex_dump(&bytes);
    Ok(text
        .lines()
        .enumerate()
        .map(|(i, line)| format!("{:08x}{}", start + i as u64 * 16, &line[8..]))
        .collect::<Vec<_>>()
        .join("\n"))
}
