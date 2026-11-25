//! Encoding-preserving editor documents and bounded local text pages.
use anyhow::{Result, bail, ensure};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Encoding {
    #[default]
    Utf8,
    Utf16Le,
    Utf16Be,
    Windows1252,
    Latin1,
}
impl Encoding {
    pub const ALL: [Self; 5] = [
        Self::Utf8,
        Self::Utf16Le,
        Self::Utf16Be,
        Self::Windows1252,
        Self::Latin1,
    ];
    /// Returns the canonical encoding name used in the viewer and editor.
    pub fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf16Le => "UTF-16 LE",
            Self::Utf16Be => "UTF-16 BE",
            Self::Windows1252 => "Windows-1252",
            Self::Latin1 => "ISO-8859-1",
        }
    }
    /// Looks up an encoding by its exact, case-sensitive [`label`](Self::label).
    pub fn named(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.label() == value)
    }
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Document {
    pub original: Vec<u8>,
    pub text: String,
    pub encoding: Encoding,
    pub original_encoding: Encoding,
    pub bom: bool,
    pub crlf: bool,
}
impl Document {
    /// Detects UTF-16 byte order from a leading BOM and otherwise decodes as UTF-8. Retains
    /// the original bytes and line-ending information for editing.
    ///
    /// # Errors
    ///
    /// Returns an error when [`Self::decode`] rejects the input.
    ///
    /// # Examples
    ///
    /// ```
    /// use rust_commander::text_document::{Document, Encoding};
    ///
    /// let doc = Document::detect(b"\xef\xbb\xbffirst\r\n".to_vec())?;
    /// assert_eq!(doc.encoding, Encoding::Utf8);
    /// assert_eq!(doc.encode("second\n")?, b"\xef\xbb\xbfsecond\r\n");
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn detect(bytes: Vec<u8>) -> Result<Self> {
        let encoding = if bytes.starts_with(&[0xff, 0xfe]) {
            Encoding::Utf16Le
        } else if bytes.starts_with(&[0xfe, 0xff]) {
            Encoding::Utf16Be
        } else {
            Encoding::Utf8
        };
        Self::decode(bytes, encoding)
    }
    /// Decodes text using the specified encoding and removes a matching BOM from the
    /// displayed text. The document retains the original bytes and records whether all line
    /// feeds use CRLF.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid encoded text or a decoded NUL character.
    pub fn decode(bytes: Vec<u8>, encoding: Encoding) -> Result<Self> {
        let bom = match encoding {
            Encoding::Utf8 => bytes.starts_with(&[0xef, 0xbb, 0xbf]),
            Encoding::Utf16Le => bytes.starts_with(&[0xff, 0xfe]),
            Encoding::Utf16Be => bytes.starts_with(&[0xfe, 0xff]),
            _ => false,
        };
        let offset = if bom {
            if encoding == Encoding::Utf8 { 3 } else { 2 }
        } else {
            0
        };
        let raw = &bytes[offset..];
        let text = match encoding {
            Encoding::Utf8 => String::from_utf8(raw.to_vec())?,
            Encoding::Utf16Le | Encoding::Utf16Be => {
                ensure!(raw.len().is_multiple_of(2), "Incomplete UTF-16 code unit");
                let units = raw
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| {
                        if encoding == Encoding::Utf16Le {
                            u16::from_le_bytes([b[0], b[1]])
                        } else {
                            u16::from_be_bytes([b[0], b[1]])
                        }
                    })
                    .collect::<Vec<_>>();
                String::from_utf16(&units)?
            }
            Encoding::Windows1252 => {
                let (text, errors) = encoding_rs::WINDOWS_1252.decode_without_bom_handling(raw);
                ensure!(!errors, "Invalid Windows-1252 text");
                text.into_owned()
            }
            Encoding::Latin1 => raw.iter().map(|b| char::from(*b)).collect(),
        };
        ensure!(!text.contains('\0'), "Binary input: choose the hex viewer");
        let crlf = text.contains("\r\n") && !text.replace("\r\n", "").contains('\n');
        Ok(Self {
            original: bytes,
            text,
            encoding,
            original_encoding: encoding,
            bom,
            crlf,
        })
    }
    /// Encodes edited text with the document's current encoding, BOM setting, and CRLF
    /// preference.
    ///
    /// # Errors
    ///
    /// Returns an error if the selected single-byte encoding cannot represent a character.
    pub fn encode(&self, text: &str) -> Result<Vec<u8>> {
        let text = if self.crlf {
            text.replace("\r\n", "\n").replace('\n', "\r\n")
        } else {
            text.to_owned()
        };
        let mut bytes = Vec::new();
        match self.encoding {
            Encoding::Utf8 => {
                if self.bom {
                    bytes.extend([0xef, 0xbb, 0xbf]);
                }
                bytes.extend(text.as_bytes());
            }
            Encoding::Utf16Le | Encoding::Utf16Be => {
                if self.bom {
                    bytes.extend(if self.encoding == Encoding::Utf16Le {
                        [0xff, 0xfe]
                    } else {
                        [0xfe, 0xff]
                    });
                }
                for unit in text.encode_utf16() {
                    bytes.extend(if self.encoding == Encoding::Utf16Le {
                        unit.to_le_bytes()
                    } else {
                        unit.to_be_bytes()
                    });
                }
            }
            Encoding::Windows1252 => {
                let (encoded, _, errors) = encoding_rs::WINDOWS_1252.encode(&text);
                ensure!(
                    !errors,
                    "Text cannot be represented in Windows-1252; select UTF-8 instead"
                );
                bytes.extend(encoded.as_ref());
            }
            Encoding::Latin1 => {
                for c in text.chars() {
                    ensure!(
                        u32::from(c) <= 255,
                        "Text cannot be represented in ISO-8859-1"
                    );
                    bytes.push(c as u8);
                }
            }
        }
        Ok(bytes)
    }
}

