use super::*;
use crate::editor_drafts::Draft;

type Listing = Receiver<Result<Vec<(u64, String)>, String>>;
#[derive(Default)]
pub(super) struct State {
    pub open: bool,
    scanned: bool,
    auto_offer: bool,
    items: Vec<(u64, String)>,
    listing: Option<Listing>,
    loading: Option<Receiver<Result<Draft, String>>>,
    error: String,
    cleanup: Vec<Receiver<Result<(), String>>>,
}
impl Adapter {
    pub(super) fn drafts_open(&mut self) {
        self.drafts.open = true;
        self.drafts.scanned = true;
        self.drafts.auto_offer = false;
        self.drafts.error.clear();
        let (send, receive) = mpsc::channel();
        self.drafts.listing = Some(receive);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = send.send(crate::editor_drafts::list().map_err(|e| format!("{e:#}")));
            ctx.request_repaint();
        });
    }
    pub(super) fn discard_editor_draft(&mut self, editor: &mut Editor) {
        let id = editor.navigation.id;
        // Retire the ID immediately. A pending worker retains its old ID and
        // cannot publish after the discard receipt has been written.
        editor.navigation.id = rand::random();
        editor.navigation.draft_written = false;
        editor.navigation.draft_saved = 0;
        let ctx = self.ctx.clone();
        let (send, receive) = mpsc::channel();
        self.drafts.cleanup.push(receive);
        std::thread::spawn(move || {
            let _ = send.send(crate::editor_drafts::discard(id).map_err(|e| format!("{e:#}")));
            ctx.request_repaint();
        });
    }
    pub(super) fn drafts_tick(&mut self) {
        self.drafts
            .cleanup
            .retain(|receive| match receive.try_recv() {
                Ok(Err(error)) => {
                    crate::debug_log::record("editor recovery cleanup", &error);
                    false
                }
                Ok(Ok(())) | Err(mpsc::TryRecvError::Disconnected) => false,
                Err(mpsc::TryRecvError::Empty) => true,
            });
        if !self.drafts.scanned && !self.blocked() {
            self.drafts_open();
            self.drafts.open = false;
            self.drafts.auto_offer = true;
        }
        if let Some(result) = crate::background::poll(&mut self.drafts.listing) {
            match result {
                Ok(items) => {
                    self.drafts.items = items;
                }
                Err(error) => self.drafts.error = error,
            }
        }
        if self.drafts.auto_offer && self.drafts.listing.is_none() && !self.blocked() {
            self.drafts.auto_offer = false;
            self.drafts.open = !self.drafts.items.is_empty();
        }
        if let Some(result) = crate::background::poll(&mut self.drafts.loading) {
            match result {
                Ok(draft) if self.app.editor.is_none() => {
                    let mut navigation =
                        viewer_tools::Navigation::new(draft.location.clone(), draft.path.clone());
                    navigation.id = draft.id;
                    navigation.draft_written = true;
                    let path = draft.location.local().map(|_| draft.path.clone());
                    let remote = match &draft.location {
                        Location::Remote { connection, .. } => {
                            Some((connection.clone(), draft.path.clone()))
                        }
                        _ => None,
                    };
                    let original = draft.document.text.clone();
                    self.app.editor = Some(Editor {
                        navigation,
                        image_revision: 0,
                        document: Some(draft.document),
                        text_page: None,
                        pdf: None,
                        image: None,
                        zoom: 1.0,
                        kind: "Text viewer",
                        title: draft.title,
                        path,
                        remote,
                        text: draft.text,
                        original,
                        editable: true,
                        can_edit: true,
                        error: None,
                        confirm_close: false,
                    });
                    self.drafts.open = false;
                }
                Ok(_) => {
                    self.drafts.error = "Close the current editor before recovering a draft".into()
                }
                Err(error) => self.drafts.error = error,
            }
        }
        let Some(mut editor) = self.app.editor.take() else {
            return;
        };
        if let Some(result) = crate::background::poll(&mut editor.navigation.draft_pending)
            && let Err(error) = result
        {
            editor.navigation.notice = format!("Draft recovery could not be saved: {error}");
            editor.navigation.draft_saved = 0;
            editor.navigation.draft_due = std::time::Instant::now() + Duration::from_secs(10);
        }
        if !editor.dirty() {
            if editor.navigation.draft_written {
                self.discard_editor_draft(&mut editor);
            }
        } else if editor.navigation.draft_pending.is_none()
            && editor.navigation.draft_revision != editor.navigation.draft_saved
            && std::time::Instant::now() >= editor.navigation.draft_due
            && let Some(document) = editor.document.clone()
        {
            let draft = Draft {
                id: editor.navigation.id,
                title: editor.title.clone(),
                location: editor.navigation.location.clone(),
                path: editor.navigation.path.clone(),
                document,
                text: editor.text.clone(),
            };
            editor.navigation.draft_saved = editor.navigation.draft_revision;
            editor.navigation.draft_written = true;
            let (send, receive) = mpsc::channel();
            editor.navigation.draft_pending = Some(receive);
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let _ = send.send(crate::editor_drafts::save(&draft).map_err(|e| format!("{e:#}")));
                ctx.request_repaint();
            });
        }
        self.app.editor = Some(editor);
    }
    pub(super) fn drafts_dialog(&self) -> Option<dialogs::Dialog> {
        if !self.drafts.open {
            return None;
        }
        let mut d = dialogs::Dialog::new("drafts", "Recover editor drafts");
        d.description = "Reopen an unsaved editor session. Saving a recovered draft checks that the original file has not changed.".into();
        d.busy = self.drafts.listing.is_some() || self.drafts.loading.is_some();
        d.error = self.drafts.error.clone();
        for (id, title) in self.drafts.items.iter().take(128) {
            d.rows.push(
                dialogs::Row::new(format!("draft-{id}"), title, "Unsaved editor session")
                    .action(
                        format!("draft-open-{id}"),
                        "Recover",
                        !d.busy && self.app.editor.is_none(),
                    )
                    .action(format!("draft-discard-{id}"), "Discard", !d.busy),
            );
        }
        if self.drafts.items.is_empty() && !d.busy {
            d.description = "No unsaved editor drafts are available.".into();
        }
        Some(d)
    }
    pub(super) fn drafts_action(&mut self, id: &str) -> bool {
        if !self.drafts.open {
            return false;
        }
        if id == "cancel" {
            self.drafts.open = false;
            return true;
        }
        if self.drafts.loading.is_some() || self.drafts.listing.is_some() {
            return true;
        }
        if let Some(id) = id
            .strip_prefix("draft-open-")
            .and_then(|s| s.parse::<u64>().ok())
        {
            let (send, receive) = mpsc::channel();
            self.drafts.loading = Some(receive);
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let _ = send.send(crate::editor_drafts::load(id).map_err(|e| format!("{e:#}")));
                ctx.request_repaint();
            });
        } else if let Some(id) = id
            .strip_prefix("draft-discard-")
            .and_then(|s| s.parse::<u64>().ok())
        {
            let (send, receive) = mpsc::channel();
            self.drafts.listing = Some(receive);
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let result = crate::editor_drafts::discard(id)
                    .and_then(|()| crate::editor_drafts::list())
                    .map_err(|e| format!("{e:#}"));
                let _ = send.send(result);
                ctx.request_repaint();
            });
        }
        true
    }
}

