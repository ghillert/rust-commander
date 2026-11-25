//! File operations shared by the desktop UI and background workers.
//!
//! Pass an [`Operation`] to [`run`] with a [`Task`] to report progress and
//! accept cancellation. Use [`crate::history::execute`] when a caller needs a
//! journal of the changes for Undo or recovery.
//!
//! For local selections, [`preflight`] maps sources to unused names in a
//! destination directory. [`copy_item`] copies one entry to an exact path.
//! [`crate::transfer`] handles decisions about conflicting names.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Instant, SystemTime},
};

mod rate;
pub(crate) mod totals;

/// A file-management action and the inputs needed to perform it.
///
/// Local copy and move actions take a destination directory. The source entries
/// keep their names unless an explicit [conflict policy](crate::transfer::ConflictPolicy)
/// chooses new ones.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Operation {
    /// Write an external editor's private working copy back to its source.
    ExternalSave {
        /// Location from which the file or archive member was opened.
        location: crate::filesystem::Location,
        /// Source entry path in the location's filesystem, connection, or archive.
        path: PathBuf,
        /// Local file edited inside the leased temporary directory.
        #[serde(skip)]
        working: PathBuf,
        /// Snapshot taken before editing, needed to detect changes and perform save-back.
        #[serde(skip)]
        original: Option<Arc<crate::private_bytes::Bytes>>,
        /// Keeps the editor's temporary directory alive until save-back finishes.
        #[serde(skip)]
        lease: Option<Arc<tempfile::TempDir>>,
        /// Fingerprint of the containing archive when opened, used for archive save-back.
        expected: Option<String>,
    },
    /// Update ownership and access rules after verifying the reviewed metadata.
    Security {
        /// Snapshot identifying the local file and the security metadata the user reviewed.
        expected: crate::security_metadata::Snapshot,
        /// Owner, group, and ACL values to apply to the verified entry.
        change: crate::security_metadata::Change,
    },
    /// Restore each selected original as a new copy in `destination`.
    RecoveryRestore {
        /// Retained originals selected from the recovery inventory.
        items: Vec<crate::recovery::Item>,
        /// Writable local or remote directory that receives the restored copies.
        destination: crate::filesystem::Location,
    },
    /// Permanently clean the backup groups represented by the selected originals.
    RecoveryCleanup(Vec<crate::recovery::Item>),
    /// Copy or move a pane selection, including transfers into or out of an editable archive.
    DropTransfer {
        /// Source location and entries captured by the drag.
        selection: crate::drag_drop::Selection,
        /// Destination directory or writable archive location.
        to: crate::filesystem::Location,
        /// Whether to move (`true`) or copy (`false`) the selected entries.
        moving: bool,
        /// Default conflict policy and directory merge behavior.
        options: crate::transfer::TransferOptions,
        /// Per-entry conflict decisions keyed by the incoming entry's destination path.
        overrides: std::collections::BTreeMap<PathBuf, crate::transfer::ConflictPolicy>,
    },
    /// Ask the operating system to empty its Trash.
    EmptyTrash,
    /// Start or resume a batch copy using [`crate::resumable::run_resolved`].
    ResumeRemote {
        /// Persistent `batch-` identifier used to find the checkpoint manifest.
        id: String,
        /// Local or remote directory containing the selected entries.
        from: crate::filesystem::Location,
        /// Selected entries as returned by the source directory listing.
        entries: Vec<crate::filesystem::Entry>,
        /// Local or remote destination directory for the selected entries.
        to: crate::filesystem::Location,
        /// Batch conflict and merge settings retained across retries.
        #[serde(default)]
        options: crate::transfer::TransferOptions,
        /// Per-destination decisions recorded for this batch.
        #[serde(default)]
        overrides: std::collections::BTreeMap<PathBuf, crate::transfer::ConflictPolicy>,
    },
    /// Save edited member contents after verifying the enclosing archive.
    ArchiveSaveBytes {
        /// Local path to the archive or its materialized copy.
        archive: PathBuf,
        /// Path of the member relative to the archive root.
        path: PathBuf,
        /// Archive fingerprint captured before opening the member for editing.
        expected: Option<String>,
        /// Complete replacement contents, already encoded for writing.
        #[serde(skip)]
        bytes: Vec<u8>,
    },
    /// Extract selected members with the policies used by [`crate::archive::extract_resolved`].
    ExtractResolved {
        /// Local path to the archive containing the selected members.
        archive: PathBuf,
        /// Archive-root-relative member paths. An empty selection includes everything under
        /// `prefix`.
        selected: Vec<PathBuf>,
        /// Directory prefix to strip from output paths. An empty path represents the archive
        /// root.
        prefix: PathBuf,
        /// Local output directory, which may already contain entries.
        destination: PathBuf,
        /// Default conflict policy and directory merge behavior.
        options: crate::transfer::TransferOptions,
        /// Per-entry conflict decisions keyed by the incoming entry's destination path.
        overrides: std::collections::BTreeMap<PathBuf, crate::transfer::ConflictPolicy>,
    },
    /// Replace a local file with encoded editor output after checking its original contents.
    SaveBytes {
        /// Local file to replace.
        path: PathBuf,
        /// Bytes read when editing began, used to detect changes before replacement.
        #[serde(skip)]
        original: Vec<u8>,
        /// Complete replacement contents, already encoded for writing.
        #[serde(skip)]
        bytes: Vec<u8>,
    },
    /// Write encoded editor output to an existing remote file.
    RemoteSaveBytes {
        /// ID of the connection profile that owns the entry.
        connection: String,
        /// Path of the existing file relative to the connection root.
        path: PathBuf,
        /// Bytes read when editing began, used to detect changes before replacement.
        #[serde(skip)]
        original: Vec<u8>,
        /// Complete replacement contents, already encoded for writing.
        #[serde(skip)]
        bytes: Vec<u8>,
    },
    /// Save externally edited bytes to a remote file using the external editor's size limit.
    RemoteExternalSave {
        /// ID of the connection profile that owns the entry.
        connection: String,
        /// Path of the existing file relative to the connection root.
        path: PathBuf,
        /// Bytes read when editing began, used to detect changes before replacement.
        #[serde(skip)]
        original: Vec<u8>,
        /// Complete replacement contents, already encoded for writing.
        #[serde(skip)]
        bytes: Vec<u8>,
    },
    /// Copy or move between local or remote folders with individual conflict decisions.
    RemoteTransferResolved {
        /// Local or remote directory containing the selected entries.
        from: crate::filesystem::Location,
        /// Selected entries as returned by the source directory listing.
        entries: Vec<crate::filesystem::Entry>,
        /// Local or remote destination directory for the selected entries.
        to: crate::filesystem::Location,
        /// Whether to move (`true`) or copy (`false`) the selected entries.
        moving: bool,
        /// Default conflict policy and directory merge behavior.
        options: crate::transfer::TransferOptions,
        /// Per-entry conflict decisions keyed by the incoming entry's destination path.
        overrides: std::collections::BTreeMap<PathBuf, crate::transfer::ConflictPolicy>,
    },

    /// Reverse the changes captured in a journal record.
    Undo(Box<crate::history::Record>),
    /// Reapply changes by reversing the journal record produced by Undo.
    Redo(Box<crate::history::Record>),
    /// Save edited UTF-8 text after checking the bytes originally read from disk.
    SaveText {
        /// Local file whose contents are being edited.
        path: PathBuf,
        /// Bytes read when editing began, used to detect changes before replacement.
        #[serde(skip)]
        original: Vec<u8>,
        /// Replacement text to write as UTF-8.
        #[serde(skip)]
        text: String,
    },
    /// Apply the choices and change checks in a reviewed folder synchronization plan.
    Sync(crate::sync::Plan),
    /// Save UTF-8 editor text to a remote file after checking its original contents.
    RemoteSave {
        /// ID of the connection profile that owns the entry.
        connection: String,
        /// Path of the existing file relative to the connection root.
        path: PathBuf,
        /// Bytes read when editing began, used to detect changes before replacement.
        #[serde(skip)]
        original: Vec<u8>,
        /// Replacement text to write as UTF-8.
        #[serde(skip)]
        text: String,
    },
    /// Continue a single-file copy or move from its local transfer checkpoint.
    ResumeLocal {
        /// Local file originally queued for transfer.
        source: PathBuf,
        /// Exact final file path, which must still be unused.
        destination: PathBuf,
        /// Whether to remove the verified source after publishing the destination file.
        moving: bool,
        /// Path to the partially copied file in the queue's transfer storage.
        partial: PathBuf,
        /// Source fingerprint or metadata stamp captured for this queue entry.
        stamp: String,
    },
    /// Copy or move local entries with per-target conflict decisions.
    ResolvedTransfer {
        /// Local entries to copy or move.
        sources: Vec<PathBuf>,
        /// Existing local directory that receives the selected entries.
        destination: PathBuf,
        /// Whether to move (`true`) or copy (`false`) the selected entries.
        moving: bool,
        /// Default conflict policy and directory merge behavior.
        options: crate::transfer::TransferOptions,
        /// Per-entry conflict decisions keyed by the incoming entry's destination path.
        overrides: std::collections::BTreeMap<PathBuf, crate::transfer::ConflictPolicy>,
    },
    /// Rename an entry within its remote parent directory.
    RemoteRename {
        /// ID of the connection profile that owns the entry.
        connection: String,
        /// Current entry path relative to the connection root.
        source: PathBuf,
        /// New filename within the same directory.
        name: String,
    },
    /// Add, remove, or rename members of an editable ZIP, tar, or 7z archive.
    ArchiveEdit {
        /// Local archive path, including a cached archive with a writable origin.
        archive: PathBuf,
        /// Member edit and the paths it operates on.
        edit: crate::archive_tools::ZipEdit,
    },
    /// Create a password-protected ZIP or 7z archive from local sources.
    PackProtected {
        /// Local file or directory paths to include at the archive root.
        sources: Vec<PathBuf>,
        /// Path of the new local archive, including the suffix that chooses its format.
        destination: PathBuf,
        /// Password used for encryption, omitted from serialized operations.
        #[serde(skip)]
        password: String,
        /// Compression level for the selected format, or `None` for the writer's default.
        #[serde(default)]
        compression_level: Option<u8>,
    },
    /// Validate an archive password and extract the selected members into a new directory.
    ExtractProtected {
        /// Local path to the archive containing the selected members.
        archive: PathBuf,
        /// Archive-root-relative member paths. An empty selection includes everything under
        /// `prefix`.
        selected: Vec<PathBuf>,
        /// Directory prefix to strip from output paths. An empty path represents the archive
        /// root.
        prefix: PathBuf,
        /// New local directory to create for the extracted contents.
        destination: PathBuf,
        /// Password used to unlock the archive, omitted from serialized operations.
        #[serde(skip)]
        password: String,
    },
    /// Copy or move local entries using a shared conflict policy.
    Transfer {
        /// Local entries to copy or move.
        sources: Vec<PathBuf>,
        /// Existing local directory that receives the selected entries.
        destination: PathBuf,
        /// Whether to move (`true`) or copy (`false`) the selected entries.
        moving: bool,
        /// Default conflict policy and directory merge behavior.
        options: crate::transfer::TransferOptions,
    },
    /// Apply a plan of `(old_path, new_path)` pairs in one local directory.
    BatchRename(Vec<(PathBuf, PathBuf)>),
    /// Summarize the selected local paths, including the contents of selected directories.
    Properties(Vec<PathBuf>),
    /// Compare the contents of two local directory trees.
    Compare {
        /// Root directory for the left-hand side of the comparison.
        left: PathBuf,
        /// Root directory for the right-hand side of the comparison.
        right: PathBuf,
    },
    /// Compare two local or remote directory trees by content.
    CompareLocations {
        /// Location used as the left root for matching relative paths.
        left: crate::filesystem::Location,
        /// Location used as the right root for matching relative paths.
        right: crate::filesystem::Location,
    },
    /// Split one local file into sequentially numbered parts.
    Split {
        /// File whose bytes will be split.
        source: PathBuf,
        /// New directory to create for the numbered parts.
        destination: PathBuf,
        /// Maximum number of bytes per part. Must be greater than zero.
        bytes: u64,
    },
    /// Concatenate local files in the supplied order to create one new file.
    Combine {
        /// Part paths in the exact order their bytes should be written.
        sources: Vec<PathBuf>,
        /// Exact path of the new combined file.
        destination: PathBuf,
    },
    /// Create an empty local file at the given unused path.
    Mkfile(PathBuf),
    /// Set the Unix permission bits of selected local entries.
    Permissions {
        /// Local paths to update. Symlinks are rejected.
        paths: Vec<PathBuf>,
        /// Octal permission and special bits in the range `0o0000..=0o7777`.
        mode: u32,
    },
    /// Set the modification time of selected local entries.
    Modified {
        /// Local files or directories to update. Symlinks are rejected.
        paths: Vec<PathBuf>,
        /// New modification timestamp to apply to every selected path.
        time: SystemTime,
    },
    /// Permanently remove the selected local paths and the contents of selected directories.
    PermanentDelete(Vec<PathBuf>),
    /// Copy or move between local or remote folders, refusing occupied names.
    RemoteTransfer {
        /// Local or remote directory containing the selected entries.
        from: crate::filesystem::Location,
        /// Selected entries as returned by the source directory listing.
        entries: Vec<crate::filesystem::Entry>,
        /// Local or remote destination directory for the selected entries.
        to: crate::filesystem::Location,
        /// Whether to move (`true`) or copy (`false`) the selected entries.
        moving: bool,
    },
    /// Create a directory on a remote connection.
    RemoteMkdir {
        /// ID of the connection profile that owns the parent directory.
        connection: String,
        /// Path of the new directory relative to the connection root.
        path: PathBuf,
    },
    /// Permanently delete remote entries, removing directory contents before their parents.
    RemoteDelete {
        /// ID of the connection profile that owns the selected entries.
        connection: String,
        /// Entries selected for deletion from the remote listing.
        entries: Vec<crate::filesystem::Entry>,
    },
    /// Copy local entries into an existing directory, refusing occupied names.
    Copy {
        /// Local files or directories to copy.
        sources: Vec<PathBuf>,
        /// Existing local directory that receives the selected entries.
        destination: PathBuf,
    },
    /// Move local entries into an existing directory, refusing occupied names.
    Move {
        /// Local files or directories to move.
        sources: Vec<PathBuf>,
        /// Existing local directory that receives the selected entries.
        destination: PathBuf,
    },
    /// Rename a local entry within its current directory, refusing an occupied target.
    Rename {
        /// Current local path of the entry.
        source: PathBuf,
        /// New filename within the source's parent directory.
        name: String,
    },
    /// Move the selected local paths to the operating system's Trash.
    Trash(Vec<PathBuf>),
    /// Create one local directory at the given path under an existing parent.
    Mkdir(PathBuf),
    /// Pack a pane selection into a new local archive.
    PackSelection {
        /// Source location and entries to include in the archive.
        selection: crate::drag_drop::Selection,
        /// Path of the new local archive, including the suffix that chooses its format.
        destination: PathBuf,
        /// Compression level for the selected format, or `None` for the writer's default.
        #[serde(default)]
        compression_level: Option<u8>,
        /// Optional ZIP archive comment. Use an empty string for other formats.
        #[serde(default)]
        comment: String,
        /// Whether the UI should let packing continue in the background.
        #[serde(default)]
        background: bool,
    },
    /// Pack a pane selection into a new local or remote archive.
    PackTo {
        /// Source location and entries to include in the archive.
        selection: crate::drag_drop::Selection,
        /// Exact local or remote output file. Its suffix selects the archive format.
        target: crate::filesystem::Location,
        /// Compression level for the selected format, or `None` for the writer's default.
        #[serde(default)]
        compression_level: Option<u8>,
        /// Optional ZIP archive comment. Use an empty string for other formats.
        #[serde(default)]
        comment: String,
        /// Whether to encrypt the new ZIP or 7z archive with `password`.
        #[serde(default)]
        encrypted: bool,
        /// Password used when encryption is enabled, omitted from serialized operations.
        #[serde(skip)]
        password: String,
        /// Whether the UI should let packing continue in the background.
        #[serde(default)]
        background: bool,
    },
    /// Create a new archive from a selection on the local filesystem.
    Pack {
        /// Local file or directory paths to include at the archive root.
        sources: Vec<PathBuf>,
        /// Path of the new local archive, including the suffix that chooses its format.
        destination: PathBuf,
        /// Compression level for the selected format, or `None` for the writer's default.
        #[serde(default)]
        compression_level: Option<u8>,
        /// Optional ZIP archive comment. Use an empty string for other formats.
        #[serde(default)]
        comment: String,
        /// Whether the UI should let packing continue in the background.
        #[serde(default)]
        background: bool,
    },
    /// Extract selected archive members into a new local directory.
    Extract {
        /// Local path to the archive containing the selected members.
        archive: PathBuf,
        /// Archive-root-relative member paths. An empty selection includes everything under
        /// `prefix`.
        selected: Vec<PathBuf>,
        /// Directory prefix to strip from output paths. An empty path represents the archive
        /// root.
        prefix: PathBuf,
        /// New local directory to create for the extracted contents.
        destination: PathBuf,
    },
    /// Compute and display MD5, SHA-1, SHA-256, and SHA-512 digests for one local file.
    Checksum(PathBuf),
}

