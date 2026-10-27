use super::{
    dialogs::{Dialog, Field, Row},
    *,
};
use anyhow::Context;

#[derive(Default)]
pub(super) struct State {
    recovery: Option<RecoveryBrowser>,
    support: Option<&'static str>,
    update: Option<crate::support::Update>,
    pending: Option<Receiver<Result<crate::support::Update, String>>>,
    error: String,
}
impl State {
    pub(super) fn active(&self) -> bool {
        self.support.is_some() || self.recovery.is_some()
    }
}
impl Adapter {
    pub(super) fn management_tick(&mut self) {
        if let Some(browser) = &mut self.management.recovery
            && let Some(result) = crate::background::poll(&mut browser.pending)
        {
            match result {
                Ok(inventory) => {
                    browser.inventory = inventory;
                    browser.selected.clear();
                    browser.confirmation.clear();
                }
                Err(error) => browser.error = error,
            }
        }
        if let Some(result) = crate::background::poll(&mut self.management.pending) {
            match result {
                Ok(update) => self.management.update = Some(update),
                Err(error) => self.management.error = error,
            }
        }
    }
    fn check_updates(&mut self) {
        self.management.update = None;
        self.management.error.clear();
        let (sender, receiver) = mpsc::channel();
        self.management.pending = Some(receiver);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let result = crate::support::check_updates().map_err(|e| format!("{e:#}"));
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }
    pub(super) fn management_action(&mut self, name: &str) -> bool {
        match name {
            "Recovery" => {
                let mut browser = RecoveryBrowser::default();
                browser.refresh(&self.ctx);
                self.management.recovery = Some(browser);
            }
            "CheckForUpdates" => {
                self.management.support = Some("updates");
                self.check_updates();
            }
            "Diagnostics" | "ReportBug" => {
                self.management.support = Some(if name == "ReportBug" {
                    "report"
                } else {
                    "diagnostics"
                });
                self.management.error.clear();
            }
            _ => return false,
        }
        true
    }
    pub(super) fn management_dialog(&self) -> Option<Dialog> {
        if let Some(browser) = &self.management.recovery {
            return Some(self.recovery_browser_dialog(browser));
        }
        let kind = self.management.support?;
        let mut dialog = Dialog::new(
            kind,
            match kind {
                "updates" => "Check for updates",
                "report" => "Report a bug",
                _ => "Diagnostics",
            },
        );
        dialog.error = self.management.error.clone();
        if kind == "updates" {
            dialog.busy = self.management.pending.is_some();
            dialog.description =
                "Checks stable GitHub releases on request. Download and installation are manual."
                    .into();
            dialog
                .fields
                .push(Field::label("Installed version", env!("CARGO_PKG_VERSION")));
            if let Some(update) = &self.management.update {
                if let Some(release) = &update.release {
                    dialog
                        .fields
                        .push(Field::label("Latest stable release", &release.tag_name));
                    dialog.fields.push(Field::label(
                        "Result",
                        if update.newer {
                            "A newer version is available."
                        } else {
                            "No newer stable release is available."
                        },
                    ));
                    dialog.button("release-page", "Open release page", true);
                } else {
                    dialog.fields.push(Field::label(
                        "Result",
                        "No stable release has been published yet.",
                    ));
                }
            }
            dialog.button("update-check", "Check again", !dialog.busy);
        } else {
            dialog.description = "This diagnostic summary contains version, platform and build information. It excludes file paths, connection profiles, credentials and operation history. Review your report in the browser before submitting.".into();
            dialog.fields.push(Field::label(
                "Diagnostic summary",
                crate::support::diagnostics(),
            ));
            dialog.button("copy-diagnostics", "Copy diagnostics", true);
            dialog.button("report-browser", "Open bug report in browser", true);
        }
        Some(dialog)
    }
    pub(super) fn management_dialog_action(&mut self, id: &str) -> bool {
        if self.management.recovery.is_some() {
            return self.recovery_browser_action(id);
        }
        if self.management.support.is_none() {
            return false;
        }
        let result = (|| -> anyhow::Result<()> {
            match id {
                "cancel" => {
                    self.management.support = None;
                    self.management.pending = None;
                }
                "update-check" if self.management.pending.is_none() => self.check_updates(),
                "copy-diagnostics" => self.ctx.copy_text(crate::support::diagnostics()),
                "report-browser" => {
                    open::that(crate::support::bug_report_url())?;
                }
                "release-page" => {
                    if let Some(release) = self
                        .management
                        .update
                        .as_ref()
                        .and_then(|update| update.release.as_ref())
                    {
                        open::that(&release.html_url)?;
                    }
                }
                _ => {}
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.management.error = format!("{error:#}");
        }
        true
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
        let (root, adapter, left, right) = super::super::tests::fixture();
        let ui = CommanderWindow::new().unwrap();
        let state = Rc::new(RefCell::new(adapter));
        super::super::wire(&ui, state.clone());
        state.borrow_mut().sync(&ui);
        (root, ui, state, left, right)
    }
    #[test]
    fn convenience_commands_preserve_marks_and_dispatch_all_numbered_windows() {
        let (_root, ui, state, left, _right) = setup();
        state
            .borrow_mut()
            .app
            .tab_mut()
            .selected
            .insert(left.join("alpha.txt"));
        state.borrow_mut().sync(&ui);
        ui.invoke_action("CopyBaseNames".into());
        assert_eq!(state.borrow().ctx.take_events().0, vec!["alpha"]);
        let count = ui.get_left_rows().row_count();
        assert!(ui.get_left_parent_visible());
        ui.invoke_action("ToggleParentEntry".into());
        assert!(!ui.get_left_parent_visible());
        assert_eq!(ui.get_left_rows().row_count(), count);
        assert!(
            state
                .borrow()
                .app
                .tab()
                .selected
                .contains(&left.join("alpha.txt"))
        );
        for index in 1..=10 {
            let mut event = slint::language::KeyEvent::default();
            event.text = format!("{}", index % 10).into();
            event.modifiers.control = true;
            event.modifiers.alt = true;
            assert!(state.borrow_mut().global_key(&ui, event));
            assert!(
                matches!(state.borrow_mut().extra.window_request.take(), Some(windows::Request::RaiseOrdinal(ordinal)) if ordinal == index - 1)
            );
        }
    }
    #[test]
    fn diagnostic_and_report_dialogs_copy_only_platform_information() {
        let (_root, ui, state, left, _right) = setup();
        ui.invoke_action("Diagnostics".into());
        assert_eq!(ui.get_dialog_title(), "Diagnostics");
        ui.invoke_dialog_action("copy-diagnostics".into());
        let text = state.borrow().ctx.take_events().0;
        assert_eq!(text, vec![crate::support::diagnostics()]);
        assert!(!text[0].contains(left.to_str().unwrap()));
        ui.invoke_dialog_action("cancel".into());
        ui.invoke_action("ReportBug".into());
        assert_eq!(ui.get_dialog_title(), "Report a bug");
        assert!(
            ui.get_dialog_buttons()
                .iter()
                .any(|b| b.id == "report-browser")
        );
        assert!(state.borrow().app.job.is_none());
        let mut event = slint::language::KeyEvent::default();
        event.text = slint::platform::Key::Escape.into();
        assert!(state.borrow_mut().global_key(&ui, event));
        assert!(!state.borrow().management.active());
    }
    #[test]
    fn update_results_and_errors_reach_the_dialog_without_blocking_the_event_loop() {
        let (_root, ui, state, _left, _right) = setup();
        let (sender, receiver) = mpsc::channel();
        {
            let mut adapter = state.borrow_mut();
            adapter.management.support = Some("updates");
            adapter.management.pending = Some(receiver);
            adapter.sync(&ui);
        }
        assert!(ui.get_dialog_busy());
        assert!(
            !ui.get_dialog_buttons()
                .iter()
                .find(|b| b.id == "update-check")
                .unwrap()
                .enabled
        );
        let release = serde_json::json!({"tag_name":"v99.0.0","html_url":"https://github.com/ghillert/rust-commander/releases/tag/v99.0.0"});
        sender
            .send(Ok(crate::support::parse_release(
                &serde_json::to_vec(&release).unwrap(),
                env!("CARGO_PKG_VERSION"),
            )
            .unwrap()))
            .unwrap();
        {
            let mut adapter = state.borrow_mut();
            adapter.management_tick();
            adapter.sync(&ui);
        }
        assert!(!ui.get_dialog_busy());
        assert!(
            ui.get_dialog_buttons()
                .iter()
                .any(|b| b.id == "release-page" && b.enabled)
        );
        let (sender, receiver) = mpsc::channel();
        state.borrow_mut().management.pending = Some(receiver);
        sender.send(Err("offline fixture".into())).unwrap();
        {
            let mut adapter = state.borrow_mut();
            adapter.management_tick();
            adapter.sync(&ui);
        }
        assert!(ui.get_dialog_error().contains("offline fixture"));
        ui.invoke_dialog_action("cancel".into());
        assert!(!state.borrow().management.active());
    }
    #[test]
    fn recovery_selection_filter_and_cleanup_confirmation_use_the_real_callbacks() {
        let (_root, ui, state, left, right) = setup();
        let item = crate::recovery::Item {
            key: crate::recovery::Key::Registered("fixture".into()),
            label: "Reviewed original".into(),
            source: Location::Local(left.clone()),
            path: left.join(".rust-commander-edit-fixture.bak"),
            destination: Location::Local(right),
            name: "alpha.txt".into(),
            stamp: "fixture".into(),
            cleanup: true,
            cleanup_group: "fixture".into(),
            cleanup_scope: "Delete one fixture backup".into(),
        };
        let mut browser = RecoveryBrowser::default();
        browser.inventory = crate::recovery::Inventory {
            items: vec![item],
            warnings: Vec::new(),
        };
        state.borrow_mut().management.recovery = Some(browser);
        state.borrow_mut().sync(&ui);
        assert_eq!(ui.get_dialog_title(), "Recovery manager");
        ui.invoke_dialog_select("recovery-item-0".into());
        assert!(ui.get_dialog_rows().row_data(0).unwrap().selected);
        assert!(
            ui.get_dialog_buttons()
                .iter()
                .find(|b| b.id == "recovery-restore")
                .unwrap()
                .enabled
        );
        assert!(
            !ui.get_dialog_buttons()
                .iter()
                .find(|b| b.id == "recovery-clean")
                .unwrap()
                .enabled
        );
        ui.invoke_dialog_action("recovery-clean".into());
        assert!(state.borrow().app.job.is_none());
        assert!(!ui.get_dialog_error().is_empty());
        ui.invoke_dialog_field("recovery-confirm".into(), "DELETE".into());
        assert!(
            ui.get_dialog_buttons()
                .iter()
                .find(|b| b.id == "recovery-clean")
                .unwrap()
                .enabled
        );
        ui.invoke_dialog_field("recovery-filter".into(), "no match".into());
        assert_eq!(ui.get_dialog_rows().row_count(), 0);
        assert!(
            state
                .borrow()
                .management
                .recovery
                .as_ref()
                .unwrap()
                .selected
                .is_empty()
        );
        assert!(
            !ui.get_dialog_buttons()
                .iter()
                .find(|b| b.id == "recovery-clean")
                .unwrap()
                .enabled
        );
        ui.invoke_dialog_action("cancel".into());
        assert!(!state.borrow().management.active());
    }
}

#[derive(Default)]
struct RecoveryBrowser {
    inventory: crate::recovery::Inventory,
    selected: std::collections::BTreeSet<usize>,
    pending: Option<Receiver<Result<crate::recovery::Inventory, String>>>,
    cancel: Arc<AtomicBool>,
    filter: String,
    destination: String,
    confirmation: String,
    import_folder: String,
    error: String,
}
impl Drop for RecoveryBrowser {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl RecoveryBrowser {
    fn refresh(&mut self, ctx: &UiSignal) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.error.clear();
        let cancel = self.cancel.clone();
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = crate::recovery::inventory(&Task::new(cancel, |_| {}))
                .map_err(|e| format!("{e:#}"));
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }
}
impl Adapter {
    fn recovery_browser_dialog(&self, browser: &RecoveryBrowser) -> Dialog {
        let mut dialog = Dialog::new("recovery-browser", "Recovery manager");
        dialog.buttons.clear();
        let idle = self.app.job.is_none() && self.app.queue.is_empty();
        dialog.busy = browser.pending.is_some();
        dialog.error = browser.error.clone();
        dialog.description = "Select retained originals to restore. Click a selected row again to unselect it. Restore keeps current files, choosing a numbered name on conflict, and retains the backup. Cleanup permanently removes the reviewed backup group.".into();
        dialog.fields.push(Field::text(
            "recovery-filter",
            "Filter originals",
            &browser.filter,
        ));
        dialog.fields.push(Field::text(
            "recovery-import",
            "Older replacement recovery folder (optional)",
            &browser.import_folder,
        ));
        dialog.fields.push(Field::text(
            "recovery-destination",
            "Restore into folder",
            &browser.destination,
        ));
        let filter = browser.filter.to_lowercase();
        for (index, item) in browser.inventory.items.iter().enumerate() {
            let detail = format!(
                "{} · {} · {}{}",
                item.label,
                crate::directory_tree::at(&item.source, &item.path).label(),
                item.cleanup_scope,
                if item.cleanup {
                    ""
                } else {
                    " · protected or incomplete"
                }
            );
            if !filter.is_empty()
                && !format!("{} {detail}", item.name)
                    .to_lowercase()
                    .contains(&filter)
            {
                continue;
            }
            let mut row = Row::new(format!("recovery-item-{index}"), &item.name, detail);
            row.selected = browser.selected.contains(&index);
            dialog.rows.push(row);
        }
        if !browser.inventory.warnings.is_empty() {
            dialog.fields.push(Field::label(
                "Items needing review",
                browser.inventory.warnings.join("\n"),
            ));
        }
        let selected = browser
            .selected
            .iter()
            .filter_map(|index| browser.inventory.items.get(*index))
            .collect::<Vec<_>>();
        if !selected.is_empty() {
            let groups = selected
                .iter()
                .map(|item| &item.cleanup_group)
                .collect::<std::collections::BTreeSet<_>>();
            dialog.fields.push(Field::label("Cleanup scope", format!("{} selected originals in {} backup group(s). Cleaning an Undo/Redo group expires the whole operation chain; checkpoint cleanup removes the whole finished copy. Current destination files are kept.", selected.len(), groups.len())));
            dialog.fields.push(Field::text(
                "recovery-confirm",
                "Type DELETE to confirm permanent cleanup",
                &browser.confirmation,
            ));
        }
        dialog.button("recovery-refresh", "Refresh originals", !dialog.busy);
        dialog.button(
            "recovery-import-folder",
            "Import older recovery folder",
            !dialog.busy && !browser.import_folder.trim().is_empty(),
        );
        dialog.button("recovery-retention", "Retention settings…", !dialog.busy);
        dialog.button(
            "recovery-browse",
            "Browse backup folder",
            selected.len() == 1 && !dialog.busy,
        );
        dialog.button("cancel", "Close", true);
        dialog.button(
            "recovery-restore",
            "Restore selected copies",
            idle && !dialog.busy
                && !selected.is_empty()
                && selected.len() <= 100
                && !browser.destination.trim().is_empty(),
        );
        dialog.button(
            "recovery-clean",
            "Delete reviewed backup groups",
            idle && !dialog.busy
                && !selected.is_empty()
                && selected.iter().all(|item| item.cleanup)
                && browser.confirmation == "DELETE",
        );
        dialog
    }
    pub(super) fn management_select(&mut self, id: &str) -> bool {
        let Some(browser) = &mut self.management.recovery else {
            return false;
        };
        if browser.pending.is_some() {
            return true;
        }
        if let Some(index) = id
            .strip_prefix("recovery-item-")
            .and_then(|s| s.parse::<usize>().ok())
            && let Some(item) = browser.inventory.items.get(index)
        {
            if !browser.selected.remove(&index) {
                browser.selected.insert(index);
            }
            if browser.selected.len() == 1 {
                browser.destination = item.destination.label();
            }
            browser.confirmation.clear();
        }
        true
    }
    pub(super) fn management_edit(&mut self, id: &str, value: &str) -> bool {
        let Some(browser) = &mut self.management.recovery else {
            return false;
        };
        match id {
            "recovery-filter" => {
                browser.filter = value.into();
                browser.selected.clear();
                browser.confirmation.clear();
            }
            "recovery-import" => browser.import_folder = value.into(),
            "recovery-destination" => browser.destination = value.into(),
            "recovery-confirm" => browser.confirmation = value.into(),
            _ => {}
        }
        true
    }
    fn recovery_browser_action(&mut self, id: &str) -> bool {
        let result = (|| -> anyhow::Result<()> {
            if id == "cancel" {
                self.management.recovery = None;
                return Ok(());
            }
            let browser = self.management.recovery.as_mut().unwrap();
            if id == "recovery-refresh" && browser.pending.is_none() {
                browser.refresh(&self.ctx);
                return Ok(());
            }
            if id == "recovery-import-folder" && browser.pending.is_none() {
                let folder = PathBuf::from(&browser.import_folder);
                browser.cancel = Arc::new(AtomicBool::new(false));
                let cancel = browser.cancel.clone();
                let (sender, receiver) = mpsc::channel();
                browser.pending = Some(receiver);
                let ctx = self.ctx.clone();
                std::thread::spawn(move || {
                    let task = Task::new(cancel, |_| {});
                    let result = crate::recovery::import_folder(&folder, &task)
                        .and_then(|_| crate::recovery::inventory(&task))
                        .map_err(|e| format!("{e:#}"));
                    let _ = sender.send(result);
                    ctx.request_repaint();
                });
                return Ok(());
            }
            if id == "recovery-retention" && browser.pending.is_none() {
                self.management.recovery = None;
                self.app.features.recovery(&self.ctx);
                return Ok(());
            }
            anyhow::ensure!(browser.pending.is_none(), "Wait for recovery discovery");
            let selected = browser
                .selected
                .iter()
                .filter_map(|index| browser.inventory.items.get(*index))
                .cloned()
                .collect::<Vec<_>>();
            if id == "recovery-browse" {
                anyhow::ensure!(selected.len() == 1, "Select one recovery item");
                let item = &selected[0];
                let path = item.path.parent().context("Missing recovery folder")?;
                let location = crate::directory_tree::at(&item.source, path);
                self.management.recovery = None;
                self.app.panes[self.app.active]
                    .tab_mut()
                    .navigate(location, &self.ctx);
                return Ok(());
            }
            if id == "recovery-restore" || id == "recovery-clean" {
                anyhow::ensure!(
                    self.app.job.is_none() && self.app.queue.is_empty(),
                    "Finish queued work before changing recovery files"
                );
                anyhow::ensure!(
                    !selected.is_empty() && selected.len() <= 100,
                    "Select between 1 and 100 originals"
                );
                let mut operations = Vec::new();
                if id == "recovery-restore" {
                    let destination = selected
                        .iter()
                        .map(|item| &item.destination)
                        .chain(self.app.panes.iter().map(|pane| &pane.tab().location))
                        .find(|location| location.label() == browser.destination)
                        .cloned()
                        .map(Ok)
                        .unwrap_or_else(|| {
                            filesystem::parse_location(
                                &browser.destination,
                                &self.app.tab().location,
                            )
                        })?;
                    operations.push(Operation::RecoveryRestore {
                        items: selected,
                        destination,
                    });
                } else {
                    anyhow::ensure!(
                        browser.confirmation == "DELETE"
                            && selected.iter().all(|item| item.cleanup),
                        "Review eligible backup groups and type DELETE first"
                    );
                    operations.push(Operation::RecoveryCleanup(selected));
                }
                self.management.recovery = None;
                for operation in operations {
                    self.app.start_job(operation, &self.ctx);
                }
            }
            Ok(())
        })();
        if let Err(error) = result
            && let Some(browser) = &mut self.management.recovery
        {
            browser.error = format!("{error:#}");
        }
        true
    }
}
