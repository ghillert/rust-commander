use super::{Item, process};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom, Write},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

const MAX_EXPORT: u64 = 4 * 1024 * 1024 * 1024;
const MAX_TOTAL: u64 = 8 * 1024 * 1024 * 1024;
static BYTES: AtomicU64 = AtomicU64::new(0);
static EXPORT: Mutex<()> = Mutex::new(());

#[derive(Default)]
struct Reservation(u64);

impl Reservation {
    fn add(&mut self, bytes: u64) -> Result<()> {
        ensure!(
            bytes <= MAX_EXPORT - self.0,
            "Docker snapshot exceeds the 4 GiB export limit."
        );
        let mut used = BYTES.load(Ordering::Relaxed);
        loop {
            ensure!(
                bytes <= MAX_TOTAL - used,
                "Docker snapshots reached the 8 GiB limit. Close other container tabs and retry."
            );
            match BYTES.compare_exchange_weak(
                used,
                used + bytes,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => used = actual,
            }
        }
        self.0 += bytes;
        Ok(())
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        BYTES.fetch_sub(self.0, Ordering::Relaxed);
    }
}

pub(super) struct Snapshot {
    file: tempfile::NamedTempFile,
    _bytes: Reservation,
    pub(super) captured: chrono::DateTime<chrono::Utc>,
    items: BTreeMap<String, (Item, u64)>,
}

impl Snapshot {
    pub(super) fn export(
        command: &mut Command,
        mounts: &[String],
        cancel: &process::Cancel,
    ) -> Result<Self> {
        let _export = loop {
            process::check(cancel)?;
            match EXPORT.try_lock() {
                Ok(guard) => break guard,
                Err(std::sync::TryLockError::WouldBlock) => {
                    std::thread::sleep(Duration::from_millis(40))
                }
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    bail!("Docker snapshot worker stopped.")
                }
            }
        };
        let mut file = tempfile::Builder::new()
            .prefix("rust-commander-docker-")
            .tempfile()?;
        let mut bytes = Reservation::default();
        let mut stream = process::Pipe::start(command, Duration::from_secs(120), cancel)?;
        let mut buffer = vec![0; 128 * 1024];
        loop {
            let n = stream.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            bytes.add(n as u64)?;
            file.write_all(&buffer[..n])?;
        }
        let captured = chrono::Utc::now();
        let items = index(file.reopen()?, bytes.0, mounts, cancel)?;
        Ok(Self {
            file,
            _bytes: bytes,
            captured,
            items,
        })
    }

    pub(super) fn stat(&self, path: &str) -> Result<Item> {
        if path.is_empty() {
            return Ok(Item::directory());
        }
        self.items
            .get(path)
            .map(|(item, _)| *item)
            .context("Docker snapshot path not found.")
    }

    pub(super) fn list(&self, path: &str) -> Result<Vec<(String, Item)>> {
        ensure!(self.stat(path)?.directory, "Docker path is not a folder.");
        let prefix = if path.is_empty() {
            String::new()
        } else {
            format!("{path}/")
        };
        Ok(self
            .items
            .range(prefix.clone()..)
            .take_while(|(key, _)| key.starts_with(&prefix))
            .filter_map(|(key, (item, _))| {
                let name = &key[prefix.len()..];
                (!name.contains('/')).then(|| (name.to_owned(), *item))
            })
            .collect())
    }

    pub(super) fn read(self: Arc<Self>, path: &str, offset: u64) -> Result<Box<dyn Read + Send>> {
        let (item, start) = self
            .items
            .get(path)
            .context("Docker snapshot file not found.")?;
        ensure!(
            !item.directory && offset <= item.size,
            "Invalid Docker snapshot file range."
        );
        let mut file = self.file.reopen()?;
        file.seek(SeekFrom::Start(start + offset))?;
        Ok(Box::new(SnapshotReader {
            file: file.take(item.size - offset),
            _snapshot: self,
        }))
    }
}

struct SnapshotReader {
    file: std::io::Take<File>,
    _snapshot: Arc<Snapshot>,
}

impl Read for SnapshotReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buffer)
    }
}