impl Operation {
    /// Returns whether the operation uses transfer controls, including copy, move, resume,
    /// and folder synchronization.
    pub fn is_transfer(&self) -> bool {
        matches!(
            self,
            Self::Copy { .. }
                | Self::Move { .. }
                | Self::Transfer { .. }
                | Self::ResolvedTransfer { .. }
                | Self::RemoteTransfer { .. }
                | Self::RemoteTransferResolved { .. }
                | Self::DropTransfer { .. }
                | Self::ResumeRemote { .. }
                | Self::ResumeLocal { .. }
                | Self::Sync(_)
        )
    }

    /// Returns the short action name used in progress and history displays.
    pub fn label(&self) -> &'static str {
        match self {
            Self::RecoveryRestore { .. } => "Restore recovery copy",
            Self::RecoveryCleanup(_) => "Clean recovery copies",
            Self::EmptyTrash => "Empty Trash",
            Self::Undo(_) => "Undo",
            Self::Redo(_) => "Redo",
            Self::ExternalSave { .. } => "Save external edit",
            Self::SaveText { .. }
            | Self::RemoteSave { .. }
            | Self::SaveBytes { .. }
            | Self::RemoteSaveBytes { .. }
            | Self::RemoteExternalSave { .. } => "Save text",
            Self::ResumeRemote { .. } => "Checkpointed copy",
            Self::ResumeLocal { .. } => "Resumable transfer",
            Self::Sync(_) => "Synchronize folders",
            Self::Transfer { moving: true, .. }
            | Self::ResolvedTransfer { moving: true, .. }
            | Self::Move { .. } => "Move files",
            Self::Transfer { .. } | Self::ResolvedTransfer { .. } | Self::Copy { .. } => {
                "Copy files"
            }
            Self::Rename { .. } | Self::RemoteRename { .. } => "Rename",
            Self::BatchRename(_) => "Batch rename",
            Self::Mkdir(_) | Self::RemoteMkdir { .. } => "New folder",
            Self::Mkfile(_) => "New file",
            Self::Trash(_) => "Move to Trash",
            Self::PermanentDelete(_) | Self::RemoteDelete { .. } => "Delete",
            Self::Pack { .. }
            | Self::PackSelection { .. }
            | Self::PackProtected { .. }
            | Self::PackTo { .. } => "Create archive",
            Self::Extract { .. } | Self::ExtractProtected { .. } | Self::ExtractResolved { .. } => {
                "Extract archive"
            }
            Self::ArchiveEdit { .. } | Self::ArchiveSaveBytes { .. } => "Edit archive",
            Self::DropTransfer { .. } => "Drag and drop transfer",
            Self::RemoteTransfer { .. } | Self::RemoteTransferResolved { .. } => "Remote transfer",
            Self::Properties(_) => "Properties",
            Self::Compare { .. } | Self::CompareLocations { .. } => "Compare folders",
            Self::Checksum(_) => "Checksums",
            Self::Permissions { .. } => "Permissions",
            Self::Security { .. } => "Change ownership and ACL",
            Self::Modified { .. } => "Modification time",
            Self::Split { .. } => "Split file",
            Self::Combine { .. } => "Combine files",
        }
    }
    /// Recipe details exclude passwords and editor buffers.
    pub fn description(&self) -> String {
        match self {
            Self::PackTo {
                selection, target, ..
            } => format!(
                "{}\n{}\n→ {}",
                selection.from.label(),
                selection
                    .entries
                    .iter()
                    .map(|e| e.name.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                target.label()
            ),
            Self::PackSelection {
                selection,
                destination,
                ..
            } => format!(
                "{}\n{}\n→ {}",
                selection.from.label(),
                selection
                    .entries
                    .iter()
                    .map(|entry| entry.name.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                destination.display()
            ),
            Self::DropTransfer { selection, to, .. } => format!(
                "{}\n{}\n→ {}",
                selection.from.label(),
                selection
                    .entries
                    .iter()
                    .map(|e| e.name.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                to.label()
            ),
            Self::ResumeLocal {
                source,
                destination,
                ..
            } => format!("{} → {}", source.display(), destination.display()),
            Self::Copy {
                sources,
                destination,
            }
            | Self::Move {
                sources,
                destination,
            }
            | Self::Transfer {
                sources,
                destination,
                ..
            }
            | Self::ResolvedTransfer {
                sources,
                destination,
                ..
            } => format!(
                "{}\n→ {}",
                sources
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n"),
                destination.display()
            ),
            Self::RemoteTransfer {
                from, entries, to, ..
            }
            | Self::RemoteTransferResolved {
                from, entries, to, ..
            }
            | Self::ResumeRemote {
                from, entries, to, ..
            } => format!(
                "{}\n{}\n→ {}",
                from.label(),
                entries
                    .iter()
                    .map(|e| e.name.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                to.label()
            ),
            Self::Sync(plan) => format!("{} ⇄ {}", plan.left.display(), plan.right.display()),
            Self::ArchiveEdit { archive, .. }
            | Self::Extract { archive, .. }
            | Self::ExtractProtected { archive, .. }
            | Self::ExtractResolved { archive, .. } => archive.display().to_string(),
            Self::SaveText { path, .. }
            | Self::RemoteSave { path, .. }
            | Self::SaveBytes { path, .. }
            | Self::RemoteSaveBytes { path, .. }
            | Self::RemoteExternalSave { path, .. } => path.display().to_string(),
            Self::Security { expected, .. } => expected.path.display().to_string(),
            Self::ExternalSave { path, .. } => path.display().to_string(),
            _ => self.label().into(),
        }
    }
}

/// Called before a path changes so the journal can capture its current state.
pub type ChangeHook = Box<dyn FnMut(&Path) -> Result<()> + Send>;

/// Progress observed by a worker during one operation.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Progress {
    /// Display path supplied by the most recent progress update.
    pub current: String,
    /// Cumulative bytes reported by the worker, including any restored checkpoint offset.
    pub bytes: u64,
    /// Number of completed streams and other entries the worker reports as processed.
    pub files: usize,
    /// Planned byte count, when the worker can determine one.
    #[serde(default)]
    pub total_bytes: Option<u64>,
    /// Bytes of planned work completed, including progress recovered from a checkpoint.
    #[serde(default)]
    pub completed_bytes: u64,
    #[serde(skip)]
    rate: rate::TransferRate,
}

impl Progress {
    /// Returns a fraction between zero and one, or `None` for an unknown total.
    /// A known total of zero yields `Some(0.0)`.
    pub fn fraction(&self) -> Option<f32> {
        self.total_bytes.map(|total| {
            if total == 0 {
                0.0
            } else {
                (self.completed_bytes as f64 / total as f64).clamp(0.0, 1.0) as f32
            }
        })
    }

    /// Recent bytes processed per second, excluding restored checkpoint bytes.
    /// Idle time is included even when the worker has not sent another report.
    pub fn bytes_per_second(&self) -> Option<u64> {
        self.rate.per_second(Instant::now())
    }
}

/// Progress reporting, cancellation, and journal hooks for one running operation.
///
/// [`Task::default`] starts with a clear cancellation flag and a silent progress
/// callback. Use [`Task::new`] to share the flag or receive progress updates.
pub struct Task {
    pub(crate) replacement_recovery: Option<crate::history::ReplacementRecovery>,
    /// Set by a worker when its result cannot safely be undone automatically.
    pub undo_blocked: bool,
    /// Additional messages to include with the operation's result, such as the location of
    /// retained recovery copies.
    pub notices: Vec<String>,
    pub(crate) private_roots: Vec<PathBuf>,
    /// Optional hook for capturing a path's current state before a change.
    pub before_change: Option<ChangeHook>,
    /// Set to `true` to request cancellation at the next [`check`](Self::check).
    pub cancel: Arc<AtomicBool>,
    /// Optional shared control for pausing, pacing, and retrying checkpointed work.
    pub control: Option<Arc<crate::transfer_control::Control>>,
    /// Latest progress, including work since the last callback.
    pub progress: Progress,
    report: Box<dyn FnMut(Progress) + Send>,
    total_planned: bool,
    last_report: Instant,
    report_interval: std::time::Duration,
}

impl Default for Task {
    fn default() -> Self {
        Self::new(Arc::new(AtomicBool::new(false)), |_| {})
    }
}

impl Task {
    /// Creates a task with a shared cancellation flag and a progress callback.
    pub fn new(cancel: Arc<AtomicBool>, report: impl FnMut(Progress) + Send + 'static) -> Self {
        Self {
            undo_blocked: false,
            replacement_recovery: None,
            notices: Vec::new(),
            private_roots: Vec::new(),
            before_change: None,
            cancel,
            control: None,
            progress: Progress::default(),
            report: Box::new(report),
            total_planned: false,
            last_report: Instant::now(),
            report_interval: std::time::Duration::from_millis(40),
        }
    }

    /// Set the progress notification interval. Zero reports every tick, which
    /// lets scripted callers coordinate deterministic interruption/recovery.
    pub fn report_every(mut self, interval: std::time::Duration) -> Self {
        self.report_interval = interval;
        self
    }

    pub(crate) fn needs_total(&self) -> bool {
        !self.total_planned
    }

    pub(crate) fn note_replacement(&mut self, recovery: crate::history::ReplacementRecovery) {
        self.replacement_recovery = Some(match &self.replacement_recovery {
            Some(previous) if previous != &recovery => crate::history::ReplacementRecovery::Manager,
            _ => recovery,
        });
    }

    /// Only the outer operation owns the total; nested copies share its budget.
    pub(crate) fn plan_total(&mut self, total: Option<u64>) {
        if !self.total_planned {
            self.total_planned = true;
            self.progress.total_bytes = total;
            self.progress.completed_bytes = 0;
            // Publish with the next ordinary tick so existing cancellation and
            // race-detection callbacks still observe actual execution points.
        }
    }

    /// Credit work already present in a verified checkpoint or a native rename,
    /// without counting it as newly transferred bytes or inflating the speed.
    pub(crate) fn advance_progress(&mut self, bytes: u64) {
        self.progress.completed_bytes = self.progress.completed_bytes.saturating_add(bytes);
        if self
            .progress
            .total_bytes
            .is_some_and(|total| self.progress.completed_bytes > total)
        {
            self.progress.total_bytes = None;
        }
    }

    /// Checks for cancellation, then runs the installed change hook for `path`.
    pub fn capture(&mut self, path: &Path) -> Result<()> {
        self.check()?;
        if let Some(mut hook) = self.before_change.take() {
            let result = hook(path);
            self.before_change = Some(hook);
            result?;
        }
        Ok(())
    }

    /// Waits while a pause is in effect and returns an error on cancellation.
    ///
    /// Workers call this between steps. Changes completed before cancellation
    /// can remain on disk.
    pub fn check(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            bail!("Operation cancelled. Completed items were kept.");
        }
        if let Some(control) = &self.control {
            control.check(&self.cancel)?;
        }
        Ok(())
    }

    /// Returns the current transfer buffer size in bytes, or 64 KiB when no transfer
    /// control is attached.
    pub fn chunk_size(&self) -> usize {
        self.control.as_ref().map_or(65536, |c| c.chunk_size())
    }
    /// Applies any configured bandwidth limit to `bytes` of completed work. With no
    /// transfer control this returns immediately.
    ///
    /// # Errors
    ///
    /// Returns an error if the transfer is cancelled during pacing.
    pub fn pace(&self, bytes: u64) -> Result<()> {
        if let Some(control) = &self.control {
            control.pace(bytes, &self.cancel)?;
        }
        Ok(())
    }
    /// Retry only a caller-provided checkpoint recovery operation. Each attempt
    /// rebuilds progress from verified receipts; streamed bytes are not credited twice.
    pub(crate) fn retry_checkpoint<T>(
        &mut self,
        mut run: impl FnMut(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let mut attempt = 0u8;
        loop {
            self.check()?;
            match run(self) {
                Ok(value) => return Ok(value),
                Err(error) => {
                    let Some(control) = self.control.clone() else {
                        return Err(error);
                    };
                    if attempt >= control.settings().retries.min(8)
                        || !crate::transfer_control::temporary(&error)
                    {
                        return Err(if attempt > 0 {
                            error.context(format!(
                                "Transfer failed after {attempt} automatic retries"
                            ))
                        } else {
                            error
                        });
                    }
                    attempt += 1;
                    control.backoff(
                        attempt,
                        std::time::Duration::from_secs((1u64 << (attempt - 1)).min(30)),
                        &self.cancel,
                    )?;
                    if attempt > control.settings().retries.min(8) {
                        return Err(error);
                    }
                    self.progress = Progress::default();
                    self.total_planned = false;
                    (self.report)(self.progress.clone());
                }
            }
        }
    }

    /// Accounts for `bytes` against the current path, updates transfer rate and progress,
    /// and emits a progress report when its interval has elapsed. Pass zero to report the
    /// path without adding bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when a cancellation or transfer-control check interrupts progress.
    pub fn tick(&mut self, path: &Path, bytes: u64) -> Result<()> {
        self.check()?;
        self.pace(bytes)?;
        self.progress.current = path.display().to_string();
        self.progress.bytes += bytes;
        self.advance_progress(bytes);
        self.progress.rate.record(bytes, Instant::now());
        if self.last_report.elapsed() >= self.report_interval {
            (self.report)(self.progress.clone());
            self.last_report = Instant::now();
        }
        Ok(())
    }

    /// Copies from the reader to the writer in chunks and returns the byte count.
    ///
    /// # Errors
    ///
    /// Cancellation and I/O failures can leave bytes in the writer. [`copy_item`]
    /// stages regular files when a complete file must be published at once.
    pub fn stream(
        &mut self,
        input: &mut dyn Read,
        output: &mut dyn Write,
        path: &Path,
    ) -> Result<u64> {
        self.progress.rate.record(0, Instant::now());
        let mut buffer = [0u8; 64 * 1024];
        let mut total = 0;
        loop {
            self.check()?;
            let count = input.read(&mut buffer[..self.chunk_size()])?;
            if count == 0 {
                break;
            }
            output.write_all(&buffer[..count])?;
            total += count as u64;
            self.tick(path, count as u64)?;
        }
        self.progress.files += 1;
        Ok(total)
    }
}

/// Returns whether entry metadata is readable, including for a dangling symlink.
pub fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

pub(crate) fn remove(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub(crate) fn validate_sources(sources: &[PathBuf]) -> Result<()> {
    if sources.is_empty() {
        bail!("Select at least one item.");
    }
    let paths: std::collections::HashSet<&Path> = sources.iter().map(PathBuf::as_path).collect();
    if paths.len() != sources.len() {
        bail!("The selection contains duplicate or nested paths.");
    }
    for source in sources {
        fs::symlink_metadata(source)
            .with_context(|| format!("Cannot read {}", source.display()))?;
        if source
            .ancestors()
            .skip(1)
            .any(|parent| paths.contains(parent))
        {
            bail!("The selection contains duplicate or nested paths.");
        }
    }
    Ok(())
}

/// Returns the proposed target paths for a local selection, in source order.
///
/// The destination must be an existing directory. This check leaves the
/// filesystem untouched. A target name can become occupied before the copy starts.
///
/// # Errors
///
/// Rejects empty, duplicate, or nested selections, unreadable sources, occupied
/// or duplicate targets, and a destination inside a selected directory.
pub fn preflight(sources: &[PathBuf], destination: &Path) -> Result<Vec<PathBuf>> {
    validate_sources(sources)?;
    let destination =
        fs::canonicalize(destination).context("The destination must be an existing folder")?;
    if !destination.is_dir() {
        bail!("The destination is not a folder.");
    }
    let mut targets = Vec::new();
    for source in sources {
        let name = source
            .file_name()
            .context("Cannot copy a filesystem root")?;
        let target = destination.join(name);
        if exists(&target) || targets.contains(&target) {
            bail!(
                "Destination already exists: {}. Rename it or choose another folder.",
                target.display()
            );
        }
        let metadata = fs::symlink_metadata(source)?;
        if metadata.is_dir() && destination.starts_with(fs::canonicalize(source)?) {
            bail!("Cannot copy or move a folder into itself.");
        }
        targets.push(target);
    }
    Ok(targets)
}

/// Creates new paths exclusively. Symlinks are copied as links, never followed.
/// Copies a file, directory tree, or symlink to the exact unused path `target`.
///
/// Choose a target outside the source tree. Symlinks are recreated with their
/// stored link text. Regular files are staged beside the target and published
/// after the contents and supported metadata have been written.
///
/// # Errors
///
/// Fails if the target exists, the source is a special file, a regular file's
/// size or modification time changes while copying, or the task encounters
/// cancellation or an I/O error.
pub fn copy_item(source: &Path, target: &Path, task: &mut Task) -> Result<()> {
    task.tick(source, 0)?;
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        let link = fs::read_link(source)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(link, target)?;
        #[cfg(windows)]
        if source.is_dir() {
            std::os::windows::fs::symlink_dir(link, target)?;
        } else {
            std::os::windows::fs::symlink_file(link, target)?;
        }
        filetime::set_symlink_file_times(
            target,
            filetime::FileTime::from_last_access_time(&metadata),
            filetime::FileTime::from_last_modification_time(&metadata),
        )?;
        task.progress.files += 1;
    } else if metadata.is_dir() {
        fs::create_dir(target)?;
        let result: Result<()> = (|| {
            for item in fs::read_dir(source)? {
                let item = item?;
                copy_item(&item.path(), &target.join(item.file_name()), task)?;
            }
            filetime::set_file_mtime(
                target,
                filetime::FileTime::from_last_modification_time(&metadata),
            )?;
            fs::set_permissions(target, metadata.permissions())?;
            crate::file_metadata::copy(source, target)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(target);
        }
        result?;
    } else if metadata.is_file() {
        let mut input = File::open(source)?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(target.parent().context("Missing parent folder")?)?;
        task.stream(&mut input, &mut temporary, source)?;
        let after = fs::symlink_metadata(source)?;
        if metadata.len() != after.len() || metadata.modified().ok() != after.modified().ok() {
            task.undo_blocked = true;
            bail!("Source changed while copying: {}", source.display());
        }
        if let Ok(modified) = metadata.modified() {
            temporary
                .as_file()
                .set_times(fs::FileTimes::new().set_modified(modified))?;
        }
        temporary
            .as_file()
            .set_permissions(metadata.permissions())?;
        crate::file_metadata::copy(source, temporary.path())?;
        temporary.as_file().sync_all()?;
        temporary.persist_noclobber(target).map_err(|e| e.error)?;
    } else {
        bail!("Special files are not supported: {}", source.display());
    }
    Ok(())
}

/// Dispatches a single operation and returns its display message.
/// [`run`] also records the outcome in the debug log and collects notices.
pub fn execute(operation: Operation, task: &mut Task) -> Result<String> {
    task.check()?;
    match operation {
        Operation::RecoveryRestore { items, destination } => {
            crate::recovery::restore_many(&items, &destination, task)
        }
        Operation::RecoveryCleanup(items) => crate::recovery::cleanup_many(&items, task),
        Operation::DropTransfer {
            selection,
            to,
            moving,
            options,
            overrides,
        } => crate::drag_drop::transfer(&selection, &to, moving, options, &overrides, task),
        Operation::ResumeRemote {
            id,
            from,
            entries,
            to,
            options,
            overrides,
        } => task.retry_checkpoint(|task| {
            crate::resumable::run_resolved(&id, &from, &entries, &to, options, &overrides, task)
        }),
        Operation::ArchiveSaveBytes {
            archive,
            path,
            expected,
            bytes,
        } => crate::external_edit::save_archive(&archive, &path, &expected, &bytes, task),
        Operation::ExtractResolved {
            archive,
            selected,
            prefix,
            destination,
            options,
            overrides,
        } => crate::archive::extract_resolved(
            &archive,
            &selected,
            &prefix,
            &destination,
            options,
            &overrides,
            task,
        ),
        Operation::EmptyTrash => {
            crate::native_actions::trash_action(true)?;
            Ok("Emptied system Trash.".into())
        }
        Operation::RemoteTransferResolved {
            from,
            entries,
            to,
            moving,
            options,
            overrides,
        } => crate::remote::transfer_resolved(
            &from, &entries, &to, moving, options, &overrides, task,
        ),
        Operation::ExternalSave {
            location,
            path,
            working,
            original,
            lease,
            expected,
        } => {
            crate::external_edit::save(&location, &path, &working, original, lease, &expected, task)
        }
        Operation::SaveBytes {
            path,
            original,
            bytes,
        } => {
            task.capture(&path)?;
            save_bytes(&path, &original, &bytes)?;
            Ok("Saved text.".into())
        }
        Operation::RemoteExternalSave {
            connection,
            path,
            original,
            bytes,
        } => crate::remote::save_bytes_bounded(
            &connection,
            &path,
            &original,
            &bytes,
            crate::external_edit::LIMIT,
            task,
        ),
        Operation::RemoteSaveBytes {
            connection,
            path,
            original,
            bytes,
        } => crate::remote::save_bytes(&connection, &path, &original, &bytes, task),
        Operation::RemoteSave {
            connection,
            path,
            original,
            text,
        } => crate::remote::save_text(&connection, &path, &original, &text, task),
        Operation::ResumeLocal {
            source,
            destination,
            moving,
            partial,
            stamp,
        } => {
            if moving {
                crate::queue::resume_local(&source, &destination, moving, &partial, &stamp, task)
            } else {
                task.retry_checkpoint(|task| {
                    crate::queue::resume_local(&source, &destination, false, &partial, &stamp, task)
                })
            }
        }
        Operation::Sync(plan) => crate::sync::apply(&plan, task),
        Operation::SaveText {
            path,
            original,
            text,
        } => {
            task.capture(&path)?;
            save_text(&path, &original, &text)?;
            Ok("Saved text.".into())
        }
        Operation::Redo(record) => {
            crate::history::undo(&record, task)?;
            Ok("Redid operation.".into())
        }
        Operation::Undo(record) => {
            crate::history::undo(&record, task)?;
            Ok(format!("Undid {}", record.label))
        }
        Operation::ResolvedTransfer {
            sources,
            destination,
            moving,
            options,
            overrides,
        } => crate::transfer::transfer_resolved(
            &sources,
            &destination,
            moving,
            options,
            &overrides,
            task,
        ),
        Operation::RemoteRename {
            connection,
            source,
            name,
        } => {
            crate::remote::rename_with_task(&connection, &source, &name, task)?;
            Ok("Remote item renamed.".into())
        }
        Operation::ArchiveEdit { archive, edit } => {
            if crate::archive_tools::origin(&archive).is_none() {
                task.capture(&archive)?;
            }
            crate::archive_tools::edit_archive(&archive, &edit, task)?;
            Ok("Archive updated and saved to its source.".into())
        }
        Operation::PackProtected {
            sources,
            destination,
            password,
            compression_level,
        } => {
            task.capture(&destination)?;
            crate::archive::pack_with_password_and_level(
                &sources,
                &destination,
                &password,
                compression_level,
                task,
            )?;
            Ok("Encrypted archive created.".into())
        }
        Operation::ExtractProtected {
            archive,
            selected,
            prefix,
            destination,
            password,
        } => {
            task.capture(&destination)?;
            crate::archive::extract_with_password(
                &archive,
                &selected,
                &prefix,
                &destination,
                &password,
                task,
            )?;
            Ok("Archive extracted.".into())
        }
        Operation::Transfer {
            sources,
            destination,
            moving,
            options,
        } => crate::transfer::transfer(&sources, &destination, moving, options, task),
        Operation::BatchRename(plan) => {
            for (source, target) in &plan {
                task.capture(source)?;
                task.capture(target)?;
            }
            crate::desktop::batch_rename(&plan, task)?;
            Ok(format!("Renamed {} item(s).", plan.len()))
        }
        Operation::Properties(paths) => Ok(format!(
            "Properties\n{}",
            crate::desktop::properties(&paths, task)?
        )),
        Operation::CompareLocations { left, right } => {
            let entries = crate::comparison::compare(&left, &right, task)?;
            Ok(format!(
                "Folder comparison\n{}\n{}\n\n{}",
                left.label(),
                right.label(),
                entries
                    .iter()
                    .map(|e| format!("{}  {}", e.status, e.name))
                    .collect::<Vec<_>>()
                    .join("\n")
            ))
        }
        Operation::Compare { left, right } => {
            let entries = crate::desktop::compare(&left, &right, task)?;
            Ok(format!(
                "Folder comparison\n{}\n{}\n\n{}",
                left.display(),
                right.display(),
                entries
                    .iter()
                    .map(|e| format!("{}  {}", e.status, e.name))
                    .collect::<Vec<_>>()
                    .join("\n")
            ))
        }
        Operation::Split {
            source,
            destination,
            bytes,
        } => {
            crate::desktop::split(&source, &destination, bytes, task)?;
            Ok("Split completed.".into())
        }
        Operation::Combine {
            sources,
            destination,
        } => {
            crate::desktop::combine(&sources, &destination, task)?;
            Ok("Combined files successfully.".into())
        }
        Operation::Mkfile(path) => {
            task.capture(&path)?;
            File::options().write(true).create_new(true).open(path)?;
            Ok("Created file.".into())
        }
        Operation::Security { expected, change } => {
            task.check()?;
            let result = crate::security_metadata::apply(&expected, &change)?;
            Ok(format!(
                "Security metadata updated for {}.\nOwner: {}\nGroup: {}\nACL:\n{}",
                result.path.display(),
                result.owner,
                result.group,
                result.acl
            ))
        }
        Operation::Permissions { paths, mode } => {
            anyhow::ensure!(mode <= 0o7777, "Permissions must be between 0000 and 7777.");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                for path in paths {
                    task.capture(&path)?;
                    task.check()?;
                    if fs::symlink_metadata(&path)?.file_type().is_symlink() {
                        bail!("Change the symlink target directly.");
                    }
                    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
                }
                Ok("Permissions updated.".into())
            }
            #[cfg(not(unix))]
            {
                let _ = (paths, mode);
                bail!("POSIX permissions are not available on this platform.");
            }
        }
        Operation::Modified { paths, time } => {
            for path in &paths {
                task.capture(path)?;
            }
            task.check()?;
            crate::desktop::change_modified(&paths, time)?;
            Ok("Modification time updated.".into())
        }
        Operation::PermanentDelete(paths) => {
            validate_sources(&paths)?;
            for path in &paths {
                task.tick(path, 0)?;
                remove(path)?;
            }
            Ok(format!("Permanently deleted {} item(s).", paths.len()))
        }
        Operation::RemoteTransfer {
            from,
            entries,
            to,
            moving,
        } => {
            crate::remote::transfer(&from, &entries, &to, moving, task)?;
            Ok(format!("Transferred {} item(s).", entries.len()))
        }
        Operation::RemoteMkdir { connection, path } => {
            crate::remote::mkdir(&connection, &path)?;
            Ok("Remote folder created.".into())
        }
        Operation::RemoteDelete {
            connection,
            entries,
        } => {
            crate::remote::delete(&connection, &entries, task)?;
            Ok("Remote items deleted.".into())
        }
        Operation::Copy {
            sources,
            destination,
        } => transfer(&sources, &destination, false, task),
        Operation::Move {
            sources,
            destination,
        } => crate::transfer::transfer(&sources, &destination, true, Default::default(), task),
        Operation::Rename { source, name } => {
            crate::filesystem::validate_name(&name)?;
            let target = source
                .parent()
                .context("Cannot rename a filesystem root")?
                .join(name);
            if exists(&target) {
                bail!("That name already exists.");
            }
            task.capture(&source)?;
            task.capture(&target)?;
            crate::desktop::rename_exclusive(&source, &target)?;
            Ok("Renamed successfully.".into())
        }
        Operation::Trash(sources) => {
            validate_sources(&sources)?;
            for source in &sources {
                task.tick(source, 0)?;
                trash::delete(source)
                    .with_context(|| format!("Could not move {} to Trash", source.display()))?;
                task.progress.files += 1;
            }
            Ok(format!("Moved {} item(s) to Trash.", sources.len()))
        }
        Operation::Mkdir(path) => {
            task.capture(&path)?;
            fs::create_dir(&path)?;
            Ok(format!("Created {}", path.display()))
        }
        Operation::PackSelection {
            selection,
            destination,
            comment,
            compression_level,
            ..
        } => {
            crate::packing::pack_selection(
                &selection,
                &destination,
                &comment,
                compression_level,
                task,
            )?;
            Ok(format!("Created {}", destination.display()))
        }
        Operation::PackTo {
            selection,
            target,
            compression_level,
            comment,
            encrypted,
            password,
            ..
        } => {
            let path = match &target {
                crate::filesystem::Location::Local(path)
                | crate::filesystem::Location::Remote { path, .. } => path,
                _ => bail!("Unsupported archive destination"),
            };
            let options = crate::packing::Options {
                format: crate::archive::format(path).context("Unsupported archive format")?,
                compression_level,
                comment,
                encrypted,
                password,
                ..Default::default()
            };
            crate::packing::pack_to(&selection, &target, &options, task)?;
            Ok(format!("Created {}", target.label()))
        }
        Operation::Pack {
            sources,
            destination,
            comment,
            compression_level,
            ..
        } => {
            validate_sources(&sources)?;
            task.capture(&destination)?;
            crate::archive::pack_with_compression(
                &sources,
                &destination,
                &comment,
                compression_level,
                task,
            )?;
            Ok(format!("Created {}", destination.display()))
        }
        Operation::Extract {
            archive,
            selected,
            prefix,
            destination,
        } => {
            task.capture(&destination)?;
            crate::archive::extract(&archive, &selected, &prefix, &destination, task)?;
            Ok(format!("Extracted to {}", destination.display()))
        }
        Operation::Checksum(path) => {
            let mut input = File::open(&path)?;
            let mut digest = Sha256::new();
            let mut md5 = md5::Md5::new();
            let mut sha1 = sha1::Sha1::new();
            let mut sha512 = sha2::Sha512::new();
            let mut buffer = [0u8; 64 * 1024];
            loop {
                task.tick(&path, 0)?;
                let n = input.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                digest.update(&buffer[..n]);
                md5.update(&buffer[..n]);
                sha1.update(&buffer[..n]);
                sha512.update(&buffer[..n]);
                task.tick(&path, n as u64)?;
            }
            Ok(format!(
                "Checksums · {}\nMD5     {:x}\nSHA-1   {:x}\nSHA-512 {:x}\nSHA-256 {:x}",
                path.display(),
                md5.finalize(),
                sha1.finalize(),
                sha512.finalize(),
                digest.finalize()
            ))
        }
    }
}

/// Runs an operation synchronously and includes its notices in the result.
///
/// # Errors
///
/// Returns the operation's error on cancellation, invalid input, or failure
/// during execution. Earlier items in a batch may already have been processed.
pub fn run(operation: Operation, task: &mut Task) -> Result<String> {
    let label = operation.label();
    crate::debug_log::record("operation", &format!("Started: {label}"));
    // Other operations may make nested copies without having a whole-job byte
    // budget. Keep those indeterminate rather than reporting one sub-copy as all.
    if !matches!(
        operation,
        Operation::Copy { .. }
            | Operation::Move { .. }
            | Operation::Transfer { .. }
            | Operation::ResolvedTransfer { .. }
            | Operation::RemoteTransfer { .. }
            | Operation::RemoteTransferResolved { .. }
            | Operation::DropTransfer { .. }
            | Operation::ResumeLocal { .. }
            | Operation::ResumeRemote { .. }
    ) {
        task.plan_total(None);
    }
    let start = task.notices.len();
    let result = execute(operation, task);
    crate::debug_log::record(
        "operation",
        if result.is_ok() {
            "Completed successfully"
        } else {
            "Failed or cancelled; review the operation result"
        },
    );
    let notes = task.notices.drain(start..).collect::<Vec<_>>().join("\n");
    match result {
        Ok(message) if !notes.is_empty() => Ok(format!("{message}\n{notes}")),
        Err(error) if !notes.is_empty() => Err(error.context(notes)),
        result => result,
    }
}

fn transfer(
    sources: &[PathBuf],
    destination: &Path,
    moving: bool,
    task: &mut Task,
) -> Result<String> {
    let targets = preflight(sources, destination)?;
    if task.needs_total() {
        task.plan_total(totals::local(sources, task)?);
    }
    let before = if moving {
        sources
            .iter()
            .map(|p| source_stamp(p, task))
            .collect::<Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    for (source, target) in sources.iter().zip(&targets) {
        task.capture(target)?;
        copy_item(source, target, task)
            .with_context(|| format!("Could not copy {}", source.display()))?;
    }
    task.check()?;
    if moving {
        for (source, before) in sources.iter().zip(&before) {
            if source_stamp(source, task)? != *before {
                bail!(
                    "A source changed during the move. All originals and completed copies were kept."
                );
            }
        }
        for source in sources {
            task.capture(source)?;
            remove(source).context("Copy completed, but an original could not be removed")?;
        }
    }
    Ok(format!(
        "{} {} item(s).",
        if moving { "Moved" } else { "Copied" },
        sources.len()
    ))
}

pub(crate) fn source_stamp(path: &Path, task: &Task) -> Result<[u8; 32]> {
    fn visit(path: &Path, digest: &mut Sha256, task: &Task) -> Result<()> {
        task.check()?;
        let metadata = fs::symlink_metadata(path)?;
        digest.update(path.as_os_str().as_encoded_bytes());
        digest.update([
            0,
            metadata.is_dir() as u8,
            metadata.file_type().is_symlink() as u8,
        ]);
        digest.update(metadata.len().to_le_bytes());
        if let Ok(modified) = metadata.modified()?.duration_since(SystemTime::UNIX_EPOCH) {
            digest.update(modified.as_nanos().to_le_bytes());
        }
        if metadata.file_type().is_symlink() {
            digest.update(fs::read_link(path)?.as_os_str().as_encoded_bytes());
        } else if metadata.is_dir() {
            let mut children = fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
            children.sort_by_key(|e| e.file_name());
            for child in children {
                visit(&child.path(), digest, task)?;
            }
        }
        Ok(())
    }
    let mut digest = Sha256::new();
    visit(path, &mut digest, task)?;
    Ok(digest.finalize().into())
}

/// Reads up to `limit` bytes of UTF-8 text for the built-in editor.
///
/// # Errors
///
/// Fails if the file exceeds `limit`, contains a NUL byte or invalid UTF-8,
/// or cannot be read.
pub fn read_text(path: &Path, limit: u64) -> Result<String> {
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        bail!(
            "Text files larger than {} cannot be opened in the built-in editor.",
            crate::filesystem::human_size(limit)
        );
    }
    if bytes.contains(&0) {
        bail!("This is a binary file. Use Open externally to view it.");
    }
    String::from_utf8(bytes).context("This file is not UTF-8. Use Open externally to view it.")
}

/// Saves UTF-8 text to an existing file using the bytes from when it was opened.
///
/// # Errors
///
/// Shares the checks in [`save_bytes`]. A stale `original` produces an error so
/// the caller can reload the file before retrying.
///
/// # Examples
///
/// ```
/// use rust_commander::operations::{read_text, save_text};
/// use std::fs;
///
/// let workspace = tempfile::tempdir()?;
/// let path = workspace.path().join("notes.txt");
/// fs::write(&path, "draft")?;
/// let original = read_text(&path, 1024)?;
///
/// save_text(&path, original.as_bytes(), "final draft")?;
/// assert!(save_text(&path, original.as_bytes(), "outdated edit").is_err());
/// assert_eq!(fs::read_to_string(&path)?, "final draft");
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn save_text(path: &Path, original: &[u8], text: &str) -> Result<()> {
    save_bytes(path, original, text.as_bytes())
}
/// Stages new bytes beside an existing file and checks `original` again before
/// replacing it. The staged file inherits permissions and supported metadata.
///
/// # Errors
///
/// Fails for a symlink, a mismatch between `original` and the on-disk bytes
/// at either check, or an I/O error while reading, staging, or replacing the file.
pub fn save_bytes(path: &Path, original: &[u8], bytes: &[u8]) -> Result<()> {
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        bail!("Edit the symlink target directly.");
    }
    if !matches_reader(&mut File::open(path)?, original, &Task::default())? {
        bail!("This file changed on disk. Reopen it before saving to avoid losing those changes.");
    }
    let permissions = fs::metadata(path)?.permissions();
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().context("Missing parent folder")?)?;
    temporary.write_all(bytes)?;
    temporary.as_file().set_permissions(permissions)?;
    crate::file_metadata::copy(path, temporary.path())?;
    temporary.as_file().sync_all()?;
    if !matches_reader(&mut File::open(path)?, original, &Task::default())? {
        bail!("This file changed while saving. The new contents were not published.");
    }
    temporary.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Compare streamed data without allocating a second document-sized buffer.
pub fn matches_reader(input: &mut dyn Read, expected: &[u8], task: &Task) -> Result<bool> {
    let mut offset = 0;
    let mut buffer = [0u8; 65536];
    loop {
        task.check()?;
        let count = input.read(&mut buffer)?;
        if count == 0 {
            return Ok(offset == expected.len());
        }
        if offset + count > expected.len() || buffer[..count] != expected[offset..offset + count] {
            return Ok(false);
        }
        offset += count;
    }
}
