use eframe::egui;
use rust_commander::{
    appearance::{self, ColorScheme, DesktopStyle},
    preferences::Preferences,
    settings::Settings,
};

#[test]
fn legacy_settings_receive_automatic_desktop_and_system_appearance() {
    let settings: Settings = serde_json::from_str(
        r#"{"dark_mode":false,"preferences":{"font_size":15,"command_bar":true}}"#,
    )
    .unwrap();
    assert_eq!(settings.preferences.desktop_style, DesktopStyle::Automatic);
    assert_eq!(settings.preferences.color_scheme, ColorScheme::System);
    assert!(settings.preferences.command_bar);
    assert_eq!(settings.preferences.font_size, 15.0);
}

#[test]
fn ubuntu_detection_uses_distribution_id_without_misidentifying_derivatives() {
    assert_eq!(
        appearance::detect_linux("NAME=Ubuntu\nID=ubuntu\nID_LIKE=debian"),
        DesktopStyle::Ubuntu
    );
    assert_eq!(
        appearance::detect_linux("ID=\"ubuntu\"\n"),
        DesktopStyle::Ubuntu
    );
    assert_eq!(
        appearance::detect_linux("ID=pop\nID_LIKE=ubuntu"),
        DesktopStyle::Gnome
    );
    assert_eq!(appearance::detect_linux("ID=fedora"), DesktopStyle::Gnome);
    assert_eq!(appearance::detect_linux(""), DesktopStyle::Gnome);
}

#[test]
fn automatic_appearance_tracks_system_changes_without_losing_desktop_style() {
    let ctx = egui::Context::default();
    appearance::apply(&ctx, DesktopStyle::Ubuntu, ColorScheme::System, 14.0);
    for theme in [egui::Theme::Light, egui::Theme::Dark, egui::Theme::Light] {
        let _ = ctx.run(
            egui::RawInput {
                system_theme: Some(theme),
                ..Default::default()
            },
            |ctx| {
                assert_eq!(ctx.theme(), theme);
                assert_eq!(
                    ctx.style().visuals.hyperlink_color,
                    appearance::Palette::new(DesktopStyle::Ubuntu, theme == egui::Theme::Dark)
                        .accent
                );
            },
        );
    }
    appearance::apply(&ctx, DesktopStyle::MacOs, ColorScheme::Light, 14.0);
    let _ = ctx.run(
        egui::RawInput {
            system_theme: Some(egui::Theme::Dark),
            ..Default::default()
        },
        |ctx| assert_eq!(ctx.theme(), egui::Theme::Light),
    );
}

#[test]
fn desktop_overrides_persist_and_available_fonts_render() {
    for desktop in DesktopStyle::ALL {
        let preferences = Preferences {
            desktop_style: desktop,
            color_scheme: ColorScheme::Dark,
            ..Default::default()
        };
        let restored: Preferences =
            serde_json::from_str(&serde_json::to_string(&preferences).unwrap()).unwrap();
        assert_eq!(restored.desktop_style, desktop);
        assert_eq!(restored.color_scheme, ColorScheme::Dark);
        let ctx = egui::Context::default();
        appearance::install_fonts(&ctx, desktop);
        appearance::apply(&ctx, desktop, ColorScheme::Light, 14.0);
        let output = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("Documents · Rust Commander · ⌘");
            });
        });
        assert!(!output.shapes.is_empty());
    }
}

#[test]
fn selected_filename_text_keeps_readable_contrast_in_every_style() {
    fn luminance(color: egui::Color32) -> f32 {
        let linear = |channel: u8| {
            let value = channel as f32 / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
    }
    for desktop in DesktopStyle::ALL {
        for dark in [false, true] {
            let palette = appearance::Palette::new(desktop, dark);
            let a = luminance(palette.selection);
            let b = luminance(palette.selection_text);
            assert!(
                (a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5,
                "{:?}, dark={dark}",
                desktop
            );
        }
    }
}
