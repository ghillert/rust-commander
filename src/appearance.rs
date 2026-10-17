//! Desktop-specific presentation. File operations stay independent of the UI.
use eframe::egui::{self, Color32, FontFamily, FontId, Stroke, Vec2};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    pub fn preference(self) -> egui::ThemePreference {
        match self {
            Self::System => egui::ThemePreference::System,
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
        }
    }
}

pub struct Palette {
    pub accent: Color32,
    pub sidebar: Color32,
    pub toolbar: Color32,
    pub background: Color32,
    pub line: Color32,
    pub stripe: Color32,
    pub selection: Color32,
    pub selection_text: Color32,
    pub inactive_selection: Color32,
    pub folder: Color32,
    pub radius: u8,
}

#[derive(Clone, Copy)]
pub enum SidebarIcon {
    Home,
    Desktop,
    Documents,
    Downloads,
    Computer,
    Folder,
}

pub fn sidebar_button(
    ui: &mut egui::Ui,
    label: &str,
    selected: bool,
    icon: SidebarIcon,
    palette: &Palette,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
    });
    let painter = ui.painter_at(rect);
    if selected || response.hovered() {
        painter.rect_filled(
            rect,
            6.0,
            if selected {
                palette.inactive_selection
            } else {
                palette.toolbar
            },
        );
    }
    let center = rect.left_center() + Vec2::new(14.0, 0.0);
    let p = |x, y| center + Vec2::new(x, y);
    let stroke = Stroke::new(1.4_f32, palette.accent);
    let outline = |min, max| {
        painter.rect_stroke(
            egui::Rect::from_min_max(min, max),
            1.5,
            stroke,
            egui::StrokeKind::Inside,
        );
    };
    match icon {
        SidebarIcon::Home => {
            painter.line_segment([p(-8.0, -1.0), p(0.0, -8.0)], stroke);
            painter.line_segment([p(0.0, -8.0), p(8.0, -1.0)], stroke);
            outline(p(-5.5, -1.0), p(5.5, 7.0));
        }
        SidebarIcon::Desktop | SidebarIcon::Computer => {
            outline(p(-8.0, -6.0), p(8.0, 4.0));
            painter.line_segment([p(0.0, 4.0), p(0.0, 7.0)], stroke);
            painter.line_segment([p(-4.0, 7.0), p(4.0, 7.0)], stroke);
        }
        SidebarIcon::Documents => {
            outline(p(-6.0, -8.0), p(6.0, 8.0));
            for y in [-3.0, 1.0, 5.0] {
                painter.line_segment([p(-3.0, y), p(3.0, y)], stroke);
            }
        }
        SidebarIcon::Downloads => {
            painter.line_segment([p(0.0, -7.0), p(0.0, 4.0)], stroke);
            painter.line_segment([p(-4.0, 0.0), p(0.0, 4.0)], stroke);
            painter.line_segment([p(0.0, 4.0), p(4.0, 0.0)], stroke);
            painter.line_segment([p(-7.0, 7.0), p(7.0, 7.0)], stroke);
        }
        SidebarIcon::Folder => {
            outline(p(-8.0, -3.0), p(8.0, 7.0));
            painter.line_segment([p(-7.0, -6.0), p(-1.0, -6.0)], stroke);
            painter.line_segment([p(-7.0, -6.0), p(-7.0, -3.0)], stroke);
        }
    }
    painter.text(
        rect.left_center() + Vec2::new(30.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::proportional(egui::TextStyle::Body.resolve(ui.style()).size - 1.0),
        ui.visuals().text_color(),
    );
    response
}
impl Palette {
    pub fn new(desktop: DesktopStyle, dark: bool) -> Self {
        let rgb = Color32::from_rgb;
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
                Color32::WHITE
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
                Color32::WHITE
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

pub fn apply(ctx: &egui::Context, desktop: DesktopStyle, scheme: ColorScheme, size: f32) {
    ctx.set_theme(scheme.preference());
    ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(match scheme {
        ColorScheme::System => egui::SystemTheme::SystemDefault,
        ColorScheme::Light => egui::SystemTheme::Light,
        ColorScheme::Dark => egui::SystemTheme::Dark,
    }));
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let palette = Palette::new(desktop, theme == egui::Theme::Dark);
        let mut style = egui::Style {
            visuals: if theme == egui::Theme::Dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            },
            ..Default::default()
        };
        style.spacing.item_spacing = Vec2::new(6.0, 5.0);
        style.spacing.button_padding = Vec2::new(10.0, 5.0);
        style.spacing.interact_size.y = 26.0;
        style.spacing.scroll = egui::style::ScrollStyle::floating();
        let size = size.clamp(10.0, 24.0);
        for (text_style, points) in [
            (egui::TextStyle::Body, size),
            (egui::TextStyle::Button, size - 1.0),
            (egui::TextStyle::Small, size - 2.0),
            (egui::TextStyle::Heading, size + 3.0),
        ] {
            style
                .text_styles
                .insert(text_style, FontId::proportional(points));
        }
        style.visuals.panel_fill = palette.background;
        style.visuals.window_fill = palette.background;
        style.visuals.extreme_bg_color = palette.background;
        style.visuals.faint_bg_color = palette.stripe;
        style.visuals.override_text_color = Some(if theme == egui::Theme::Dark {
            Color32::from_rgb(238, 238, 240)
        } else {
            Color32::from_rgb(32, 32, 34)
        });
        style.visuals.weak_text_color = Some(if theme == egui::Theme::Dark {
            Color32::from_rgb(170, 170, 176)
        } else {
            Color32::from_rgb(108, 108, 114)
        });
        style.visuals.selection.bg_fill = palette.selection;
        style.visuals.selection.stroke = Stroke::new(1.0_f32, palette.selection_text);
        style.visuals.hyperlink_color = palette.accent;
        style.visuals.window_corner_radius = egui::CornerRadius::same(palette.radius + 3);
        style.visuals.window_stroke = Stroke::new(1.0_f32, palette.line);
        for widget in [
            &mut style.visuals.widgets.noninteractive,
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
            &mut style.visuals.widgets.open,
        ] {
            widget.corner_radius = egui::CornerRadius::same(palette.radius);
            widget.bg_stroke = Stroke::new(1.0_f32, palette.line);
        }
        style.visuals.widgets.inactive.bg_fill = palette.toolbar;
        style.visuals.widgets.inactive.weak_bg_fill = palette.toolbar;
        style.visuals.widgets.hovered.bg_fill = palette.inactive_selection;
        style.visuals.widgets.hovered.weak_bg_fill = palette.inactive_selection;
        style.visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, palette.accent);
        ctx.set_style_of(theme, style);
    }
}

