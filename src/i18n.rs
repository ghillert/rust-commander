use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    sync::{
        OnceLock,
        atomic::{AtomicU8, Ordering},
    },
};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Language {
    #[default]
    System,
    English,
    German,
    Spanish,
}
impl Language {
    pub const ALL: [Self; 4] = [Self::System, Self::English, Self::German, Self::Spanish];
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "Follow system",
            Self::English => "English",
            Self::German => "Deutsch",
            Self::Spanish => "Español",
        }
    }
    fn resolved(self) -> Self {
        if self != Self::System {
            return self;
        }
        let locale = std::env::var("LC_ALL")
            .or_else(|_| std::env::var("LANG"))
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(sys_locale::get_locale)
            .unwrap_or_default()
            .to_lowercase();
        if locale.starts_with("de") {
            Self::German
        } else if locale.starts_with("es") {
            Self::Spanish
        } else {
            Self::English
        }
    }
}
static CURRENT: AtomicU8 = AtomicU8::new(1);
static DE: OnceLock<BTreeMap<String, String>> = OnceLock::new();
static ES: OnceLock<BTreeMap<String, String>> = OnceLock::new();
pub fn set(language: Language) {
    CURRENT.store(
        match language.resolved() {
            Language::German => 2,
            Language::Spanish => 3,
            _ => 1,
        },
        Ordering::Relaxed,
    );
}
pub fn translate(language: Language, text: &str) -> Cow<'_, str> {
    let catalog = match language.resolved() {
        Language::German => DE.get_or_init(|| {
            serde_json::from_str(include_str!("../locales/de.json")).expect("German catalog")
        }),
        Language::Spanish => ES.get_or_init(|| {
            serde_json::from_str(include_str!("../locales/es.json")).expect("Spanish catalog")
        }),
        _ => return Cow::Borrowed(text),
    };
    if let Some(value) = catalog.get(text) {
        return Cow::Owned(value.clone());
    }
    for suffix in [
        " ^", " v", " ↑", " ↓", "…", "  F2", "  F3", "  F4", "  F5", "  F6", "  F7", "  F8",
    ] {
        if let Some(base) = text.strip_suffix(suffix)
            && let Some(value) = catalog.get(base)
        {
            return Cow::Owned(format!("{value}{suffix}"));
        }
    }
    // Translate application context while leaving filenames and provider diagnostics intact.
    if let Some((context, detail)) = text.split_once(": ")
        && let Some(value) = catalog.get(context)
    {
        return Cow::Owned(format!("{value}: {detail}"));
    }
    Cow::Borrowed(text)
}
pub fn text(text: &str) -> Cow<'_, str> {
    translate(
        match CURRENT.load(Ordering::Relaxed) {
            2 => Language::German,
            3 => Language::Spanish,
            _ => Language::English,
        },
        text,
    )
}
