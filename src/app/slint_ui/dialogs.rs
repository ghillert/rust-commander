use super::*;
mod commands;
mod fields;

#[derive(Clone, PartialEq)]
pub(super) struct Field {
    pub id: String,
    pub title: String,
    pub value: String,
    pub kind: i32,
    pub choices: Vec<String>,
}
impl Field {
    pub(super) fn text(
        id: impl Into<String>,
        title: impl Into<String>,
        value: impl ToString,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            value: value.to_string(),
            kind: 0,
            choices: Vec::new(),
        }
    }
    fn password(id: &str, title: &str, value: &str) -> Self {
        Self {
            kind: 1,
            ..Self::text(id, title, value)
        }
    }
    pub(super) fn check(id: impl Into<String>, title: impl Into<String>, value: bool) -> Self {
        Self {
            kind: 2,
            ..Self::text(id, title, value)
        }
    }
    pub(super) fn choice(
        id: impl Into<String>,
        title: impl Into<String>,
        value: impl ToString,
        choices: impl IntoIterator<Item = impl ToString>,
    ) -> Self {
        Self {
            kind: 3,
            choices: choices.into_iter().map(|s| s.to_string()).collect(),
            ..Self::text(id, title, value)
        }
    }
    fn multiline(id: &str, title: &str, value: impl ToString) -> Self {
        Self {
            kind: 4,
            ..Self::text(id, title, value)
        }
    }
    pub(super) fn label(title: impl Into<String>, value: impl ToString) -> Self {
        Self {
            kind: 6,
            ..Self::text("", title, value)
        }
    }
    pub(super) fn slint(&self) -> FormField {
        FormField {
            id: self.id.as_str().into(),
            title: tr(&self.title).into_owned().into(),
            value: self.value.as_str().into(),
            kind: self.kind,
            choices: strings(self.choices.iter().map(String::as_str)),
        }
    }
}
#[derive(Clone, PartialEq)]
pub(super) struct Row {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub selected: bool,
    pub actions: Vec<(String, String, bool)>,
}
impl Row {
    pub(super) fn new(id: impl Into<String>, title: impl ToString, detail: impl ToString) -> Self {
        Self {
            id: id.into(),
            title: title.to_string(),
            detail: detail.to_string(),
            selected: false,
            actions: Vec::new(),
        }
    }
    pub(super) fn action(mut self, id: impl Into<String>, title: &str, enabled: bool) -> Self {
        self.actions.push((id.into(), title.into(), enabled));
        self
    }
    pub(super) fn slint(&self) -> FormRow {
        FormRow {
            id: self.id.as_str().into(),
            title: self.title.as_str().into(),
            detail: self.detail.as_str().into(),
            selected: self.selected,
            actions: Rc::new(VecModel::from(
                self.actions
                    .iter()
                    .map(|(id, title, enabled)| Command {
                        id: id.into(),
                        title: tr(title).into_owned().into(),
                        enabled: *enabled,
                    })
                    .collect::<Vec<_>>(),
            ))
            .into(),
        }
    }
}
#[derive(Default)]
pub(super) struct Dialog {
    pub kind: String,
    pub title: String,
    pub description: String,
    pub error: String,
    pub busy: bool,
    pub fields: Vec<Field>,
    pub rows: Vec<Row>,
    pub buttons: Vec<(String, String, bool)>,
}
impl Dialog {
    pub(super) fn new(kind: &str, title: &str) -> Self {
        Self {
            kind: kind.into(),
            title: tr(title).into_owned(),
            buttons: vec![(
                "cancel".into(),
                if matches!(
                    kind,
                    "prompt" | "preferences" | "archive" | "rename" | "sync" | "open-with"
                ) {
                    "Cancel".into()
                } else {
                    "Close".into()
                },
                true,
            )],
            ..Default::default()
        }
    }
    pub(super) fn button(&mut self, id: &str, title: &str, enabled: bool) {
        self.buttons.push((id.into(), title.into(), enabled));
    }
}
const POLICIES: [&str; 4] = ["Stop on conflict", "Skip", "Keep both", "Replace"];
fn policy_label(policy: crate::transfer::ConflictPolicy) -> &'static str {
    use crate::transfer::ConflictPolicy::*;
    match policy {
        Refuse => POLICIES[0],
        Skip => POLICIES[1],
        Rename => POLICIES[2],
        Replace => POLICIES[3],
    }
}
fn parse_policy(value: &str) -> crate::transfer::ConflictPolicy {
    use crate::transfer::ConflictPolicy::*;
    match value {
        "Skip" => Skip,
        "Keep both" => Rename,
        "Replace" => Replace,
        _ => Refuse,
    }
}

