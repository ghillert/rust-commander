//! Preserve native format IDs: standard Windows formats are not registered names.
use clipboard_win::{Clipboard, formats::*, raw};

pub struct Snapshot(Vec<(u32, Vec<u8>)>);

impl Snapshot {
    pub fn capture() -> Self {
        let _clipboard = Clipboard::new_attempts(10).expect("Open clipboard for snapshot");
        let formats = raw::EnumFormats::new().collect::<Vec<_>>();
        let saved = formats
            .into_iter()
            .map(|format| {
                // GDI/private handles cannot be preserved as byte buffers.
                // Refuse them before changing any existing clipboard contents.
                assert!(
                    matches!(
                        format,
                        CF_TEXT | CF_OEMTEXT | CF_UNICODETEXT | CF_LOCALE | CF_DIB | CF_DIBV5
                            | CF_HDROP | CF_DIF | CF_SYLK | CF_TIFF | CF_RIFF | CF_WAVE
                            | CF_PENDATA | CF_DSPTEXT
                    ) || (0xc000..=0xffff).contains(&format),
                    "Cannot preserve handle-based clipboard format {format}; use a text or file clipboard item for this interactive test"
                );
                let mut bytes = Vec::new();
                raw::get_vec(format, &mut bytes)
                    .unwrap_or_else(|error| panic!("Read clipboard format {format}: {error}"));
                assert!(
                    !bytes.is_empty(),
                    "Cannot preserve empty clipboard format {format}"
                );
                (format, bytes)
            })
            .collect();
        Self(saved)
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        let _clipboard = Clipboard::new_attempts(10).expect("Open clipboard for restore");
        raw::empty().expect("Clear test clipboard contents");
        for (format, bytes) in &self.0 {
            raw::set_without_clear(*format, bytes)
                .unwrap_or_else(|error| panic!("Restore clipboard format {format}: {error}"));
        }
    }
}