pub const PAGE_SIZE: u64 = 256 * 1024;
#[derive(Clone, Debug)]
pub struct Page {
    pub offset: u64,
    pub next: u64,
    pub total: u64,
    pub document: Document,
    pub source: Option<std::sync::Arc<crate::viewer::source::Source>>,
}
impl Page {
    /// Reads a bounded local text page near the byte offset, clamping it to the file length
    /// and aligning UTF-16 reads to a code unit. Continue at the returned page's `next`
    /// offset.
    ///
    /// # Errors
    ///
    /// Returns an error on file I/O or if the requested page cannot be decoded.
    pub fn read(path: &Path, offset: u64, encoding: Encoding) -> Result<Self> {
        let mut file = File::open(path)?;
        let total = file.metadata()?.len();
        let mut offset = offset.min(total);
        if matches!(encoding, Encoding::Utf16Le | Encoding::Utf16Be) {
            offset -= offset % 2;
        }
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = Vec::new();
        file.take(PAGE_SIZE + 4).read_to_end(&mut bytes)?;
        Self::from_bytes(bytes, offset, total, encoding)
    }
    /// Decodes a page from bytes beginning at `offset` in a file of `total` bytes. Supply
    /// up to [`PAGE_SIZE`] plus four bytes of lookahead so UTF-8 characters and UTF-16
    /// surrogate pairs can be kept whole at page boundaries.
    ///
    /// # Errors
    ///
    /// Returns an error if decoding fails or the page cannot advance before EOF.
    pub fn from_bytes(
        mut bytes: Vec<u8>,
        mut offset: u64,
        total: u64,
        encoding: Encoding,
    ) -> Result<Self> {
        if encoding == Encoding::Utf8 {
            while offset > 0 && bytes.first().is_some_and(|b| b & 0xc0 == 0x80) {
                bytes.remove(0);
                offset += 1;
            }
            bytes.truncate(PAGE_SIZE as usize);
            if offset + (bytes.len() as u64) < total {
                while std::str::from_utf8(&bytes).is_err_and(|e| e.error_len().is_none()) {
                    bytes.pop();
                }
            }
        } else {
            bytes.truncate(PAGE_SIZE as usize);
            if matches!(encoding, Encoding::Utf16Le | Encoding::Utf16Be)
                && bytes.len() >= 2
                && offset + (bytes.len() as u64) < total
            {
                let last = &bytes[bytes.len() - 2..];
                let unit = if encoding == Encoding::Utf16Le {
                    u16::from_le_bytes([last[0], last[1]])
                } else {
                    u16::from_be_bytes([last[0], last[1]])
                };
                if (0xd800..=0xdbff).contains(&unit) {
                    bytes.truncate(bytes.len() - 2);
                }
            }
        }
        if offset > 0
            && matches!(encoding, Encoding::Utf16Le | Encoding::Utf16Be)
            && bytes.len() >= 2
        {
            let first = if encoding == Encoding::Utf16Le {
                u16::from_le_bytes([bytes[0], bytes[1]])
            } else {
                u16::from_be_bytes([bytes[0], bytes[1]])
            };
            if (0xdc00..=0xdfff).contains(&first) {
                bytes.drain(..2);
                offset += 2;
            }
        }
        let next = offset + bytes.len() as u64;
        let document = Document::decode(bytes, encoding)?;
        if next == offset && offset < total {
            bail!("Could not decode this text page");
        }
        Ok(Self {
            offset,
            next,
            total,
            document,
            source: None,
        })
    }
}
