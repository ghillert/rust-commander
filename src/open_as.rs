//! Explicit interpretation without changing the selected file's name or contents.
use crate::{
    archive::Format,
    filesystem::{Entry, Location},
    viewer::Mode,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Open the selected entry using the specified built-in viewer mode.
    Viewer(Mode),
    /// Interpret the selected entry as the specified archive format.
    Archive(Format),
}
pub const CHOICES: &[(&str, Kind)] = &[
    ("Text", Kind::Viewer(Mode::Text)),
    ("Hexadecimal", Kind::Viewer(Mode::Hex)),
    ("Image", Kind::Viewer(Mode::Image)),
    ("PDF", Kind::Viewer(Mode::Pdf)),
    ("ZIP archive", Kind::Archive(Format::Zip)),
    ("7z archive", Kind::Archive(Format::SevenZ)),
    ("TAR archive", Kind::Archive(Format::Tar)),
    ("TAR.GZ archive", Kind::Archive(Format::TarGz)),
    ("TAR.BZ2 archive", Kind::Archive(Format::TarBz2)),
    ("TAR.XZ archive", Kind::Archive(Format::TarXz)),
    (
        "Other archive (detect format)",
        Kind::Archive(Format::Extended),
    ),
];
pub struct Form {
    /// Pane location providing access to the selected entry.
    pub location: Location,
    /// Entry whose contents will be opened with the selected interpretation.
    pub entry: Entry,
    /// Viewer mode or archive format selected in the dialog.
    pub kind: Kind,
    /// Password entered for opening an encrypted archive, empty if none was supplied.
    pub password: String,
}
impl Form {
    /// Returns the dialog label for the selected viewer or archive interpretation.
    ///
    /// # Panics
    ///
    /// Panics if the form contains an interpretation absent from [`CHOICES`].
    pub fn label(&self) -> &'static str {
        CHOICES.iter().find(|(_, k)| *k == self.kind).unwrap().0
    }
}
