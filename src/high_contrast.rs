//! OS accessibility contrast preferences, with a manual shared-view override.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Contrast {
    #[default]
    System,
    High,
}
impl Contrast {
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "Follow system",
            Self::High => "High contrast",
        }
    }
}
pub struct Colors {
    pub background: slint::Color,
    pub foreground: slint::Color,
    pub highlight: slint::Color,
    pub highlight_text: slint::Color,
    pub disabled: slint::Color,
}
impl Colors {
    pub fn fallback(dark: bool) -> Self {
        let rgb = slint::Color::from_rgb_u8;
        Self {
            background: if dark {
                rgb(0, 0, 0)
            } else {
                rgb(255, 255, 255)
            },
            foreground: if dark {
                rgb(255, 255, 255)
            } else {
                rgb(0, 0, 0)
            },
            highlight: if dark {
                rgb(255, 255, 0)
            } else {
                rgb(0, 0, 128)
            },
            highlight_text: if dark {
                rgb(0, 0, 0)
            } else {
                rgb(255, 255, 255)
            },
            disabled: if dark {
                rgb(192, 192, 192)
            } else {
                rgb(85, 85, 85)
            },
        }
    }
}
#[cfg(target_os = "windows")]
pub fn system() -> Option<Colors> {
    use windows_sys::Win32::{
        Graphics::Gdi::*,
        UI::{
            Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
            WindowsAndMessaging::*,
        },
    };
    let mut high: HIGHCONTRASTW = unsafe { std::mem::zeroed() };
    high.cbSize = std::mem::size_of::<HIGHCONTRASTW>() as u32;
    if unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            high.cbSize,
            (&mut high as *mut HIGHCONTRASTW).cast(),
            0,
        )
    } == 0
        || high.dwFlags & HCF_HIGHCONTRASTON == 0
    {
        return None;
    }
    let color = |id| {
        let v = unsafe { GetSysColor(id) };
        slint::Color::from_rgb_u8(v as u8, (v >> 8) as u8, (v >> 16) as u8)
    };
    Some(Colors {
        background: color(COLOR_WINDOW),
        foreground: color(COLOR_WINDOWTEXT),
        highlight: color(COLOR_HIGHLIGHT),
        highlight_text: color(COLOR_HIGHLIGHTTEXT),
        disabled: color(COLOR_GRAYTEXT),
    })
}
#[cfg(target_os = "linux")]
pub fn system() -> Option<Colors> {
    use std::sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    };
    static HIGH: AtomicBool = AtomicBool::new(false);
    static LAST: OnceLock<Mutex<Option<std::time::Instant>>> = OnceLock::new();
    let mut last = LAST
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if last.is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(3)) {
        *last = Some(std::time::Instant::now());
        std::thread::spawn(|| {
            let gnome = std::process::Command::new("gsettings")
                .args(["get", "org.gnome.desktop.interface", "gtk-theme"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .is_some_and(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .to_ascii_lowercase()
                        .contains("highcontrast")
                });
            let kde = std::fs::read_to_string(crate::filesystem::home().join(".config/kdeglobals"))
                .unwrap_or_default()
                .lines()
                .any(|l| {
                    l.starts_with("ColorScheme=") && l.to_ascii_lowercase().contains("highcontrast")
                });
            HIGH.store(gnome || kde, Ordering::Relaxed);
        });
    }
    HIGH.load(Ordering::Relaxed)
        .then(|| Colors::fallback(false))
}
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn system() -> Option<Colors> {
    None
}
