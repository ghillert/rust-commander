//! SMB2/3 access without a mounted share. Handles stay inside worker threads.
use crate::{
    filesystem::Entry,
    operations::Task,
    remote::{Connection, Secrets},
};
use anyhow::{Context, Result};
use std::{io::Read, path::Path, time::Duration};
fn server_address(endpoint: &str) -> Result<(url::Url, String)> {
    let url = if endpoint.contains("://") {
        url::Url::parse(endpoint)?
    } else {
        url::Url::parse(&format!("smb://{endpoint}"))?
    };
    let host = url.host_str().context("Enter an SMB server hostname")?;
    anyhow::ensure!(
        url.scheme() == "smb",
        "Enter an SMB server URL such as smb://nas.local"
    );
    let address = format!("{host}:{}", url.port().unwrap_or(445));
    Ok((url, address))
}

async fn connect_server(
    addr: String,
    username: &str,
    secrets: &Secrets,
) -> Result<smb2::SmbClient> {
    let (domain, username) = username.split_once('\\').unwrap_or(("", username));
    Ok(smb2::SmbClient::connect(smb2::ClientConfig {
        addr,
        username: username.into(),
        domain: domain.into(),
        password: secrets.password.clone(),
        timeout: Duration::from_secs(15),
        ..Default::default()
    })
    .await?)
}

fn run<T>(f: impl std::future::Future<Output = Result<T>>, secrets: &Secrets) -> Result<T> {
    crate::remote::runtime()
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(30), f)
                .await
                .context("SMB request timed out")?
        })
        .map_err(|e| crate::remote::clean_error(e, secrets))
}

pub(crate) fn list_shares(profile: &Connection, secrets: &Secrets) -> Result<Vec<String>> {
    crate::remote::validate(profile)?;
    let (_, address) = server_address(&profile.endpoint)?;
    run(
        async {
            let mut client = connect_server(address, &profile.username, secrets).await?;
            Ok(client
                .list_shares()
                .await?
                .into_iter()
                .filter(|share| {
                    !share.name.ends_with('$')
                        && crate::filesystem::validate_name(&share.name).is_ok()
                })
                .map(|share| share.name)
                .collect())
        },
        secrets,
    )
}

#[derive(Clone)]
pub struct Share {
    profile: Connection,
    secrets: Secrets,
}
impl Share {
    pub fn new(profile: &Connection, secrets: &Secrets) -> Result<Self> {
        let share = Self {
            profile: profile.clone(),
            secrets: secrets.clone(),
        };
        share.address()?;
        Ok(share)
    }
    fn address(&self) -> Result<(String, String)> {
        let (url, address) = server_address(&self.profile.endpoint)?;
        let share = if self.profile.bucket.is_empty() {
            url.path().trim_matches('/').to_owned()
        } else {
            self.profile.bucket.clone()
        };
        crate::filesystem::validate_name(&share)?;
        Ok((address, share))
    }
    fn path(&self, path: &Path) -> Result<String> {
        let root = self.profile.root.trim_matches('/');
        let root = crate::remote::key(Path::new(root), false)?;
        let path = crate::remote::key(path, false)?;
        Ok([root, path]
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("/"))
    }

