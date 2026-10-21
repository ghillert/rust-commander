//! URI resolution belongs to the initiating tab, never whichever pane is active later.
use super::*;
use crate::{
    remote::{Connection, Secrets},
    uri,
};
use anyhow::Context;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Ticket {
    pub side: usize,
    pub tab: u64,
    pub epoch: u64,
    pub path: PathBuf,
}
pub(super) struct Pending {
    ticket: Ticket,
    connection: Connection,
    secrets: Secrets,
    cancel: Arc<AtomicBool>,
    receiver: Receiver<Result<Resolved, String>>,
}
struct Resolved {
    location: Location,
    entries: Vec<Entry>,
    cursor: Option<PathBuf>,
}
impl Ticket {
    fn current(&self, app: &Commander) -> bool {
        app.panes[self.side]
            .tabs
            .iter()
            .any(|t| t.id == self.tab && t.navigation_epoch == self.epoch)
    }
}

impl Commander {
    pub(super) fn uri_ticket(&mut self, path: PathBuf) -> Ticket {
        let side = self.active;
        let tab = self.tab_mut();
        tab.navigation_epoch = tab.navigation_epoch.wrapping_add(1);
        Ticket {
            side,
            tab: tab.id,
            epoch: tab.navigation_epoch,
            path,
        }
    }
    pub(super) fn navigate_input(&mut self, side: usize, text: &str, ctx: &UiSignal) {
        self.active = side;
        let result = uri::parse(text, &self.tab().location);
        match result {
            Ok(uri::Input::Location(location)) => {
                if let Location::Remote { connection, path } = &location
                    && let Some(c) = crate::remote::profile(connection)
                {
                    let ticket = self.uri_ticket(path.clone());
                    self.start_uri(ticket, c, Default::default(), false, false, ctx);
                    return;
                }
                let (location, cursor) = if let Location::Local(path) = &location {
                    if path.is_file() {
                        (
                            Location::Local(path.parent().unwrap_or(Path::new("/")).into()),
                            Some(path.clone()),
                        )
                    } else {
                        (location, None)
                    }
                } else {
                    (location, None)
                };
                self.tab_mut().navigate(location, ctx);
                if cursor.is_some() {
                    self.tab_mut().cursor = cursor;
                }
            }
            Ok(uri::Input::Remote(target)) => {
                let mut ticket = self.uri_ticket(target.path.clone());
                let matches = self
                    .connections
                    .iter()
                    .filter(|c| uri::matches_profile(&target, c))
                    .cloned()
                    .collect::<Vec<_>>();
                if matches.len() > 1 {
                    self.connection_form = Some(ConnectionForm {
                        connection: target.connection,
                        secrets: target.secrets,
                        navigation: Some(ticket),
                        choices: matches,
                        ..Default::default()
                    });
                } else {
                    let mut connection = matches
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| target.connection.clone());
                    match uri::relative_to_profile(&ticket.path, &connection) {
                        Ok(path) => ticket.path = path,
                        Err(error) => {
                            self.fail(error);
                            return;
                        }
                    }
                    if !target.secrets.request_query.is_empty()
                        || (!target.secrets.password.is_empty() && connection.service == "sftp")
                    {
                        connection.credentials_from = connection.id.clone();
                        connection.id = target.connection.id.clone();
                        connection.listed = false;
                        if !target.secrets.password.is_empty() && connection.service == "sftp" {
                            connection.key_file.clear();
                            connection.native_sftp = true;
                        }
                    }
                    connection.query_required = !target.secrets.request_query.is_empty();
                    self.start_uri(ticket, connection, target.secrets, false, false, ctx);
                }
            }
            Err(error) => {
                self.tab_mut().navigation_epoch = self.tab().navigation_epoch.wrapping_add(1);
                self.tab_mut().error = Some(error.to_string());
                self.tab_mut().focus_address = true;
                self.tab_mut().loading = false;
            }
        }
    }
    pub(super) fn start_uri(
        &mut self,
        ticket: Ticket,
        connection: Connection,
        mut secrets: Secrets,
        remember: bool,
        authorize: bool,
        ctx: &UiSignal,
    ) {
        secrets.request_path = crate::remote::display_path(&ticket.path);
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        if let Some(tab) = self.panes[ticket.side]
            .tabs
            .iter_mut()
            .find(|t| t.id == ticket.tab)
        {
            tab.loading = true;
            tab.error = None;
            // Passwords and signed queries disappear from the editable address immediately.
            crate::remote::register_profiles(std::slice::from_ref(&connection));
            tab.address = uri::address(&connection.id, &ticket.path);
        }
        let worker_connection = connection.clone();
        let credential_source = if connection.credentials_from.is_empty() {
            connection.id.clone()
        } else {
            connection.credentials_from.clone()
        };
        let stored = self.connections.iter().any(|c| c.id == credential_source);
        let worker_secrets = secrets.clone();
        let worker_ticket = ticket.clone();
        let worker_cancel = cancel.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<Resolved> {
                anyhow::ensure!(
                    !worker_cancel.load(Ordering::Relaxed),
                    "Connection cancelled."
                );
                anyhow::ensure!(
                    worker_connection.service != "smb" || !worker_connection.bucket.is_empty(),
                    "Enter an SMB share name."
                );
                let mut secrets = crate::remote::navigation_credentials(
                    &credential_source,
                    worker_secrets.clone(),
                    stored,
                )?;
                if authorize {
                    secrets = crate::oauth::authorize(
                        &worker_connection,
                        secrets,
                        worker_cancel.clone(),
                    )?;
                }
                crate::remote::connect(&worker_connection, secrets.clone(), false)
                    .map_err(|e| crate::remote::clean_error(e, &secrets))?;
                anyhow::ensure!(
                    !worker_cancel.load(Ordering::Relaxed),
                    "Connection cancelled."
                );
                let mut location = Location::Remote {
                    connection: worker_connection.id.clone(),
                    path: worker_ticket.path.clone(),
                };
                let mut cursor = None;
                let entries = match filesystem::list(&location) {
                    Ok(entries) => entries,
                    Err(error) if worker_connection.service != "http" => {
                        if let Some(entry) = crate::remote::entry_at(&location, &worker_ticket.path)
                            .ok()
                            .flatten()
                            .filter(|e| !e.is_dir)
                        {
                            cursor = Some(entry.path);
                            location = location
                                .parent()
                                .context("Remote file has no parent folder")?;
                            filesystem::list(&location)?
                        } else {
                            return Err(error);
                        }
                    }
                    Err(error) => return Err(error),
                };
                anyhow::ensure!(
                    !worker_cancel.load(Ordering::Relaxed),
                    "Connection cancelled."
                );
                if remember {
                    crate::remote::remember_session(&worker_connection.id)?;
                }
                Ok(Resolved {
                    location,
                    entries,
                    cursor,
                })
            })()
            .map_err(|e| crate::remote::clean_error(e, &worker_secrets).to_string());
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.pending_uris.push(Pending {
            ticket,
            connection,
            secrets,
            cancel,
            receiver,
        });
    }
    pub(super) fn uri_connecting(&self) -> bool {
        self.connection_form
            .as_ref()
            .and_then(|f| f.navigation.as_ref())
            .is_some_and(|t| self.pending_uris.iter().any(|p| &p.ticket == t))
    }
    pub(super) fn poll_uris(&mut self, ctx: &UiSignal) {
        let mut pending = std::mem::take(&mut self.pending_uris);
        for mut request in pending.drain(..) {
            if !request.ticket.current(self) {
                request.cancel.store(true, Ordering::Relaxed);
                // Moving a tab to the other pane cancels its in-flight URI too.
                // Do not clear the spinner belonging to a newer navigation.
                for pane in &mut self.panes {
                    for tab in &mut pane.tabs {
                        if tab.id == request.ticket.tab
                            && tab.navigation_epoch == request.ticket.epoch
                        {
                            tab.loading = false;
                        }
                    }
                }
                continue;
            }
            match request.receiver.try_recv() {
                Err(mpsc::TryRecvError::Empty) => self.pending_uris.push(request),
                result => {
                    let result = result.unwrap_or_else(|_| {
                        Err("Connection worker stopped. Retry the address.".into())
                    });
                    let tab = self.panes[request.ticket.side]
                        .tabs
                        .iter_mut()
                        .find(|t| t.id == request.ticket.tab)
                        .unwrap();
                    tab.loading = false;
                    match result {
                        Ok(resolved) => {
                            tab.navigate_loaded(
                                resolved.location,
                                resolved.entries,
                                resolved.cursor,
                                ctx,
                            );
                            self.connections.retain(|c| c.id != request.connection.id);
                            self.connections.push(request.connection);
                            if self
                                .connection_form
                                .as_ref()
                                .is_some_and(|f| f.navigation.as_ref() == Some(&request.ticket))
                            {
                                self.connection_form = None;
                            }
                            self.save_settings();
                        }
                        Err(error) => {
                            tab.error = Some(error.clone());
                            tab.focus_address = true;
                            if self
                                .connection_form
                                .as_ref()
                                .is_none_or(|f| f.navigation.as_ref() == Some(&request.ticket))
                            {
                                self.connection_form = Some(ConnectionForm {
                                    connection: request.connection,
                                    secrets: std::mem::take(&mut request.secrets),
                                    navigation: Some(request.ticket),
                                    error: Some(error),
                                    ..Default::default()
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    pub(super) fn cancel_uri_form(&mut self) {
        if let Some(ticket) = self
            .connection_form
            .as_ref()
            .and_then(|f| f.navigation.as_ref())
        {
            for request in &self.pending_uris {
                if &request.ticket == ticket {
                    request.cancel.store(true, Ordering::Relaxed);
                }
            }
            if let Some(tab) = self.panes[ticket.side]
                .tabs
                .iter_mut()
                .find(|t| t.id == ticket.tab)
            {
                tab.navigation_epoch = tab.navigation_epoch.wrapping_add(1);
                tab.loading = false;
            }
        }
    }
    pub(super) fn cancel_uri_navigation(&mut self, side: usize) {
        let tab = self.panes[side].tab_mut();
        if self.pending_uris.iter().any(|p| {
            p.ticket.side == side
                && p.ticket.tab == tab.id
                && p.ticket.epoch == tab.navigation_epoch
        }) {
            tab.navigation_epoch = tab.navigation_epoch.wrapping_add(1);
            tab.loading = false;
            tab.address = tab.location.label();
        }
    }
    pub(super) fn save_current_connection(&mut self) {
        if let Location::Remote { connection, path } = &self.tab().location
            && let Some(mut c) = self
                .connections
                .iter()
                .find(|c| &c.id == connection)
                .cloned()
        {
            c.initial_path = path.clone();
            self.connection_form = Some(ConnectionForm {
                connection: c,
                save_only: true,
                ..Default::default()
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> (tempfile::TempDir, Commander, UiSignal) {
        let dir = tempfile::tempdir().unwrap();
        let ctx = UiSignal::default();
        let mut app = Commander::with_settings(&ctx, &[dir.path().into()], Ok(Settings::default()));
        app.settings_valid = false;
        app.queue = Default::default();
        (dir, app, ctx)
    }
    fn completion(app: &mut Commander, ctx: &UiSignal) -> mpsc::Sender<Result<Resolved, String>> {
        let ticket = app.uri_ticket("folder".into());
        let (sender, receiver) = mpsc::channel();
        app.pending_uris.push(Pending {
            ticket,
            connection: Connection::default(),
            secrets: Secrets::default(),
            cancel: Arc::new(AtomicBool::new(false)),
            receiver,
        });
        ctx.request_repaint();
        sender
    }
    #[test]
    fn stale_completions_and_cancellation_never_replace_new_navigation() {
        let (root, mut app, ctx) = app();
        let sender = completion(&mut app, &ctx);
        app.tab_mut()
            .navigate(Location::Local(root.path().join("new")), &ctx);
        sender
            .send(Ok(Resolved {
                location: Location::Local("/obsolete".into()),
                entries: vec![],
                cursor: None,
            }))
            .unwrap_or_else(|_| panic!("Receiver closed"));
        app.poll_uris(&ctx);
        assert_eq!(app.tab().location, Location::Local(root.path().join("new")));
        let _sender = completion(&mut app, &ctx);
        app.cancel_uri_navigation(0);
        app.poll_uris(&ctx);
        assert!(app.pending_uris.is_empty());
    }
    #[test]
    fn completion_targets_the_originating_tab_after_pane_and_tab_switches() {
        let (root, mut app, ctx) = app();
        let original = app.tab().id;
        let sender = completion(&mut app, &ctx);
        app.panes[0]
            .tabs
            .push(Tab::new(Location::Local(root.path().join("other")), &ctx));
        app.panes[0].active = 1;
        app.active = 1;
        sender
            .send(Ok(Resolved {
                location: Location::Local(root.path().join("result")),
                entries: vec![],
                cursor: None,
            }))
            .unwrap_or_else(|_| panic!("Receiver closed"));
        app.poll_uris(&ctx);
        assert_eq!(
            app.panes[0]
                .tabs
                .iter()
                .find(|t| t.id == original)
                .unwrap()
                .location,
            Location::Local(root.path().join("result"))
        );
        assert_eq!(
            app.panes[0].tab().location,
            Location::Local(root.path().join("other"))
        );
        assert_eq!(app.active, 1);
    }
}
