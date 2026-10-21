//! Filename-only resolution for bundled icons. Shared by local and virtual files.
mod catalog;
use std::{collections::HashMap, sync::OnceLock};

struct Lookup {
    names: HashMap<&'static str, &'static str>,
    extensions: HashMap<&'static str, &'static str>,
}

pub fn file_icon(name: &str, directory: bool) -> &'static str {
    if directory {
        return catalog::FOLDER;
    }
    let name = name.to_ascii_lowercase();
    static LOOKUP: OnceLock<Lookup> = OnceLock::new();
    let lookup = LOOKUP.get_or_init(|| Lookup {
        names: catalog::RULES
            .iter()
            .flat_map(|rule| rule.names.iter().map(move |name| (*name, rule.icon)))
            .collect(),
        extensions: catalog::RULES
            .iter()
            .flat_map(|rule| rule.extensions.iter().map(move |ext| (*ext, rule.icon)))
            .collect(),
    });
    if let Some(icon) = lookup.names.get(name.as_str()) {
        return icon;
    }
    // Prefixes cover explicit conventional filename families such as .env.*.
    if let Some(rule) = catalog::RULES
        .iter()
        .find(|rule| rule.prefixes.iter().any(|prefix| name.starts_with(prefix)))
    {
        return rule.icon;
    }
    // Walk suffixes from longest to shortest, with no per-file scan of the
    // complete extension catalog, even in directories with many thousands of rows.
    for (dot, _) in name.match_indices('.') {
        if let Some(icon) = lookup.extensions.get(&name[dot + 1..]) {
            return icon;
        }
    }
    catalog::FILE
}

pub(crate) fn specific(icon: &str) -> bool {
    icon != catalog::FILE && icon != catalog::FOLDER
}

#[cfg(feature = "screenshot")]
pub(crate) fn catalog_ids() -> &'static [&'static str] {
    catalog::IDS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_compound_extensions_and_case_resolve_without_reading_files() {
        for (name, expected) in [
            ("sample.PDF", "pdf"),
            ("sample.XML", "xml"),
            ("README.md", "readme"),
            ("Cargo.toml", "rust"),
            (".gitignore", "git"),
            (".env.local", "settings"),
            ("Dockerfile.prod", "docker"),
            ("types.d.ts", "typescript"),
            ("archive.TAR.GZ", "zip"),
            ("Makefile", "makefile"),
            ("CMakeLists.txt", "cmake"),
            ("unknown", "file"),
            (".unknown", "file"),
            ("résumé.pdf", "pdf"),
        ] {
            assert_eq!(file_icon(name, false), expected, "{name}");
        }
        assert_eq!(file_icon("folder.pdf", true), "folder");
        assert!(specific("xml"));
        assert!(!specific("file"));
    }
}