impl Adapter {
    pub(super) fn dialog(&self) -> Dialog {
        if !self.app.confirm_exit
            && self.app.message.is_none()
            && let Some(dialog) = self.enhancements_dialog()
        {
            return dialog;
        }
        if let Some(dialog) = self.parity_dialog() {
            return dialog;
        }
        let idle = self.app.job.is_none() && self.app.queue.is_empty();
        if self.app.confirm_exit {
            let mut d = Dialog::new("exit", "Close Rust Commander?");
            d.description = if self.extra.quit_all {
                "Quit every Rust Commander window? Running work will be cancelled and unsaved text in all windows discarded.".into()
            } else {
                tr("There is a running operation or unsaved text. Closing cancels the operation and discards unsaved text.").into_owned()
            };
            d.buttons = vec![
                ("cancel".into(), "Keep working".into(), true),
                ("accept".into(), "Cancel work and close".into(), true),
            ];
            return d;
        }
        if let Some(message) = &self.app.message {
            let mut d = Dialog::new("message", "Results");
            d.fields.push(Field::label("", message));
            d.button("copy-result", "Copy result", true);
            return d;
        }
        if self.app.help {
            let mut d = Dialog::new("help", "Rust Commander Help");
            d.description = format!("Slint desktop interface · {}", env!("CARGO_PKG_VERSION"));
            for (key, value) in [
                ("Tab", "Switch active pane"),
                (
                    "↑ / ↓ · Enter · Backspace",
                    "Navigate files · Open · Parent folder",
                ),
                ("Space / Insert", "Mark or unmark current file"),
                ("Shift / Cmd / Ctrl + click", "Range / multiple selection"),
                ("F2 / F3 / F4", "Rename / View / Edit"),
                ("F5 / F6 / F7 / F8", "Copy / Move / New folder / Trash"),
                ("Cmd / Ctrl + L / F", "Focus address / filter"),
                ("Cmd / Ctrl + T / W", "New tab / close tab"),
                ("Cmd / Ctrl + C / X / V", "System file copy / cut / paste"),
                (
                    "Cmd / Ctrl + Z / Shift+Z / J",
                    "Undo / Redo / queue and history",
                ),
                ("Cmd / Ctrl + P", "Preview pane"),
                ("Alt + ← / →", "Back / forward"),
                ("Escape", "Clear marks and filter / close dialog"),
            ] {
                d.fields.push(Field::label(key, tr(value)));
            }
            d.fields.push(Field::label(
                "GNU General Public License v3",
                include_str!("../../../LICENSE"),
            ));
            return d;
        }
        if let Some(prompt) = &self.app.prompt {
            let mut d = Dialog::new("prompt", &prompt.title);
            d.description = tr(&prompt.description).into_owned();
            d.error = prompt.error.clone().unwrap_or_default();
            d.busy = prompt.pending_conflicts.is_some();
            if !matches!(
                prompt.kind,
                PromptKind::Trash(_)
                    | PromptKind::RemoteDelete { .. }
                    | PromptKind::PermanentDelete(_)
            ) {
                d.fields.push(Field::text(
                    "prompt",
                    if matches!(prompt.kind, PromptKind::DropTransfer { .. }) {
                        "Destination"
                    } else {
                        "Value"
                    },
                    &prompt.value,
                ));
            }
            if let PromptKind::DropTransfer {
                selection, moving, ..
            } = &prompt.kind
            {
                d.fields.push(Field::choice(
                    "drop-mode",
                    "Transfer action",
                    if *moving { "Move" } else { "Copy" },
                    ["Copy", "Move"]
                        .into_iter()
                        .take(if selection.can_move() { 2 } else { 1 }),
                ));
            }
            if matches!(
                prompt.kind,
                PromptKind::Transfer { .. }
                    | PromptKind::DropTransfer { .. }
                    | PromptKind::RemoteTransfer { .. }
                    | PromptKind::Extract { .. }
            ) {
                if matches!(
                    prompt.kind,
                    PromptKind::RemoteTransfer { moving: false, .. }
                ) {
                    d.fields.push(Field::check(
                        "resumable",
                        "Checkpoint this copy (new targets; retry from queue)",
                        prompt.resumable,
                    ));
                }
                d.fields.push(Field::choice(
                    "policy",
                    "When a target exists",
                    policy_label(prompt.transfer_options.conflicts),
                    POLICIES,
                ));
                if matches!(
                    prompt.kind,
                    PromptKind::Transfer { .. }
                        | PromptKind::DropTransfer { .. }
                        | PromptKind::RemoteTransfer { .. }
                        | PromptKind::Extract { .. }
                ) {
                    d.fields.push(Field::check(
                        "merge",
                        "Merge existing folders",
                        prompt.transfer_options.merge_folders,
                    ));
                }
                for (i, conflict) in prompt.conflicts.iter().enumerate() {
                    d.fields.push(Field::choice(
                        format!("conflict-{i}"),
                        format!(
                            "{} · {} → {}",
                            conflict.target.display(),
                            filesystem::human_size(conflict.source_size),
                            filesystem::human_size(conflict.target_size)
                        ),
                        policy_label(
                            prompt
                                .overrides
                                .get(&conflict.target)
                                .copied()
                                .unwrap_or(prompt.transfer_options.conflicts),
                        ),
                        POLICIES,
                    ));
                }
                d.button("apply-all", "Apply to all", true);
                if prompt.transfer_options.conflicts == crate::transfer::ConflictPolicy::Replace {
                    d.fields.push(Field::label("Recoverable replacement","Replaced items stay in a hidden recovery folder in the destination. The result reports its path."));
                }
            }
            d.button(
                "accept",
                if matches!(prompt.kind, PromptKind::Trash(_)) {
                    "Move to Trash"
                } else {
                    "Confirm"
                },
                !d.busy,
            );
            return d;
        }
        if let Some(p) = &self.app.preferences_form {
            let mut d = Dialog::new("preferences", "Settings");
            d.description="Appearance profiles change colors, fonts and layout. Controls follow the style chosen for this platform.".into();
            d.error = self.app.preferences_error.clone().unwrap_or_default();
            d.fields.extend([
                Field::choice(
                    "style",
                    "Desktop appearance",
                    p.desktop_style.label(),
                    [
                        crate::appearance::DesktopStyle::Automatic,
                        crate::appearance::DesktopStyle::MacOs,
                        crate::appearance::DesktopStyle::Windows,
                        crate::appearance::DesktopStyle::Ubuntu,
                        crate::appearance::DesktopStyle::Gnome,
                    ]
                    .map(|v| v.label()),
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
                Field::choice(
                    "language",
                    "Language",
                    p.language.label(),
                    [
                        crate::i18n::Language::System,
                        crate::i18n::Language::English,
                        crate::i18n::Language::German,
                        crate::i18n::Language::Spanish,
                    ]
                    .map(|v| v.label()),
                ),
                Field::choice(
                    "file-icons",
                    "File icons",
                    tr(p.file_icon_source.label()),
                    crate::preferences::FileIconSource::ALL
                        .map(|source| tr(source.label()).into_owned()),
                ),
                Field::text("font", "Font size", p.font_size),
                Field::choice(
                    "location-bar",
                    "Location bar",
                    tr(p.location_bar.label()),
                    crate::preferences::LocationBar::ALL.map(|m| tr(m.label()).into_owned()),
                ),
                Field::check("toolbar", "Show toolbar", p.toolbar),
                Field::check("sidebar", "Show sidebar", p.sidebar),
                Field::check("commands", "Show command bar", p.command_bar),
                Field::check("status", "Show status bar", p.status_bar),
                Field::check("single", "Single pane", p.single_pane),
                Field::check(
                    "auto-refresh",
                    "Automatically refresh changed folders",
                    p.auto_refresh,
                ),
                Field::check("folders-first", "Folders first", p.folders_first),
                Field::check("preview", "Preview pane", p.preview),
            ]);
            for column in crate::preferences::Column::ALL {
                d.fields.push(Field::check(
                    format!("column-{column:?}"),
                    format!("Show {} column", column.label()),
                    p.columns.iter().any(|c| c.kind == column),
                ));
            }
            for (i, column) in p.columns.iter().enumerate() {
                d.fields.push(Field::text(
                    format!("width-{i}"),
                    format!("{} column width", column.kind.label()),
                    column.width,
                ));
                d.rows.push(
                    Row::new(format!("column-{i}"), tr(column.kind.label()), "")
                        .action(format!("column-up-{i}"), "Move up", i > 1)
                        .action(
                            format!("column-down-{i}"),
                            "Move down",
                            i > 0 && i + 1 < p.columns.len(),
                        ),
                );
            }
            for action in crate::preferences::ToolbarAction::ALL {
                d.fields.push(Field::check(
                    format!("toolbar-{action:?}"),
                    format!("Toolbar: {}", tr(action.label())),
                    p.toolbar_actions.contains(&action),
                ));
            }
            for (i, action) in p.toolbar_actions.iter().enumerate() {
                d.rows.push(
                    Row::new(format!("toolbar-{i}"), tr(action.label()), "")
                        .action(format!("toolbar-up-{i}"), "Move up", i > 0)
                        .action(
                            format!("toolbar-down-{i}"),
                            "Move down",
                            i + 1 < p.toolbar_actions.len(),
                        ),
                );
            }
            let catalog = super::parity::command_catalog();
            let choices = std::iter::once("None".to_owned())
                .chain(catalog.iter().map(|(_, title)| title.clone()))
                .collect::<Vec<_>>();
            for index in 0..12 {
                let title = p
                    .command_actions
                    .get(index)
                    .and_then(|id| catalog.iter().find(|(candidate, _)| candidate == id))
                    .map(|(_, title)| title.as_str())
                    .unwrap_or("None");
                d.fields.push(Field::choice(
                    format!("command-slot-{index}"),
                    format!("Command bar button {}", index + 1),
                    title,
                    choices.clone(),
                ));
            }
            d.fields.push(Field::text(
                "shortcut-filter",
                "Find a shortcut",
                &self.extra.shortcut_filter,
            ));
            let query = self.extra.shortcut_filter.to_lowercase();
            for (action, title) in catalog {
                if !format!("{action} {title}").to_lowercase().contains(&query) {
                    continue;
                }
                let binding = p.shortcuts.get(&action).map(String::as_str).unwrap_or("");
                d.fields.push(Field::text(
                    format!("shortcut-{action}"),
                    format!("Shortcut: {title}"),
                    binding,
                ));
                d.fields.push(Field::text(
                    format!("alternate-{action}"),
                    format!("Alternate shortcut: {title}"),
                    p.alternate_shortcuts
                        .get(&action)
                        .map(String::as_str)
                        .unwrap_or(""),
                ));
            }
            d.button("defaults", "Restore defaults", true);
            d.button("accept", "Apply", true);
            return d;
        }
        if let Some(form) = &self.app.connection_form {
            let mut d = Dialog::new("connections", "Connections");
            d.error = form.error.clone().unwrap_or_default();
            d.busy = self.app.uri_connecting();
            if form.save_only {
                d.title = tr("Save connection…").into_owned();
                d.description =
                    crate::uri::address(&form.connection.id, &form.connection.initial_path);
                d.fields
                    .push(Field::text("name", "Name", &form.connection.name));
                d.button("accept", "Save", true);
                return d;
            }
            if !form.choices.is_empty() {
                d.description = tr("Choose a connection for this address").into_owned();
                for (i, c) in form.choices.iter().enumerate() {
                    d.rows.push(
                        Row::new(format!("uri-profile-{i}"), &c.name, &c.username).action(
                            format!("uri-profile-{i}"),
                            "Open",
                            true,
                        ),
                    );
                }
                return d;
            }
            for (i, c) in self.app.connections.iter().enumerate() {
                if !c.listed || form.navigation.is_some() {
                    continue;
                }
                d.rows.push(
                    Row::new(format!("connection-{i}"), &c.name, &c.service)
                        .action(format!("connection-open-{i}"), "Open", !d.busy)
                        .action(format!("connection-edit-{i}"), "Edit", !d.busy)
                        .action(
                            format!("connection-forget-{i}"),
                            "Forget credentials",
                            !d.busy,
                        ),
                );
            }
            let c = &form.connection;
            let s = &form.secrets;
            d.fields.extend([
                Field::text("name", "Name", &c.name),
                Field::choice("service", "Service", &c.service, crate::remote::SERVICES),
                Field::text("endpoint", "Endpoint", &c.endpoint),
                Field::text("root", "Root folder", &c.root),
                Field::text(
                    "bucket",
                    if c.service == "smb" {
                        "Share"
                    } else {
                        "Bucket"
                    },
                    &c.bucket,
                ),
                Field::text("region", "Region", &c.region),
                Field::text("username", "Username", &c.username),
                Field::text("key-file", "Private key file", &c.key_file),
                Field::password("password", "Password", &s.password),
                Field::password("access-key", "Access key ID", &s.access_key_id),
                Field::password("secret-key", "Secret access key", &s.secret_access_key),
                Field::password(
                    "token",
                    "Access token / base64 GCS credentials",
                    &s.access_token,
                ),
                Field::check(
                    "remember",
                    "Remember credentials in the operating system credential store",
                    form.remember,
                ),
            ]);
            if matches!(c.service.as_str(), "gdrive" | "dropbox" | "onedrive") {
                d.fields.extend([
                    Field::text("client-id", "OAuth client ID", &c.oauth_client_id),
                    Field::password(
                        "client-secret",
                        "OAuth client secret",
                        &s.oauth_client_secret,
                    ),
                    Field::text("tenant", "Microsoft tenant", &c.oauth_tenant),
                    Field::text("port", "Callback port", c.oauth_port),
                ]);
                let host = if c.service == "onedrive" {
                    "localhost"
                } else {
                    "127.0.0.1"
                };
                d.fields.push(Field::label(
                    "Register redirect URI",
                    format!("http://{host}:{}/callback", c.oauth_port),
                ));
                d.button("authorize", "Sign in with browser", !d.busy);
            }
            if c.service == "sftp" {
                d.fields.push(Field::label("SFTP","SFTP supports passwords, private keys and SSH agents, with strict known-host verification. Trust the server in known_hosts first."));
                d.fields.push(Field::text(
                    "known-hosts",
                    "Known hosts file (optional)",
                    &c.known_hosts_file,
                ));
            }
            if c.service == "ftp" {
                d.fields.push(Field::choice(
                    "ftp-tls",
                    "FTP security",
                    tr(match c.tls_mode() {
                        crate::remote::FtpTls::Plain => "Plain FTP",
                        crate::remote::FtpTls::Implicit => "Implicit TLS",
                        _ => "Explicit TLS",
                    }),
                    ["Plain FTP", "Explicit TLS", "Implicit TLS"].map(|s| tr(s).into_owned()),
                ));
                d.fields.push(Field::text(
                    "tls-ca",
                    "TLS CA certificate (DER, optional)",
                    &c.tls_ca_file,
                ));
            }
            if c.service == "smb" {
                d.fields.push(Field::label("SMB","Endpoint smb://nas.local[:445], Share = share name, Root folder = path within share. Domain accounts may use DOMAIN\\user."));
            }
            if c.service == "http" {
                d.button("open-browser", "Open in browser", !d.busy);
            }
            d.button(
                "accept",
                if form.save_only {
                    "Save connection…"
                } else {
                    "Connect"
                },
                !d.busy,
            );
            return d;
        }
        if let Some(form) = &self.app.archive_form {
            let modes = ["Edit archive", "Create encrypted archive", "Unlock archive"];
            let edits = ["Add files", "Remove entries", "Rename entry"];
            let mut d = Dialog::new("archive", "Archive tools");
            d.error = form.error.clone().unwrap_or_default();
            d.fields.extend([
                Field::choice("archive-mode", "Action", modes[form.mode.min(2)], modes),
                Field::text("archive-file", "Archive", &form.file),
            ]);
            if form.mode == 0 {
                d.fields.push(Field::choice(
                    "edit-mode",
                    "Edit",
                    edits[form.edit_mode.min(2)],
                    edits,
                ));
                match form.edit_mode {
                    0 => d.fields.push(Field::multiline(
                        "additions",
                        "One local path per line",
                        &form.additions,
                    )),
                    1 => d.fields.push(Field::label(
                        "Selected entries",
                        form.selected
                            .iter()
                            .map(|p| p.display().to_string())
                            .collect::<Vec<_>>()
                            .join("\n"),
                    )),
                    _ => d.fields.extend([
                        Field::text("source", "Source entry", &form.source),
                        Field::text("target", "New entry path", &form.target),
                    ]),
                }
                d.button("browse-archive", "Browse archive", true);
            } else if form.mode == 1 {
                d.fields
                    .push(Field::text("destination", "Destination", &form.destination));
                d.fields.push(Field::label(
                    "Selected files",
                    form.sources
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join("\n"),
                ));
            }
            if form.mode > 0 {
                d.fields.push(Field::password(
                    "archive-password",
                    "Password",
                    &form.password,
                ));
            }
            d.button(
                "accept",
                modes[form.mode.min(2)],
                self.app.pending_archive.is_none(),
            );
            return d;
        }
        if let Some(form) = &self.app.tools.rename {
            let mut d = Dialog::new("rename", "Batch rename");
            d.error = form.error.clone().unwrap_or_default();
            let o = &form.options;
            d.fields.extend([
                Field::text("find", "Find", &o.find),
                Field::text("replace", "Replace", &o.replace),
                Field::check("regex", "Use regular expression", o.regex),
                Field::text("template", "Template", &o.template),
                Field::text("start", "First number", o.start),
                Field::text("padding", "Padding", o.width),
            ]);
            d.description="{name} = filename without extension · {ext} = extension including dot · {n} = number".into();
            match crate::desktop::rename_plan(&form.sources, o) {
                Ok(plan) => {
                    for (source, target) in plan {
                        d.rows.push(Row::new(
                            "",
                            source.file_name().unwrap_or_default().to_string_lossy(),
                            format!(
                                "→ {}",
                                target.file_name().unwrap_or_default().to_string_lossy()
                            ),
                        ));
                    }
                }
                Err(error) => d.error = error.to_string(),
            }
            d.button("accept", "Rename", idle && d.error.is_empty());
            return d;
        }
        if let Some(form) = &self.app.tools.search {
            let mut d = Dialog::new("search", "Find files");
            d.error = form.error.clone().unwrap_or_default();
            d.busy = form.pending.is_some();
            let o = &form.options;
            d.fields.extend([
                Field::text("search-root", "Root folder", o.root.display()),
                Field::text("pattern", "Filename pattern", &o.pattern),
                Field::text("content", "Containing text", &o.content),
                Field::check("search-hidden", "Include hidden files", o.hidden),
                Field::text("limit", "Maximum results", o.max_results),
            ]);
            if let Some(result) = &form.result {
                d.description = format!(
                    "{} results{} · {} unreadable entries",
                    result.paths.len(),
                    if result.truncated {
                        " (limit reached)"
                    } else {
                        ""
                    },
                    result.errors.len()
                );
                for (i, path) in result.paths.iter().enumerate() {
                    d.rows
                        .push(Row::new(format!("search-{i}"), path.display(), "").action(
                            format!("search-open-{i}"),
                            "Open folder",
                            true,
                        ));
                }
                for error in &result.errors {
                    d.fields.push(Field::label("Unreadable entry", error));
                }
            }
            d.button("stop", "Stop", d.busy);
            d.button("accept", "Search", !d.busy);
            return d;
        }
        if let Some(form) = &self.app.features.sync {
            let directions = ["Review manually", "Left → right", "Right → left"];
            let mut d = Dialog::new("sync", "Synchronize folders");
            d.error = form.error.clone().unwrap_or_default();
            d.busy = form.pending.is_some();
            d.fields.extend([
                Field::text("sync-left", "Left folder", &form.left),
                Field::text("sync-right", "Right folder", &form.right),
                Field::choice(
                    "direction",
                    "Direction",
                    directions[usize::from(form.direction.min(2))],
                    directions,
                ),
                Field::check("deletions", "Allow deletion choices", form.deletions),
                Field::check(
                    "confirmed",
                    "I reviewed the plan and confirm deletion choices",
                    form.confirmed,
                ),
            ]);
            if let Some(plan) = &form.plan {
                for (i, item) in plan.items.iter().enumerate() {
                    let mut choices = vec![crate::sync::Choice::Skip];
                    if item.left {
                        choices.push(crate::sync::Choice::LeftToRight);
                    }
                    if item.right {
                        choices.push(crate::sync::Choice::RightToLeft);
                    }
                    if form.deletions {
                        if item.left {
                            choices.push(crate::sync::Choice::DeleteLeft);
                        }
                        if item.right {
                            choices.push(crate::sync::Choice::DeleteRight);
                        }
                    }
                    choices.retain(|choice| plan.allows(item, *choice));
                    d.fields.push(Field::choice(
                        format!("sync-item-{i}"),
                        format!(
                            "{}{}",
                            item.path.display(),
                            if item.conflict { " · conflict" } else { "" }
                        ),
                        item.choice.label(),
                        choices.iter().map(|c| c.label()),
                    ));
                }
            }
            d.button("sync-preview", "Preview plan", idle && !d.busy);
            d.button(
                "accept",
                "Apply reviewed plan",
                idle && !d.busy && form.plan.is_some(),
            );
            return d;
        }
        if let Some(form) = &self.app.features.recovery {
            let mut d = Dialog::new("recovery", "Recovery storage");
            d.error = form.error.clone().unwrap_or_default();
            d.busy = form.pending.is_some() || form.cleaning.is_some();
            d.description="Recovery copies contain file contents. Cleanup expires Undo/Redo for eligible chains. Pinned chains and the latest reversible chain are retained.".into();
            if let Some(stats) = &form.stats {
                d.fields.push(Field::label(
                    "Storage",
                    format!(
                        "{} used · {} backups · {} in incomplete journals",
                        filesystem::human_size(stats.bytes),
                        stats.backups,
                        filesystem::human_size(stats.orphan_bytes)
                    ),
                ));
            }
            let p = &self.app.preferences.recovery;
            d.fields.extend([
                Field::text("max-mib", "Limit (MiB)", p.max_mib),
                Field::text("max-days", "Days", p.max_days),
                Field::check(
                    "automatic",
                    "Automatically clean eligible recovery copies after operations",
                    p.automatic,
                ),
            ]);
            for (i, record) in self
                .app
                .history
                .iter()
                .enumerate()
                .rev()
                .filter(|(_, r)| !r.expired && !r.changes.is_empty())
            {
                d.fields.push(Field::check(
                    format!("pin-{i}"),
                    format!("Pin: {} · {}", tr(&record.label), record.id),
                    record.pinned,
                ));
            }
            if let Some(ids) = &form.candidates {
                d.fields.push(Field::label(
                    "Cleanup preview",
                    format!(
                        "{} recovery records will expire\n{}",
                        ids.len(),
                        ids.join("\n")
                    ),
                ));
                d.button(
                    "expire-backups",
                    "Expire these records and delete their backups",
                    idle && !d.busy && !ids.is_empty(),
                );
            }
            d.button("open-recovery", "Open recovery folder", true);
            d.button("recovery-preview", "Preview cleanup", idle && !d.busy);
            return d;
        }
        if self.app.features.workspaces {
            let mut d = Dialog::new("workspaces", "Named workspaces");
            d.description="Save both panes, tabs, active locations and layout. Connection credentials stay in the OS credential store.".into();
            d.error = self.app.features.error.clone().unwrap_or_default();
            d.fields.push(Field::text(
                "workspace-name",
                "Name",
                &self.app.features.workspace_name,
            ));
            for (i, w) in self.app.workspaces.iter().enumerate() {
                let mut row = Row::new(
                    format!("workspace-{i}"),
                    &w.name,
                    format!(
                        "{} left tabs · {} right tabs",
                        w.tabs[0].len(),
                        w.tabs[1].len()
                    ),
                )
                .action(format!("workspace-load-{i}"), "Load", idle)
                .action(format!("workspace-rename-{i}"), "Rename", true)
                .action(format!("workspace-delete-{i}"), "Delete", true);
                row.selected = self.app.features.workspace_selected == Some(i);
                d.rows.push(row);
            }
            d.button("accept", "Save current workspace", true);
            return d;
        }
        if let Some((path, application)) = &self.app.features.open_with {
            let mut d = Dialog::new("open-with", "Open With");
            d.description = path.display().to_string();
            d.error = self.app.features.error.clone().unwrap_or_default();
            d.fields.push(Field::text(
                "application-filter",
                "Find application",
                &self.app.features.application_filter,
            ));
            let filter = self.app.features.application_filter.to_lowercase();
            let choices = self
                .app
                .features
                .applications
                .iter()
                .filter(|p| p.to_string_lossy().to_lowercase().contains(&filter))
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>();
            d.fields.push(Field::choice(
                "installed-application",
                "Installed applications",
                application,
                choices,
            ));
            d.fields.push(Field::check(
                "remember-application",
                "Remember this application",
                self.app.features.remember_application,
            ));
            d.fields.push(Field::text(
                "application-pattern",
                "Filename pattern",
                &self.app.features.application_pattern,
            ));
            d.fields.push(Field::text(
                "application",
                "Application or executable",
                application,
            ));
            d.button("accept", "Open", true);
            return d;
        }
        if self.app.show_history {
            let mut d = Dialog::new("history", "Transfer queue and history");
            if let Some(job) = &self.app.job {
                d.description = format!("{} · {}", tr(&job.label), job.progress.current);
                d.button("cancel-job", "Cancel running operation", true);
            }
            d.button(
                "pause-queue",
                if self.app.queue.paused {
                    "Resume queue"
                } else {
                    "Pause queue"
                },
                true,
            );
            for (i, operation) in self.app.queue.iter().enumerate() {
                d.rows.push(
                    Row::new(
                        format!("queue-{i}"),
                        format!("Queued: {}", tr(operation.label())),
                        "",
                    )
                    .action(format!("queue-up-{i}"), "Move up", i > 0)
                    .action(
                        format!("queue-down-{i}"),
                        "Move down",
                        i + 1 < self.app.queue.len(),
                    )
                    .action(format!("queue-remove-{i}"), "Remove", true),
                );
            }
            for (i, operation) in self.app.queue.failed.iter().enumerate() {
                d.rows.push(
                    Row::new(
                        format!("failed-{i}"),
                        format!("Failed: {}", tr(operation.label())),
                        "",
                    )
                    .action(format!("queue-retry-{i}"), "Retry", idle)
                    .action(format!("queue-discard-{i}"), "Discard", true),
                );
            }
            for (i, record) in self.app.history.iter().enumerate().rev() {
                d.rows.push(
                    Row::new(format!("record-{i}"), tr(&record.label), &record.result)
                        .action(
                            format!("record-undo-{i}"),
                            if record.is_undo { "Redo" } else { "Undo" },
                            idle && record.success
                                && !record.changes.is_empty()
                                && !record.undone
                                && !record.undo_blocked
                                && !record.expired,
                        )
                        .action(
                            format!("record-pin-{i}"),
                            if record.pinned { "Unpin" } else { "Pin" },
                            idle,
                        )
                        .action(format!("record-details-{i}"), "Details", true),
                );
            }
            return d;
        }
        Dialog::default()
    }

    pub(super) fn poll_dialogs(&mut self) {
        if let Some(form) = &mut self.app.tools.search
            && let Some(result) = crate::background::poll(&mut form.pending)
        {
            match result {
                Ok(result) => form.result = Some(result),
                Err(error) => form.error = Some(error),
            }
        }
        if let Some(form) = &mut self.app.features.sync
            && let Some(result) = crate::background::poll(&mut form.pending)
        {
            match result {
                Ok(plan) => form.plan = Some(plan),
                Err(error) => form.error = Some(error),
            }
        }
        let mut reload = false;
        if let Some(form) = &mut self.app.features.recovery {
            if let Some(result) = crate::background::poll(&mut form.pending) {
                match result {
                    Ok(stats) => form.stats = Some(stats),
                    Err(error) => form.error = Some(error),
                }
            }
            if let Some(result) = crate::background::poll(&mut form.cleaning) {
                match result {
                    Ok(()) => {
                        reload = true;
                        form.candidates = None;
                        form.refresh(&self.ctx);
                    }
                    Err(error) => form.error = Some(error),
                }
            }
        }
        if reload {
            self.app
                .apply_feature_effect(crate::next_ui::Effect::ReloadHistory, &self.ctx);
        }
    }
}