fn index(
    mut file: File,
    length: u64,
    mounts: &[String],
    cancel: &process::Cancel,
) -> Result<BTreeMap<String, (Item, u64)>> {
    ensure!(
        length >= 1024 && length.is_multiple_of(512),
        "Invalid Docker snapshot archive size."
    );
    file.seek(SeekFrom::End(-1024))?;
    let mut footer = [0; 1024];
    file.read_exact(&mut footer)?;
    ensure!(
        footer.iter().all(|&byte| byte == 0),
        "Truncated Docker snapshot archive."
    );
    file.rewind()?;
    let mut archive = tar::Archive::new(BufReader::new(file));
    let mut items = BTreeMap::new();
    let mut parents = BTreeSet::new();
    let mut path_bytes = 0usize;
    for (count, entry) in archive.entries_with_seek()?.enumerate() {
        process::check(cancel)?;
        ensure!(count < 250_000, "Docker snapshot has too many entries.");
        let entry = entry.context("Invalid Docker snapshot archive")?;
        let raw = entry.path_bytes();
        ensure!(
            !raw.starts_with(b"/")
                && !raw.contains(&0)
                && !raw.split(|&b| b == b'/').any(|p| p == b".."),
            "Unsafe path in Docker snapshot."
        );
        let Ok(raw) = std::str::from_utf8(&raw) else {
            continue;
        };
        let path = raw.trim_start_matches("./").trim_end_matches('/');
        if path.is_empty()
            || !super::valid_key(path)
            || mounts.iter().any(|m| super::inside(path, m))
        {
            continue;
        }
        let kind = entry.header().entry_type();
        let item = if kind.is_dir() {
            Item::directory()
        } else if kind.is_file() {
            let offset = entry.raw_file_position();
            ensure!(
                offset <= length && entry.size() <= length - offset,
                "Truncated Docker snapshot file."
            );
            Item {
                directory: false,
                size: entry.size(),
                modified: entry
                    .header()
                    .mtime()
                    .ok()
                    .and_then(|t| i64::try_from(t).ok()),
            }
        } else {
            continue;
        };
        path_bytes += path.len();
        for (slash, _) in path.match_indices('/') {
            let parent = &path[..slash];
            if !parents.contains(parent) {
                path_bytes += parent.len();
                parents.insert(parent.to_owned());
            }
        }
        ensure!(
            path_bytes <= 64 * 1024 * 1024 && items.len() + parents.len() < 250_000,
            "Docker snapshot index is too large."
        );
        items.insert(path.to_owned(), (item, entry.raw_file_position()));
    }
    for parent in parents {
        let (item, _) = items.entry(parent).or_insert((Item::directory(), 0));
        ensure!(item.directory, "Conflicting paths in Docker snapshot.");
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_indexes_nested_files_and_masks_mounts() {
        let mut file = tempfile::tempfile().unwrap();
        {
            let mut tar = tar::Builder::new(&mut file);
            for path in ["app/nested/readme.txt", "data/underlying.txt"] {
                let mut header = tar::Header::new_gnu();
                header.set_size(3);
                header.set_mode(0o644);
                header.set_cksum();
                tar.append_data(&mut header, path, &b"abc"[..]).unwrap();
            }
            tar.finish().unwrap();
        }
        let len = file.metadata().unwrap().len();
        file.rewind().unwrap();
        let items = index(file, len, &["data".into()], &Default::default()).unwrap();
        assert_eq!(items["app/nested/readme.txt"].0.size, 3);
        assert!(items["app/nested"].0.directory);
        assert!(!items.keys().any(|p| p.starts_with("data")));
    }

    #[test]
    fn snapshot_rejects_truncated_tar_and_parent_paths() {
        let mut file = tempfile::tempfile().unwrap();
        assert!(index(file.try_clone().unwrap(), 0, &[], &Default::default()).is_err());
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o644);
        header.as_mut_bytes()[..11].copy_from_slice(b"../escape  ");
        header.set_cksum();
        file.write_all(header.as_bytes()).unwrap();
        file.write_all(&[0; 1024]).unwrap();
        assert!(
            index(file, 1536, &[], &Default::default())
                .err()
                .unwrap()
                .to_string()
                .contains("Unsafe path")
        );
    }

    #[test]
    fn snapshot_budget_rejects_large_exports_and_releases_reservations() {
        let mut reservation = Reservation::default();
        assert!(reservation.add(MAX_EXPORT + 1).is_err());
        reservation.add(20).unwrap();
        assert_eq!(reservation.0, 20);
        drop(reservation);
        assert_eq!(BYTES.load(Ordering::Relaxed), 0);
    }
}
