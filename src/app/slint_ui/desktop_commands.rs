//! Desktop command dialogs: bookmarks, email, support links and event console.
use super::{
    dialogs::{Dialog, Field, Row},
    *,
};
use anyhow::{Context, ensure};
use std::collections::BTreeSet;

#[derive(Default)]
pub(super) struct State {
    kind: Option<&'static str>,
    bookmarks: Vec<Bookmark>,
    selected: Option<usize>,
    filter: String,
    name: String,
    donation: String,
    browser_url: String,
    error: String,
    events: Vec<crate::debug_log::Event>,
    paused: bool,
    email: Option<Email>,
    #[cfg(test)]
    opened_urls: Vec<String>,
}
impl State {
    pub(super) fn active(&self) -> bool {
        self.kind.is_some()
    }
    pub(super) fn sending(&self) -> bool {
        self.email
            .as_ref()
            .is_some_and(|mail| mail.sending.is_some())
    }
    pub(super) fn email_open(&self) -> bool {
        self.email.is_some()
    }
}

struct Email {
    settings: crate::mail::Settings,
    password: String,
    to: String,
    subject: String,
    body: String,
    show_settings: bool,
    prepared: Option<crate::mail::Prepared>,
    selected: BTreeSet<usize>,
    pending: Option<Receiver<Result<crate::mail::Prepared, String>>>,
    sending: Option<Receiver<Result<(), String>>>,
    cancel: Arc<AtomicBool>,
}
impl Drop for Email {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl Adapter {
    fn save_desktop_settings(&self) -> anyhow::Result<()> {
        let settings = self.app.snapshot_settings();
        #[cfg(test)]
        if let Some(path) = &self.desktop_settings_path {
            return crate::settings::save_to(path, &settings);
        }
        crate::settings::save(&settings)
    }
    fn launch_website(&mut self, value: &str) -> anyhow::Result<()> {
        let url = crate::support::web_link(value)?;
        self.launch_browser(url.as_str())
    }
    fn launch_browser(&mut self, value: &str) -> anyhow::Result<()> {
        let url = crate::support::browser_link(value)?;
        #[cfg(test)]
        {
            self.desktop_commands.opened_urls.push(url.to_string());
        }
        #[cfg(not(test))]
        {
            open::that(url.as_str())?;
        }
        Ok(())
    }
    pub(super) fn desktop_command_action(&mut self, name: &str) -> bool {
        match name {
            "MinimizeWindow" => self.extra.window_request = Some(super::windows::Request::Minimize),
            "MaximizeWindow" => self.extra.window_request = Some(super::windows::Request::Maximize),
            "OpenURLInBrowser" => {
                let browser_url = if let Location::Remote { connection, path } =
                    &self.app.tab().location
                    && crate::remote::profile(connection).is_some_and(|p| p.service == "http")
                {
                    let chosen = self.app.tab().chosen(self.app.show_hidden);
                    let path = if chosen.len() == 1 {
                        &chosen[0].path
                    } else {
                        path
                    };
                    crate::uri::address(connection, path)
                } else {
                    String::new()
                };
                self.desktop_commands = State {
                    kind: Some("browser-url"),
                    browser_url,
                    ..Default::default()
                };
            }
            "BringAllToFront" => {
                self.extra.window_request = Some(super::windows::Request::RaiseAll)
            }
            "GoToDocumentation" | "GoToForums" | "GoToWebsite" => {
                let url = match name {
                    "GoToDocumentation" => crate::support::DOCUMENTATION,
                    "GoToForums" => crate::support::COMMUNITY,
                    _ => crate::support::REPOSITORY,
                };
                if let Err(error) = self.launch_website(url) {
                    self.app.fail(error);
                }
            }
            "Donate" => {
                self.desktop_commands = State {
                    kind: Some("donate"),
                    donation: self.app.preferences.donation_url.clone(),
                    ..Default::default()
                };
            }
            "ExploreBookmarks" => {
                self.desktop_commands = State {
                    kind: Some("bookmarks"),
                    bookmarks: self.app.bookmarks.clone(),
                    ..Default::default()
                };
            }
            "ShowDebugConsole" => {
                self.desktop_commands = State {
                    kind: Some("debug-console"),
                    events: crate::debug_log::snapshot(),
                    ..Default::default()
                };
            }
            "Email" => {
                let selection = crate::drag_drop::Selection {
                    from: self.app.tab().location.clone(),
                    entries: self.app.tab().chosen(self.app.show_hidden),
                };
                if let Err(error) = selection.validate() {
                    self.app.fail(error);
                    return true;
                }
                let (sender, receiver) = mpsc::channel();
                let cancel = Arc::new(AtomicBool::new(false));
                let worker_cancel = cancel.clone();
                let ctx = self.ctx.clone();
                std::thread::spawn(move || {
                    let result =
                        crate::mail::prepare(&selection, &Task::new(worker_cancel, |_| {}))
                            .map_err(|error| format!("{error:#}"));
                    let _ = sender.send(result);
                    ctx.request_repaint();
                });
                let mut settings = self.app.preferences.mail.clone();
                if settings.port == 0 {
                    settings.port = 587;
                }
                let show_settings = settings.host.is_empty();
                self.desktop_commands = State {
                    kind: Some("email"),
                    email: Some(Email {
                        settings,
                        password: String::new(),
                        to: String::new(),
                        subject: String::new(),
                        body: String::new(),
                        show_settings,
                        prepared: None,
                        selected: Default::default(),
                        pending: Some(receiver),
                        sending: None,
                        cancel,
                    }),
                    ..Default::default()
                };
            }
            _ => return false,
        }
        true
    }
    pub(super) fn desktop_commands_tick(&mut self) {
        if self.desktop_commands.kind == Some("debug-console") && !self.desktop_commands.paused {
            self.desktop_commands.events = crate::debug_log::snapshot();
        }
        let Some(email) = &mut self.desktop_commands.email else {
            return;
        };
        if let Some(result) = crate::background::poll(&mut email.pending) {
            match result {
                Ok(prepared) => {
                    email.selected = (0..prepared.files.len()).collect();
                    email.prepared = Some(prepared);
                }
                Err(error) => self.desktop_commands.error = error,
            }
        }
        if let Some(result) = crate::background::poll(&mut email.sending) {
            match result {
                Ok(()) => {
                    crate::debug_log::record("email", "SMTP server accepted the message");
                    self.desktop_commands = Default::default();
                    self.app.message=Some("The SMTP server accepted the email. Final recipient delivery depends on the mail service.".into());
                }
                Err(error) => {
                    crate::debug_log::record(
                        "email",
                        "SMTP delivery failed or is unconfirmed; no automatic retry",
                    );
                    self.desktop_commands.error = error;
                }
            }
        }
    }
    pub(super) fn desktop_commands_dialog(&self) -> Option<Dialog> {
        let state = &self.desktop_commands;
        let kind = state.kind?;
        let mut dialog = Dialog::new(
            kind,
            match kind {
                "bookmarks" => "Explore bookmarks",
                "email" => "Email files",
                "donate" => "Donate",
                "browser-url" => "Open URL in browser",
                _ => "Debug console",
            },
        );
        dialog.error = state.error.clone();
        match kind {
            "browser-url" => {
                dialog.description = "Enter or paste a URL, review it, then open it in your system browser. A selected HTTP entry is filled in automatically.".into();
                dialog
                    .fields
                    .push(Field::text("browser-url", "URL", &state.browser_url));
                dialog.button(
                    "browser-open",
                    "Open in browser",
                    !state.browser_url.trim().is_empty(),
                );
            }
            "bookmarks" => {
                dialog.description="Browse saved bookmarks, open them in either pane, rename, reorder or remove them. Removing a bookmark keeps its files.".into();
                dialog.fields.push(Field::text(
                    "bookmark-filter",
                    "Filter bookmarks",
                    &state.filter,
                ));
                dialog
                    .fields
                    .push(Field::text("bookmark-name", "Bookmark name", &state.name));
                if let Some(bookmark) = state.selected.and_then(|index| state.bookmarks.get(index))
                {
                    dialog
                        .fields
                        .push(Field::label("Location", bookmark.location.label()));
                }
                for (index, bookmark) in state.bookmarks.iter().enumerate() {
                    if !format!("{} {}", bookmark.name, bookmark.location.label())
                        .to_lowercase()
                        .contains(&state.filter.to_lowercase())
                    {
                        continue;
                    }
                    let mut row = Row::new(
                        format!("bookmark-item-{index}"),
                        &bookmark.name,
                        bookmark.location.label(),
                    );
                    row.selected = state.selected == Some(index);
                    dialog.rows.push(row);
                }
                let selected = state.selected.is_some();
                dialog.button("bookmark-refresh", "Refresh", true);
                dialog.button("bookmark-open", "Open bookmark", selected);
                dialog.button("bookmark-other", "Open in other pane", selected);
                dialog.button(
                    "bookmark-rename",
                    "Rename",
                    selected && !state.name.trim().is_empty(),
                );
                dialog.button(
                    "bookmark-add",
                    "Add current folder",
                    !state.name.trim().is_empty(),
                );
                dialog.button("bookmark-remove", "Remove bookmark", selected);
                dialog.button(
                    "bookmark-up",
                    "Move up",
                    state.filter.is_empty() && state.selected.is_some_and(|i| i > 0),
                );
                dialog.button(
                    "bookmark-down",
                    "Move down",
                    state.filter.is_empty()
                        && state
                            .selected
                            .is_some_and(|i| i + 1 < state.bookmarks.len()),
                );
            }
            "debug-console" => {
                dialog.description="Live application events from this process. Paths, credentials, message bodies and attachment contents are excluded. Up to 2000 events are retained in memory.".into();
                dialog.fields.push(Field::text(
                    "console-filter",
                    "Filter events",
                    &state.filter,
                ));
                dialog
                    .fields
                    .push(Field::check("console-pause", "Pause display", state.paused));
                let text = crate::debug_log::format(&state.events, &state.filter);
                dialog.fields.push(Field::label(
                    "Events",
                    if text.is_empty() {
                        "No matching events".into()
                    } else {
                        text
                    },
                ));
                dialog.button("console-copy", "Copy displayed events", true);
                dialog.button("console-clear", "Clear events", true);
            }
            "donate" => {
                dialog.description="Choose the project's donation page. Opening the page does not make a payment. The HTTPS address is saved in preferences.".into();
                dialog.fields.push(Field::text(
                    "donation-url",
                    "Donation page",
                    &state.donation,
                ));
                dialog.button(
                    "donation-open",
                    "Save and open donation page",
                    !state.donation.trim().is_empty(),
                );
            }
            "email" => self.email_dialog(&mut dialog),
            _ => {}
        }
        Some(dialog)
    }
    fn email_dialog(&self, dialog: &mut Dialog) {
        let Some(email) = &self.desktop_commands.email else {
            return;
        };
        dialog.busy = email.pending.is_some() || email.sending.is_some();
        dialog.buttons[0].2 = email.sending.is_none();
        dialog.description=if email.sending.is_some(){"Sending once over encrypted SMTP. Wait for the delivery result; closing or quitting is protected. An unconfirmed delivery is never retried automatically."}else{"Review recipients and the prepared file attachments before sending. Folders are flattened into regular files. Up to 100 files / 25 MiB; links are refused. SMTP passwords stay in memory."}.into();
        dialog.fields.push(Field::text(
            "mail-to",
            "To (comma-separated addresses)",
            &email.to,
        ));
        dialog
            .fields
            .push(Field::text("mail-subject", "Subject", &email.subject));
        dialog
            .fields
            .push(Field::multiline("mail-body", "Message", &email.body));
        dialog
            .fields
            .push(Field::label("From", &email.settings.sender));
        dialog.button(
            "mail-settings",
            if email.show_settings {
                "Hide SMTP settings"
            } else {
                "SMTP settings…"
            },
            !dialog.busy,
        );
        if email.show_settings {
            dialog.fields.push(Field::text(
                "mail-host",
                "SMTP hostname",
                &email.settings.host,
            ));
            dialog
                .fields
                .push(Field::text("mail-port", "SMTP port", email.settings.port));
            dialog.fields.push(Field::check(
                "mail-tls",
                "TLS on connection (otherwise required STARTTLS)",
                email.settings.implicit_tls,
            ));
            dialog.fields.push(Field::text(
                "mail-sender",
                "Sender email address",
                &email.settings.sender,
            ));
            dialog.fields.push(Field::text(
                "mail-user",
                "SMTP username (optional)",
                &email.settings.username,
            ));
        }
        if !email.settings.username.is_empty() {
            dialog.fields.push(Field::password(
                "mail-password",
                "SMTP password (not saved)",
                &email.password,
            ));
        }
        if let Some(prepared) = &email.prepared {
            let total: usize = email
                .selected
                .iter()
                .filter_map(|i| prepared.files.get(*i))
                .map(|file| file.bytes.len())
                .sum();
            dialog.fields.push(Field::label(
                "Attachments",
                format!(
                    "{} selected · {} bytes before MIME encoding",
                    email.selected.len(),
                    total
                ),
            ));
            for (index, file) in prepared.files.iter().enumerate() {
                let mut row = Row::new(
                    format!("mail-file-{index}"),
                    &file.name,
                    format!("{} bytes", file.bytes.len()),
                );
                row.selected = email.selected.contains(&index);
                dialog.rows.push(row);
            }
        }
        dialog.button(
            "mail-send",
            "Send email",
            !dialog.busy
                && !email.selected.is_empty()
                && !email.to.trim().is_empty()
                && (email.settings.username.is_empty() || !email.password.is_empty()),
        );
    }
    pub(super) fn desktop_commands_select(&mut self, id: &str) -> bool {
        let state = &mut self.desktop_commands;
        if !state.active() {
            return false;
        }
        if let Some(index) = id
            .strip_prefix("bookmark-item-")
            .and_then(|s| s.parse::<usize>().ok())
            && let Some(bookmark) = state.bookmarks.get(index)
        {
            state.selected = Some(index);
            state.name = bookmark.name.clone();
            state.error.clear();
        }
        if let Some(email) = &mut state.email
            && email.sending.is_none()
            && let Some(index) = id
                .strip_prefix("mail-file-")
                .and_then(|s| s.parse::<usize>().ok())
            && email
                .prepared
                .as_ref()
                .is_some_and(|p| index < p.files.len())
            && !email.selected.remove(&index)
        {
            email.selected.insert(index);
        }
        true
    }
    pub(super) fn desktop_commands_edit(&mut self, id: &str, value: &str) -> bool {
        let state = &mut self.desktop_commands;
        if !state.active() {
            return false;
        }
        if state.sending() {
            return true;
        }
        state.error.clear();
        match id {
            "bookmark-filter" => {
                state.filter = value.into();
                state.selected = None;
                state.name.clear();
            }
            "bookmark-name" => state.name = value.into(),
            "donation-url" => state.donation = value.into(),
            "browser-url" => state.browser_url = value.into(),
            "console-filter" => state.filter = value.into(),
            "console-pause" => state.paused = value == "true",
            _ => {
                if let Some(email) = &mut state.email {
                    match id {
                        "mail-to" => email.to = value.into(),
                        "mail-subject" => email.subject = value.into(),
                        "mail-body" => email.body = value.into(),
                        "mail-host" => email.settings.host = value.into(),
                        "mail-sender" => email.settings.sender = value.into(),
                        "mail-user" => email.settings.username = value.into(),
                        "mail-password" => email.password = value.into(),
                        "mail-tls" => {
                            email.settings.implicit_tls = value == "true";
                            if [0, 465, 587].contains(&email.settings.port) {
                                email.settings.port = if email.settings.implicit_tls {
                                    465
                                } else {
                                    587
                                };
                            }
                        }
                        "mail-port" => match value.parse::<u16>() {
                            Ok(port) if port > 0 => email.settings.port = port,
                            _ => {
                                email.settings.port = 0;
                                state.error = "Enter a port between 1 and 65535".into();
                            }
                        },
                        _ => {}
                    }
                }
            }
        }
        true
    }
    pub(super) fn desktop_commands_dialog_action(&mut self, id: &str) -> bool {
        if !self.desktop_commands.active() {
            return false;
        }
        let result = (|| -> anyhow::Result<()> {
            if self.desktop_commands.sending() {
                anyhow::bail!(
                    "Wait for the email delivery result before closing or changing this dialog"
                );
            }
            self.desktop_commands.error.clear();
            if id == "cancel" {
                self.desktop_commands = Default::default();
                return Ok(());
            }
            match id {
                "browser-open" | "accept" if self.desktop_commands.kind == Some("browser-url") => {
                    let value = self.desktop_commands.browser_url.trim().to_owned();
                    self.launch_browser(&value)?;
                    #[cfg(test)]
                    let opened_urls = std::mem::take(&mut self.desktop_commands.opened_urls);
                    self.desktop_commands = Default::default();
                    #[cfg(test)]
                    {
                        self.desktop_commands.opened_urls = opened_urls;
                    }
                }
                "console-copy" => self.ctx.copy_text(crate::debug_log::format(
                    &self.desktop_commands.events,
                    &self.desktop_commands.filter,
                )),
                "console-clear" => {
                    crate::debug_log::clear();
                    self.desktop_commands.events.clear();
                }
                "donation-open" => {
                    ensure!(
                        self.app.settings_valid,
                        "Repair the invalid configuration before saving a donation link"
                    );
                    let url = crate::support::web_link(self.desktop_commands.donation.trim())?;
                    let before = self.app.preferences.donation_url.clone();
                    self.app.preferences.donation_url = url.to_string();
                    if let Err(error) = self.save_desktop_settings() {
                        self.app.preferences.donation_url = before;
                        return Err(error);
                    }
                    self.launch_website(url.as_str())?;
                }
                "mail-settings" => {
                    if let Some(email) = &mut self.desktop_commands.email {
                        email.show_settings = !email.show_settings;
                    }
                }
                "mail-send" => self.send_reviewed_email()?,
                _ if id.starts_with("bookmark-") => self.bookmark_action(id)?,
                _ => {}
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.desktop_commands.error = format!("{error:#}");
        }
        true
    }
    fn send_reviewed_email(&mut self) -> anyhow::Result<()> {
        ensure!(
            self.app.settings_valid,
            "Repair the invalid configuration before saving SMTP settings"
        );
        let email = self
            .desktop_commands
            .email
            .as_mut()
            .context("Open Email files first")?;
        ensure!(
            email.pending.is_none(),
            "Wait for attachments to finish preparing"
        );
        email.settings.validate()?;
        ensure!(
            email.settings.username.is_empty() || !email.password.is_empty(),
            "Enter the SMTP password for this send"
        );
        let prepared = email.prepared.as_ref().context("No prepared attachments")?;
        let files = email
            .selected
            .iter()
            .map(|index| prepared.files[*index].clone())
            .collect::<Vec<_>>();
        let message = crate::mail::message(
            &email.settings,
            &email.to,
            &email.subject,
            &email.body,
            &files,
        )?;
        let settings = email.settings.clone();
        let before = self.app.preferences.mail.clone();
        self.app.preferences.mail = settings.clone();
        if let Err(error) = self.save_desktop_settings() {
            self.app.preferences.mail = before;
            return Err(error);
        }
        let email = self.desktop_commands.email.as_mut().unwrap();
        let password = std::mem::take(&mut email.password);
        let (sender, receiver) = mpsc::channel();
        email.sending = Some(receiver);
        let ctx = self.ctx.clone();
        crate::debug_log::record(
            "email",
            "User confirmed SMTP send with reviewed attachments",
        );
        std::thread::spawn(move || {
            let result = crate::mail::send(&settings, password, message)
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        Ok(())
    }
    fn bookmark_action(&mut self, id: &str) -> anyhow::Result<()> {
        if id == "bookmark-refresh" {
            self.desktop_commands.bookmarks = self.app.bookmarks.clone();
            self.desktop_commands.selected = None;
            self.desktop_commands.name.clear();
            return Ok(());
        }
        ensure!(
            serde_json::to_vec(&self.desktop_commands.bookmarks)?
                == serde_json::to_vec(&self.app.bookmarks)?,
            "Bookmarks changed in another window; refresh before editing"
        );
        let selected = self.desktop_commands.selected;
        if matches!(id, "bookmark-open" | "bookmark-other") {
            let location = self
                .desktop_commands
                .bookmarks
                .get(selected.context("Select a bookmark")?)
                .context("Bookmark missing")?
                .location
                .clone();
            let side = if id == "bookmark-other" {
                1 - self.app.active
            } else {
                self.app.active
            };
            self.desktop_commands = Default::default();
            self.app.panes[side].tab_mut().navigate(location, &self.ctx);
            return Ok(());
        }
        ensure!(
            self.app.settings_valid,
            "Repair the invalid configuration before saving bookmarks"
        );
        let mut bookmarks = self.app.bookmarks.clone();
        let mut next = selected;
        if matches!(id, "bookmark-add" | "bookmark-rename") {
            let name = self.desktop_commands.name.trim();
            ensure!(
                !name.is_empty() && name.len() <= 256 && !name.contains(['\r', '\n', '\0']),
                "Enter a bookmark name up to 256 bytes"
            );
            ensure!(
                !bookmarks
                    .iter()
                    .enumerate()
                    .any(|(i, b)| b.name.eq_ignore_ascii_case(name)
                        && (id == "bookmark-add" || Some(i) != selected)),
                "A bookmark with that name already exists"
            );
            if id == "bookmark-add" {
                ensure!(bookmarks.len() < 10000, "Bookmark limit reached");
                bookmarks.push(Bookmark {
                    name: name.into(),
                    location: self.app.tab().location.clone(),
                });
                next = Some(bookmarks.len() - 1);
            } else {
                bookmarks
                    .get_mut(selected.context("Select a bookmark")?)
                    .context("Bookmark missing")?
                    .name = name.into();
            }
        } else {
            let index = selected.context("Select a bookmark")?;
            ensure!(index < bookmarks.len(), "Bookmark missing");
            match id {
                "bookmark-remove" => {
                    bookmarks.remove(index);
                    next = None;
                }
                "bookmark-up" if index > 0 => {
                    bookmarks.swap(index, index - 1);
                    next = Some(index - 1);
                }
                "bookmark-down" if index + 1 < bookmarks.len() => {
                    bookmarks.swap(index, index + 1);
                    next = Some(index + 1);
                }
                _ => return Ok(()),
            }
        }
        let before = std::mem::replace(&mut self.app.bookmarks, bookmarks);
        if let Err(error) = self.save_desktop_settings() {
            self.app.bookmarks = before;
            return Err(error);
        }
        self.desktop_commands.bookmarks = self.app.bookmarks.clone();
        self.desktop_commands.selected = next;
        if let Some(index) = next {
            self.desktop_commands.name = self.app.bookmarks[index].name.clone();
        } else {
            self.desktop_commands.name.clear();
        }
        crate::debug_log::record("bookmarks", "Saved bookmark collection");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (
        tempfile::TempDir,
        CommanderWindow,
        Rc<RefCell<Adapter>>,
        PathBuf,
        PathBuf,
    ) {
        i_slint_backend_testing::init_no_event_loop();
        let (root, mut adapter, left, right) = super::super::tests::fixture();
        adapter.app.settings_valid = true;
        adapter.desktop_settings_path = Some(root.path().join("config/settings.json"));
        adapter.app.tab_mut().cursor = Some(left.join("alpha.txt"));
        let ui = CommanderWindow::new().unwrap();
        let state = Rc::new(RefCell::new(adapter));
        super::super::wire(&ui, state.clone());
        state.borrow_mut().sync(&ui);
        (root, ui, state, left, right)
    }
    fn wait_prepared(state: &Rc<RefCell<Adapter>>, ui: &CommanderWindow) {
        for _ in 0..400 {
            let mut adapter = state.borrow_mut();
            adapter.desktop_commands_tick();
            adapter.sync(ui);
            if adapter
                .desktop_commands
                .email
                .as_ref()
                .is_some_and(|email| email.pending.is_none())
            {
                return;
            }
            drop(adapter);
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("Attachment worker did not finish");
    }
    #[test]
    fn all_eight_commands_are_registered_and_browser_links_are_dispatched() {
        let (_root, ui, state, _, _) = setup();
        let menus = action_menus();
        assert_eq!(
            menus.iter().filter(|(title, _)| *title == "Help").count(),
            1
        );
        ui.invoke_menu("Help".into());
        for command in [
            "GoToDocumentation",
            "GoToForums",
            "GoToWebsite",
            "Donate",
            "ShowDebugConsole",
            "CheckForUpdates",
            "ReportBug",
        ] {
            assert!(
                ui.get_menu_items().iter().any(|item| item.id == command),
                "Help menu is missing {command}"
            );
        }
        for command in [
            "BringAllToFront",
            "MinimizeWindow",
            "MaximizeWindow",
            "OpenURLInBrowser",
            "Email",
            "ExploreBookmarks",
            "GoToDocumentation",
            "GoToForums",
            "GoToWebsite",
            "Donate",
            "ShowDebugConsole",
        ] {
            assert!(action_named(command).is_some(), "{command}");
            assert!(
                supported_commands().contains(&command.to_owned()),
                "{command}"
            );
            assert!(
                menus
                    .iter()
                    .flat_map(|(_, items)| items)
                    .any(|(_, action)| format!("{action:?}") == command)
            );
        }
        for command in ["GoToDocumentation", "GoToForums", "GoToWebsite"] {
            ui.invoke_action(command.into());
        }
        assert_eq!(
            state.borrow().desktop_commands.opened_urls,
            vec![
                crate::support::DOCUMENTATION,
                crate::support::COMMUNITY,
                crate::support::REPOSITORY
            ]
        );
        ui.invoke_action("BringAllToFront".into());
        assert!(matches!(
            state.borrow().extra.window_request,
            Some(super::super::windows::Request::RaiseAll)
        ));
    }
    #[test]
    fn browser_url_is_reviewed_and_window_commands_use_configured_shortcuts() {
        let (_root, ui, state, _, _) = setup();
        for (command, request) in [
            ("MinimizeWindow", "minimize"),
            ("MaximizeWindow", "maximize"),
        ] {
            state
                .borrow_mut()
                .app
                .preferences
                .shortcuts
                .insert(command.into(), "Command+Alt+M".into());
            let mut event = slint::language::KeyEvent::default();
            event.text = "m".into();
            event.modifiers.control = true;
            event.modifiers.alt = true;
            assert!(state.borrow_mut().global_key(&ui, event));
            let result = state.borrow_mut().extra.window_request.take();
            assert!(matches!(
                (&result, request),
                (Some(super::super::windows::Request::Minimize), "minimize")
                    | (Some(super::super::windows::Request::Maximize), "maximize")
            ));
            state.borrow_mut().app.preferences.shortcuts.remove(command);
        }
        ui.invoke_action("OpenURLInBrowser".into());
        assert_eq!(ui.get_dialog_title(), "Open URL in browser");
        ui.invoke_action("MinimizeWindow".into());
        assert!(matches!(
            state.borrow_mut().extra.window_request.take(),
            Some(super::super::windows::Request::Minimize)
        ));
        assert_eq!(state.borrow().desktop_commands.kind, Some("browser-url"));
        assert!(state.borrow().desktop_commands.opened_urls.is_empty());
        for value in [
            "file:///tmp/report",
            "javascript:alert(1)",
            "https://user:secret@example.test/",
        ] {
            ui.invoke_dialog_field("browser-url".into(), value.into());
            ui.invoke_dialog_action("browser-open".into());
            assert!(!ui.get_dialog_error().is_empty());
            assert!(state.borrow().desktop_commands.opened_urls.is_empty());
        }
        ui.invoke_dialog_field(
            "browser-url".into(),
            "http://example.test/report?q=hello%20world".into(),
        );
        ui.invoke_dialog_action("browser-open".into());
        assert!(!state.borrow().desktop_commands.active());
        assert_eq!(
            state.borrow().desktop_commands.opened_urls,
            vec!["http://example.test/report?q=hello%20world"]
        );
        let profile = crate::remote::Connection {
            service: "http".into(),
            endpoint: "https://example.test".into(),
            ..Default::default()
        };
        crate::remote::register_profiles(std::slice::from_ref(&profile));
        let mut selected = state.borrow().app.tab().entries[0].clone();
        selected.path = "docs/report.html".into();
        selected.name = "report.html".into();
        selected.is_dir = false;
        {
            let mut adapter = state.borrow_mut();
            adapter.app.tab_mut().location = Location::Remote {
                connection: profile.id,
                path: "docs".into(),
            };
            adapter.app.tab_mut().entries.clear();
            adapter.app.tab_mut().selected.clear();
            adapter.app.tab_mut().cursor = None;
        }
        ui.invoke_action("OpenURLInBrowser".into());
        assert_eq!(
            state.borrow().desktop_commands.browser_url,
            "https://example.test/docs"
        );
        ui.invoke_dialog_action("cancel".into());
        {
            let mut adapter = state.borrow_mut();
            adapter.app.tab_mut().cursor = Some(selected.path.clone());
            adapter.app.tab_mut().entries = vec![selected];
        }
        ui.invoke_action("OpenURLInBrowser".into());
        assert_eq!(
            state.borrow().desktop_commands.browser_url,
            "https://example.test/docs/report.html"
        );
    }
    #[test]
    fn donation_link_is_validated_saved_and_opened_only_after_review() {
        let (root, ui, state, _, _) = setup();
        ui.invoke_action("Donate".into());
        assert!(state.borrow().desktop_commands.opened_urls.is_empty());
        ui.invoke_dialog_field("donation-url".into(), "javascript:alert(1)".into());
        ui.invoke_dialog_action("donation-open".into());
        assert!(!ui.get_dialog_error().is_empty());
        assert!(!root.path().join("config/settings.json").exists());
        ui.invoke_dialog_field("donation-url".into(), "https://example.test/support".into());
        ui.invoke_dialog_action("donation-open".into());
        assert_eq!(
            state.borrow().desktop_commands.opened_urls,
            vec!["https://example.test/support"]
        );
        let saved: Settings = serde_json::from_slice(
            &std::fs::read(root.path().join("config/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            saved.preferences.donation_url,
            "https://example.test/support"
        );
        ui.invoke_dialog_action("cancel".into());
        assert!(!state.borrow().desktop_commands.active());
    }
    #[test]
    fn bookmark_explorer_edits_reorders_and_opens_locations_without_touching_files() {
        let (root, ui, state, left, right) = setup();
        state.borrow_mut().app.bookmarks = vec![
            Bookmark {
                name: "Left".into(),
                location: Location::Local(left.clone()),
            },
            Bookmark {
                name: "Right".into(),
                location: Location::Local(right),
            },
        ];
        ui.invoke_action("ExploreBookmarks".into());
        ui.invoke_dialog_select("bookmark-item-0".into());
        ui.invoke_dialog_field("bookmark-name".into(), "Projects".into());
        ui.invoke_dialog_action("bookmark-rename".into());
        ui.invoke_dialog_action("bookmark-down".into());
        assert_eq!(state.borrow().app.bookmarks[1].name, "Projects");
        ui.invoke_dialog_action("bookmark-remove".into());
        assert_eq!(state.borrow().app.bookmarks.len(), 1);
        ui.invoke_dialog_field("bookmark-name".into(), "Current".into());
        ui.invoke_dialog_action("bookmark-add".into());
        assert_eq!(
            state.borrow().app.bookmarks[1].location,
            Location::Local(left.clone())
        );
        let saved: Settings = serde_json::from_slice(
            &std::fs::read(root.path().join("config/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(saved.bookmarks.len(), 2);
        ui.invoke_dialog_action("bookmark-other".into());
        assert_eq!(
            state.borrow().app.panes[1].tab().location,
            Location::Local(left.clone())
        );
        assert_eq!(std::fs::read(left.join("alpha.txt")).unwrap(), b"alpha");
        assert!(!state.borrow().desktop_commands.active());
    }
    #[test]
    fn bookmark_explorer_refuses_stale_collections_and_rolls_back_failed_saves() {
        let (root, ui, state, left, _) = setup();
        state.borrow_mut().app.bookmarks = vec![Bookmark {
            name: "Saved".into(),
            location: Location::Local(left),
        }];
        ui.invoke_action("ExploreBookmarks".into());
        ui.invoke_dialog_select("bookmark-item-0".into());
        state.borrow_mut().app.bookmarks[0].name = "Changed elsewhere".into();
        ui.invoke_dialog_field("bookmark-name".into(), "Overwrite".into());
        ui.invoke_dialog_action("bookmark-rename".into());
        assert!(ui.get_dialog_error().contains("changed in another window"));
        assert_eq!(state.borrow().app.bookmarks[0].name, "Changed elsewhere");
        ui.invoke_dialog_action("bookmark-refresh".into());
        ui.invoke_dialog_select("bookmark-item-0".into());
        std::fs::write(root.path().join("not-a-directory"), b"preserve").unwrap();
        state.borrow_mut().desktop_settings_path =
            Some(root.path().join("not-a-directory/settings.json"));
        ui.invoke_dialog_field("bookmark-name".into(), "Cannot save".into());
        ui.invoke_dialog_action("bookmark-rename".into());
        assert!(!ui.get_dialog_error().is_empty());
        assert_eq!(state.borrow().app.bookmarks[0].name, "Changed elsewhere");
        state.borrow_mut().app.settings_valid = false;
        ui.invoke_dialog_action("bookmark-remove".into());
        assert_eq!(state.borrow().app.bookmarks.len(), 1);
    }
    #[test]
    fn console_filters_pauses_and_copies_application_events_without_draft_text() {
        let (_root, ui, state, _, _) = setup();
        let marker = format!("test-event-{:x}", rand::random::<u64>());
        crate::debug_log::record("test", &marker);
        ui.invoke_action("ShowDebugConsole".into());
        ui.invoke_dialog_field("console-filter".into(), marker.clone().into());
        ui.invoke_dialog_field("console-pause".into(), "true".into());
        crate::debug_log::record("test", &format!("{marker}-later"));
        state.borrow_mut().desktop_commands_tick();
        ui.invoke_dialog_action("console-copy".into());
        let (copies, _) = state.borrow().ctx.take_events();
        assert_eq!(copies.len(), 1);
        assert!(copies[0].contains(&marker));
        assert!(!copies[0].contains("-later"));
        ui.invoke_dialog_field("console-pause".into(), "false".into());
        state.borrow_mut().desktop_commands_tick();
        ui.invoke_dialog_action("console-copy".into());
        assert!(state.borrow().ctx.take_events().0[0].contains("-later"));
        ui.invoke_dialog_action("console-clear".into());
        assert!(state.borrow().desktop_commands.events.is_empty());
    }
    #[test]
    fn email_reviews_marked_files_protects_pending_delivery_and_never_queues_a_send() {
        let (_root, ui, state, left, _) = setup();
        state
            .borrow_mut()
            .app
            .tab_mut()
            .selected
            .extend([left.join("alpha.txt"), left.join("café.txt")]);
        ui.invoke_action("Email".into());
        wait_prepared(&state, &ui);
        assert_eq!(ui.get_dialog_rows().row_count(), 2);
        ui.invoke_dialog_select("mail-file-0".into());
        assert_eq!(
            state
                .borrow()
                .desktop_commands
                .email
                .as_ref()
                .unwrap()
                .selected
                .len(),
            1
        );
        for (id, value) in [
            ("mail-host", "smtp.example.test"),
            ("mail-sender", "sender@example.test"),
            ("mail-user", "sender"),
            ("mail-password", "private-password-marker"),
            ("mail-to", "not an address"),
            ("mail-body", "private-body-marker"),
        ] {
            ui.invoke_dialog_field(id.into(), value.into());
        }
        ui.invoke_dialog_action("mail-send".into());
        assert!(!ui.get_dialog_error().is_empty());
        assert!(!state.borrow().desktop_commands.sending());
        let (sender, receiver) = mpsc::channel();
        state
            .borrow_mut()
            .desktop_commands
            .email
            .as_mut()
            .unwrap()
            .sending = Some(receiver);
        state.borrow_mut().request_close();
        assert!(!state.borrow().app.exiting);
        ui.invoke_dialog_action("cancel".into());
        assert!(state.borrow().desktop_commands.active());
        sender.send(Ok(())).unwrap();
        state.borrow_mut().desktop_commands_tick();
        assert!(!state.borrow().desktop_commands.active());
        assert!(state.borrow().app.queue.is_empty());
        let logs = crate::debug_log::format(&crate::debug_log::snapshot(), "");
        assert!(!logs.contains("private-password-marker") && !logs.contains("private-body-marker"));
        assert_eq!(state.borrow().app.tab().selected.len(), 2);
    }
}
