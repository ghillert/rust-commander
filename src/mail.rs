//! Reviewed MIME attachments and explicit SMTP delivery, never queued or retried.
use crate::{
    drag_drop::Selection,
    filesystem::{self, Location},
    operations::Task,
};
use anyhow::{Context, Result, ensure};
use lettre::{
    Message, SmtpTransport, Transport,
    message::{Attachment, Mailbox, MultiPart, SinglePart, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Combined limit on raw attachment contents in one email, in bytes (25 MiB).
pub const ATTACHMENT_LIMIT: usize = 25 * 1024 * 1024;
/// SMTP connection and sender settings used to compose and send email.
///
/// The password is supplied separately to [`send`].
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// SMTP server hostname. Supply the port separately in [`Self::port`].
    pub host: String,
    /// TCP port used to connect to the SMTP server.
    pub port: u16,
    /// Use TLS from the start of the connection. When false, the transport requires
    /// STARTTLS.
    pub implicit_tls: bool,
    /// SMTP login name. An empty value selects delivery without authentication.
    pub username: String,
    /// Sender mailbox used in the message's `From` header.
    pub sender: String,
}
impl Settings {
    /// Checks the SMTP server, port, optional username, and sender mailbox before composing
    /// or sending mail.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid hostname, zero port, invalid username, or
    /// unparseable sender.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.host.is_empty()
                && self.host.len() <= 253
                && !self.host.contains(['/', '\\', '@', ':'])
                && !self.host.chars().any(char::is_whitespace),
            "Enter an SMTP hostname without a scheme or port"
        );
        ensure!(self.port > 0, "Enter an SMTP port between 1 and 65535");
        ensure!(
            self.username.len() <= 1024 && !self.username.contains(['\r', '\n', '\0']),
            "Invalid SMTP username"
        );
        mailbox(&self.sender)?;
        Ok(())
    }
}
/// One attachment with its display name and raw contents.
#[derive(Clone)]
pub struct File {
    /// Attachment name relative to the selection, with any directory components flattened
    /// when constructing the email.
    pub name: String,
    /// Complete contents of this attachment before MIME encoding.
    pub bytes: Vec<u8>,
}
/// Files gathered and checked from the pane selection by [`prepare`]. Pass them to
/// [`message`] to compose an email.
#[derive(Clone)]
pub struct Prepared {
    /// Attachments in filename order, including files selected through directories.
    pub files: Vec<File>,
}
/// Collects regular files from a local, remote, or archive selection into attachments
/// sorted by name. Expands directories, checks for changes, and allows up to 100 files and
/// 25 MiB in total.
///
/// # Errors
///
/// Returns an error for an empty attachment set, a link, a changed or unreadable source, an
/// exceeded traversal or attachment limit, or cancellation.
pub fn prepare(selection: &Selection, task: &Task) -> Result<Prepared> {
    selection.validate()?;
    let archive_stamp = match &selection.from {
        Location::Archive { file, .. } => crate::history::fingerprint(file, task)?,
        _ => None,
    };
    let mut pending = selection
        .entries
        .iter()
        .cloned()
        .map(|entry| {
            let name = entry.name.clone();
            (entry, name, 0)
        })
        .collect::<Vec<_>>();
    let mut files = Vec::new();
    let mut total = 0usize;
    let mut visited = 0usize;
    while let Some((entry, name, depth)) = pending.pop() {
        task.check()?;
        visited += 1;
        ensure!(
            depth < 64 && visited <= 5000,
            "Email selection exceeds 5000 entries or 64 levels"
        );
        ensure!(
            !entry.is_symlink,
            "Email attachments do not follow links; select regular files"
        );
        if entry.is_dir {
            for child in filesystem::list(&crate::directory_tree::at(&selection.from, &entry.path))?
            {
                let relative = format!("{name}/{}", child.name);
                pending.push((child, relative, depth + 1));
            }
            continue;
        }
        ensure!(
            files.len() < 100,
            "Email supports at most 100 file attachments"
        );
        if entry.size_known {
            ensure!(
                entry.size <= (ATTACHMENT_LIMIT - total) as u64,
                "Email attachments exceed 25 MiB"
            );
        }
        if matches!(selection.from, Location::Local(_)) {
            let metadata = std::fs::symlink_metadata(&entry.path)?;
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "Email attachment is not a regular file"
            );
        }
        let bytes = crate::viewer::read(
            &selection.from,
            &entry.path,
            (ATTACHMENT_LIMIT - total) as u64,
        )?;
        task.check()?;
        ensure!(
            crate::viewer::read(
                &selection.from,
                &entry.path,
                (ATTACHMENT_LIMIT - total) as u64
            )? == bytes,
            "Attachment changed while preparing; reopen Email files"
        );
        total += bytes.len();
        ensure!(total <= ATTACHMENT_LIMIT, "Email attachments exceed 25 MiB");
        files.push(File { name, bytes });
    }
    if let Location::Archive { file, .. } = &selection.from {
        ensure!(
            crate::history::fingerprint(file, task)? == archive_stamp,
            "Archive changed while preparing attachments"
        );
    }
    ensure!(
        !files.is_empty(),
        "Select at least one regular file to email"
    );
    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Prepared { files })
}
fn mailbox(value: &str) -> Result<Mailbox> {
    ensure!(
        value.len() <= 1024 && !value.contains(['\r', '\n', '\0']),
        "Invalid email address"
    );
    value.parse().context("Enter a valid email address")
}
/// Builds a multipart email with a plain-text body and the supplied attachments. Accepts
/// one to 50 comma- or semicolon-separated recipients. Directory separators in attachment
/// names are flattened for display.
///
/// # Errors
///
/// Returns an error for invalid addresses or header fields, invalid attachment names, or
/// attachments exceeding the count or byte limits.
pub fn message(
    settings: &Settings,
    to: &str,
    subject: &str,
    body: &str,
    files: &[File],
) -> Result<Message> {
    ensure!(
        to.len() <= 8192
            && subject.len() <= 1024
            && body.len() <= 65536
            && !subject.contains(['\r', '\n', '\0']),
        "Email fields exceed their limits or contain invalid header characters"
    );
    let recipients = to
        .split(';')
        .flat_map(|s| s.split(','))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(mailbox)
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        !recipients.is_empty() && recipients.len() <= 50,
        "Enter between 1 and 50 recipients, separated by commas"
    );
    ensure!(
        !files.is_empty() && files.len() <= 100,
        "Select between 1 and 100 attachments"
    );
    ensure!(
        files
            .iter()
            .try_fold(0usize, |sum, file| sum.checked_add(file.bytes.len()))
            .is_some_and(|sum| sum <= ATTACHMENT_LIMIT),
        "Email attachments exceed 25 MiB"
    );
    let mut builder = Message::builder()
        .from(mailbox(&settings.sender)?)
        .subject(subject);
    for recipient in recipients {
        builder = builder.to(recipient);
    }
    let mut multipart = MultiPart::mixed().singlepart(SinglePart::plain(body.to_owned()));
    for file in files {
        ensure!(
            !file.name.is_empty()
                && file.name.len() <= 4096
                && !file.name.contains(['\r', '\n', '\0']),
            "Invalid attachment name"
        );
        multipart =
            multipart.singlepart(Attachment::new(file.name.replace(['/', '\\'], " - ")).body(
                file.bytes.clone(),
                ContentType::parse("application/octet-stream")?,
            ));
    }
    Ok(builder.multipart(multipart)?)
}
/// Submits the message to the configured SMTP server over implicit TLS or STARTTLS. Uses
/// the supplied password if a username is configured.
///
/// # Errors
///
/// Returns an error for invalid settings or credentials, or if delivery cannot be
/// confirmed. A delivery error can have an uncertain outcome and is not automatically
/// retried.
pub fn send(settings: &Settings, password: String, message: Message) -> Result<()> {
    settings.validate()?;
    ensure!(password.len() <= 4096, "SMTP password is too long");
    ensure!(
        settings.username.is_empty() || !password.is_empty(),
        "Enter the SMTP password for this send"
    );
    let mut transport = if settings.implicit_tls {
        SmtpTransport::relay(&settings.host)?
    } else {
        SmtpTransport::starttls_relay(&settings.host)?
    }
    .port(settings.port)
    .timeout(Some(Duration::from_secs(30)));
    if !settings.username.is_empty() {
        transport = transport.credentials(Credentials::new(settings.username.clone(), password));
    }
    transport.build().send(&message).map_err(|_|anyhow::anyhow!("SMTP delivery could not be confirmed. Check your mailbox/server before retrying; this message was not retried automatically."))?;
    Ok(())
}
