use super::*;

impl Adapter {
    pub(super) fn connections_dialog(&self, form: &ConnectionForm) -> Dialog {
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
        d.busy = self.app.uri_connecting() || form.discovery.is_some();
        if form
            .navigation
            .as_ref()
            .is_some_and(|ticket| ticket.browse_server)
        {
            d.title = tr("Browse SMB shares").into_owned();
            d.description = format!("{} · {}", form.connection.name, form.connection.endpoint);
            d.buttons[0].1 = "Cancel".into();
            d.fields.extend([
                Field::text("username", "Username", &form.connection.username),
                Field::password("password", "Password", &form.secrets.password),
            ]);
            d.button("accept", "Browse shares", !d.busy);
            return d;
        }
        if form.save_only {
            d.title = tr("Save connection…").into_owned();
            d.description = crate::uri::address(&form.connection.id, &form.connection.initial_path);
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
        if form.navigation.is_none() && form.view != ConnectionView::Edit {
            match form.view {
                ConnectionView::Saved => {
                    d.description = tr("Open a saved connection, browse nearby servers and devices, or add a connection manually.").into_owned();
                    for (i, c) in self
                        .app
                        .connections
                        .iter()
                        .enumerate()
                        .filter(|(_, c)| c.listed)
                    {
                        d.rows.push(
                            Row::new(
                                format!("connection-{i}"),
                                &c.name,
                                format!(
                                    "{} · {}",
                                    tr(service_title(&c.service)),
                                    if c.service == "adb" {
                                        &c.bucket
                                    } else {
                                        &c.endpoint
                                    }
                                ),
                            )
                            .action(format!("connection-open-{i}"), "Open", !d.busy)
                            .action(
                                format!("connection-edit-{i}"),
                                "Edit",
                                !d.busy,
                            ),
                        );
                    }
                    if d.rows.is_empty() {
                        d.fields.push(Field::label("Saved connections", "No saved connections yet. After connecting, use Save connection beside the location bar."));
                    }
                    d.rows.push(
                        Row::new(
                            "network",
                            tr("Network servers"),
                            tr("Find SMB, SFTP, FTP and NFS servers on your local network."),
                        )
                        .action(
                            "discover-network",
                            "Browse network",
                            !d.busy,
                        ),
                    );
                    d.rows.push(
                        Row::new(
                            "android",
                            tr("Android devices"),
                            tr("Browse files on an authorized USB or wireless device."),
                        )
                        .action(
                            "discover-android",
                            "Browse devices",
                            !d.busy,
                        ),
                    );
                    d.button("connection-new", "New connection…", !d.busy);
                }
                ConnectionView::Network | ConnectionView::Android => {
                    let android = form.view == ConnectionView::Android;
                    d.title = tr(if android {
                        "Android devices"
                    } else {
                        "Network servers"
                    })
                    .into_owned();
                    d.description = tr(if android {
                        "Connect and authorize your device, then start the local ADB server. Devices open read-only."
                    } else {
                        "Find servers advertised on your local network. Select one to review its address and sign-in details."
                    }).into_owned();
                    d.fields.push(Field::label(
                        if d.busy { "Scanning…" } else { "Discovery" },
                        if d.busy {
                            tr("Looking for available servers or devices…").into_owned()
                        } else {
                            tr(&form.discovery_status).into_owned()
                        },
                    ));
                    for (i, c) in form.discovered.iter().enumerate() {
                        let id = format!("discovered-{i}");
                        d.rows.push(
                            Row::new(
                                &id,
                                &c.name,
                                format!(
                                    "{} · {}",
                                    tr(service_title(&c.service)),
                                    if android { &c.bucket } else { &c.endpoint }
                                ),
                            )
                            .action(&id, "Set up…", !d.busy),
                        );
                    }
                    d.button("connections-home", "Back", !d.busy);
                    d.button(
                        if android {
                            "discover-android"
                        } else {
                            "discover-network"
                        },
                        "Scan again",
                        !d.busy,
                    );
                }
                ConnectionView::Edit => unreachable!(),
            }
            return d;
        }

        let c = &form.connection;
        let s = &form.secrets;
        let oauth = matches!(c.service.as_str(), "gdrive" | "dropbox" | "onedrive");
        let saved = self
            .app
            .connections
            .iter()
            .enumerate()
            .find(|(_, saved)| saved.listed && saved.id == c.id);
        d.title = tr(if saved.is_some() {
            "Edit connection"
        } else {
            "New connection"
        })
        .into_owned();
        d.description = tr(match c.service.as_str() {
            "sftp" => "Enter a server URL such as sftp://host. Use a password, SSH agent or private key.",
            "smb" => "Connect to a shared folder on a NAS or another computer.",
            "ftp" => "Enter a server URL such as ftp://host and choose the server's security mode.",
            "webdav" => "Connect to a WebDAV folder using its HTTP or HTTPS address.",
            "nfs" => "Enter a server URL such as nfs://host and its full export path. Files open read-only.",
            "adb" => "Browse an authorized Android device read-only through the local ADB server.",
            "s3" => "Connect to an Amazon S3 or S3-compatible bucket.",
            "gcs" => "Connect to a Google Cloud Storage bucket using service account credentials.",
            "http" => "Browse a web directory or download a file. HTTP connections are read-only.",
            "webhdfs" => "Connect to Hadoop storage through a WebHDFS endpoint.",
            _ => "Sign in with your browser using your OAuth app, or provide an access token in Advanced options.",
        }).into_owned();
        d.buttons[0].1 = "Cancel".into();
        if form.navigation.is_none() {
            d.button("connections-back", "Back", !d.busy);
        }
        d.fields.push(Field::choice(
            "service",
            "Connection type",
            tr(service_title(&c.service)),
            crate::remote::SERVICES
                .iter()
                .map(|id| tr(service_title(id)).into_owned()),
        ));
        if !oauth && c.service != "adb" {
            d.fields.push(Field::text(
                "endpoint",
                if matches!(c.service.as_str(), "s3" | "gcs") {
                    "Endpoint (optional)"
                } else {
                    "Server URL"
                },
                &c.endpoint,
            ));
        }
        if matches!(c.service.as_str(), "smb" | "s3" | "gcs" | "adb") {
            d.fields.push(Field::text(
                "bucket",
                match c.service.as_str() {
                    "smb" => "Share name",
                    "adb" => "Device serial",
                    _ => "Bucket",
                },
                &c.bucket,
            ));
        }
        if c.service == "s3" {
            d.fields.extend([
                Field::text("region", "Region", &c.region),
                Field::password("access-key", "Access key ID", &s.access_key_id),
                Field::password("secret-key", "Secret access key", &s.secret_access_key),
            ]);
        }
        if c.service == "gcs" {
            d.fields.push(Field::password(
                "token",
                "Credentials (base64)",
                &s.access_token,
            ));
        }
        if matches!(
            c.service.as_str(),
            "sftp" | "smb" | "ftp" | "webdav" | "webhdfs"
        ) {
            d.fields
                .push(Field::text("username", "Username", &c.username));
            if c.service != "webhdfs" {
                d.fields
                    .push(Field::password("password", "Password", &s.password));
            }
        }
        if c.service == "ftp" {
            d.fields.push(Field::choice(
                "ftp-tls",
                "Security",
                tr(match c.tls_mode() {
                    crate::remote::FtpTls::Plain => "Plain FTP",
                    crate::remote::FtpTls::Implicit => "Implicit TLS",
                    _ => "Explicit TLS",
                }),
                ["Plain FTP", "Explicit TLS", "Implicit TLS"].map(|s| tr(s).into_owned()),
            ));
        }
        if oauth {
            d.fields.extend([
                Field::text("client-id", "OAuth client ID", &c.oauth_client_id),
                Field::password(
                    "client-secret",
                    "Client secret (optional)",
                    &s.oauth_client_secret,
                ),
            ]);
            let host = if c.service == "onedrive" {
                "localhost"
            } else {
                "127.0.0.1"
            };
            d.fields.push(Field::label(
                "Redirect URI",
                format!("http://{host}:{}/callback", c.oauth_port),
            ));
            d.button("authorize", "Sign in with browser", !d.busy);
        }
        d.fields.push(Field::text(
            "root",
            if c.service == "nfs" {
                "Export path"
            } else {
                "Starting folder"
            },
            &c.root,
        ));
        if !c.uses_local_identity() {
            d.fields.push(Field::check(
                "remember",
                "Remember credentials",
                form.remember,
            ));
        }
        // Less common settings are retained when collapsed; hiding never resets a value.
        d.fields.push(Field {
            kind: 8,
            ..Field::check("connection-advanced", "Advanced options", form.advanced)
        });
        if form.advanced {
            d.fields
                .push(Field::text("name", "Connection name", &c.name));
            match c.service.as_str() {
                "sftp" => d.fields.extend([
                    Field::text("key-file", "Private key file", &c.key_file),
                    Field::password("ssh-passphrase", "Key passphrase", &s.ssh_passphrase),
                    Field::check(
                        "ssh-config",
                        "Use SSH configuration and host aliases",
                        c.ssh_use_config,
                    ),
                    Field::text("ssh-config-file", "SSH config file", &c.ssh_config_file),
                    Field::text("known-hosts", "Known hosts file", &c.known_hosts_file),
                ]),
                "ftp" => d
                    .fields
                    .push(Field::text("tls-ca", "TLS CA file (DER)", &c.tls_ca_file)),
                "http" => d.fields.extend([
                    Field::text("username", "Username", &c.username),
                    Field::password("password", "Password", &s.password),
                    Field::password("token", "Access token", &s.access_token),
                ]),
                _ => {}
            }
            if oauth {
                d.fields
                    .push(Field::password("token", "Access token", &s.access_token));
                if c.service == "onedrive" {
                    d.fields
                        .push(Field::text("tenant", "Microsoft tenant", &c.oauth_tenant));
                }
                d.fields
                    .push(Field::text("port", "Callback port", c.oauth_port));
            }
            if let Some((i, _)) = saved.filter(|_| !c.uses_local_identity()) {
                d.rows.push(Row::new("credentials", tr("Stored credentials"), tr("Remove the saved sign-in details from the operating system credential store."))
                    .action(format!("connection-forget-{i}"), "Forget credentials", !d.busy));
            }
        }
        if c.service == "http" {
            d.button("open-browser", "Open in browser", !d.busy);
        }
        if matches!(c.service.as_str(), "smb" | "nfs") {
            d.button(
                "browse-server",
                if c.service == "smb" {
                    "Browse shares…"
                } else {
                    "Browse exports…"
                },
                !d.busy && !c.endpoint.trim().is_empty(),
            );
        }
        d.button("accept", "Connect", !d.busy);
        d
    }
}

pub(super) fn service_title(service: &str) -> &str {
    match service {
        "sftp" => "SFTP (SSH)",
        "smb" => "SMB / Windows share",
        "ftp" => "FTP / FTPS",
        "webdav" => "WebDAV",
        "nfs" => "NFS (read-only)",
        "adb" => "Android device (ADB)",
        "s3" => "Amazon S3 / compatible",
        "http" => "HTTP / HTTPS (read-only)",
        "dropbox" => "Dropbox",
        "gdrive" => "Google Drive",
        "onedrive" => "OneDrive",
        "gcs" => "Google Cloud Storage",
        "webhdfs" => "Hadoop (WebHDFS)",
        other => other,
    }
}
