//! Settings pages share one draft: tab switches never apply or discard edits.
use super::*;

pub(super) const GENERAL: i32 = 0;
pub(super) const FOLDERS: i32 = 1;
pub(super) const APPEARANCE: i32 = 2;
pub(super) const SHORTCUTS: i32 = 3;
pub(super) const MAIL: i32 = 4;
pub(super) const ADVANCED: i32 = 5;
pub(super) const TOOLBAR: i32 = 6;
pub(super) const COMMAND_BAR: i32 = 7;
pub(super) const COLUMNS: i32 = 8;

fn section(d: &mut Dialog, page: i32, title: &str, fields: impl IntoIterator<Item = Field>) {
    d.fields.push(Field {
        kind: 10,
        page,
        ..Field::label(title, "")
    });
    d.fields.extend(fields.into_iter().map(|mut field| {
        field.page = page;
        field
    }));
}

impl Adapter {
    pub(super) fn preferences_dialog(&self, p: &crate::preferences::Preferences) -> Dialog {
        let mut d = Dialog::new("preferences", "Settings");
        d.error = self.app.preferences_error.clone().unwrap_or_default();
        section(
            &mut d,
            GENERAL,
            "Language",
            [Field::choice(
                "language",
                "Application language",
                p.language.label(),
                crate::i18n::Language::ALL.map(|v| v.label()),
            )],
        );
        section(
            &mut d,
            GENERAL,
            "Window layout",
            [
                Field::check("single", "Use a single file pane", p.single_pane),
                Field::check("preview", "Show preview pane", p.preview),
                Field::check("directory-tree", "Show folder tree", p.directory_tree),
                Field::check("toolbar", "Show toolbar", p.toolbar),
                Field::check("sidebar", "Show sidebar", p.sidebar),
                Field::check("commands", "Show command bar", p.command_bar),
                Field::check("status", "Show status bar", p.status_bar),
            ],
        );
        section(
            &mut d,
            FOLDERS,
            "Browsing",
            [
                Field::choice(
                    "location-bar",
                    "Location bar",
                    tr(p.location_bar.label()),
                    crate::preferences::LocationBar::ALL.map(|m| tr(m.label()).into_owned()),
                ),
                Field::check("parent-entry", "Show parent directory (..)", p.parent_entry),
                Field::check(
                    "folders-first",
                    "List folders before files",
                    p.folders_first,
                ),
                Field::check(
                    "auto-refresh",
                    "Automatically refresh changed folders",
                    p.auto_refresh,
                ),
            ],
        );
        section(
            &mut d,
            COLUMNS,
            "Visible columns",
            crate::preferences::Column::ALL.map(|column| {
                Field::check(
                    format!("column-{column:?}"),
                    column.label(),
                    p.columns.iter().any(|c| c.kind == column),
                )
            }),
        );
        section(
            &mut d,
            COLUMNS,
            "Column sizing",
            std::iter::once(Field::check(
                "auto-size",
                "Automatically size columns",
                p.auto_size_columns,
            ))
            .chain(p.columns.iter().enumerate().map(|(i, column)| {
                Field::text(
                    format!("width-{i}"),
                    format!("{} — {}", tr(column.kind.label()), tr("Width (px)")),
                    column.width,
                )
            })),
        );
        for (i, column) in p.columns.iter().enumerate() {
            let mut row = Row::new(format!("column-{i}"), tr(column.kind.label()), "")
                .action(format!("column-up-{i}"), "Move up", i > 1)
                .action(
                    format!("column-down-{i}"),
                    "Move down",
                    i > 0 && i + 1 < p.columns.len(),
                );
            row.page = COLUMNS;
            d.rows.push(row);
        }
        section(
            &mut d,
            APPEARANCE,
            "Theme",
            [
                Field::choice(
                    "style",
                    "Desktop appearance",
                    p.desktop_style.label(),
                    crate::appearance::DesktopStyle::ALL.map(|v| v.label()),
                ),
                Field::choice(
                    "scheme",
                    "Color scheme",
                    p.color_scheme.label(),
                    [
                        crate::appearance::ColorScheme::System,
                        crate::appearance::ColorScheme::Light,
                        crate::appearance::ColorScheme::Dark,
                    ]
                    .map(|v| v.label()),
                ),
            ],
        );
        section(
            &mut d,
            APPEARANCE,
            "Text and icons",
            [
                Field::text("font", "Font size (10–24 px)", p.font_size),
                Field::choice(
                    "file-icons",
                    "File icons",
                    tr(p.file_icon_source.label()),
                    crate::preferences::FileIconSource::ALL.map(|s| tr(s.label()).into_owned()),
                ),
            ],
        );
        section(
            &mut d,
            TOOLBAR,
            "Toolbar buttons",
            crate::preferences::ToolbarAction::ALL.map(|action| {
                Field::check(
                    format!("toolbar-{action:?}"),
                    tr(action.label()),
                    p.toolbar_actions.contains(&action),
                )
            }),
        );
        for (i, action) in p.toolbar_actions.iter().enumerate() {
            let mut row = Row::new(format!("toolbar-{i}"), tr(action.label()), "")
                .action(format!("toolbar-up-{i}"), "Move up", i > 0)
                .action(
                    format!("toolbar-down-{i}"),
                    "Move down",
                    i + 1 < p.toolbar_actions.len(),
                );
            row.page = TOOLBAR;
            d.rows.push(row);
        }
        let catalog = super::super::parity::command_catalog();
        let choices = std::iter::once(tr("None").into_owned())
            .chain(catalog.iter().map(|(_, title)| tr(title).into_owned()))
            .collect::<Vec<_>>();
        section(
            &mut d,
            COMMAND_BAR,
            "Command bar buttons",
            [Field::label(
                "",
                tr("Choose up to twelve actions in display order. None removes a button."),
            )],
        );
        for index in 0..12 {
            let title = p
                .command_actions
                .get(index)
                .and_then(|id| catalog.iter().find(|(candidate, _)| candidate == id))
                .map(|(_, title)| title.as_str())
                .unwrap_or("None");
            d.fields.push(Field {
                page: COMMAND_BAR,
                ..Field::choice(
                    format!("command-slot-{index}"),
                    format!("{} {}", tr("Button"), index + 1),
                    tr(title),
                    choices.clone(),
                )
            });
        }
        section(
            &mut d,
            SHORTCUTS,
            "Keyboard shortcuts",
            [
                Field::text(
                    "shortcut-filter",
                    "Find a command",
                    &self.extra.shortcut_filter,
                ),
                Field::label(
                    "",
                    tr(
                        "Use combinations such as Command+K, Control+F or F5. Leave a binding blank to disable it.",
                    ),
                ),
            ],
        );
        d.fields.push(Field {
            page: SHORTCUTS,
            kind: 12,
            choices: vec![tr("Alternate shortcut").into_owned()],
            ..Field::label(tr("Command"), tr("Primary shortcut"))
        });
        let query = self.extra.shortcut_filter.to_lowercase();
        let mut matches = 0;
        for (action, title) in catalog {
            if !format!("{action} {title} {}", tr(&title))
                .to_lowercase()
                .contains(&query)
            {
                continue;
            }
            matches += 1;
            d.fields.push(Field {
                page: SHORTCUTS,
                kind: 11,
                choices: vec![
                    p.alternate_shortcuts
                        .get(&action)
                        .cloned()
                        .unwrap_or_default(),
                    format!("alternate-{action}"),
                ],
                ..Field::text(
                    format!("shortcut-{action}"),
                    tr(&title),
                    p.shortcuts.get(&action).map(String::as_str).unwrap_or(""),
                )
            });
        }
        if matches == 0 {
            d.fields.push(Field {
                page: SHORTCUTS,
                ..Field::label("", tr("No commands match your search."))
            });
        }
        section(
            &mut d,
            MAIL,
            "SMTP server",
            [
                Field::text("mail-host", "Hostname", &p.mail.host),
                Field::text(
                    "mail-port",
                    "Port",
                    if p.mail.port == 0 { 587 } else { p.mail.port },
                ),
                Field::check(
                    "mail-tls",
                    "Use implicit TLS (usually port 465)",
                    p.mail.implicit_tls,
                ),
                Field::label(
                    "",
                    tr("Other connections use STARTTLS, usually on port 587."),
                ),
            ],
        );
        section(
            &mut d,
            MAIL,
            "Sender and authentication",
            [
                Field::text("mail-sender", "From address", &p.mail.sender),
                Field::text("mail-user", "Username", &p.mail.username),
                Field::label(
                    "",
                    tr("Your password is requested in Email files. It is not stored in Settings."),
                ),
            ],
        );
        section(
            &mut d,
            ADVANCED,
            "Transfers",
            [
                Field::text(
                    "transfer-limit",
                    "Speed limit (KiB/s; 0 = unlimited)",
                    p.transfers.kib_per_second,
                ),
                Field::text(
                    "transfer-retries",
                    "Retry attempts (0–8)",
                    p.transfers.retries,
                ),
            ],
        );
        section(
            &mut d,
            ADVANCED,
            "Recovery retention",
            [
                Field::text("retention-mib", "Storage limit (MiB)", p.recovery.max_mib),
                Field::text(
                    "retention-days",
                    "Keep backups for (days)",
                    p.recovery.max_days,
                ),
                Field::check(
                    "retention-auto",
                    "Automatically clean up expired recovery backups",
                    p.recovery.automatic,
                ),
                Field::label(
                    "",
                    tr(
                        "Pinned backups are preserved. Use Recovery manager to review or restore retained files.",
                    ),
                ),
            ],
        );
        d.button("defaults", "Restore defaults", true);
        d.button("apply", "Apply", true);
        d.button("accept", "OK", true);
        d
    }
}
