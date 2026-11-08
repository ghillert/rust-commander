use super::*;
mod commands;
mod fields;
#[derive(Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) enum Picker {
    File,
    Directory,
    NewDirectory,
    SaveFile,
}
impl Picker {
    pub(super) fn selected(self, path: std::path::PathBuf, current: &str) -> String {
        let path = if self == Self::NewDirectory {
            path.join(
                std::path::Path::new(current)
                    .file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("extracted")),
            )
        } else {
            path
        };
        path.to_string_lossy().into_owned()
    }
}
mod folders;
#[cfg(target_os = "macos")]
pub(super) mod native_extract;
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub(super) mod native_host;
pub(super) mod native_intent;
mod preferences;

#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct Field {
    pub id: String,
    pub title: String,
    pub value: String,
    pub kind: i32,
    pub choices: Vec<String>,
    pub page: i32,
    #[serde(default)]
    pub picker: Option<Picker>,
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
            page: 0,
            picker: None,
        }
    }
    pub(super) fn password(id: &str, title: &str, value: &str) -> Self {
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
    pub(super) fn multiline(id: &str, title: &str, value: impl ToString) -> Self {
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
            page: self.page,
        }
    }
}
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct Row {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub selected: bool,
    pub actions: Vec<(String, String, bool)>,
    pub page: i32,
}
impl Row {
    pub(super) fn new(id: impl Into<String>, title: impl ToString, detail: impl ToString) -> Self {
        Self {
            id: id.into(),
            title: title.to_string(),
            detail: detail.to_string(),
            selected: false,
            actions: Vec::new(),
            page: 0,
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
            page: self.page,
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
#[derive(Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct Dialog {
    pub kind: String,
    pub title: String,
    pub description: String,
    pub error: String,
    pub busy: bool,
    pub fields: Vec<Field>,
    pub rows: Vec<Row>,
    pub buttons: Vec<(String, String, bool)>,
    pub default_action: bool,
    pub error_page: Option<i32>,
}
impl Dialog {
    pub(super) fn native_pickers(&mut self, prompt: Option<&Prompt>) {
        for field in &mut self.fields {
            if !matches!(field.kind, 0 | 5) {
                continue;
            }
            field.picker = match field.id.as_str() {
                "key-file" | "known-hosts" | "ssh-config-file" | "application" | "archive-file" => {
                    Some(Picker::File)
                }
                "directory" if self.kind == "import" => Some(Picker::Directory),
                "destination" if self.kind == "archive" && !field.value.contains("://") => {
                    Some(Picker::SaveFile)
                }
                "prompt" => prompt.and_then(|p| match &p.kind {
                    PromptKind::Pack { .. } if !p.value.contains("://") => Some(Picker::SaveFile),
                    PromptKind::Extract { .. } => Some(Picker::NewDirectory),
                    PromptKind::Transfer { .. } if !p.value.contains("://") => {
                        Some(Picker::Directory)
                    }
                    PromptKind::RemoteTransfer { to, .. } | PromptKind::DropTransfer { to, .. }
                        if matches!(
                            filesystem::parse_location(&p.value, to),
                            Ok(Location::Local(_))
                        ) =>
                    {
                        Some(Picker::Directory)
                    }
                    _ => None,
                }),
                _ => None,
            };
        }
    }
    pub(super) fn new(kind: &str, title: &str) -> Self {
        Self {
            kind: kind.into(),
            title: tr(title).into_owned(),
            buttons: vec![(
                "cancel".into(),
                if matches!(
                    kind,
                    "prompt"
                        | "pack"
                        | "preferences"
                        | "archive"
                        | "rename"
                        | "sync"
                        | "open-with"
                        | "open-as"
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

/// Native form controls suit editable forms and short confirmations. File panes,
/// viewers, text diffs and streaming result/queue consoles retain their shared UI.
pub(super) fn native_supported(dialog: &Dialog) -> bool {
    matches!(
        dialog.kind.as_str(),
        "preferences"
            | "prompt"
            | "pack"
            | "connections"
            | "archive"
            | "rename"
            | "sync"
            | "recovery"
            | "open-as"
            | "open-with"
            | "exit"
            | "quit"
            | "message"
            | "browser-url"
            | "donate"
            | "email"
            | "external-edit"
            | "associations"
            | "updates"
            | "diagnostics"
            | "report"
            | "security"
            | "drafts"
            | "selection"
            | "tab-title"
            | "empty-trash"
            | "import"
            | "run-command"
    ) && dialog
        .fields
        .iter()
        .all(|f| matches!(f.kind, 0..=6 | 8..=12))
        && dialog.description.len() <= 4096
        && dialog.fields.len() <= 512
        && dialog.rows.len() <= 128
        && dialog.buttons.len() <= 5
}
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
    pub(super) fn native_dialog(&self, spec: &Dialog) -> Dialog {
        let mut native = spec.clone();
        native.default_action = !matches!(spec.kind.as_str(), "exit" | "quit" | "empty-trash")
            && !self.app.prompt.as_ref().is_some_and(|p| {
                matches!(
                    p.kind,
                    PromptKind::Trash(_)
                        | PromptKind::PermanentDelete(_)
                        | PromptKind::RemoteDelete { .. }
                )
            });
        native.error_page = self
            .invalid_fields
            .keys()
            .next()
            .and_then(|id| spec.fields.iter().find(|field| &field.id == id))
            .map(|field| field.page);
        native.error = self
            .invalid_fields
            .values()
            .next()
            .map(|(_, error)| error.clone())
            .or_else(|| self.field_error.clone())
            .unwrap_or_else(|| spec.error.clone());
        if !native.error.is_empty() {
            for (id, _, enabled) in &mut native.buttons {
                if id == "accept" || id == "apply" {
                    *enabled = false;
                }
            }
        }
        native
    }
    pub(super) fn dialog(&self) -> Dialog {
        if let Some(confirmation) = &self.quit_confirmation {
            let mut dialog = Dialog::new("quit", "Quit Rust Commander?");
            dialog.description = if confirmation.window_count == 1 {
                tr("You have one window open. Are you sure you want to quit?").into_owned()
            } else {
                tr("You have {count} windows open. Are you sure you want to quit?")
                    .replace("{count}", &confirmation.window_count.to_string())
            };
            if confirmation.has_pending_work {
                dialog.description.push_str("\n\n");
                dialog.description.push_str(&tr(
                    "Running operations will be cancelled and unsaved changes will be discarded.",
                ));
            }
            dialog.fields.push(Field::check(
                "quit-show-next-time",
                "Show next time",
                confirmation.show_next_time,
            ));
            dialog.buttons = vec![
                ("cancel".into(), "Cancel".into(), true),
                ("confirm-quit".into(), "Quit".into(), true),
            ];
            return dialog;
        }
        if !self.app.confirm_exit {
            if let Some(dialog) = self.drafts_dialog() {
                return dialog;
            }
            if let Some(dialog) = self.security_dialog() {
                return dialog;
            }
        }
        if let Some(dialog) = self.desktop_commands_dialog() {
            return dialog;
        }
        if !self.app.confirm_exit
            && self.app.message.is_none()
            && let Some(dialog) = self.management_dialog()
        {
            return dialog;
        }
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
            d.description = tr("There is a running operation or unsaved text. Closing cancels the operation and discards unsaved text.").into_owned();
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
                ("Space / Insert", "Mark or unmark current file and advance"),
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
        if let Some(job) = &self.app.job
            && job.foreground
            && job.owner == self.app.window_id
        {
            let mut d = Dialog::new("pack-progress", "Packing files");
            d.busy = true;
            d.description = if job.cancel.load(Ordering::Relaxed) {
                "Cancelling…".into()
            } else {
                "Creating the archive…".into()
            };
            d.fields
                .push(Field::label("Current file", &job.progress.current));
            d.fields.push(Field::label(
                "Progress",
                format!(
                    "{} file(s) · {}",
                    job.progress.files,
                    filesystem::human_size(job.progress.bytes)
                ),
            ));
            d.buttons = vec![
                ("pack-background".into(), "Run in background".into(), true),
                (
                    "cancel-pack".into(),
                    "Cancel".into(),
                    !job.cancel.load(Ordering::Relaxed),
                ),
            ];
            return d;
        }
        if let Some(prompt) = &self.app.prompt {
            if let PromptKind::Pack {
                sources,
                selection,
                options,
                ..
            } = &prompt.kind
            {
                let mut d = Dialog::new("pack", "Pack files");
                d.description = if selection.is_some() {
                    "Pack the selected remote or archive files using private temporary space. Choose a local path or a configured remote destination."
                } else {
                    "Choose a local path or a configured remote destination for the new archive."
                }.into();
                d.error = prompt.error.clone().unwrap_or_default();
                d.fields.push(Field::text(
                    "prompt",
                    "Add selected files to",
                    &prompt.value,
                ));
                d.fields.push(Field::check(
                    "pack-background",
                    "Run in background",
                    options.background,
                ));
                d.fields.push(Field::choice(
                    "pack-format",
                    "Archive format",
                    options.label(),
                    crate::packing::FORMATS.iter().map(|(_, label, _)| *label),
                ));
                if options.format == crate::archive::Format::Tar {
                    d.fields
                        .push(Field::label("Compression", "None (uncompressed TAR)"));
                } else {
                    d.fields.push(Field::choice(
                        "pack-level",
                        "Compression level",
                        tr(&crate::packing::level_label(
                            options.format,
                            options.compression_level,
                        )),
                        crate::packing::level_choices(options.format)
                            .into_iter()
                            .map(|label| tr(&label).into_owned()),
                    ));
                }
                if options.format == crate::archive::Format::Zip {
                    d.fields.push(Field::multiline(
                        "pack-comment",
                        "Comment",
                        &options.comment,
                    ));
                }
                if matches!(
                    options.format,
                    crate::archive::Format::Zip | crate::archive::Format::SevenZ
                ) {
                    d.fields.push(Field::check(
                        "pack-encrypted",
                        "Encrypt archive",
                        options.encrypted,
                    ));
                    if options.encrypted {
                        d.fields.push(Field::password(
                            "pack-password",
                            "Password",
                            &options.password,
                        ));
                    }
                }
                d.fields.push(Field {
                    kind: 8,
                    ..Field::text(
                        "pack-files",
                        format!("{} file(s)", sources.len()),
                        options.expanded,
                    )
                });
                if options.expanded {
                    for (index, source) in sources.iter().enumerate() {
                        d.fields.push(Field {
                            kind: 9,
                            choices: vec![
                                if selection
                                    .as_ref()
                                    .map_or_else(|| source.is_dir(), |s| s.entries[index].is_dir)
                                {
                                    "folder"
                                } else {
                                    "file"
                                }
                                .into(),
                            ],
                            ..Field::text("", "", source.display())
                        });
                    }
                }
                d.button(
                    "accept",
                    "Pack",
                    !sources.is_empty() && (!options.encrypted || !options.password.is_empty()),
                );
                return d;
            }

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
                    match prompt.kind {
                        PromptKind::DropTransfer { .. }
                        | PromptKind::Transfer { .. }
                        | PromptKind::RemoteTransfer { .. }
                        | PromptKind::Extract { .. } => "Destination",
                        PromptKind::Rename(_) | PromptKind::RemoteRename { .. } => "New name",
                        PromptKind::Mkdir(_) | PromptKind::RemoteMkdir { .. } => "Folder name",
                        PromptKind::Permissions(_) => "Octal permissions",
                        _ => "Value",
                    },
                    &prompt.value,
                ));
            }
            if matches!(prompt.kind, PromptKind::Permissions(_))
                && let Ok(mode) = crate::metadata::parse_mode(&prompt.value)
            {
                d.fields.push(Field::label(
                    "Symbolic permissions",
                    crate::metadata::symbolic(mode),
                ));
                for (title, bit) in crate::metadata::PERMISSION_BITS {
                    d.fields.push(Field::check(
                        format!("permission-bit-{bit:o}"),
                        title,
                        mode & bit != 0,
                    ));
                }
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
                        | PromptKind::Transfer { moving: false, .. }
                ) || matches!(&prompt.kind, PromptKind::DropTransfer { selection, moving: false, .. } if !matches!(selection.from, Location::Archive { .. }))
                {
                    d.fields.push(Field::check(
                        "resumable",
                        "Checkpoint this copy (automatic retry and recovery)",
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
                    d.fields.push(Field::label("Recoverable replacement", if prompt.resumable { "Originals stay in this copy’s private recovery folder under the configuration directory. The result reports its path; review it before deleting." } else { "Replaced items stay in a hidden recovery folder in the destination. The result reports its path." }));
                }
            }
            d.button(
                "accept",
                match prompt.kind {
                    PromptKind::Trash(_) => "Move to Trash",
                    PromptKind::Extract { .. } => "Extract",
                    PromptKind::Transfer { moving: true, .. }
                    | PromptKind::RemoteTransfer { moving: true, .. }
                    | PromptKind::DropTransfer { moving: true, .. } => "Move",
                    PromptKind::Transfer { .. }
                    | PromptKind::RemoteTransfer { .. }
                    | PromptKind::DropTransfer { .. } => "Copy",
                    PromptKind::Rename(_) | PromptKind::RemoteRename { .. } => "Rename",
                    PromptKind::Mkdir(_) | PromptKind::RemoteMkdir { .. } => "Create",
                    PromptKind::PermanentDelete(_) | PromptKind::RemoteDelete { .. } => "Delete",
                    _ => "Confirm",
                },
                !d.busy,
            );
            return d;
        }
        if let Some(p) = &self.app.preferences_form {
            return self.preferences_dialog(p);
        }
        if let Some(form) = &self.app.connection_form {
            let mut d = Dialog::new("connections", "Connections");
            if let Some(challenge) = &form.ssh_challenge {
                d.error = form.error.clone().unwrap_or_default();
                d.title = tr("Verify SSH host key").into_owned();
                d.description = tr("This host is not in known_hosts. Compare the SHA-256 fingerprint with the server administrator using a trusted channel before continuing.").into_owned();
                d.fields.extend([
                    Field::label("Host", format!("{}:{}", challenge.host, challenge.port)),
                    Field::label("Known hosts identity", &challenge.lookup),
                    Field::label("Key algorithm", &challenge.algorithm),
                    Field::label("SHA-256 fingerprint", &challenge.fingerprint),
                ]);
                if let Some(path) = &challenge.save_to {
                    d.fields
                        .push(Field::label("Save host key to", path.display()));
                }
                d.button("ssh-trust-once", "Trust this key once", true);
                if challenge.save_to.is_some() {
                    d.button("ssh-trust-save", "Trust and save key", true);
                }
                return d;
            }
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
                d.fields.push(Field::label("SFTP", "Review unknown host fingerprints before connecting. Changed and revoked keys are refused."));
                d.fields.push(Field::check(
                    "ssh-config",
                    "Use SSH configuration and host aliases",
                    c.ssh_use_config,
                ));
                d.fields.push(Field::text(
                    "ssh-config-file",
                    "SSH config file (optional)",
                    &c.ssh_config_file,
                ));
                d.fields.push(Field::password(
                    "ssh-passphrase",
                    "Private key passphrase",
                    &s.ssh_passphrase,
                ));
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
            d.fields.extend([Field::choice(
                "archive-mode",
                "Action",
                modes[form.mode.min(2)],
                modes,
            )]);
            if let Some(source) = &form.source_display {
                d.fields.push(Field::label("Archive source", source));
                d.fields.push(Field::label(
                    "Save-back",
                    "Edits are checked and saved through the containing archives to this source.",
                ));
            } else {
                d.fields
                    .push(Field::text("archive-file", "Archive", &form.file));
            }
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
                let format = crate::archive::format(Path::new(&form.destination))
                    .unwrap_or(crate::archive::Format::Zip);
                d.fields.push(Field::choice(
                    "archive-level",
                    "Compression level",
                    tr(&crate::packing::level_label(format, form.compression_level)),
                    crate::packing::level_choices(format)
                        .into_iter()
                        .map(|label| tr(&label).into_owned()),
                ));
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
        if let Some(dialog) = self.folder_dialog() {
            return dialog;
        }
        if let Some(form) = &self.app.tools.search {
            let mut d = Dialog::new("search", "Find files");
            d.description = "Patterns match names or relative paths; separate patterns with ;. Literal content searches stream across the entire file. Regex matches one line at a time (8 MiB maximum per line).".into();
            d.error = form.error.clone().unwrap_or_default();
            d.busy = form.pending.is_some();
            let o = &form.options;
            d.fields.extend([
                Field::text("search-root", "Root folder", o.root.display()),
                Field::text("pattern", "Filename pattern", &o.pattern),
                Field::text("content", "Containing text", &o.content),
                Field::check(
                    "search-regex",
                    "Content is a regular expression (per line)",
                    o.filters.content_regex,
                ),
                Field::check(
                    "search-content-case",
                    "Match content case",
                    o.filters.content_case_sensitive,
                ),
                Field::check(
                    "search-name-case",
                    "Match filename case",
                    o.filters.name_case_sensitive,
                ),
                Field::text(
                    "search-exclude",
                    "Exclude files/folders (glob patterns separated by ;)",
                    &o.filters.exclude,
                ),
                Field::text(
                    "search-min-size",
                    "Minimum file size (bytes; blank for any)",
                    o.filters
                        .min_size
                        .map(|n| n.to_string())
                        .unwrap_or_default(),
                ),
                Field::text(
                    "search-max-size",
                    "Maximum file size (bytes; blank for any)",
                    o.filters
                        .max_size
                        .map(|n| n.to_string())
                        .unwrap_or_default(),
                ),
                Field::text(
                    "search-after",
                    "Modified on/after (RFC 3339; blank for any)",
                    crate::search::date_label(o.filters.modified_after),
                ),
                Field::text(
                    "search-before",
                    "Modified on/before (RFC 3339; blank for any)",
                    crate::search::date_label(o.filters.modified_before),
                ),
                Field::check("search-hidden", "Include hidden files", o.hidden),
                Field::text("limit", "Maximum results", o.max_results),
            ]);
            if let Some(result) = &form.result {
                d.description = format!(
                    "{} results{} · {} entries could not be searched",
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
            folders::rules_fields(&mut d, &form.rules);
            folders::presets(
                &mut d,
                &self.app.preferences.folder_tool_presets,
                &form.preset_name,
            );
            if form.rules.filtered() {
                d.description = "Filters apply to files inside folders. Missing folders are created without copying excluded contents. Directory deletion and file/folder replacement are unavailable with filters.".into();
            }
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
        if let Some(form) = &self.app.features.open_as {
            let mut d = Dialog::new("open-as", "Open As");
            d.description = form.entry.path.display().to_string();
            d.fields.push(Field::choice(
                "open-format",
                "Open as",
                tr(form.label()),
                crate::open_as::CHOICES
                    .iter()
                    .map(|(label, _)| tr(label).into_owned())
                    .collect::<Vec<_>>(),
            ));
            if matches!(
                form.kind,
                crate::open_as::Kind::Archive(
                    crate::archive::Format::Zip | crate::archive::Format::SevenZ
                )
            ) {
                d.fields.push(Field::password(
                    "open-password",
                    "Archive password (if required)",
                    &form.password,
                ));
            }
            d.button("accept", "Open", true);
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
                if let Some(control) = &job.control {
                    let status = control.status();
                    if !status.is_empty() {
                        d.description = format!("{status} · {}", d.description);
                    }
                    d.button(
                        "pause-job",
                        if control.paused() {
                            "Resume transfer"
                        } else {
                            "Pause transfer"
                        },
                        true,
                    );
                }
            }
            d.fields.push(Field::text(
                "transfer-limit",
                "Bandwidth limit (KiB/s; 0 = unlimited)",
                self.app.preferences.transfers.kib_per_second.to_string(),
            ));
            d.fields.push(Field::text(
                "transfer-retries",
                "Automatic retries (0–8; checkpointed copies only)",
                self.app.preferences.transfers.retries.to_string(),
            ));
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
        if let Some(form) = &mut self.app.features.comparison {
            form.poll();
        }
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
