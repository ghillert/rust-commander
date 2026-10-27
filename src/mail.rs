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

pub const ATTACHMENT_LIMIT: usize = 25 * 1024 * 1024;
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub host: String,
    pub port: u16,
    pub implicit_tls: bool,
    pub username: String,
    pub sender: String,
}
impl Settings {
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
#[derive(Clone)]
pub struct File {
    pub name: String,
    pub bytes: Vec<u8>,
}
#[derive(Clone)]
pub struct Prepared {
    pub files: Vec<File>,
}
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
