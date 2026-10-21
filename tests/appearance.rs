use rust_commander::{
    appearance::{self, ColorScheme, DesktopStyle},
    preferences::{FileIconSource, Preferences},
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
    assert_eq!(
        settings.preferences.file_icon_source,
        FileIconSource::SystemWithFallback
    );
    assert!(settings.preferences.command_bar);
    assert_eq!(settings.preferences.font_size, 15.0);
}

#[test]
fn file_icon_source_persists_in_settings_and_workspaces() {
    for source in FileIconSource::ALL {
        let mut settings = Settings::default();
        settings.preferences.file_icon_source = source;
        settings
            .workspaces
            .push(rust_commander::workspaces::Workspace {
                name: "Icons".into(),
                tabs: [vec![], vec![]],
                tab_states: Default::default(),
                active_tabs: [0, 0],
                active_pane: 0,
                preferences: settings.preferences.clone(),
            });
        let saved = serde_json::to_string(&settings).unwrap();
        let restored: Settings = serde_json::from_str(&saved).unwrap();
        assert_eq!(restored.preferences.file_icon_source, source);
        assert_eq!(restored.workspaces[0].preferences.file_icon_source, source);
    }
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
    for dark in [false, true, false] {
        assert_eq!(appearance::is_dark(ColorScheme::System, dark), dark);
        assert_eq!(
            appearance::Palette::new(DesktopStyle::Ubuntu, dark).accent,
            slint::Color::from_rgb_u8(233, 84, 32)
        );
    }
    assert!(!appearance::is_dark(ColorScheme::Light, true));
    assert!(appearance::is_dark(ColorScheme::Dark, false));
}

#[test]
fn desktop_overrides_and_font_preferences_persist() {
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
        assert_eq!(restored.font_size, preferences.font_size);
    }
}

#[test]
fn selected_filename_text_keeps_readable_contrast_in_every_style() {
    fn luminance(color: slint::Color) -> f32 {
        let linear = |channel: u8| {
            let value = channel as f32 / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.red())
            + 0.7152 * linear(color.green())
            + 0.0722 * linear(color.blue())
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
