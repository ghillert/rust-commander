//! Platform appearance preferences and colors. Slint renders the widgets.
use serde::{Deserialize, Serialize};
/// Desktop styling preset used to choose colors and control geometry.
///
/// [`Self::Automatic`] resolves to a platform preset through [`Self::resolve`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DesktopStyle {
    /// Detects the desktop style from the host OS and, on Linux, the distribution.
    #[default]
    Automatic,
    /// Uses the macOS palette and control styling.
    MacOs,
    /// Uses the Windows palette and control styling.
    Windows,
    /// Uses the Ubuntu palette with its orange accent.
    Ubuntu,
    /// Uses the GNOME palette, also used for Linux distributions without a dedicated style.
    Gnome,
}
impl DesktopStyle {
    /// Desktop styles in the order shown in the appearance selector.
    pub const ALL: [Self; 5] = [
        Self::Automatic,
        Self::MacOs,
        Self::Windows,
        Self::Ubuntu,
        Self::Gnome,
    ];
    /// Returns the desktop style name shown in the appearance settings.
    pub fn label(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic (this desktop)",
            Self::MacOs => "macOS",
            Self::Windows => "Windows",
            Self::Ubuntu => "Ubuntu",
            Self::Gnome => "Linux / GNOME",
        }
    }
    /// Resolves the automatic style for the current OS. An explicit style is returned as
    /// selected.
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
/// Chooses the Ubuntu palette when `release` contains an Ubuntu `ID=` entry from
/// `os-release`. Other distributions use the GNOME palette.
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

/// Preference for light or dark colors, with an option to follow the system.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorScheme {
    /// Follows the desktop’s light or dark appearance.
    #[default]
    System,
    /// Uses the light appearance regardless of the system setting.
    Light,
    /// Uses the dark appearance regardless of the system setting.
    Dark,
}
impl ColorScheme {
    /// Returns the display label for the system, light, or dark color preference.
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "Follow system",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
}
/// Colors and corner radius for the file manager's shared views.
///
/// [`Self::new`] builds a palette from a desktop style and the effective light or dark
/// appearance.
pub struct Palette {
    /// Accent color for active controls and focus indicators.
    pub accent: slint::Color,
    /// Background color of the sidebar.
    pub sidebar: slint::Color,
    /// Background color of the toolbar.
    pub toolbar: slint::Color,
    /// Base background color of file panes.
    pub background: slint::Color,
    /// Color of borders and separators between UI elements.
    pub line: slint::Color,
    /// Background color for alternating rows in the file list.
    pub stripe: slint::Color,
    /// Background behind the cursor in the active file pane.
    pub selection: slint::Color,
    /// Foreground color used over the selection background.
    pub selection_text: slint::Color,
    /// Text color for entries marked for an operation.
    pub marked_text: slint::Color,
    /// Background for a marked entry that also holds the active pane’s cursor.
    pub marked_cursor: slint::Color,
    /// Cursor background in the pane without keyboard focus.
    pub inactive_selection: slint::Color,
    /// Tint for folder icons.
    pub folder: slint::Color,
    /// Corner radius in logical pixels for this desktop style.
    pub radius: u8,
}

impl Palette {
    /// Builds the file manager palette and corner sizes for the resolved desktop style and
    /// the supplied light or dark appearance.
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
            marked_text: if dark {
                rgb(255, 153, 153)
            } else {
                rgb(189, 16, 16)
            },
            marked_cursor: if dark {
                rgb(20, 55, 95)
            } else {
                rgb(204, 232, 255)
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

/// Returns whether to use dark colors. The system preference is consulted only for
/// [`ColorScheme::System`].
pub fn is_dark(scheme: ColorScheme, system_dark: bool) -> bool {
    match scheme {
        ColorScheme::Dark => true,
        ColorScheme::Light => false,
        ColorScheme::System => system_dark,
    }
}
