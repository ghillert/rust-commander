use super::dialogs::{Dialog, Field, Row};
use super::*;
use std::collections::BTreeSet;
struct TreeUpdate {
    key: String,
    nodes: Vec<crate::directory_tree::Node>,
    expanded: BTreeSet<PathBuf>,
    location: Location,
    active: PathBuf,
}
type TreeResult = Result<TreeUpdate, String>;
#[derive(Default)]
pub(super) struct State {
    tree_root: Option<Location>,
    applications: Option<Receiver<Result<Vec<PathBuf>, String>>>,
    external_pending: Option<Receiver<Result<crate::external_edit::Session, String>>>,
    pub external: Option<crate::external_edit::Session>,
    external_saving: bool,
    association_dialog: bool,
    application: String,
    pattern: String,
    remember: bool,
    notice: String,
    tree_key: String,
    expanded: BTreeSet<PathBuf>,
    nodes: Vec<crate::directory_tree::Node>,
    published_nodes: Vec<crate::directory_tree::Node>,
    published_active: Option<PathBuf>,
    resolved_active: Option<(Location, PathBuf)>,
    tree_model: Rc<VecModel<TreeRow>>,
    pending_reveal: Option<usize>,
    pending: Option<Receiver<TreeResult>>,
}
impl Adapter {
    pub(super) fn enhancements_tick(&mut self) {
        if let Some(Ok(applications)) = crate::background::poll(&mut self.enhancements.applications)
        {
            self.app.features.applications = applications;
        }
        if let Some(result) = crate::background::poll(&mut self.enhancements.external_pending) {
            match result {
                Ok(session) => self.enhancements.external = Some(session),
                Err(e) => self.app.fail(e),
            }
        }
        if self.enhancements.external_saving && self.app.job.is_none() && self.app.queue.is_empty()
        {
            self.enhancements.external_saving = false;
            if self.app.history.last().is_some_and(|r| r.success) {
                self.enhancements.external = None;
                self.app.refresh(&self.ctx);
            } else {
                self.enhancements.notice="Save failed. The working copy is retained; review the operation result before retrying.".into();
            }
        }
        if let Some(result) = crate::background::poll(&mut self.enhancements.pending) {
            match result {
                Ok(update) if update.key == self.enhancements.tree_key => {
                    self.enhancements.nodes = update.nodes;
                    self.enhancements.expanded = update.expanded;
                    self.enhancements.resolved_active = Some((update.location, update.active));
                }
                Err(e) => self.app.fail(e),
                _ => {}
            }
        }
        if !self.app.preferences.directory_tree {
            return;
        }
        let root = crate::directory_tree::root(&self.app.tab().location);
        if self.enhancements.tree_root.as_ref() != Some(&root) {
            self.enhancements.tree_root = Some(root.clone());
            self.enhancements.expanded.clear();
            self.enhancements.nodes.clear();
            self.enhancements.tree_key.clear();
        }
        let location = self.app.tab().location.clone();
        let key = format!(
            "{}:{}:{}:{:?}",
            location.label(),
            self.app.tab().revision,
            self.app.show_hidden,
            self.enhancements.expanded
        );
        if self.enhancements.tree_key == key || self.enhancements.pending.is_some() {
            return;
        }
        // Navigation follows the active folder, expanding its ancestors once.
        let follow = !self
            .enhancements
            .tree_key
            .starts_with(&format!("{}:", location.label()));
        if follow {
            for path in crate::directory_tree::path(&location).ancestors() {
                if path != crate::directory_tree::path(&location) {
                    self.enhancements.expanded.insert(path.into());
                }
            }
            self.enhancements
                .expanded
                .insert(crate::directory_tree::path(&root).into());
        }
        self.enhancements.tree_key = format!(
            "{}:{}:{}:{:?}",
            location.label(),
            self.app.tab().revision,
            self.app.show_hidden,
            self.enhancements.expanded
        );
        let key = self.enhancements.tree_key.clone();
        let mut expanded = self.enhancements.expanded.clone();
        let hidden = self.app.show_hidden;
        let ctx = self.ctx.clone();
        let (sender, receiver) = mpsc::channel();
        self.enhancements.pending = Some(receiver);
        std::thread::spawn(move || {
            let mut root = root;
            let mut active = crate::directory_tree::path(&location).to_owned();
            if matches!(location, Location::Local(_)) {
                // Resolve aliases on the worker, including macOS /var ->
                // /private/var. Expansion still refuses symlink descendants.
                active = active.canonicalize().unwrap_or(active);
                expanded = expanded
                    .into_iter()
                    .map(|path| path.canonicalize().unwrap_or(path))
                    .collect();
                root = crate::directory_tree::root(&Location::Local(active.clone()));
                if follow {
                    expanded.extend(active.ancestors().skip(1).map(Path::to_owned));
                }
            }
            if follow {
                expanded.insert(active.clone());
            }
            let result = crate::directory_tree::build(&root, &expanded, hidden, &Task::default())
                .map_err(|e| format!("{e:#}"));
            let _ = sender.send(result.map(|nodes| TreeUpdate {
                key,
                nodes,
                expanded,
                location,
                active,
            }));
            ctx.request_repaint();
        });
    }
    pub(super) fn sync_tree(&mut self, ui: &CommanderWindow) {
        ui.set_tree_visible(self.app.preferences.directory_tree);
        // ListView clamps scrolling against its laid-out viewport. Defer the
        // reveal until the tick after publishing a new model.
        if let Some(index) = self.enhancements.pending_reveal.take() {
            ui.invoke_reveal_tree(index as i32);
        }
        let location = &self.app.tab().location;
        let active = self
            .enhancements
            .resolved_active
            .as_ref()
            .filter(|(resolved, _)| resolved == location)
            .map(|(_, path)| path.as_path())
            .unwrap_or_else(|| crate::directory_tree::path(location));
        if self.enhancements.nodes != self.enhancements.published_nodes
            || self.enhancements.published_active.as_deref() != Some(active)
        {
            self.enhancements.tree_model.set_vec(
                self.enhancements
                    .nodes
                    .iter()
                    .enumerate()
                    .map(|(index, n)| TreeRow {
                        index: index as i32,
                        title: n.title.as_str().into(),
                        depth: n.depth as i32,
                        expanded: n.expanded,
                        selected: n.path == active,
                        detail: n.error.as_str().into(),
                    })
                    .collect::<Vec<_>>(),
            );
            self.enhancements.published_nodes = self.enhancements.nodes.clone();
            self.enhancements.published_active = Some(active.into());
            if let Some(index) = self
                .enhancements
                .nodes
                .iter()
                .position(|n| n.path == active)
            {
                ui.set_tree_focused_index(index as i32);
                self.enhancements.pending_reveal = Some(index);
            }
        }
        ui.set_tree_rows(self.enhancements.tree_model.clone().into());
    }
    pub(super) fn tree_key(&mut self, index: i32, event: slint::language::KeyEvent) -> i32 {
        if self.blocked()
            || self.enhancements.nodes.is_empty()
            || event.modifiers.control
            || event.modifiers.alt
            || event.modifiers.meta
        {
            return -1;
        }
        let index = (index.max(0) as usize).min(self.enhancements.nodes.len() - 1);
        let node = &self.enhancements.nodes[index];
        let depth = node.depth;
        let expanded = node.expanded;
        let is = |key| event.text == SharedString::from(key);
        if is(slint::platform::Key::UpArrow) {
            return index.saturating_sub(1) as i32;
        }
        if is(slint::platform::Key::DownArrow) {
            return (index + 1).min(self.enhancements.nodes.len() - 1) as i32;
        }
        if is(slint::platform::Key::LeftArrow) {
            if expanded {
                self.tree_action(index as i32, true);
            } else if let Some(parent) = (0..index)
                .rev()
                .find(|i| self.enhancements.nodes[*i].depth < depth)
            {
                return parent as i32;
            }
        } else if is(slint::platform::Key::RightArrow) {
            if !expanded {
                self.tree_action(index as i32, true);
            } else if self
                .enhancements
                .nodes
                .get(index + 1)
                .is_some_and(|n| n.depth > depth)
            {
                return index as i32 + 1;
            }
        } else if is(slint::platform::Key::Return) {
            self.tree_action(index as i32, false);
        } else {
            return -1;
        }
        index as i32
    }
    pub(super) fn tree_action(&mut self, index: i32, toggle: bool) {
        if self.blocked() {
            return;
        }
        let Some(node) = self.enhancements.nodes.get(index as usize) else {
            return;
        };
        let path = node.path.clone();
        if toggle {
            if !self.enhancements.expanded.remove(&path) {
                self.enhancements.expanded.insert(path);
            }
        } else if let Some(root) = &self.enhancements.tree_root {
            let next = crate::directory_tree::at(root, &path);
            self.app.tab_mut().navigate(next, &self.ctx);
        }
    }
}