impl State {
    pub(super) fn cleanup_pending(&self) -> bool {
        !self.cleanup.is_empty()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn autosave_recovery_preserves_original_guard_and_discard_retires_the_draft() {
        let (_root, ui, state, left, _right) = super::super::context_menus::setup();
        ui.invoke_selected(0, path_id(&left.join("alpha.txt")).into(), false, false);
        ui.invoke_action("Edit".into());
        for _ in 0..1000 {
            let mut a = state.borrow_mut();
            let ctx = a.ctx.clone();
            a.app.poll(&ctx);
            if a.app.editor.is_some() {
                break;
            }
            drop(a);
            std::thread::sleep(Duration::from_millis(3));
        }
        ui.invoke_viewer_edit("Recovered editor text".into());
        let id = {
            let mut a = state.borrow_mut();
            a.drafts.scanned = true;
            a.app.editor.as_mut().unwrap().navigation.draft_due = std::time::Instant::now();
            a.drafts_tick();
            a.app.editor.as_ref().unwrap().navigation.id
        };
        for _ in 0..1000 {
            state.borrow_mut().drafts_tick();
            if state
                .borrow()
                .app
                .editor
                .as_ref()
                .unwrap()
                .navigation
                .draft_pending
                .is_none()
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(3));
        }
        let saved = crate::editor_drafts::load(id).unwrap();
        assert_eq!(saved.text, "Recovered editor text");
        assert_ne!(saved.document.original, b"Recovered editor text");
        // Simulate loss of the in-memory editor. Drop deliberately retains recovery.
        state.borrow_mut().app.editor = None;
        ui.invoke_action("RecoverEditorDrafts".into());
        for _ in 0..1000 {
            state.borrow_mut().drafts_tick();
            if state.borrow().drafts.listing.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(3));
        }
        ui.invoke_dialog_action(format!("draft-open-{id}").into());
        for _ in 0..1000 {
            state.borrow_mut().drafts_tick();
            if state.borrow().app.editor.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(3));
        }
        assert_eq!(
            state.borrow().app.editor.as_ref().unwrap().text,
            "Recovered editor text"
        );
        std::fs::write(left.join("alpha.txt"), b"Changed elsewhere").unwrap();
        ui.invoke_viewer_action("save".into());
        for _ in 0..1000 {
            let mut a = state.borrow_mut();
            let ctx = a.ctx.clone();
            a.app.poll(&ctx);
            if a.app.job.is_none() {
                break;
            }
            drop(a);
            std::thread::sleep(Duration::from_millis(3));
        }
        assert!(state.borrow().app.editor.as_ref().unwrap().error.is_some());
        assert_eq!(
            std::fs::read(left.join("alpha.txt")).unwrap(),
            b"Changed elsewhere"
        );
        ui.invoke_viewer_action("discard".into());
        for _ in 0..1000 {
            state.borrow_mut().drafts_tick();
            if !state.borrow().drafts.cleanup_pending() {
                break;
            }
            std::thread::sleep(Duration::from_millis(3));
        }
        assert!(crate::editor_drafts::load(id).is_err());
    }
}
