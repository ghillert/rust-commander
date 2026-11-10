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
    pub browse_server: bool,
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
    server_credentials: Option<Secrets>,
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
            browse_server: false,
        }
    }
    pub(super) fn open_container(&mut self, entry: Entry, ctx: &UiSignal) {
        let target = match crate::uri_backend::docker::target_from_entry(&entry.path) {
            Ok(target) => target,
            Err(error) => {
                self.fail(error);
                return;
            }
        };
        use sha2::{Digest, Sha256};
        let id = format!(
            "docker-{:x}",
            Sha256::digest(entry.path.as_os_str().as_encoded_bytes())
        );
        let profile = Connection {
            id: id.clone(),
            name: entry.name,
            service: "docker".into(),
            listed: false,
            docker: Some(target),
            ..Default::default()
        };
        crate::remote::register_profiles(std::slice::from_ref(&profile));
        self.connections.retain(|c| c.id != id);
        self.connections.push(profile);
        self.tab_mut().navigate(
            Location::Remote {
                connection: id,
                path: PathBuf::new(),
            },
            ctx,
        );
    }

    pub(super) fn open_network_entry(&mut self, entry: Entry, ctx: &UiSignal) {
        self.tab_mut().stop_network_scan();
        if self.tab().location == (Location::Network { server: None }) && entry.path.is_absolute() {
            self.tab_mut().navigate(Location::Local(entry.path), ctx);
            return;
        }
        let mut ticket = self.uri_ticket(PathBuf::new());
        let Location::Network { server } = &self.tab().location else {
            return;
        };
        let connection = if let Some(id) = server {
            let Some(mut profile) = crate::remote::profile(id) else {
                self.fail("Reopen the server from Network.");
                return;
            };
            profile.network_parent = Some(id.clone());
            if profile.service == "smb" {
                profile.credentials_from = id.clone();
            }
            profile.name = entry.name.clone();
            if profile.service == "smb" {
                profile.bucket = entry.name;
            } else {
                profile.root = entry.name;
            }
            profile.id = self
                .connections
                .iter()
                .find(|c| {
                    !c.listed
                        && c.network_parent.as_ref() == Some(id)
                        && c.bucket == profile.bucket
                        && c.root == profile.root
                })
                .map(|c| c.id.clone())
                .unwrap_or_else(|| Connection::default().id);
            profile
        } else {
            let endpoint = entry.path.to_string_lossy();
            let service = match endpoint.split_once("://").map(|(service, _)| service) {
                Some("smb") => "smb",
                Some("nfs") => "nfs",
                _ => {
                    self.navigate_input(self.active, &endpoint, ctx);
                    return;
                }
            };
            ticket.browse_server = true;
            self.connections
                .iter()
                .rev()
                .find(|c| c.id.starts_with("network-") && c.endpoint == endpoint)
                .cloned()
                .unwrap_or_else(|| Connection {
                    name: entry.name,
                    service: service.into(),
                    endpoint: endpoint.into_owned(),
                    listed: false,
                    ..Default::default()
                })
        };
        self.start_uri(ticket, connection, Secrets::default(), false, false, ctx);
    }

    pub(super) fn navigate_input(&mut self, side: usize, text: &str, ctx: &UiSignal) {
        self.active = side;
        let result = uri::parse(text, &self.tab().location);
        match result {
            Ok(uri::Input::Location(location)) => {
                if let Location::Remote { connection, path } = &location
                    && let Some(c) = crate::remote::profile(connection)
                    && c.service != "docker"
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
                    self.tab_mut().filter.clear();
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
        mut connection: Connection,
        mut secrets: Secrets,
        remember: bool,
        authorize: bool,
        ctx: &UiSignal,
    ) {
        if ticket.browse_server {
            // Credentials for distinct SMB accounts must remain separate, even in
            // two panes browsing the same server at the same time.
            use sha2::{Digest, Sha256};
            let mut identity = Sha256::new();
            identity.update(connection.endpoint.as_bytes());
            identity.update([0]);
            identity.update(connection.username.as_bytes());
            connection.id = format!("network-{:x}", identity.finalize());
        }
        secrets.request_path = crate::remote::display_path(&ticket.path);
        if connection.service == "sftp" {
            let accepted = secrets.ssh_review.as_ref().and_then(|r| r.accepted.clone());
            secrets.ssh_review = Some(crate::ssh::Review {
                accepted,
                ..Default::default()
            });
        }
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
            if !ticket.browse_server {
                crate::remote::register_profiles(std::slice::from_ref(&connection));
                tab.address = uri::address(&connection.id, &ticket.path);
            }
        }
        let worker_connection = connection.clone();
        let credential_source = if connection.credentials_from.is_empty() {
            connection.id.clone()
        } else {
            connection.credentials_from.clone()
        };
        let stored = !ticket.browse_server
            && !connection.uses_local_identity()
            && self.connections.iter().any(|c| c.id == credential_source);
        let remember = remember && !ticket.browse_server && !connection.uses_local_identity();
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
                let mut secrets = crate::remote::navigation_credentials(
                    &credential_source,
                    worker_secrets.clone(),
                    stored,
                )?;
                if worker_ticket.browse_server {
                    let entries = crate::discovery::server_entries(&worker_connection, &secrets)?;
                    anyhow::ensure!(
                        !worker_cancel.load(Ordering::Relaxed),
                        "Connection cancelled."
                    );
                    return Ok(Resolved {
                        location: Location::Network {
                            server: Some(worker_connection.id.clone()),
                        },
                        entries,
                        cursor: None,
                        server_credentials: (worker_connection.service == "smb").then_some(secrets),
                    });
                }
                anyhow::ensure!(
                    worker_connection.service != "smb" || !worker_connection.bucket.is_empty(),
                    "Enter an SMB share name."
                );
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
                    server_credentials: None,
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
                            if request.ticket.browse_server {
                                if let Some(secrets) = resolved.server_credentials
                                    && let Err(error) = crate::remote::keep_server_credentials(
                                        &request.connection.id,
                                        secrets,
                                    )
                                {
                                    tab.error = Some(error.to_string());
                                    continue;
                                }
                                crate::remote::register_profiles(std::slice::from_ref(
                                    &request.connection,
                                ));
                            }
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
                            tab.focus_address = !request.ticket.browse_server;
                            if (!request.ticket.browse_server
                                || request.connection.service == "smb")
                                && self
                                    .connection_form
                                    .as_ref()
                                    .is_none_or(|f| f.navigation.as_ref() == Some(&request.ticket))
                            {
                                let ssh_challenge =
                                    request.secrets.ssh_review.as_ref().and_then(|r| {
                                        r.offered.lock().unwrap_or_else(|e| e.into_inner()).take()
                                    });
                                self.connection_form = Some(ConnectionForm {
                                    connection: request.connection,
                                    secrets: std::mem::take(&mut request.secrets),
                                    navigation: Some(request.ticket),
                                    error: if ssh_challenge.is_some() {
                                        None
                                    } else {
                                        Some(error)
                                    },
                                    ssh_challenge,
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
                .find(|c| &c.id == connection && c.service != "docker")
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
    #[test]
    #[ignore = "requires the disposable Samba fixture on RUST_COMMANDER_TEST_SMB_PORT"]
    fn network_smb_authenticates_lists_and_opens_a_share() {
        let port: u16 = std::env::var("RUST_COMMANDER_TEST_SMB_PORT")
            .unwrap()
            .parse()
            .unwrap();
        let (_root, mut app, ctx) = app();
        let network = Location::Network { server: None };
        let server = Entry {
            name: "Fixture SMB server".into(),
            path: format!("smb://127.0.0.1:{port}").into(),
            is_dir: true,
            is_symlink: false,
            size: 0,
            size_known: false,
            modified: None,
        };
        app.tab_mut()
            .navigate_loaded(network, vec![server.clone()], None, &ctx);
        assert!(app.tab().chosen(false).is_empty());
        let other = app.panes[1].tab().location.clone();

        fn finish(app: &mut Commander, ctx: &UiSignal) {
            let deadline = std::time::Instant::now() + Duration::from_secs(35);
            while !app.pending_uris.is_empty() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "SMB fixture timed out"
                );
                app.poll_uris(ctx);
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        app.open_entry(server, &ctx);
        app.active = 1;
        finish(&mut app, &ctx);
        assert_eq!(app.tab().location, other);
        let mut form = app
            .connection_form
            .take()
            .expect("Protected SMB server needs credentials");
        assert!(form.navigation.as_ref().unwrap().browse_server);
        form.connection.username = "commander".into();
        form.secrets.password = "commander-test-only".into();
        app.connect_form(&mut form, false, &ctx);
        app.connection_form = Some(form);
        finish(&mut app, &ctx);
        assert!(
            app.connection_form.is_none(),
            "{:?}",
            app.panes[0].tab().error
        );
        assert_eq!(app.tab().location, other);

        app.active = 0;
        let server = app.tab().location.clone();
        assert!(matches!(server, Location::Network { server: Some(_) }));
        let share = app
            .tab()
            .entries
            .iter()
            .find(|row| row.name == "commander")
            .unwrap()
            .clone();
        assert!(
            filesystem::list(&server)
                .unwrap()
                .iter()
                .any(|row| row.name == share.name)
        );
        app.open_entry(share, &ctx);
        finish(&mut app, &ctx);
        assert!(
            matches!(app.tab().location, Location::Remote { .. }),
            "{:?}",
            app.tab().error
        );
        assert_eq!(app.tab().location.parent(), Some(server));
    }

    #[test]
    #[ignore = "requires disposable Linux containers in RUST_COMMANDER_DOCKER_FIXTURES"]
    fn docker_browses_live_and_snapshot_containers_in_either_pane() {
        use crate::uri_backend::docker;

        let ids: [String; 3] =
            serde_json::from_str(&std::env::var("RUST_COMMANDER_DOCKER_FIXTURES").unwrap())
                .unwrap();
        let (root, mut app, ctx) = app();

        fn wait(tab: &mut Tab) {
            let deadline = std::time::Instant::now() + Duration::from_secs(125);
            while tab.loading {
                assert!(
                    std::time::Instant::now() < deadline,
                    "Docker fixture timed out"
                );
                tab.poll(false);
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(tab.error.is_none(), "{:?}", tab.error);
        }

        let mut leases = Vec::new();
        for (index, id) in ids.iter().enumerate() {
            app.active = index % 2;
            app.tab_mut().navigate(Location::Containers, &ctx);
            wait(app.tab_mut());
            let row = app
                .tab()
                .entries
                .iter()
                .find(|row| {
                    docker::target_from_entry(&row.path).is_ok_and(|target| &target.container == id)
                })
                .unwrap()
                .clone();
            let target = docker::target_from_entry(&row.path).unwrap();
            let lease = docker::attach(&target);
            leases.push(Arc::downgrade(&lease));
            app.open_entry(row, &ctx);
            wait(app.tab_mut());
            let location = app.tab().location.clone();
            let Location::Remote { connection, .. } = &location else {
                panic!("Expected Docker file pane")
            };
            assert!(!crate::remote::profile(connection).unwrap().writable());
            assert_eq!(location.parent(), Some(Location::Containers));
            let status = app.tab().docker_status().unwrap();
            if index == 0 {
                assert_eq!(status, "Live");
            } else {
                assert!(status.starts_with("Snapshot"), "{status}");
            }
            let content =
                crate::viewer::read(&location, Path::new("tmp/rc-docker.txt"), 1024).unwrap();
            assert_eq!(content, b"fixture content\n");
            let mount = crate::remote::list(connection, Path::new("mnt/rc-test"));
            if index == 0 {
                assert!(
                    mount
                        .unwrap()
                        .iter()
                        .any(|entry| entry.name == "mounted.txt")
                );
                let denied = crate::remote::list(connection, Path::new("root")).unwrap_err();
                assert!(format!("{denied:#}").contains("Permission denied"));
                assert_eq!(app.tab().docker_status().as_deref(), Some("Live"));
                let entries = crate::remote::list(connection, Path::new("tmp")).unwrap();
                let entry = entries
                    .into_iter()
                    .find(|entry| entry.name == "rc-docker.txt")
                    .unwrap();
                crate::remote::transfer(
                    &location,
                    &[entry],
                    &Location::Local(root.path().into()),
                    false,
                    &mut crate::operations::Task::default(),
                )
                .unwrap();
                assert_eq!(
                    std::fs::read(root.path().join("rc-docker.txt")).unwrap(),
                    content
                );
            } else {
                assert!(mount.is_err(), "Mounted paths must be hidden in snapshots");
            }

            // Change only containers explicitly created by the disposable fixture.
            let docker = |args: &[&str]| {
                std::process::Command::new("docker")
                    .env_remove("DOCKER_CONTEXT")
                    .env_remove("DOCKER_HOST")
                    .args(["--host", &target.endpoint])
                    .args(args)
                    .output()
                    .unwrap()
            };
            let label = docker(&[
                "inspect",
                "--format",
                "{{index .Config.Labels \"io.github.ghillert.rust-commander.docker-fixture\"}}",
                id,
            ]);
            assert!(label.status.success() && label.stdout.starts_with(b"true"));
            let updated = root.path().join("refreshed.txt");
            std::fs::write(&updated, b"refreshed fixture\n").unwrap();
            let copy = docker(&[
                "cp",
                updated.to_str().unwrap(),
                &format!("{id}:/tmp/rc-docker.txt"),
            ]);
            assert!(
                copy.status.success(),
                "{}",
                String::from_utf8_lossy(&copy.stderr)
            );
            if index != 0 {
                assert_eq!(
                    crate::viewer::read(&location, Path::new("tmp/rc-docker.txt"), 1024).unwrap(),
                    content
                );
            }
            app.tab_mut().refresh_docker(&ctx);
            wait(app.tab_mut());
            assert_eq!(app.tab().location, location);
            assert_eq!(
                crate::viewer::read(&location, Path::new("tmp/rc-docker.txt"), 1024).unwrap(),
                b"refreshed fixture\n"
            );
            assert_eq!(
                app.tab().docker_status().unwrap().starts_with("Snapshot"),
                index != 0
            );
        }
        let missing = crate::remote::DockerTarget {
            endpoint: docker::target_from_entry(
                &docker::list_containers(&Default::default()).unwrap()[0].path,
            )
            .unwrap()
            .endpoint,
            container: "0".repeat(64),
        };
        assert!(
            docker::attach(&missing)
                .entries(Path::new(""), false, &Default::default())
                .is_err()
        );
        let offline = crate::remote::DockerTarget {
            endpoint: format!("unix://{}", root.path().join("missing.sock").display()),
            ..missing
        };
        assert!(
            docker::attach(&offline)
                .entries(Path::new(""), false, &Default::default())
                .is_err()
        );
        drop(app);
        assert!(leases.iter().all(|lease| lease.upgrade().is_none()));
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
                server_credentials: None,
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
                server_credentials: None,
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
