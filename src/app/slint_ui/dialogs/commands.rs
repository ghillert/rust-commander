use super::*;

impl Adapter {
    pub(in super::super) fn dialog_select(&mut self, id: &str) {
        if self.desktop_commands_select(id) {
            return;
        }
        if self.management_select(id) {
            return;
        }
        if self.parity_select(id) {
            return;
        }
        if let Some(index) = id
            .strip_prefix("workspace-")
            .and_then(|s| s.parse::<usize>().ok())
            && let Some(workspace) = self.app.workspaces.get(index)
        {
            self.app.features.workspace_selected = Some(index);
            self.app.features.workspace_name = workspace.name.clone();
        } else if let Some(index) = id
            .strip_prefix("search-")
            .and_then(|s| s.parse::<usize>().ok())
        {
            self.dialog_action(&format!("search-open-{index}"));
        }
    }

    pub(in super::super) fn dialog_action(&mut self, id: &str) {
        if self.desktop_commands_dialog_action(id) {
            return;
        }
        if self.management_dialog_action(id) {
            return;
        }
        if self.enhancements_dialog_action(id) {
            return;
        }
        if self.parity_dialog_action(id) {
            return;
        }
        if id == "cancel" {
            self.close_dialog();
            return;
        }
        if matches!(
            id,
            "accept" | "authorize" | "sync-preview" | "recovery-preview" | "expire-backups"
        ) && !self.invalid_fields.is_empty()
        {
            return;
        }
        self.field_error = None;
        let result = (|| -> anyhow::Result<()> {
            if matches!(self.dialog_kind.as_str(), "comparison" | "sync")
                && id.starts_with("folder-preset-")
            {
                anyhow::ensure!(
                    self.invalid_fields.is_empty(),
                    "Correct invalid fields first"
                );
                self.folder_preset_action(id)?;
                return Ok(());
            }
            let idle = self.app.job.is_none() && self.app.queue.is_empty();
            match self.dialog_kind.as_str() {
                "exit" if id == "accept" => {
                    if self.extra.quit_all {
                        self.extra.window_request =
                            Some(super::super::windows::Request::ConfirmQuit);
                        self.extra.quit_all = false;
                        self.app.confirm_exit = false;
                        return Ok(());
                    }
                    if let Some(job) = &self.app.job
                        && job.owner == self.app.window_id
                    {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                    if let Some(preparation) = &self.drag.preparation {
                        preparation.cancel.store(true, Ordering::Relaxed);
                    }
                    self.app.editor = None;
                    self.app.confirm_exit = false;
                    self.app.exiting = true;
                }
                "message" if id == "copy-result" => {
                    if let Some(message) = &self.app.message {
                        self.app.native_files.copy_text(message.clone())?;
                    }
                }
                "pack-progress" => {
                    if let Some(job) = &mut self.app.job {
                        match id {
                            "pack-background" => job.foreground = false,
                            "cancel-pack" => job.cancel.store(true, Ordering::Relaxed),
                            _ => {}
                        }
                    }
                }
                "prompt" | "pack" => match id {
                    "pack-files" => {
                        if let Some(Prompt {
                            kind: PromptKind::Pack { options, .. },
                            ..
                        }) = &mut self.app.prompt
                        {
                            options.expanded = !options.expanded;
                        }
                    }
                    "accept" => {
                        self.app.poll_prompt(&self.ctx);
                        if self
                            .app
                            .prompt
                            .as_ref()
                            .is_some_and(|p| p.pending_conflicts.is_none())
                        {
                            self.app.accept_prompt(&self.ctx);
                        }
                    }
                    "apply-all" => {
                        if let Some(prompt) = &mut self.app.prompt {
                            prompt.overrides.clear();
                        }
                    }
                    _ => {}
                },
                "preferences" => {
                    if id == "defaults" {
                        self.app.preferences_form = Some(Default::default());
                        self.invalid_fields.clear();
                    } else if id == "accept" {
                        let Some(preferences) = self.app.preferences_form.as_ref() else {
                            return Ok(());
                        };
                        crate::preferences::validate(preferences)?;
                        self.app.preferences = preferences.clone();
                        self.app.preferences_form = None;
                        self.app.preferences_error = None;
                        crate::i18n::set(self.app.preferences.language);
                        #[cfg(target_os = "macos")]
                        if let Some(menu) = &self.app.native_menu {
                            menu.localize();
                        }
                        self.app.refresh(&self.ctx);
                        self.app.save_settings();
                    } else if let Some(index) = id
                        .strip_prefix("column-up-")
                        .and_then(|s| s.parse::<usize>().ok())
                        && let Some(p) = &mut self.app.preferences_form
                    {
                        if index > 1 && index < p.columns.len() {
                            p.columns.swap(index, index - 1);
                        }
                    } else if let Some(index) = id
                        .strip_prefix("column-down-")
                        .and_then(|s| s.parse::<usize>().ok())
                        && let Some(p) = &mut self.app.preferences_form
                    {
                        if index > 0 && index + 1 < p.columns.len() {
                            p.columns.swap(index, index + 1);
                        }
                    } else if let Some(index) = id
                        .strip_prefix("toolbar-up-")
                        .and_then(|s| s.parse::<usize>().ok())
                        && let Some(p) = &mut self.app.preferences_form
                    {
                        if index > 0 && index < p.toolbar_actions.len() {
                            p.toolbar_actions.swap(index, index - 1);
                        }
                    } else if let Some(index) = id
                        .strip_prefix("toolbar-down-")
                        .and_then(|s| s.parse::<usize>().ok())
                        && let Some(p) = &mut self.app.preferences_form
                        && index + 1 < p.toolbar_actions.len()
                    {
                        p.toolbar_actions.swap(index, index + 1);
                    }
                }
                "connections" => {
                    if id == "accept"
                        && self
                            .app
                            .connection_form
                            .as_ref()
                            .is_some_and(|f| f.save_only)
                    {
                        let mut connection = self
                            .app
                            .connection_form
                            .as_ref()
                            .unwrap()
                            .connection
                            .clone();
                        crate::remote::validate(&connection)?;
                        connection.listed = true;
                        self.app.connections.retain(|c| c.id != connection.id);
                        self.app.connections.push(connection.clone());
                        crate::remote::register_profiles(&[connection]);
                        self.app.save_settings();
                        self.app.connection_form = None;
                    } else if let Some(index) = id
                        .strip_prefix("uri-profile-")
                        .and_then(|s| s.parse::<usize>().ok())
                    {
                        if let Some(mut form) = self.app.connection_form.take() {
                            if let Some(mut c) = form.choices.get(index).cloned() {
                                if let Some(ticket) = &mut form.navigation {
                                    ticket.path =
                                        crate::uri::relative_to_profile(&ticket.path, &c)?;
                                }
                                if !form.secrets.request_query.is_empty()
                                    || (!form.secrets.password.is_empty() && c.service == "sftp")
                                {
                                    c.credentials_from = c.id.clone();
                                    c.id = form.connection.id.clone();
                                    c.listed = false;
                                    c.query_required = !form.secrets.request_query.is_empty();
                                    if !form.secrets.password.is_empty() && c.service == "sftp" {
                                        c.native_sftp = true;
                                        c.key_file.clear();
                                    }
                                }
                                form.connection = c;
                                form.choices.clear();
                                self.app.connect_form(&mut form, false, &self.ctx);
                            }
                            self.app.connection_form = Some(form);
                        }
                    } else if id == "open-browser" {
                        if let Some(form) = &self.app.connection_form {
                            let path = form
                                .navigation
                                .as_ref()
                                .map(|t| t.path.as_path())
                                .unwrap_or(Path::new(""));
                            open::that(crate::uri::address(&form.connection.id, path))?;
                        }
                    } else if matches!(id, "accept" | "authorize") && !self.app.uri_connecting() {
                        if let Some(mut form) = self.app.connection_form.take() {
                            self.app
                                .connect_form(&mut form, id == "authorize", &self.ctx);
                            self.app.connection_form = Some(form);
                        }
                    } else if let Some(index) = id
                        .strip_prefix("connection-open-")
                        .and_then(|s| s.parse::<usize>().ok())
                        && let Some(connection) = self.app.connections.get(index)
                    {
                        let location = Location::Remote {
                            connection: connection.id.clone(),
                            path: connection.initial_path.clone(),
                        };
                        self.app.tab_mut().navigate(location, &self.ctx);
                        self.app.connection_form = None;
                    } else if let Some(index) = id
                        .strip_prefix("connection-edit-")
                        .and_then(|s| s.parse::<usize>().ok())
                        && let Some(connection) = self.app.connections.get(index)
                        && let Some(form) = &mut self.app.connection_form
                    {
                        form.connection = connection.clone();
                        form.secrets = Default::default();
                        form.error = None;
                        self.invalid_fields.clear();
                    } else if let Some(index) = id
                        .strip_prefix("connection-forget-")
                        .and_then(|s| s.parse::<usize>().ok())
                        && let Some(connection) = self.app.connections.get(index)
                    {
                        crate::remote::forget_credentials(&connection.id)?;
                    }
                }
                "archive" => {
                    if id == "browse-archive" {
                        if let Some(form) = &self.app.archive_form {
                            let file = PathBuf::from(&form.file);
                            self.app.archive_form = None;
                            self.app.apply_archive_action(
                                crate::feature_ui::ArchiveAction::Browse(file),
                                &self.ctx,
                            );
                        }
                    } else if id == "accept" {
                        let Some(form) = &self.app.archive_form else {
                            return Ok(());
                        };
                        anyhow::ensure!(
                            !form.file.trim().is_empty() || form.mode == 1,
                            "Enter an archive path."
                        );
                        let action = match form.mode {
                            0 => {
                                let edit = match form.edit_mode {
                                    0 => {
                                        let paths = form
                                            .additions
                                            .lines()
                                            .map(str::trim)
                                            .filter(|s| !s.is_empty())
                                            .map(PathBuf::from)
                                            .collect::<Vec<_>>();
                                        anyhow::ensure!(
                                            !paths.is_empty(),
                                            "Choose local files to add."
                                        );
                                        crate::archive_tools::ZipEdit::Add {
                                            sources: paths,
                                            prefix: form.prefix.clone(),
                                        }
                                    }
                                    1 => {
                                        anyhow::ensure!(
                                            !form.selected.is_empty(),
                                            "Browse the archive and select entries first."
                                        );
                                        crate::archive_tools::ZipEdit::Remove(form.selected.clone())
                                    }
                                    _ => crate::archive_tools::ZipEdit::Rename {
                                        source: form.source.clone().into(),
                                        target: form.target.clone().into(),
                                    },
                                };
                                crate::feature_ui::ArchiveAction::Run(Box::new(
                                    Operation::ArchiveEdit {
                                        archive: form.file.clone().into(),
                                        edit,
                                    },
                                ))
                            }
                            1 => {
                                anyhow::ensure!(
                                    !form.sources.is_empty() && !form.password.is_empty(),
                                    "Select local files and enter a password."
                                );
                                crate::feature_ui::ArchiveAction::Run(Box::new(
                                    Operation::PackProtected {
                                        sources: form.sources.clone(),
                                        destination: form.destination.clone().into(),
                                        password: form.password.clone(),
                                        compression_level: form.compression_level,
                                    },
                                ))
                            }
                            _ => crate::feature_ui::ArchiveAction::Unlock(
                                form.file.clone().into(),
                                form.password.clone(),
                            ),
                        };
                        self.app.archive_form = None;
                        self.app.apply_archive_action(action, &self.ctx);
                    }
                }
                "rename" if id == "accept" && idle => {
                    if let Some(form) = &self.app.tools.rename {
                        let plan = crate::desktop::rename_plan(&form.sources, &form.options)?;
                        self.app.tools.rename = None;
                        self.app.start_job(Operation::BatchRename(plan), &self.ctx);
                    }
                }
                "search" => {
                    if id == "accept" {
                        self.start_search();
                    } else if id == "stop" {
                        if let Some(form) = &self.app.tools.search {
                            form.cancel.store(true, Ordering::Relaxed);
                        }
                    } else if let Some(index) = id
                        .strip_prefix("search-open-")
                        .and_then(|s| s.parse::<usize>().ok())
                        && let Some(form) = &self.app.tools.search
                        && let Some(path) = form.result.as_ref().and_then(|r| r.paths.get(index))
                    {
                        let folder = path.parent().unwrap_or(&form.options.root).to_owned();
                        let location = if let Some(connection) = &form.connection {
                            Location::Remote {
                                connection: connection.clone(),
                                path: folder,
                            }
                        } else {
                            Location::Local(folder)
                        };
                        self.app.tools.search = None;
                        self.app.tab_mut().navigate(location, &self.ctx);
                    }
                }
                "comparison" => {
                    if let Some(form) = &mut self.app.features.comparison {
                        if id == "accept" && form.pending.is_none() && form.diff_pending.is_none() {
                            form.start(&self.ctx);
                        } else if id == "stop" {
                            form.cancel.store(true, Ordering::Relaxed);
                        } else if let Some(index) = id
                            .strip_prefix("comparison-diff-")
                            .and_then(|s| s.parse::<usize>().ok())
                        {
                            form.start_diff(index, &self.ctx);
                        }
                    }
                }
                "sync" => {
                    if id == "sync-preview" && idle {
                        if let Some(form) = &mut self.app.features.sync {
                            form.start_preview(&self.ctx);
                        }
                    } else if id == "accept" && idle {
                        let Some(form) = &mut self.app.features.sync else {
                            return Ok(());
                        };
                        let Some(plan) = &form.plan else {
                            anyhow::bail!("Preview and review the synchronization plan first.");
                        };
                        if plan.items.iter().any(|i| {
                            matches!(
                                i.choice,
                                crate::sync::Choice::DeleteLeft | crate::sync::Choice::DeleteRight
                            )
                        }) {
                            anyhow::ensure!(
                                form.deletions && form.confirmed,
                                "Review and confirm deletion choices first."
                            );
                        }
                        let plan = form.plan.take().unwrap();
                        self.app.features.sync = None;
                        self.app.start_job(Operation::Sync(plan), &self.ctx);
                    }
                }
                "recovery" => {
                    if id == "open-recovery" {
                        open::that(crate::history::root()?)?;
                    } else if id == "recovery-preview" && idle {
                        let candidates = crate::history::cleanup_candidates(
                            &self.app.history,
                            &crate::history::root()?,
                            &self.app.preferences.recovery,
                            crate::oauth::now(),
                        )?;
                        if let Some(form) = &mut self.app.features.recovery {
                            form.candidates = Some(candidates);
                        }
                    } else if id == "expire-backups"
                        && idle
                        && let Some(form) = &mut self.app.features.recovery
                        && form.cleaning.is_none()
                        && let Some(ids) = form.candidates.clone()
                    {
                        let (sender, receiver) = mpsc::channel();
                        form.cleaning = Some(receiver);
                        let ctx = self.ctx.clone();
                        std::thread::spawn(move || {
                            let result = crate::history::root()
                                .and_then(|root| crate::history::cleanup_in(&root, &ids))
                                .map(|_| ())
                                .map_err(|e| format!("{e:#}"));
                            let _ = sender.send(result);
                            ctx.request_repaint();
                        });
                    }
                }
                "workspaces" => {
                    use crate::next_ui::Effect;
                    let effect = if id == "accept" {
                        Some(Effect::SaveWorkspace(
                            self.app.features.workspace_name.clone(),
                        ))
                    } else if let Some(index) = id
                        .strip_prefix("workspace-load-")
                        .and_then(|s| s.parse::<usize>().ok())
                    {
                        if idle {
                            self.app.features.workspaces = false;
                            Some(Effect::LoadWorkspace(index))
                        } else {
                            None
                        }
                    } else if let Some(index) = id
                        .strip_prefix("workspace-rename-")
                        .and_then(|s| s.parse::<usize>().ok())
                    {
                        Some(Effect::RenameWorkspace(
                            index,
                            self.app.features.workspace_name.clone(),
                        ))
                    } else {
                        id.strip_prefix("workspace-delete-")
                            .and_then(|s| s.parse::<usize>().ok())
                            .map(Effect::DeleteWorkspace)
                    };
                    if let Some(effect) = effect {
                        self.app.apply_feature_effect(effect, &self.ctx);
                    }
                }
                "open-as" if id == "accept" => {
                    if let Some(form) = self.app.features.open_as.take() {
                        self.app.open_as(form, &self.ctx);
                    }
                }
                "open-with" if id == "accept" => {
                    if let Some((path, application)) = &self.app.features.open_with {
                        let rule = crate::associations::Rule {
                            pattern: self.app.features.application_pattern.clone(),
                            application: PathBuf::from(application),
                        };
                        rule.validate()?;
                        crate::native_actions::open_with(path, &rule.application)?;
                        if self.app.features.remember_application {
                            self.app
                                .preferences
                                .associations
                                .retain(|r| !r.pattern.eq_ignore_ascii_case(&rule.pattern));
                            self.app.preferences.associations.insert(0, rule);
                            self.app.save_settings();
                        }
                        self.app.features.open_with = None;
                    }
                }
                "history" => self.history_action(id, idle)?,
                _ => {}
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.field_error = Some(format!("{error:#}"));
        }
    }

    fn close_dialog(&mut self) {
        match self.dialog_kind.as_str() {
            "exit" => {
                self.app.confirm_exit = false;
                self.extra.quit_all = false;
            }
            "message" => self.app.message = None,
            "help" => self.app.help = false,
            "prompt" | "pack" => self.app.prompt = None,
            "pack-progress" => {
                if let Some(job) = &mut self.app.job {
                    job.cancel.store(true, Ordering::Relaxed);
                }
            }
            "preferences" => {
                self.app.preferences_form = None;
                self.app.preferences_error = None;
            }
            "connections" => {
                self.app.cancel_uri_form();
                self.app.connection_form = None;
                if let Some(cancel) = self.app.oauth_cancel.take() {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
            "archive" => self.app.archive_form = None,
            "rename" => self.app.tools.rename = None,
            "comparison" => self.app.features.comparison = None,
            "text-diff" => {
                if let Some(form) = &mut self.app.features.comparison {
                    form.diff = None;
                }
            }
            "search" => {
                if let Some(form) = self.app.tools.search.take() {
                    form.cancel.store(true, Ordering::Relaxed);
                }
            }
            "sync" => {
                if let Some(form) = self.app.features.sync.take() {
                    form.cancel.store(true, Ordering::Relaxed);
                }
            }
            "recovery" => {
                if self
                    .app
                    .features
                    .recovery
                    .as_ref()
                    .is_none_or(|f| f.cleaning.is_none())
                {
                    self.app.features.recovery = None;
                    self.app.save_settings();
                }
            }
            "workspaces" => {
                self.app.features.workspaces = false;
                self.app.features.error = None;
            }
            "open-with" => self.app.features.open_with = None,
            "open-as" => self.app.features.open_as = None,
            "history" => self.app.show_history = false,
            _ => {}
        }
        self.field_error = None;
        self.invalid_fields.clear();
    }

    fn start_search(&mut self) {
        let Some(form) = &mut self.app.tools.search else {
            return;
        };
        if form.pending.is_some() {
            return;
        }
        if let Err(error) = crate::search::validate(&form.options) {
            form.error = Some(format!("{error:#}"));
            return;
        }
        form.cancel = Arc::new(AtomicBool::new(false));
        let cancel = form.cancel.clone();
        let options = form.options.clone();
        let connection = form.connection.clone();
        let ctx = self.ctx.clone();
        let (sender, receiver) = mpsc::channel();
        form.pending = Some(receiver);
        form.result = None;
        form.error = None;
        std::thread::spawn(move || {
            let mut task = Task::new(cancel, |_| {});
            let result = if let Some(connection) = connection {
                crate::remote::search(&connection, &options, &mut task)
            } else {
                crate::desktop::search(&options, &mut task)
            }
            .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }

    fn history_action(&mut self, id: &str, idle: bool) -> anyhow::Result<()> {
        if id == "pause-job" {
            if let Some(control) = self.app.job.as_ref().and_then(|j| j.control.as_ref()) {
                control.toggle_pause();
            }
            return Ok(());
        }
        if id == "pause-queue" {
            self.app.queue.paused = !self.app.queue.paused;
            return Ok(());
        }
        if id == "cancel-job" {
            if let Some(job) = &self.app.job {
                job.cancel.store(true, Ordering::Relaxed);
            }
            return Ok(());
        }
        if let Some(i) = id
            .strip_prefix("queue-up-")
            .and_then(|s| s.parse::<usize>().ok())
        {
            if i > 0 && i < self.app.queue.len() {
                self.app.queue.swap(i, i - 1);
                self.app.queue.save()?;
            }
        } else if let Some(i) = id
            .strip_prefix("queue-down-")
            .and_then(|s| s.parse::<usize>().ok())
        {
            if i + 1 < self.app.queue.len() {
                self.app.queue.swap(i, i + 1);
                self.app.queue.save()?;
            }
        } else if let Some(i) = id
            .strip_prefix("queue-remove-")
            .and_then(|s| s.parse::<usize>().ok())
        {
            if i < self.app.queue.len() {
                self.app.queue.remove(i);
                self.app.queue.save()?;
            }
        } else if let Some(i) = id
            .strip_prefix("queue-retry-")
            .and_then(|s| s.parse::<usize>().ok())
            && idle
            && let Some(operation) = self.app.queue.failed.get(i).cloned()
        {
            anyhow::ensure!(
                !crate::queue::Queue::requires_input(&operation),
                "This operation needs its password or editor contents again. Recreate it from the corresponding tool."
            );
            self.app.start_job(operation, &self.ctx);
            if self.app.job.is_some() {
                self.app.queue.failed.remove(i);
                self.app.queue.save()?;
            }
        } else if let Some(i) = id
            .strip_prefix("queue-discard-")
            .and_then(|s| s.parse::<usize>().ok())
            && let Some(operation) = self.app.queue.failed.get(i)
        {
            crate::queue::discard_checkpoint(operation)?;
            self.app.queue.failed.remove(i);
            self.app.queue.save()?;
        } else if let Some(i) = id
            .strip_prefix("record-undo-")
            .and_then(|s| s.parse::<usize>().ok())
            && idle
            && let Some(record) = self.app.history.get(i).cloned()
        {
            let operation = if record.is_undo {
                Operation::Redo(Box::new(record))
            } else {
                Operation::Undo(Box::new(record))
            };
            self.app.start_job(operation, &self.ctx);
        } else if let Some(i) = id
            .strip_prefix("record-pin-")
            .and_then(|s| s.parse::<usize>().ok())
            && idle
            && let Some(record) = self.app.history.get_mut(i)
        {
            record.pinned = !record.pinned;
            crate::history::save(&self.app.history)?;
        } else if let Some(i) = id
            .strip_prefix("record-details-")
            .and_then(|s| s.parse::<usize>().ok())
            && let Some(record) = self.app.history.get(i)
        {
            self.app.message = Some(record.result.clone());
        }
        Ok(())
    }
}
