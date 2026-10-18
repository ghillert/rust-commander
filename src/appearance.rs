//! Platform appearance preferences and colors. Slint renders the widgets.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DesktopStyle {
    #[default]
    Automatic,
    MacOs,
    Windows,
    Ubuntu,
    Gnome,
}
impl DesktopStyle {
    pub const ALL: [Self; 5] = [
        Self::Automatic,
        Self::MacOs,
        Self::Windows,
        Self::Ubuntu,
        Self::Gnome,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic (this desktop)",
            Self::MacOs => "macOS",
            Self::Windows => "Windows",
            Self::Ubuntu => "Ubuntu",
            Self::Gnome => "Linux / GNOME",
        }
    }
    pub fn resolve(self) -> Self {
        if self != Self::Automatic {
            return self;
        }
        match std::env::consts::OS {
            "macos" => Self::MacOs,
            "windows" => Self::Windows,
            _ => detect_linux(&std::fs::read_to_string("/etc/os-release").unwrap_or_default()),
        }
    }
}
pub fn detect_linux(release: &str) -> DesktopStyle {
    if release.lines().any(|line| {
        line.strip_prefix("ID=")
            .is_some_and(|id| id.trim_matches(['\'', '"']) == "ubuntu")
    }) {
        DesktopStyle::Ubuntu
    } else {
        DesktopStyle::Gnome
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorScheme {
    #[default]
    System,
    Light,
    Dark,
}
impl ColorScheme {
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "Follow system",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
}
pub struct Palette {
    pub accent: slint::Color,
    pub sidebar: slint::Color,
    pub toolbar: slint::Color,
    pub background: slint::Color,
    pub line: slint::Color,
    pub stripe: slint::Color,
    pub selection: slint::Color,
    pub selection_text: slint::Color,
    pub inactive_selection: slint::Color,
    pub folder: slint::Color,
    pub radius: u8,
}

impl Palette {
    pub fn new(desktop: DesktopStyle, dark: bool) -> Self {
        let rgb = slint::Color::from_rgb_u8;
        let accent = match desktop.resolve() {
            DesktopStyle::MacOs => {
                if dark {
                    rgb(10, 132, 255)
                } else {
                    rgb(0, 112, 235)
                }
            }
            DesktopStyle::Windows => {
                if dark {
                    rgb(96, 205, 255)
                } else {
                    rgb(0, 103, 192)
                }
            }
            DesktopStyle::Ubuntu => rgb(233, 84, 32),
            _ => {
                if dark {
                    rgb(120, 174, 237)
                } else {
                    rgb(53, 132, 228)
                }
            }
        };
        let windows = desktop.resolve() == DesktopStyle::Windows;
        Self {
            accent,
            sidebar: if dark {
                rgb(38, 38, 42)
            } else {
                rgb(239, 239, 243)
            },
            toolbar: if dark {
                rgb(45, 45, 48)
            } else {
                rgb(246, 246, 248)
            },
            background: if dark {
                rgb(29, 29, 31)
            } else {
                slint::Color::from_rgb_u8(255, 255, 255)
            },
            line: if dark {
                rgb(65, 65, 69)
            } else {
                rgb(218, 218, 223)
            },
            stripe: if dark {
                rgb(33, 33, 36)
            } else {
                rgb(247, 248, 250)
            },
            selection: if windows {
                if dark {
                    rgb(43, 64, 79)
                } else {
                    rgb(204, 232, 255)
                }
            } else {
                match desktop.resolve() {
                    DesktopStyle::Ubuntu => rgb(177, 55, 15),
                    DesktopStyle::MacOs => rgb(0, 101, 211),
                    _ => rgb(35, 105, 185),
                }
            },
            selection_text: if windows {
                if dark {
                    rgb(245, 245, 245)
                } else {
                    rgb(24, 24, 24)
                }
            } else {
                slint::Color::from_rgb_u8(255, 255, 255)
            },
            inactive_selection: if dark {
                rgb(70, 70, 75)
            } else {
                rgb(222, 223, 227)
            },
            folder: if desktop.resolve() == DesktopStyle::MacOs {
                rgb(75, 164, 224)
            } else {
                rgb(213, 161, 65)
            },
            radius: if windows { 4 } else { 6 },
        }
    }
}

pub fn is_dark(scheme: ColorScheme, system_dark: bool) -> bool {
    match scheme {
        ColorScheme::Dark => true,
        ColorScheme::Light => false,
        ColorScheme::System => system_dark,
    }
}