/// Uses fonts installed on this machine; system fonts are never bundled.
pub fn install_fonts(ctx: &egui::Context, desktop: DesktopStyle) {
    let mut fonts = egui::FontDefinitions::default();
    let candidates: Vec<PathBuf> = match desktop.resolve() {
        DesktopStyle::MacOs => [
            "/System/Library/Fonts/SFNS.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
        ]
        .map(PathBuf::from)
        .into(),
        DesktopStyle::Windows => {
            let root = std::env::var_os("WINDIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
            [
                root.join("Fonts/segoeui.ttf"),
                root.join("Fonts/seguiemj.ttf"),
            ]
            .into()
        }
        DesktopStyle::Ubuntu => [
            "/usr/share/fonts/truetype/ubuntu/Ubuntu-R.ttf",
            "/usr/share/fonts/truetype/ubuntu/UbuntuSans-Regular.ttf",
            "/usr/share/fonts/truetype/ubuntu/UbuntuSans[wdth,wght].ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ]
        .map(PathBuf::from)
        .into(),
        _ => [
            "/usr/share/fonts/truetype/cantarell/Cantarell-Regular.ttf",
            "/usr/share/fonts/cantarell/Cantarell-Regular.otf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ]
        .map(PathBuf::from)
        .into(),
    };
    if let Some(bytes) = candidates
        .into_iter()
        .find_map(|path| std::fs::read(path).ok())
    {
        fonts.font_data.insert(
            "desktop-ui".into(),
            egui::FontData::from_owned(bytes).into(),
        );
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "desktop-ui".into());
    }
    ctx.set_fonts(fonts);
}