    async fn connect(&self) -> Result<(smb2::SmbClient, smb2::Tree)> {
        let (addr, share) = self.address()?;
        let mut client = connect_server(addr, &self.profile.username, &self.secrets).await?;
        let tree = client.connect_share(&share).await?;
        Ok((client, tree))
    }
    fn run<T>(&self, f: impl std::future::Future<Output = Result<T>>) -> Result<T> {
        run(f, &self.secrets)
    }
    pub fn list(&self, path: &Path) -> Result<Vec<Entry>> {
        self.run(async {
            let (mut client, mut tree) = self.connect().await?;
            let rows = client.list_directory(&mut tree, &self.path(path)?).await?;
            rows.into_iter()
                .filter(|e| e.name != "." && e.name != "..")
                .map(|e| {
                    crate::filesystem::validate_name(&e.name)?;
                    Ok(Entry {
                        path: path.join(&e.name),
                        name: e.name,
                        is_dir: e.is_directory,
                        is_symlink: false,
                        size_known: true,
                        size: e.size,
                        modified: e.modified.to_system_time(),
                    })
                })
                .collect()
        })
    }
    pub fn stat(&self, path: &Path) -> Result<Option<Entry>> {
        self.run(async {
            let (mut client, mut tree) = self.connect().await?;
            let info = match client.stat(&mut tree, &self.path(path)?).await {
                Ok(i) => i,
                Err(e) if e.kind() == smb2::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e.into()),
            };
            Ok(Some(Entry {
                path: path.into(),
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                is_dir: info.is_directory,
                is_symlink: false,
                size_known: true,
                size: info.size,
                modified: info.modified.to_system_time(),
            }))
        })
    }
    pub fn mkdir(&self, path: &Path) -> Result<()> {
        self.run(async {
            let (mut c, t) = self.connect().await?;
            c.create_directory(&t, &self.path(path)?).await?;
            Ok(())
        })
    }
    pub fn delete(&self, entry: &Entry, task: &mut Task) -> Result<()> {
        task.check()?;
        if entry.is_dir {
            for child in self.list(&entry.path)? {
                self.delete(&child, task)?;
            }
        }
        self.delete_leaf(entry)
    }
    pub(crate) fn delete_leaf(&self, entry: &Entry) -> Result<()> {
        self.run(async {
            let (mut c, t) = self.connect().await?;
            if entry.is_dir {
                c.delete_directory(&t, &self.path(&entry.path)?).await?;
            } else {
                c.delete_file(&t, &self.path(&entry.path)?).await?;
            }
            Ok(())
        })
    }
    pub fn reader(&self, path: &Path) -> Result<Box<dyn Read>> {
        self.reader_at(path, 0)
    }
    pub fn reader_at(&self, path: &Path, offset: u64) -> Result<Box<dyn Read>> {
        let reader = self.run(async {
            let (mut c, t) = self.connect().await?;
            Ok(c.open_file_reader(&t, &self.path(path)?).await?)
        })?;
        Ok(Box::new(Reader {
            reader: Some(reader),
            offset,
        }))
    }
    /// The pinned SMB client sets ReplaceIfExists=false for this rename.
    pub fn publish_checkpoint(&self, stage: &Path, target: &Path) -> Result<()> {
        self.run(async {
            let (mut client, tree) = self.connect().await?;
            client
                .rename(&tree, &self.path(stage)?, &self.path(target)?)
                .await?;
            Ok(())
        })
    }
    pub fn write(
        &self,
        path: &Path,
        input: &mut dyn Read,
        task: &mut Task,
        exclusive: bool,
    ) -> Result<()> {
        let mut writer = self.run(async {
            let (mut c, t) = self.connect().await?;
            let p = self.path(path)?;
            Ok(if exclusive {
                c.create_file_writer_exclusive(&t, &p).await?
            } else {
                c.create_file_writer(&t, &p).await?
            })
        })?;
        let mut buffer = [0; 65536];
        loop {
            task.check()?;
            let n = input.read(&mut buffer[..task.chunk_size()])?;
            if n == 0 {
                break;
            }
            self.run(async {
                writer.write_chunk(&buffer[..n]).await?;
                Ok(())
            })?;
            task.tick(path, n as u64)?;
        }
        self.run(async {
            writer.finish().await?;
            Ok(())
        })
    }
}
struct Reader {
    reader: Option<smb2::client::stream::FileReader>,
    offset: u64,
}
impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let reader = self.reader.as_ref().unwrap();
        let bytes = crate::remote::runtime()
            .block_on(async {
                tokio::time::timeout(
                    Duration::from_secs(30),
                    reader.read_at(self.offset, buf.len().min(65536) as u64),
                )
                .await
            })
            .map_err(std::io::Error::other)?
            .map_err(std::io::Error::other)?;
        buf[..bytes.len()].copy_from_slice(&bytes);
        self.offset += bytes.len() as u64;
        Ok(bytes.len())
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        if let Some(reader) = self.reader.take() {
            let _ = crate::remote::runtime().block_on(async {
                tokio::time::timeout(Duration::from_secs(5), reader.close()).await
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn smb_paths_cannot_escape_the_configured_root() {
        let share = Share::new(
            &Connection {
                service: "smb".into(),
                endpoint: "smb://localhost".into(),
                bucket: "share".into(),
                root: "/inside".into(),
                ..Default::default()
            },
            &Secrets::default(),
        )
        .unwrap();
        assert_eq!(
            share.path(Path::new("folder/file.txt")).unwrap(),
            "inside/folder/file.txt"
        );
        for path in ["../outside", "folder\\..\\outside", "/absolute", "C:drive"] {
            assert!(share.path(Path::new(path)).is_err(), "{path}");
        }
    }
}
