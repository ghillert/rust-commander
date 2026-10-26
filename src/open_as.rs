//! Explicit interpretation without changing the selected file's name or contents.
use crate::{
    archive::Format,
    filesystem::{Entry, Location},
    viewer::Mode,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Viewer(Mode),
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
    pub location: Location,
    pub entry: Entry,
    pub kind: Kind,
    pub password: String,
}
impl Form {
    pub fn label(&self) -> &'static str {
        CHOICES.iter().find(|(_, k)| *k == self.kind).unwrap().0
    }
}
