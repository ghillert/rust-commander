//! Security drafts are prepared off the UI thread and applied through the queue.
use super::*;
use crate::security_metadata::{Change, Snapshot};
#[derive(Default)]
pub(super) struct Form {
    pending: Option<Receiver<Result<Snapshot, String>>>,
    snapshot: Option<Snapshot>,
    owner: String,
    group: String,
    acl: String,
    error: String,
    saving: bool,
}
impl Adapter {
    pub(super) fn security_open(&mut self) {
        let selected = self.app.tab().chosen(self.app.show_hidden);
        if self.app.tab().location.local().is_none() || selected.len() != 1 {
            self.app
                .fail("Select exactly one local file or directory to edit ownership and ACL rules");
            return;
        }
        let path = selected[0].path.clone();
        let (sender, receiver) = mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ =
                sender.send(crate::security_metadata::read(&path).map_err(|e| format!("{e:#}")));
            ctx.request_repaint();
        });
        self.security = Some(Form {
            pending: Some(receiver),
            ..Default::default()
        });
    }
    pub(super) fn security_tick(&mut self) {
        let Some(form) = &mut self.security else {
            return;
        };
        if let Some(result) = crate::background::poll(&mut form.pending) {
            match result {
                Ok(snapshot) => {
                    form.owner = snapshot.owner.clone();
                    form.group = snapshot.group.clone();
                    form.acl = snapshot.acl.clone();
                    form.snapshot = Some(snapshot);
                }
                Err(error) => form.error = error,
            }
        }
        if form.saving && self.app.job.is_none() && self.app.queue.is_empty() {
            form.saving = false;
            if self.app.history.last().is_some_and(|r| r.success) {
                self.security = None;
                self.app.refresh(&self.ctx);
            } else {
                form.error = self
                    .app
                    .history
                    .last()
                    .map(|r| r.result.clone())
                    .unwrap_or_else(|| {
                        "Security update failed; reload the metadata before retrying".into()
                    });
            }
        }
    }
    pub(super) fn security_dialog(&self) -> Option<dialogs::Dialog> {
        let form = self.security.as_ref()?;
        let mut d = dialogs::Dialog::new("security", "Ownership and ACL");
        d.busy = form.pending.is_some() || form.saving;
        d.error = form.error.clone();
        d.description="Review owner, group and ACL rules before applying. The operating system enforces your permissions. These metadata changes do not have automatic Undo.".into();
        if let Some(snapshot) = &form.snapshot {
            d.fields.extend([
                dialogs::Field::label("File", snapshot.path.display()),
                dialogs::Field::text("security-owner", "Owner name or ID", &form.owner),
                dialogs::Field::text("security-group", "Group name or ID", &form.group),
                dialogs::Field::label("ACL syntax", crate::security_metadata::syntax()),
                dialogs::Field::multiline("security-acl", "ACL rules", &form.acl),
            ]);
            d.button("security-reset", "Restore draft", !d.busy);
            d.button("security-apply", "Apply security changes", !d.busy);
        }
        d.buttons[0].2 = !form.saving;
        Some(d)
    }
    pub(super) fn security_edit(&mut self, id: &str, value: &str) -> bool {
        let Some(form) = &mut self.security else {
            return false;
        };
        match id {
            "security-owner" => form.owner = value.into(),
            "security-group" => form.group = value.into(),
            "security-acl" => form.acl = value.into(),
            _ => return false,
        }
        form.error.clear();
        true
    }
    pub(super) fn security_action(&mut self, id: &str) -> bool {
        let Some(form) = &mut self.security else {
            return false;
        };
        if form.saving {
            return true;
        }
        match id {
            "cancel" => self.security = None,
            "security-reset" => {
                if let Some(snapshot) = &form.snapshot {
                    form.owner = snapshot.owner.clone();
                    form.group = snapshot.group.clone();
                    form.acl = snapshot.acl.clone();
                    form.error.clear();
                }
            }
            "security-apply" => {
                if let Some(snapshot) = &form.snapshot {
                    let op = Operation::Security {
                        expected: snapshot.clone(),
                        change: Change {
                            owner: form.owner.clone(),
                            group: form.group.clone(),
                            acl: form.acl.clone(),
                        },
                    };
                    self.app.start_job(op, &self.ctx);
                    form.saving = self.app.job.is_some();
                }
            }
            _ => return false,
        }
        true
    }
}
