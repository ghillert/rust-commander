use super::*;
use anyhow::Context;

fn number<T: std::str::FromStr>(value: &str) -> anyhow::Result<T> {
    value
        .parse()
        .map_err(|_| anyhow::anyhow!("Enter a valid number."))
}
fn compression_level(format: crate::archive::Format, value: &str) -> anyhow::Result<Option<u8>> {
    let label = crate::packing::level_choices(format)
        .into_iter()
        .find(|label| label == value || tr(label).as_ref() == value)
        .context("Choose a supported compression level.")?;
    crate::packing::parse_level(format, &label)
}
fn checked(value: &str) -> bool {
    value == "true"
}

impl Adapter {
    pub(in super::super) fn edit_field(&mut self, id: &str, value: &str) {
        if id == "drop-mode" {
            if let Some(Prompt {
                kind:
                    PromptKind::DropTransfer {
                        selection, moving, ..
                    },
                resumable,
                ..
            }) = &mut self.app.prompt
            {
                *moving = value == "Move" && selection.can_move();
                if *moving {
                    *resumable = false;
                }
            }
            return;
        }

        if self.enhancements_edit(id, value) {
            return;
        }
        if self.parity_edit(id, value) {
            return;
        }
        let result = (|| -> anyhow::Result<()> {
            match self.dialog_kind.as_str() {
                "prompt" | "pack" => {
                    if let Some(prompt) = &mut self.app.prompt {
                        match id {
                            "prompt" => prompt.value = value.into(),
                            "pack-format" => {
                                if let PromptKind::Pack { options, .. } = &mut prompt.kind {
                                    options.set_format(value, &mut prompt.value)?;
                                }
                            }
                            "pack-level" => {
                                if let PromptKind::Pack { options, .. } = &mut prompt.kind {
                                    options.compression_level =
                                        compression_level(options.format, value)?;
                                }
                            }
                            "pack-comment" => {
                                if let PromptKind::Pack { options, .. } = &mut prompt.kind {
                                    options.comment = value.into();
                                }
                            }
                            "pack-background" => {
                                if let PromptKind::Pack { options, .. } = &mut prompt.kind {
                                    options.background = checked(value);
                                }
                            }
                            "resumable" => prompt.resumable = checked(value),
                            "policy" => prompt.transfer_options.conflicts = parse_policy(value),
                            "merge" => prompt.transfer_options.merge_folders = checked(value),
                            _ => {
                                if let Some(index) = id
                                    .strip_prefix("conflict-")
                                    .and_then(|s| s.parse::<usize>().ok())
                                    && let Some(conflict) = prompt.conflicts.get(index)
                                {
                                    prompt
                                        .overrides
                                        .insert(conflict.target.clone(), parse_policy(value));
                                }
                            }
                        }
                    }
                }
                "history" => {
                    let mut settings = self.app.preferences.transfers;
                    match id {
                        "transfer-limit" => settings.kib_per_second = number::<u32>(value)?,
                        "transfer-retries" => {
                            settings.retries = number::<u8>(value)?;
                            anyhow::ensure!(settings.retries <= 8, "Choose 0 to 8 retries.");
                        }
                        _ => return Ok(()),
                    }
                    self.app.preferences.transfers = settings;
                    if let Some(control) = self.app.job.as_ref().and_then(|j| j.control.as_ref()) {
                        control.update(settings);
                    }
                    self.app.save_settings();
                }
                "preferences" => {
                    if let Some(p) = &mut self.app.preferences_form {
                        match id {
                            "style" => {
                                if let Some(style) = crate::appearance::DesktopStyle::ALL
                                    .into_iter()
                                    .find(|style| style.label() == value)
                                {
                                    p.desktop_style = style;
                                }
                            }
                            "scheme" => {
                                if let Some(scheme) = [
                                    crate::appearance::ColorScheme::System,
                                    crate::appearance::ColorScheme::Light,
                                    crate::appearance::ColorScheme::Dark,
                                ]
                                .into_iter()
                                .find(|s| s.label() == value)
                                {
                                    p.color_scheme = scheme;
                                }
                            }
                            "language" => {
                                if let Some(language) = crate::i18n::Language::ALL
                                    .into_iter()
                                    .find(|language| language.label() == value)
                                {
                                    p.language = language;
                                }
                            }
                            "font" => {
                                let size = number::<f32>(value)?;
                                anyhow::ensure!(
                                    size.is_finite() && (10.0..=24.0).contains(&size),
                                    "Font size must be between 10 and 24."
                                );
                                p.font_size = size;
                            }
                            "file-icons" => {
                                p.file_icon_source = crate::preferences::FileIconSource::ALL
                                    .into_iter()
                                    .find(|source| {
                                        source.label() == value || tr(source.label()) == value
                                    })
                                    .context("Unknown file icon source")?;
                            }
                            "location-bar" => {
                                p.location_bar = crate::preferences::LocationBar::ALL
                                    .into_iter()
                                    .find(|m| m.label() == value || tr(m.label()) == value)
                                    .context("Unknown location bar mode")?
                            }
                            "toolbar" => p.toolbar = checked(value),
                            "sidebar" => p.sidebar = checked(value),
                            "commands" => p.command_bar = checked(value),
                            "status" => p.status_bar = checked(value),
                            "single" => p.single_pane = checked(value),
                            "auto-refresh" => p.auto_refresh = checked(value),
                            "folders-first" => p.folders_first = checked(value),
                            "preview" => p.preview = checked(value),
                            _ => {
                                if let Some(name) = id.strip_prefix("column-")
                                    && let Some(kind) = crate::preferences::Column::ALL
                                        .into_iter()
                                        .find(|column| format!("{column:?}") == name)
                                {
                                    if checked(value) {
                                        if !p.columns.iter().any(|c| c.kind == kind) {
                                            p.columns.push(crate::preferences::ColumnSpec {
                                                kind,
                                                width: 120.0,
                                            });
                                        }
                                    } else {
                                        anyhow::ensure!(
                                            kind != crate::preferences::Column::Name,
                                            "The Name column must remain first."
                                        );
                                        p.columns.retain(|c| c.kind != kind);
                                    }
                                } else if let Some(index) = id
                                    .strip_prefix("width-")
                                    .and_then(|s| s.parse::<usize>().ok())
                                    && let Some(column) = p.columns.get_mut(index)
                                {
                                    let width = number::<f32>(value)?;
                                    anyhow::ensure!(
                                        width.is_finite() && (40.0..=800.0).contains(&width),
                                        "Column width must be between 40 and 800."
                                    );
                                    column.width = width;
                                } else if let Some(name) = id.strip_prefix("toolbar-")
                                    && let Some(action) = crate::preferences::ToolbarAction::ALL
                                        .into_iter()
                                        .find(|action| format!("{action:?}") == name)
                                {
                                    if checked(value) {
                                        if !p.toolbar_actions.contains(&action) {
                                            p.toolbar_actions.push(action);
                                        }
                                    } else {
                                        p.toolbar_actions.retain(|a| *a != action);
                                    }
                                } else if let Some(name) = id.strip_prefix("shortcut-") {
                                    p.shortcuts.insert(name.into(), value.into());
                                }
                            }
                        }
                    }
                }
                "connections" => {
                    if let Some(form) = &mut self.app.connection_form {
                        let c = &mut form.connection;
                        let s = &mut form.secrets;
                        match id {
                            "name" => c.name = value.into(),
                            "service" => {
                                anyhow::ensure!(
                                    crate::remote::SERVICES.contains(&value),
                                    "Unsupported remote service."
                                );
                                c.service = value.into();
                            }
                            "endpoint" => c.endpoint = value.into(),
                            "root" => c.root = value.into(),
                            "bucket" => c.bucket = value.into(),
                            "region" => c.region = value.into(),
                            "username" => c.username = value.into(),
                            "key-file" => c.key_file = value.into(),
                            "known-hosts" => c.known_hosts_file = value.into(),
                            "tls-ca" => c.tls_ca_file = value.into(),
                            "ftp-tls" => {
                                c.ftp_tls = match value {
                                    s if s == "Implicit TLS" || s == tr("Implicit TLS") => {
                                        crate::remote::FtpTls::Implicit
                                    }
                                    s if s == "Plain FTP" || s == tr("Plain FTP") => {
                                        crate::remote::FtpTls::Plain
                                    }
                                    _ => crate::remote::FtpTls::Explicit,
                                }
                            }
                            "password" => s.password = value.into(),
                            "access-key" => s.access_key_id = value.into(),
                            "secret-key" => s.secret_access_key = value.into(),
                            "token" => s.access_token = value.into(),
                            "remember" => form.remember = checked(value),
                            "client-id" => c.oauth_client_id = value.into(),
                            "client-secret" => s.oauth_client_secret = value.into(),
                            "tenant" => c.oauth_tenant = value.into(),
                            "port" => {
                                let port = number::<u16>(value)?;
                                anyhow::ensure!(
                                    port >= 1024,
                                    "Callback port must be between 1024 and 65535."
                                );
                                c.oauth_port = port;
                            }
                            _ => {}
                        }
                    }
                }
                "archive" => {
                    if let Some(form) = &mut self.app.archive_form {
                        match id {
                            "archive-mode" => {
                                form.mode = match value {
                                    "Create encrypted archive" => 1,
                                    "Unlock archive" => 2,
                                    _ => 0,
                                }
                            }
                            "archive-file" => form.file = value.into(),
                            "edit-mode" => {
                                form.edit_mode = match value {
                                    "Remove entries" => 1,
                                    "Rename entry" => 2,
                                    _ => 0,
                                }
                            }
                            "additions" => form.additions = value.into(),
                            "source" => form.source = value.into(),
                            "target" => form.target = value.into(),
                            "destination" => form.destination = value.into(),
                            "archive-level" => {
                                let format = crate::archive::format(Path::new(&form.destination))
                                    .unwrap_or(crate::archive::Format::Zip);
                                form.compression_level = compression_level(format, value)?;
                            }
                            "archive-password" => form.password = value.into(),
                            _ => {}
                        }
                    }
                }
                "rename" => {
                    if let Some(form) = &mut self.app.tools.rename {
                        let o = &mut form.options;
                        match id {
                            "find" => o.find = value.into(),
                            "replace" => o.replace = value.into(),
                            "regex" => o.regex = checked(value),
                            "template" => o.template = value.into(),
                            "start" => o.start = number(value)?,
                            "padding" => {
                                let width = number::<usize>(value)?;
                                anyhow::ensure!(width <= 12, "Padding must be between 0 and 12.");
                                o.width = width;
                            }
                            _ => {}
                        }
                    }
                }
                "search" => {
                    if let Some(form) = &mut self.app.tools.search {
                        form.cancel.store(true, Ordering::Relaxed);
                        form.pending = None;
                        form.result = None;
                        let o = &mut form.options;
                        match id {
                            "search-root" => o.root = PathBuf::from(value),
                            "pattern" => o.pattern = value.into(),
                            "content" => o.content = value.into(),
                            "search-regex" => o.filters.content_regex = checked(value),
                            "search-content-case" => {
                                o.filters.content_case_sensitive = checked(value)
                            }
                            "search-name-case" => o.filters.name_case_sensitive = checked(value),
                            "search-exclude" => o.filters.exclude = value.into(),
                            "search-min-size" => {
                                o.filters.min_size = if value.trim().is_empty() {
                                    None
                                } else {
                                    Some(number::<u64>(value.trim())?)
                                }
                            }
                            "search-max-size" => {
                                o.filters.max_size = if value.trim().is_empty() {
                                    None
                                } else {
                                    Some(number::<u64>(value.trim())?)
                                }
                            }
                            "search-after" => {
                                o.filters.modified_after = crate::search::parse_date(value)?
                            }
                            "search-before" => {
                                o.filters.modified_before = crate::search::parse_date(value)?
                            }
                            "search-hidden" => o.hidden = checked(value),
                            "limit" => {
                                let limit = number::<usize>(value)?;
                                anyhow::ensure!(
                                    (1..=100_000).contains(&limit),
                                    "Maximum results must be between 1 and 100,000."
                                );
                                o.max_results = limit;
                            }
                            _ => {}
                        }
                    }
                }
                "sync" => {
                    if let Some(form) = &mut self.app.features.sync {
                        match id {
                            "folder-preset-name" => form.preset_name = value.into(),
                            "folder-include" | "folder-exclude" | "folder-case" => {
                                form.cancel.store(true, Ordering::Relaxed);
                                form.pending = None;
                                form.plan = None;
                                form.confirmed = false;
                                match id {
                                    "folder-include" => form.rules.include = value.into(),
                                    "folder-exclude" => form.rules.exclude = value.into(),
                                    _ => form.rules.case_sensitive = checked(value),
                                }
                            }
                            "sync-left" => {
                                form.cancel.store(true, Ordering::Relaxed);
                                form.pending = None;
                                form.left = value.into();
                                form.plan = None;
                                form.confirmed = false;
                            }
                            "sync-right" => {
                                form.cancel.store(true, Ordering::Relaxed);
                                form.pending = None;
                                form.right = value.into();
                                form.plan = None;
                                form.confirmed = false;
                            }
                            "direction" => {
                                form.cancel.store(true, Ordering::Relaxed);
                                form.pending = None;
                                form.direction = match value {
                                    "Left → right" => 1,
                                    "Right → left" => 2,
                                    _ => 0,
                                };
                                form.plan = None;
                                form.confirmed = false;
                            }
                            "deletions" => {
                                form.deletions = checked(value);
                                form.confirmed = false;
                                if !form.deletions
                                    && let Some(plan) = &mut form.plan
                                {
                                    for item in &mut plan.items {
                                        if matches!(
                                            item.choice,
                                            crate::sync::Choice::DeleteLeft
                                                | crate::sync::Choice::DeleteRight
                                        ) {
                                            item.choice = crate::sync::Choice::Skip;
                                        }
                                    }
                                }
                            }
                            "confirmed" => form.confirmed = checked(value),
                            _ => {
                                if let Some(index) = id
                                    .strip_prefix("sync-item-")
                                    .and_then(|s| s.parse::<usize>().ok())
                                    && let Some(item) =
                                        form.plan.as_mut().and_then(|p| p.items.get_mut(index))
                                {
                                    let choice = [
                                        crate::sync::Choice::Skip,
                                        crate::sync::Choice::LeftToRight,
                                        crate::sync::Choice::RightToLeft,
                                        crate::sync::Choice::DeleteLeft,
                                        crate::sync::Choice::DeleteRight,
                                    ]
                                    .into_iter()
                                    .find(|c| c.label() == value)
                                    .unwrap_or(crate::sync::Choice::Skip);
                                    if matches!(
                                        choice,
                                        crate::sync::Choice::DeleteLeft
                                            | crate::sync::Choice::DeleteRight
                                    ) {
                                        anyhow::ensure!(
                                            form.deletions,
                                            "Enable deletion choices first."
                                        );
                                    }
                                    item.choice = choice;
                                    form.confirmed = false;
                                }
                            }
                        }
                    }
                }
                "comparison" => self.comparison_edit(id, value),
                "recovery" => {
                    let p = &mut self.app.preferences.recovery;
                    match id {
                        "max-mib" => {
                            let size = number::<u64>(value)?;
                            anyhow::ensure!(
                                (1..=1_000_000).contains(&size),
                                "Limit must be between 1 and 1,000,000 MiB."
                            );
                            p.max_mib = size;
                        }
                        "max-days" => {
                            let days = number::<u64>(value)?;
                            anyhow::ensure!(
                                (1..=3650).contains(&days),
                                "Retention must be between 1 and 3650 days."
                            );
                            p.max_days = days;
                        }
                        "automatic" => p.automatic = checked(value),
                        _ => {
                            if let Some(index) = id
                                .strip_prefix("pin-")
                                .and_then(|s| s.parse::<usize>().ok())
                                && self.app.job.is_none()
                                && self.app.queue.is_empty()
                                && let Some(record) = self.app.history.get_mut(index)
                            {
                                record.pinned = checked(value);
                                crate::history::save(&self.app.history)?;
                            }
                        }
                    }
                    if let Some(form) = &mut self.app.features.recovery {
                        form.candidates = None;
                    }
                }
                "workspaces" if id == "workspace-name" => {
                    self.app.features.workspace_name = value.into()
                }
                "open-as" if id == "open-password" => {
                    if let Some(form) = &mut self.app.features.open_as {
                        form.password = value.into();
                    }
                }
                "open-as" if id == "open-format" => {
                    let kind = crate::open_as::CHOICES
                        .iter()
                        .find(|(label, _)| *label == value || tr(label).as_ref() == value)
                        .context("Choose a supported file format.")?
                        .1;
                    if let Some(form) = &mut self.app.features.open_as {
                        form.kind = kind;
                    }
                }
                "open-with" if id == "application-filter" => {
                    self.app.features.application_filter = value.into()
                }
                "open-with" if id == "application-pattern" => {
                    self.app.features.application_pattern = value.into()
                }
                "open-with" if id == "remember-application" => {
                    self.app.features.remember_application = checked(value)
                }
                "open-with" if id == "application" || id == "installed-application" => {
                    if let Some((_, application)) = &mut self.app.features.open_with {
                        *application = value.into();
                    }
                }
                _ => {}
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.invalid_fields.remove(id);
                self.field_error = None;
            }
            Err(error) => {
                self.invalid_fields
                    .insert(id.into(), (value.into(), error.to_string()));
            }
        }
    }
}