impl Adapter {
    pub(super) fn enhancements_modal(&self) -> bool {
        self.enhancements.external.is_some()
            || self.enhancements.external_pending.is_some()
            || self.enhancements.association_dialog
    }
    fn discover_applications(&mut self) {
        if self.enhancements.applications.is_some() || !self.app.features.applications.is_empty() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.enhancements.applications = Some(receiver);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(Ok(crate::associations::discover()));
            ctx.request_repaint();
        });
    }
    pub(super) fn enhancements_action(&mut self, name: &str) -> bool {
        match name {
            "OpenWith" => {
                self.app.act(Action::OpenWith, &self.ctx);
                if let Some((path, application)) = &mut self.app.features.open_with {
                    *application =
                        crate::associations::application(&self.app.preferences.associations, path)
                            .map(|p| p.display().to_string())
                            .unwrap_or_default();
                    self.app.features.application_pattern = path
                        .extension()
                        .map(|e| format!("*.{}", e.to_string_lossy()))
                        .unwrap_or_else(|| {
                            path.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned()
                        });
                }
                self.discover_applications();
                true
            }
            "Associations" => {
                self.enhancements.association_dialog = true;
                self.enhancements.notice.clear();
                self.discover_applications();
                true
            }
            "ExternalEdit" => {
                if self.enhancements.external_pending.is_some() {
                    return true;
                }
                let entries = self.app.tab().chosen(self.app.show_hidden);
                let Some(entry) = entries.first().filter(|e| !e.is_dir) else {
                    self.app.fail("Select a file to edit externally");
                    return true;
                };
                let location = self.app.tab().location.clone();
                let path = entry.path.clone();
                self.enhancements.application =
                    crate::associations::application(&self.app.preferences.associations, &path)
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                self.enhancements.pattern = path
                    .extension()
                    .map(|e| format!("*.{}", e.to_string_lossy()))
                    .unwrap_or_else(|| entry.name.clone());
                self.enhancements.notice.clear();
                self.discover_applications();
                let (sender, receiver) = mpsc::channel();
                self.enhancements.external_pending = Some(receiver);
                let ctx = self.ctx.clone();
                std::thread::spawn(move || {
                    let result = crate::external_edit::Session::prepare(location, path)
                        .map_err(|e| format!("{e:#}"));
                    let _ = sender.send(result);
                    ctx.request_repaint();
                });
                true
            }
            _ => false,
        }
    }
    pub(super) fn enhancements_dialog(&self) -> Option<Dialog> {
        if self.enhancements.external_pending.is_some() {
            let mut d = Dialog::new("external-edit", "Preparing editor copy");
            d.busy = true;
            return Some(d);
        }
        if let Some(session) = &self.enhancements.external {
            let mut d = Dialog::new("external-edit", "External editor");
            d.description = format!(
                "{}\nEdit the working copy, save in your editor, then review and save back here.\nWorking copy: {}",
                session.source.display(),
                session.file.display()
            );
            d.fields.push(Field::choice(
                "application",
                "Installed application",
                &self.enhancements.application,
                self.app
                    .features
                    .applications
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>(),
            ));
            d.fields.push(Field::text(
                "application",
                "Application path (blank uses system default)",
                &self.enhancements.application,
            ));
            d.fields.push(Field::check(
                "remember",
                "Remember application for this pattern",
                self.enhancements.remember,
            ));
            d.fields.push(Field::text(
                "pattern",
                "Filename pattern",
                &self.enhancements.pattern,
            ));
            d.error = self.enhancements.notice.clone();
            d.busy = self.enhancements.external_saving;
            d.buttons.clear();
            d.button("editor-open", "Open editor", !d.busy);
            d.button("editor-review", "Review changes", !d.busy);
            d.button("editor-save", "Save back", !d.busy);
            d.button("editor-discard", "Discard working copy", !d.busy);
            return Some(d);
        }
        if self.enhancements.association_dialog {
            let mut d = Dialog::new("associations", "Application associations");
            d.description="Rules are checked in order when opening local files. F3/F4 continue to use the built-in viewer/editor.".into();
            d.fields.push(Field::text(
                "pattern",
                "Filename pattern (for example *.txt)",
                &self.enhancements.pattern,
            ));
            d.fields.push(Field::choice(
                "application",
                "Installed application",
                &self.enhancements.application,
                self.app
                    .features
                    .applications
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>(),
            ));
            d.fields.push(Field::text(
                "application",
                "Application path",
                &self.enhancements.application,
            ));
            for (i, rule) in self.app.preferences.associations.iter().enumerate() {
                d.rows.push(Row {
                    id: format!("rule-{i}"),
                    title: rule.pattern.clone(),
                    detail: rule.application.display().to_string(),
                    selected: false,
                    actions: vec![(format!("remove-rule-{i}"), "Remove".into(), true)],
                });
            }
            d.error = self.enhancements.notice.clone();
            d.button("add-rule", "Save rule", true);
            return Some(d);
        }
        None
    }
    pub(super) fn enhancements_edit(&mut self, id: &str, value: &str) -> bool {
        if !matches!(self.dialog_kind.as_str(), "external-edit" | "associations") {
            return false;
        }
        match id {
            "application" => self.enhancements.application = value.into(),
            "pattern" => self.enhancements.pattern = value.into(),
            "remember" => self.enhancements.remember = value == "true",
            _ => {}
        }
        true
    }
    pub(super) fn enhancements_dialog_action(&mut self, id: &str) -> bool {
        if !matches!(self.dialog_kind.as_str(), "external-edit" | "associations") {
            return false;
        }
        let result = (|| -> anyhow::Result<()> {
            if self.enhancements.external_saving {
                return Ok(());
            }
            if let Some(index) = id
                .strip_prefix("remove-rule-")
                .and_then(|s| s.parse::<usize>().ok())
            {
                if index < self.app.preferences.associations.len() {
                    self.app.preferences.associations.remove(index);
                    self.app.save_settings();
                }
                return Ok(());
            }
            if id == "add-rule" || (id == "editor-open" && self.enhancements.remember) {
                let rule = crate::associations::Rule {
                    pattern: self.enhancements.pattern.clone(),
                    application: PathBuf::from(&self.enhancements.application),
                };
                rule.validate()?;
                self.app
                    .preferences
                    .associations
                    .retain(|r| !r.pattern.eq_ignore_ascii_case(&rule.pattern));
                self.app.preferences.associations.insert(0, rule);
                self.app.save_settings();
            }
            match id {
                "cancel" => {
                    self.enhancements.association_dialog = false;
                    self.enhancements.external_pending = None;
                }
                "editor-discard" => {
                    self.enhancements.external = None;
                }
                "editor-open" => {
                    if let Some(session) = &self.enhancements.external {
                        if self.enhancements.application.is_empty() {
                            open::that_detached(&session.file)?;
                        } else {
                            crate::native_actions::open_with(
                                &session.file,
                                Path::new(&self.enhancements.application),
                            )?;
                        }
                    }
                }
                "editor-review" => {
                    if let Some(session) = &self.enhancements.external {
                        self.enhancements.notice = if session.changed()? {
                            format!(
                                "Edited copy: {} bytes. Save back checks that the original has not changed.",
                                session.bytes()?.len()
                            )
                        } else {
                            "No changes yet. Save the file in the external editor first.".into()
                        };
                    }
                }
                "editor-save" => {
                    if let Some(session) = &self.enhancements.external {
                        let op = session.save_operation()?;
                        self.app.start_job(op, &self.ctx);
                        self.enhancements.external_saving = true;
                    }
                }
                _ => {}
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.enhancements.notice = format!("{e:#}");
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
        wire(&ui, state.clone());
        state.borrow_mut().sync(&ui);
        (root, ui, state, left, right)
    }
    fn wait(state: &Rc<RefCell<Adapter>>, ui: &CommanderWindow, done: impl Fn(&Adapter) -> bool) {
        for _ in 0..500 {
            {
                let mut a = state.borrow_mut();
                let ctx = a.ctx.clone();
                a.app.poll(&ctx);
                a.enhancements_tick();
                a.viewer_tools_tick();
                a.sync(ui);
                if done(&a) {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("UI worker did not finish");
    }
    #[test]
    fn tab_switches_file_panes_but_fields_keep_natural_focus_traversal() {
        let (_root, ui, state, _left, _right) = setup();
        ui.show().unwrap();
        let press = |key: slint::platform::Key| {
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: key.into() });
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::KeyReleased { text: key.into() });
        };
        ui.invoke_focus_files(0);
        assert_eq!(ui.get_focused_control(), 0);
        press(slint::platform::Key::Tab);
        assert_eq!(state.borrow().app.active, 1);
        assert_eq!(ui.get_focused_control(), 3);
        ui.invoke_focus_filter(1);
        let mut tab = slint::language::KeyEvent::default();
        tab.text = slint::platform::Key::Tab.into();
        assert!(
            !ui.invoke_global_key(tab),
            "Fields must retain native Tab traversal"
        );
        assert_eq!(state.borrow().app.active, 1);
        ui.invoke_focus_filter(0);
        let mut next = slint::language::KeyEvent::default();
        next.text = slint::platform::Key::F6.into();
        next.modifiers.control = true;
        assert!(ui.invoke_global_key(next));
        assert_eq!(
            ui.get_focused_control(),
            3,
            "Focus next must start at the actual focused field"
        );
        ui.invoke_focus_filter(1);
        ui.invoke_action("FocusNext".into());
        assert_eq!(
            ui.get_focused_control(),
            0,
            "Menu focus commands also start at the actual control"
        );
        ui.invoke_action("Mkdir".into());
        press(slint::platform::Key::Tab);
        assert!(!ui.get_dialog_title().is_empty());
        assert_eq!(
            ui.get_focused_control(),
            -1,
            "Modal traversal must not focus background panes"
        );
    }
    #[cfg(debug_assertions)]
    #[test]
    fn accessibility_actions_select_files_and_expose_tree_expansion() {
        use i_slint_backend_testing::{AccessibleRole, ElementHandle};
        let (_root, ui, state, left, _right) = setup();
        ui.show().unwrap();
        let row = ElementHandle::find_by_accessible_label(&ui, "alpha.txt")
            .find(|item| item.accessible_role() == Some(AccessibleRole::ListItem))
            .unwrap();
        assert_eq!(row.accessible_item_selectable(), Some(true));
        row.invoke_accessible_default_action();
        assert_eq!(
            state.borrow().app.tab().cursor.as_deref(),
            Some(left.join("alpha.txt").as_path())
        );
        assert_eq!(row.accessible_item_selected(), Some(true));
        ui.invoke_action("DirectoryTree".into());
        {
            let mut a = state.borrow_mut();
            a.enhancements.tree_root = Some(Location::Local(left.clone()));
            a.enhancements.nodes = crate::directory_tree::build(
                &Location::Local(left.clone()),
                &BTreeSet::from([left.clone()]),
                false,
                &Task::default(),
            )
            .unwrap();
            a.sync(&ui);
        }
        let tree = ElementHandle::find_by_accessible_label(&ui, "Folders")
            .find(|item| item.accessible_role() == Some(AccessibleRole::Tree))
            .unwrap();
        assert!(tree.accessible_item_count().unwrap() > 0);
        let row = ElementHandle::find_by_accessible_label(&ui, "folder")
            .find(|item| item.accessible_expandable() == Some(true))
            .unwrap();
        assert_eq!(row.accessible_expanded(), Some(false));
        row.invoke_accessible_default_action();
        assert_eq!(
            state.borrow().app.tab().location,
            Location::Local(left.join("folder"))
        );
    }
    #[cfg(unix)]
    #[test]
    fn tree_follows_the_active_folder_through_a_local_alias() {
        let (root, ui, state, left, _right) = setup();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&left, &alias).unwrap();
        {
            let mut a = state.borrow_mut();
            let ctx = a.ctx.clone();
            a.app.panes[0]
                .tab_mut()
                .navigate(Location::Local(alias.clone()), &ctx);
        }
        ui.invoke_action("DirectoryTree".into());
        let canonical = left.canonicalize().unwrap();
        wait(&state, &ui, |a| {
            a.enhancements
                .nodes
                .iter()
                .any(|node| node.path == canonical)
        });
        let a = state.borrow();
        assert_eq!(
            a.enhancements.resolved_active.as_ref().unwrap(),
            &(Location::Local(alias), canonical)
        );
        assert!(
            ui.get_tree_rows()
                .iter()
                .any(|row| row.selected && row.title == "left")
        );
    }
    #[test]
    fn tree_callbacks_navigate_and_association_rules_are_editable() {
        let (_root, ui, state, left, _right) = setup();
        ui.invoke_action("DirectoryTree".into());
        assert!(ui.get_tree_visible());
        {
            let mut a = state.borrow_mut();
            a.enhancements.tree_root = Some(Location::Local(left.clone()));
            a.enhancements.nodes = crate::directory_tree::build(
                &Location::Local(left.clone()),
                &BTreeSet::from([left.clone()]),
                false,
                &Task::default(),
            )
            .unwrap();
            a.sync(&ui);
        }
        ui.invoke_tree_action(1, false);
        assert_eq!(
            state.borrow().app.tab().location,
            Location::Local(left.join("folder"))
        );
        ui.invoke_action("Associations".into());
        assert_eq!(ui.get_dialog_title(), "Application associations");
        ui.invoke_dialog_field("pattern".into(), "*.txt".into());
        ui.invoke_dialog_field(
            "application".into(),
            std::env::current_exe()
                .unwrap()
                .display()
                .to_string()
                .into(),
        );
        ui.invoke_dialog_action("add-rule".into());
        assert_eq!(state.borrow().app.preferences.associations.len(), 1);
        ui.invoke_dialog_action("remove-rule-0".into());
        assert!(state.borrow().app.preferences.associations.is_empty());
        ui.invoke_dialog_action("cancel".into());
        assert!(ui.get_dialog_title().is_empty());
    }
    #[test]
    fn external_editor_working_copy_requires_explicit_discard_and_protects_window_close() {
        let (_root, ui, state, left, _right) = setup();
        ui.invoke_selected(0, path_id(&left.join("alpha.txt")).into(), false, false);
        ui.invoke_action("ExternalEdit".into());
        wait(&state, &ui, |a| a.enhancements.external.is_some());
        let copy = state
            .borrow()
            .enhancements
            .external
            .as_ref()
            .unwrap()
            .file
            .clone();
        std::fs::write(&copy, b"external editor draft").unwrap();
        ui.invoke_dialog_action("editor-review".into());
        assert!(ui.get_dialog_error().contains("21 bytes"));
        {
            let mut a = state.borrow_mut();
            a.request_close();
            a.sync(&ui);
        }
        assert_eq!(ui.get_dialog_title(), "Close Rust Commander?");
        ui.invoke_dialog_action("cancel".into());
        assert!(copy.exists());
        ui.invoke_dialog_action("editor-discard".into());
        assert!(!copy.exists());
        assert_eq!(std::fs::read(left.join("alpha.txt")).unwrap(), b"alpha");
    }
    #[test]
    fn pdf_controls_unlock_rotate_and_jump_using_real_workers() {
        let (_root, ui, state, left, _right) = setup();
        let path = left.join("protected.pdf");
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/two-pages-protected.pdf"),
        )
        .unwrap();
        {
            let mut a = state.borrow_mut();
            let entry = filesystem::entry_from_path(&path).unwrap();
            let ctx = a.ctx.clone();
            a.app.view(entry, false, &ctx);
        }
        wait(&state, &ui, |a| a.app.pending_view.is_none());
        assert!(ui.get_viewer_locked());
        ui.invoke_viewer_action("unlock:wrong".into());
        wait(&state, &ui, |a| a.app.pending_pdf.is_none());
        assert!(ui.get_viewer_locked());
        assert!(!ui.get_viewer_error().is_empty());
        ui.invoke_viewer_action("unlock:commander-test".into());
        wait(&state, &ui, |a| a.app.pending_pdf.is_none());
        assert!(!ui.get_viewer_locked());
        assert_eq!(ui.get_viewer_page(), "1 / 2");
        ui.invoke_viewer_action("rotate".into());
        wait(&state, &ui, |a| a.app.pending_pdf.is_none());
        assert_eq!(
            state
                .borrow()
                .app
                .editor
                .as_ref()
                .unwrap()
                .pdf
                .as_ref()
                .unwrap()
                .rotation,
            1
        );
        ui.invoke_viewer_action("page:2".into());
        wait(&state, &ui, |a| a.app.pending_pdf.is_none());
        assert_eq!(ui.get_viewer_page(), "2 / 2");
    }
    #[test]
    fn viewer_search_crosses_large_file_pages_and_byte_jump_works() {
        let (_root, ui, state, left, _right) = setup();
        let path = left.join("large.txt");
        let mut bytes = vec![b'x'; 3 * 1024 * 1024];
        bytes[700_000..700_006].copy_from_slice(b"needle");
        std::fs::write(&path, bytes).unwrap();
        {
            let mut a = state.borrow_mut();
            let ctx = a.ctx.clone();
            a.app
                .view(filesystem::entry_from_path(&path).unwrap(), false, &ctx);
        }
        wait(&state, &ui, |a| a.app.pending_view.is_none());
        ui.invoke_viewer_action("find:needle".into());
        wait(&state, &ui, |a| {
            a.app.editor.as_ref().unwrap().navigation.pending.is_none()
        });
        assert!(ui.get_viewer_notice().contains("700000"));
        assert!(ui.get_viewer_text().contains("needle"));
        ui.invoke_viewer_action("jump:0x10".into());
        assert_eq!(
            state
                .borrow()
                .app
                .editor
                .as_ref()
                .unwrap()
                .text_page
                .as_ref()
                .unwrap()
                .offset,
            16
        );
    }
}
