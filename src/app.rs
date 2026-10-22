use crate::ui_signal::UiSignal;
mod uri_navigation;
use crate::{
    filesystem::{self, Entry, Location},
    model::{Pane, Tab},
    operations::{Operation, Progress, Task},
    settings::{self, Bookmark, Settings},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::Duration,
};

fn tr(text: &str) -> std::borrow::Cow<'_, str> {
    crate::i18n::text(text)
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Action {
    DirectoryTree,
    ToggleSidebar,
    ExternalEdit,
    Associations,
    FocusAddress,
    FocusFilter,
    SelectNextRow,
    SelectPreviousRow,
    SelectNextPage,
    SelectPreviousPage,
    SelectFirstRow,
    SelectLastRow,
    MarkNextRow,
    MarkPreviousRow,
    MarkNextPage,
    MarkPreviousPage,
    MarkToFirstRow,
    MarkToLastRow,
    MarkSelectedFile,
    ClearSelection,

    QuickFind,
    QuickFindNext,
    QuickFindPrevious,
    SelectPattern,
    UnselectPattern,
    SelectExtension,
    SelectNextBlock,
    SelectPreviousBlock,
    MarkNextBlock,
    MarkPreviousBlock,
    LockTab,
    TabTitle,
    MoveTabOther,
    MoveTabLeft,
    MoveTabRight,
    SideBySide,
    Stacked,
    EqualPanes,
    QuickLists,
    RecentLocations,
    RecentFiles,
    BookmarksList,
    TabsList,
    RootsList,
    OpenBoth,
    ParentBoth,
    ParentOther,
    SwitchPane,
    FocusNext,
    FocusPrevious,
    FileContextMenu,
    AutoSizeColumns,
    OpenTrash,
    EmptyTrash,
    NewWindow,
    CloseWindow,
    NextWindow,
    PreviousWindow,
    WindowsList,
    ImportSettings,
    ImportedCommands,

    Undo,
    Redo,
    Recovery,
    Synchronize,
    Preview,
    QuickLook,
    Reveal,
    OpenWith,
    PackageContents,
    Workspaces,
    Queue,
    ArchiveTools,
    NativeDrag,
    Quit,
    ToggleHidden,
    Preferences,
    Search,
    BatchRename,
    Properties,
    Mkfile,
    Split,
    Combine,
    Permissions,
    Modified,
    PermanentDelete,
    Compare,
    Swap,
    SameFolder,
    CloneTab,
    NextTab,
    PreviousTab,
    CloseOtherTabs,
    CloseDuplicateTabs,
    OpenOther,
    OpenNewTab,
    InvertSelection,
    MarkAll,
    UnmarkAll,
    CopyNames,
    CopyPaths,
    CopyClipboard,
    CutClipboard,
    PasteClipboard,
    Terminal,
    Connections,
    Copy,
    Move,
    Rename,
    Trash,
    Mkdir,
    Pack,
    Extract,
    View,
    Edit,
    External,
    Checksum,
    Refresh,
    Help,
    Bookmark,
    NewTab,
    CloseTab,
    Parent,
}

pub(crate) fn all_actions() -> Vec<Action> {
    vec![
        Action::DirectoryTree,
        Action::ToggleSidebar,
        Action::ExternalEdit,
        Action::Associations,
        Action::FocusAddress,
        Action::FocusFilter,
        Action::SelectNextRow,
        Action::SelectPreviousRow,
        Action::SelectNextPage,
        Action::SelectPreviousPage,
        Action::SelectFirstRow,
        Action::SelectLastRow,
        Action::MarkNextRow,
        Action::MarkPreviousRow,
        Action::MarkNextPage,
        Action::MarkPreviousPage,
        Action::MarkToFirstRow,
        Action::MarkToLastRow,
        Action::MarkSelectedFile,
        Action::ClearSelection,
        Action::QuickFind,
        Action::QuickFindNext,
        Action::QuickFindPrevious,
        Action::SelectPattern,
        Action::UnselectPattern,
        Action::SelectExtension,
        Action::SelectNextBlock,
        Action::SelectPreviousBlock,
        Action::MarkNextBlock,
        Action::MarkPreviousBlock,
        Action::LockTab,
        Action::TabTitle,
        Action::MoveTabOther,
        Action::MoveTabLeft,
        Action::MoveTabRight,
        Action::SideBySide,
        Action::Stacked,
        Action::EqualPanes,
        Action::QuickLists,
        Action::RecentLocations,
        Action::RecentFiles,
        Action::BookmarksList,
        Action::TabsList,
        Action::RootsList,
        Action::OpenBoth,
        Action::ParentBoth,
        Action::ParentOther,
        Action::SwitchPane,
        Action::FocusNext,
        Action::FocusPrevious,
        Action::FileContextMenu,
        Action::AutoSizeColumns,
        Action::OpenTrash,
        Action::EmptyTrash,
        Action::NewWindow,
        Action::CloseWindow,
        Action::NextWindow,
        Action::PreviousWindow,
        Action::WindowsList,
        Action::ImportSettings,
        Action::ImportedCommands,
        Action::Undo,
        Action::Redo,
        Action::Recovery,
        Action::Synchronize,
        Action::Preview,
        Action::QuickLook,
        Action::Reveal,
        Action::OpenWith,
        Action::PackageContents,
        Action::Workspaces,
        Action::Queue,
        Action::ArchiveTools,
        Action::NativeDrag,
        Action::Quit,
        Action::ToggleHidden,
        Action::Preferences,
        Action::Search,
        Action::BatchRename,
        Action::Properties,
        Action::Mkfile,
        Action::Split,
        Action::Combine,
        Action::Permissions,
        Action::Modified,
        Action::PermanentDelete,
        Action::Compare,
        Action::Swap,
        Action::SameFolder,
        Action::CloneTab,
        Action::NextTab,
        Action::PreviousTab,
        Action::CloseOtherTabs,
        Action::CloseDuplicateTabs,
        Action::OpenOther,
        Action::OpenNewTab,
        Action::InvertSelection,
        Action::MarkAll,
        Action::UnmarkAll,
        Action::CopyNames,
        Action::CopyPaths,
        Action::CopyClipboard,
        Action::CutClipboard,
        Action::PasteClipboard,
        Action::Terminal,
        Action::Connections,
        Action::Copy,
        Action::Move,
        Action::Rename,
        Action::Trash,
        Action::Mkdir,
        Action::Pack,
        Action::Extract,
        Action::View,
        Action::Edit,
        Action::External,
        Action::Checksum,
        Action::Refresh,
        Action::Help,
        Action::Bookmark,
        Action::NewTab,
        Action::CloseTab,
        Action::Parent,
    ]
}

pub(crate) fn action_menus() -> Vec<(&'static str, Vec<(&'static str, Action)>)> {
    let mut groups = Vec::from([
        (
            "Navigation tools",
            vec![
                ("Quick find", Action::QuickFind),
                ("Find next filename", Action::QuickFindNext),
                ("Find previous filename", Action::QuickFindPrevious),
                ("Quick lists…", Action::QuickLists),
                ("Recent locations…", Action::RecentLocations),
                ("Recently opened files…", Action::RecentFiles),
                ("Bookmarks…", Action::BookmarksList),
                ("Open tabs…", Action::TabsList),
                ("Roots and drives…", Action::RootsList),
                ("Open in both panes", Action::OpenBoth),
                ("Parent in both panes", Action::ParentBoth),
                ("Parent in other pane", Action::ParentOther),
                ("Switch pane", Action::SwitchPane),
                ("Focus next control", Action::FocusNext),
                ("Focus previous control", Action::FocusPrevious),
                ("File context menu", Action::FileContextMenu),
            ],
        ),
        (
            "Tab and layout",
            vec![
                ("Lock/unlock tab", Action::LockTab),
                ("Name tab…", Action::TabTitle),
                ("Move tab to other pane", Action::MoveTabOther),
                ("Move tab left", Action::MoveTabLeft),
                ("Move tab right", Action::MoveTabRight),
                ("Directory tree", Action::DirectoryTree),
                ("Side-by-side panes", Action::SideBySide),
                ("Stacked panes", Action::Stacked),
                ("Equal pane sizes", Action::EqualPanes),
                ("Fit columns to content", Action::AutoSizeColumns),
            ],
        ),
        (
            "More tools",
            vec![
                ("Select by pattern…", Action::SelectPattern),
                ("Unselect by pattern…", Action::UnselectPattern),
                ("Select same extension", Action::SelectExtension),
                ("Next block", Action::SelectNextBlock),
                ("Previous block", Action::SelectPreviousBlock),
                ("Mark next block", Action::MarkNextBlock),
                ("Mark previous block", Action::MarkPreviousBlock),
                ("Open Trash", Action::OpenTrash),
                ("Empty Trash…", Action::EmptyTrash),
                ("New window", Action::NewWindow),
                ("Close window", Action::CloseWindow),
                ("Next window", Action::NextWindow),
                ("Previous window", Action::PreviousWindow),
                ("Windows…", Action::WindowsList),
                ("Import muCommander settings…", Action::ImportSettings),
                ("Imported commands…", Action::ImportedCommands),
            ],
        ),
        (
            "File",
            vec![
                ("Connections…", Action::Connections),
                ("Archive tools…", Action::ArchiveTools),
                ("New file", Action::Mkfile),
                ("Batch rename…", Action::BatchRename),
                ("Properties", Action::Properties),
                ("Permissions…", Action::Permissions),
                ("Modification time…", Action::Modified),
                ("View  F3", Action::View),
                ("Edit  F4", Action::Edit),
                ("Open externally", Action::External),
                ("Open With…", Action::OpenWith),
                ("Edit externally…", Action::ExternalEdit),
                ("Application associations…", Action::Associations),
                ("Reveal in file manager", Action::Reveal),
                ("Show package contents", Action::PackageContents),
                ("Rename  F2", Action::Rename),
                ("New folder  F7", Action::Mkdir),
                ("Move to Trash  F8", Action::Trash),
                ("Permanently delete…", Action::PermanentDelete),
            ],
        ),
        (
            "Operations",
            vec![
                ("Undo last operation", Action::Undo),
                ("Redo last operation", Action::Redo),
                ("Recovery storage…", Action::Recovery),
                ("Synchronize folders…", Action::Synchronize),
                ("Transfer queue and history…", Action::Queue),
                ("Copy  F5", Action::Copy),
                ("Move  F6", Action::Move),
                ("Create archive", Action::Pack),
                ("Extract archive", Action::Extract),
                ("Checksums", Action::Checksum),
                ("Find files…", Action::Search),
                ("Compare folders", Action::Compare),
                ("Split file…", Action::Split),
                ("Combine files…", Action::Combine),
                ("Open terminal", Action::Terminal),
            ],
        ),
        (
            "Navigate",
            vec![
                ("Preview pane", Action::Preview),
                ("Quick Look", Action::QuickLook),
                ("Workspaces…", Action::Workspaces),
                ("Parent folder", Action::Parent),
                ("New tab", Action::NewTab),
                ("Close tab", Action::CloseTab),
                ("Add bookmark", Action::Bookmark),
                ("Refresh", Action::Refresh),
                ("Swap panes", Action::Swap),
                ("Same folder in both panes", Action::SameFolder),
                ("Clone tab to other pane", Action::CloneTab),
                ("Open in other pane", Action::OpenOther),
                ("Open in new tab", Action::OpenNewTab),
                ("Next tab", Action::NextTab),
                ("Previous tab", Action::PreviousTab),
                ("Close other tabs", Action::CloseOtherTabs),
                ("Close duplicate tabs", Action::CloseDuplicateTabs),
            ],
        ),
        (
            "Selection",
            vec![
                ("Mark all", Action::MarkAll),
                ("Unmark all", Action::UnmarkAll),
                ("Invert selection", Action::InvertSelection),
                ("Copy names", Action::CopyNames),
                ("Copy paths", Action::CopyPaths),
                ("Copy files", Action::CopyClipboard),
                ("Cut files", Action::CutClipboard),
                ("Paste files", Action::PasteClipboard),
                ("Prepare files for external drag…", Action::NativeDrag),
            ],
        ),
    ]);
    let extras = groups
        .drain(..3)
        .flat_map(|(_, items)| items)
        .collect::<Vec<_>>();
    groups.push((
        "View",
        vec![
            ("Show hidden files", Action::ToggleHidden),
            ("Toggle sidebar", Action::ToggleSidebar),
            ("Settings…", Action::Preferences),
        ],
    ));
    groups.push(("Window", Vec::new()));
    for item in extras {
        let target = match item.1 {
            Action::NewWindow
            | Action::CloseWindow
            | Action::NextWindow
            | Action::PreviousWindow
            | Action::WindowsList => "Window",
            Action::DirectoryTree
            | Action::SideBySide
            | Action::Stacked
            | Action::EqualPanes
            | Action::AutoSizeColumns => "View",
            Action::SelectPattern
            | Action::UnselectPattern
            | Action::SelectExtension
            | Action::SelectNextBlock
            | Action::SelectPreviousBlock
            | Action::MarkNextBlock
            | Action::MarkPreviousBlock => "Selection",
            Action::OpenTrash
            | Action::EmptyTrash
            | Action::ImportSettings
            | Action::ImportedCommands => "File",
            _ => "Navigate",
        };
        if let Some((_, items)) = groups.iter_mut().find(|(title, _)| *title == target) {
            items.push(item);
        }
    }
    groups
}

struct Prompt {
    title: String,
    description: String,
    value: String,
    kind: PromptKind,
    error: Option<String>,
    resumable: bool,
    transfer_options: crate::transfer::TransferOptions,
    overrides: std::collections::BTreeMap<PathBuf, crate::transfer::ConflictPolicy>,
    conflicts: Vec<crate::transfer::Conflict>,
    conflict_key: String,
    pending_conflicts: Option<Receiver<Result<Vec<crate::transfer::Conflict>, String>>>,
}
enum PromptKind {
    DropTransfer {
        selection: crate::drag_drop::Selection,
        to: Location,
        moving: bool,
    },
    Mkfile(PathBuf),
    Split {
        source: PathBuf,
        base: PathBuf,
    },
    Combine {
        sources: Vec<PathBuf>,
        base: PathBuf,
    },
    Permissions(Vec<PathBuf>),
    Modified(Vec<PathBuf>),
    PermanentDelete(Vec<PathBuf>),
    RemoteTransfer {
        from: Location,
        entries: Vec<Entry>,
        to: Location,
        moving: bool,
    },
    RemoteMkdir {
        connection: String,
        path: PathBuf,
    },
    RemoteDelete {
        connection: String,
        entries: Vec<Entry>,
    },
    Transfer {
        sources: Vec<PathBuf>,
        moving: bool,
        base: PathBuf,
    },
    RemoteRename {
        connection: String,
        source: PathBuf,
    },
    Rename(PathBuf),
    Mkdir(PathBuf),
    Trash(Vec<PathBuf>),
    Pack {
        sources: Vec<PathBuf>,
        base: PathBuf,
    },
    Extract {
        archive: PathBuf,
        selected: Vec<PathBuf>,
        prefix: PathBuf,
        base: PathBuf,
    },
    Bookmark(Location),
}

struct Editor {
    navigation: slint_ui::viewer_tools::Navigation,
    image_revision: u64,
    document: Option<crate::text_document::Document>,
    text_page: Option<crate::text_document::Page>,
    pdf: Option<PdfState>,
    image: Option<crate::viewer::RasterImage>,
    zoom: f32,
    kind: &'static str,
    title: String,
    path: Option<PathBuf>,
    remote: Option<(String, PathBuf)>,
    text: String,
    original: String,
    editable: bool,
    can_edit: bool,
    error: Option<String>,
    confirm_close: bool,
}
struct PdfState {
    bytes: Arc<Vec<u8>>,
    page: usize,
    pages: usize,
    password: String,
    rotation: usize,
    locked: bool,
    cache: Arc<std::sync::Mutex<crate::viewer::PdfCache>>,
}
impl PdfState {
    fn new(bytes: Arc<Vec<u8>>, pages: usize) -> Self {
        Self {
            bytes,
            page: 0,
            pages,
            password: String::new(),
            rotation: 0,
            locked: pages == 0,
            cache: Arc::new(std::sync::Mutex::new(Default::default())),
        }
    }
}
type PdfResult = Result<(String, usize, usize, usize, crate::viewer::RasterImage), String>;
impl Editor {
    fn dirty(&self) -> bool {
        self.editable
            && (self.text != self.original
                || self
                    .document
                    .as_ref()
                    .is_some_and(|d| d.encoding != d.original_encoding))
    }
}

enum JobEvent {
    Progress(Progress),
    Done(Result<crate::history::Record, String>),
}
struct Job {
    owner: usize,
    label: String,

    receiver: Receiver<JobEvent>,
    cancel: Arc<AtomicBool>,
    progress: Progress,
}

#[derive(Default)]
struct ConnectionForm {
    connection: crate::remote::Connection,
    secrets: crate::remote::Secrets,
    remember: bool,
    error: Option<String>,
    navigation: Option<uri_navigation::Ticket>,
    choices: Vec<crate::remote::Connection>,
    save_only: bool,
}

pub struct Commander {
    saved_windows: Vec<crate::settings::WindowSession>,
    window_id: usize,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    native_window: Option<crate::native_files::Window>,
    native_files: crate::native_files::NativeFiles,
    queue: crate::queue::Queue,
    features: crate::next_ui::Features,
    preview: crate::preview::Preview,
    workspaces: Vec<crate::workspaces::Workspace>,
    recent_locations: Vec<crate::navigation::RecentLocation>,
    recent_files: Vec<PathBuf>,
    imported_commands: Vec<crate::config_import::ImportedCommand>,
    pending_save: Option<(String, String)>,
    history: Vec<crate::history::Record>,
    show_history: bool,
    archive_form: Option<crate::feature_ui::ArchiveForm>,
    pending_archive: Option<Receiver<Result<Location, String>>>,
    pending_archive_target: Option<(usize, u64)>,
    oauth_cancel: Option<Arc<AtomicBool>>,
    #[cfg(target_os = "macos")]
    native_menu: Option<crate::native_menu::NativeMenu>,
    pending_pdf: Option<Receiver<PdfResult>>,
    terminal: Option<crate::terminal::Terminal>,
    terminal_visible: bool,
    preferences: crate::preferences::Preferences,
    preferences_form: Option<crate::preferences::Preferences>,
    preferences_error: Option<String>,
    tools: crate::tools_ui::Tools,
    clipboard: Option<(Location, Vec<Entry>, bool)>,
    connections: Vec<crate::remote::Connection>,
    connection_form: Option<ConnectionForm>,
    pending_uris: Vec<uri_navigation::Pending>,
    panes: [Pane; 2],
    active: usize,
    bookmarks: Vec<Bookmark>,
    show_hidden: bool,
    status: String,
    status_error: bool,
    prompt: Option<Prompt>,
    editor: Option<Editor>,
    pending_view: Option<Receiver<Result<Editor, String>>>,
    job: Option<Job>,
    help: bool,
    message: Option<String>,
    settings_valid: bool,
    confirm_exit: bool,
    exiting: bool,
}

impl Commander {
    fn action_available(&self, id: &str) -> bool {
        let writable = |location: &Location| match location {
            Location::Remote { connection, .. } => {
                crate::remote::profile(connection).is_none_or(|c| c.service != "http")
            }
            _ => true,
        };
        match id {
            "Move" => {
                writable(&self.tab().location)
                    && writable(&self.panes[1 - self.active].tab().location)
            }
            "Copy" => writable(&self.panes[1 - self.active].tab().location),
            "Mkdir" | "Rename" | "Trash" | "CutClipboard" | "PasteClipboard" | "ExternalEdit" => {
                writable(&self.tab().location)
            }
            _ => true,
        }
    }
    fn with_settings(ctx: &UiSignal, paths: &[PathBuf], loaded: anyhow::Result<Settings>) -> Self {
        let settings_valid = loaded.is_ok();
        let error = loaded.as_ref().err().map(|e| format!("{e:#}"));
        let mut settings = loaded.unwrap_or_default();
        settings.preferences.upgrade();
        crate::remote::register_profiles(&settings.connections);
        for (side, path) in paths.iter().enumerate().take(2) {
            let path = filesystem::resolve_path(
                &path.to_string_lossy(),
                &std::env::current_dir().unwrap_or_else(|_| filesystem::home()),
            );
            settings.tabs[side] = vec![Location::Local(path)];
            settings.tab_states[side].clear();
            settings.active_tabs[side] = 0;
        }
        let panes = std::array::from_fn(|i| {
            let locations = if settings.tabs[i].is_empty() {
                vec![Location::Local(filesystem::home())]
            } else {
                settings.tabs[i].clone()
            };
            let tabs: Vec<_> = locations
                .into_iter()
                .take(32)
                .enumerate()
                .map(|(index, l)| {
                    let mut tab = Tab::new(l, ctx);
                    tab.state = settings.tab_states[i]
                        .get(index)
                        .cloned()
                        .unwrap_or_default();
                    tab
                })
                .collect();
            Pane {
                active: settings.active_tabs[i].min(tabs.len() - 1),
                tabs,
            }
        });
        if settings.dark_mode
            && settings.preferences.color_scheme == crate::appearance::ColorScheme::System
        {
            settings.preferences.color_scheme = crate::appearance::ColorScheme::Dark;
        }
        crate::i18n::set(settings.preferences.language);

        Self {
            saved_windows: settings.window_sessions.clone(),
            window_id: 0,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            native_window: None,
            native_files: Default::default(),
            queue: crate::queue::Queue::load(),
            features: Default::default(),
            preview: Default::default(),
            recent_locations: settings.recent_locations,
            recent_files: settings.recent_files,
            imported_commands: settings.imported_commands,
            workspaces: settings.workspaces,
            pending_save: None,
            history: crate::history::load().unwrap_or_default(),
            show_history: false,
            archive_form: None,
            pending_archive: None,
            pending_archive_target: None,
            oauth_cancel: None,
            #[cfg(target_os = "macos")]
            native_menu: None,
            pending_pdf: None,
            terminal: None,
            terminal_visible: false,
            preferences: settings.preferences,
            preferences_form: None,
            preferences_error: None,
            tools: crate::tools_ui::Tools::default(),
            clipboard: None,
            connections: settings.connections,
            connection_form: None,
            pending_uris: Vec::new(),
            panes,
            active: settings.active_pane.min(1),
            bookmarks: settings.bookmarks,
            show_hidden: settings.show_hidden,
            status: error.unwrap_or_else(|| "Ready · Select files to get started".into()),
            status_error: !settings_valid,
            prompt: None,
            editor: None,
            pending_view: None,
            job: None,
            help: false,
            message: None,
            settings_valid,
            confirm_exit: false,
            exiting: false,
        }
    }
    fn tab(&self) -> &Tab {
        self.panes[self.active].tab()
    }
    fn tab_mut(&mut self) -> &mut Tab {
        self.panes[self.active].tab_mut()
    }
    fn fail(&mut self, error: impl ToString) {
        self.status = error.to_string();
        self.status_error = true;
    }
    fn refresh(&mut self, ctx: &UiSignal) {
        for pane in &mut self.panes {
            for tab in &mut pane.tabs {
                tab.refresh(ctx);
            }
        }
    }
    fn start_job(&mut self, operation: Operation, ctx: &UiSignal) {
        let operation = match crate::queue::Queue::prepare(operation) {
            Ok(op) => op,
            Err(e) => {
                self.fail(e);
                return;
            }
        };
        if self.job.is_some() {
            if self.queue.len() >= 1000 {
                self.fail("The transfer queue is full.");
                return;
            }
            self.queue.push_back(operation);
            if let Err(e) = self.queue.save() {
                self.queue.pop_back();
                self.fail(e);
                return;
            }
            self.status = format!("Queued {} operation(s).", self.queue.len());
            return;
        }
        let label = operation.label().to_owned();
        self.queue.active = Some(operation.clone());
        if let Err(e) = self.queue.save() {
            self.queue.active = None;
            self.fail(e);
            return;
        }
        let retention = self.preferences.recovery.clone();
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker_ctx = ctx.clone();
        self.status = "Working…".into();
        self.status_error = false;
        std::thread::spawn(move || {
            let progress_sender = sender.clone();
            let progress_ctx = worker_ctx.clone();
            let mut task = Task::new(worker_cancel, move |p| {
                let _ = progress_sender.send(JobEvent::Progress(p));
                progress_ctx.request_repaint();
            });
            let result =
                crate::history::execute(operation, &mut task).map_err(|e| format!("{e:#}"));
            if retention.automatic
                && let Ok(root) = crate::history::root()
                && let Ok(records) = crate::history::load_in(&root)
                && let Ok(ids) = crate::history::cleanup_candidates(
                    &records,
                    &root,
                    &retention,
                    crate::oauth::now(),
                )
            {
                let _ = crate::history::cleanup_in(&root, &ids);
            }
            let _ = sender.send(JobEvent::Done(result));
            worker_ctx.request_repaint();
        });
        self.job = Some(Job {
            owner: self.window_id,
            label,
            receiver,
            cancel,
            progress: Progress::default(),
        });
    }
    fn poll(&mut self, ctx: &UiSignal) {
        if let Some(result) = crate::background::poll(&mut self.pending_archive) {
            match result {
                Ok(location) => {
                    if let Some((side, id)) = self.pending_archive_target.take() {
                        if let Some(tab) = self.panes[side].tabs.iter_mut().find(|tab| tab.id == id)
                        {
                            tab.navigate(location, ctx);
                        }
                    } else {
                        self.tab_mut().navigate(location, ctx);
                    }
                }
                Err(error) => self.fail(error),
            }
        }
        if let Some(terminal) = &mut self.terminal {
            terminal.poll();
        }
        if let Some(result) = crate::background::poll(&mut self.pending_pdf) {
            match result {
                Ok((title, page, rotation, pages, image)) => {
                    if let Some(editor) = &mut self.editor
                        && format!("{}#{}", editor.title, editor.navigation.id) == title
                        && let Some(pdf) = &mut editor.pdf
                    {
                        pdf.page = page;
                        pdf.rotation = rotation;
                        pdf.pages = pages;
                        pdf.locked = false;
                        editor.image_revision += 1;
                        editor.image = Some(image);
                        editor.error = None;
                    }
                }
                Err(error) => {
                    if let Some(editor) = &mut self.editor {
                        editor.error = Some(error);
                    }
                }
            }
        }
        self.poll_uris(ctx);
        for pane in &mut self.panes {
            let requested = pane
                .tabs
                .iter_mut()
                .find_map(|tab| tab.requested_location.take());
            if let Some(location) = requested {
                if pane.tabs.len() < 32 {
                    pane.tabs.push(Tab::new(location, ctx));
                    pane.active = pane.tabs.len() - 1;
                } else {
                    pane.tab_mut().error = Some("At most 32 tabs per pane are supported.".into());
                }
            }
            for tab in &mut pane.tabs {
                tab.folders_first = self.preferences.folders_first;
                if self.preferences.auto_refresh {
                    tab.poll_changes(ctx);
                }
                tab.poll(self.show_hidden);
            }
        }
        let mut completed = None;
        if let Some(job) = self.job.as_mut().filter(|j| j.owner == self.window_id) {
            loop {
                match job.receiver.try_recv() {
                    Ok(JobEvent::Progress(progress)) => job.progress = progress,
                    Ok(JobEvent::Done(result)) => {
                        completed = Some(result);
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        completed = Some(Err("The operation stopped unexpectedly. Completed output and originals may remain; review them before retrying.".into()));
                        break;
                    }
                }
            }
        }
        if let Some(result) = completed {
            self.job = None;
            let recipe = self.queue.active.take();
            let success = result.as_ref().is_ok_and(|r| r.success);
            if !success && let Some(recipe) = recipe {
                self.queue.failed.push(recipe);
            }
            if let Some((title, text)) = self.pending_save.take()
                && let Some(editor) = &mut self.editor
                && editor.title == title
            {
                if success {
                    if let Some(document) = &mut editor.document
                        && let Ok(bytes) = document.encode(&text)
                    {
                        document.original = bytes;
                        document.original_encoding = document.encoding;
                        document.text = text.clone();
                    }
                    editor.original = text;
                    editor.error = None;
                } else {
                    editor.error = Some(match &result {
                        Ok(r) => r.result.clone(),
                        Err(e) => e.clone(),
                    });
                }
            }
            if let Err(e) = self.queue.save() {
                self.fail(e);
            }
            match result {
                Ok(record) => {
                    self.status = record.result.clone();
                    self.status_error = !record.success;
                    if record.result.contains("Replaced originals are kept")
                        || record.result.starts_with("Checksums")
                        || record.result.starts_with("Properties")
                        || record.result.starts_with("Folder comparison")
                    {
                        self.message = Some(record.result.clone());
                    }
                    self.history = crate::history::load().unwrap_or_else(|_| self.history.clone());
                    if !self.history.iter().any(|r| r.id == record.id) {
                        self.history.push(record);
                    }
                    if let Err(error) = crate::history::save(&self.history) {
                        self.fail(format!(
                            "Operation finished, but history could not be saved: {error:#}"
                        ));
                    }
                }
                Err(error) => self.fail(error),
            }
            self.refresh(ctx);
        }
        if self.job.is_none()
            && !self.queue.paused
            && !self.exiting
            && let Some(operation) = self.queue.pop_front()
        {
            if crate::queue::Queue::requires_input(&operation) {
                self.queue.push_front(operation);
                self.queue.paused = true;
                self.fail("This saved operation needs its password or editor contents again. Open the queue to review it.");
            } else {
                self.start_job(operation, ctx);
            }
        }
        if let Some(result) = crate::background::poll(&mut self.pending_view) {
            match result {
                Ok(editor) => self.editor = Some(editor),
                Err(error) => self.fail(error),
            }
        }
        if self.job.is_some()
            || self.pending_view.is_some()
            || self.pending_pdf.is_some()
            || !self.pending_uris.is_empty()
            || self.pending_archive.is_some()
            || self.panes.iter().any(|p| p.tabs.iter().any(|t| t.loading))
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if self.exiting && self.job.is_none() {
            ctx.request_close();
        }
    }
    fn open_entry(&mut self, entry: Entry, ctx: &UiSignal) {
        if self.tab().location.local().is_some()
            && crate::native_actions::is_application(&entry.path)
        {
            if let Err(e) = open::that_detached(&entry.path) {
                self.fail(e);
            }
            return;
        }
        if !entry.is_dir
            && self.tab().location.local().is_none()
            && crate::archive::format(&entry.path).is_some()
        {
            let location = self.tab().location.clone();
            let path = entry.path.clone();
            let (sender, receiver) = mpsc::channel();
            self.pending_archive = Some(receiver);
            self.pending_archive_target = Some((self.active, self.tab().id));
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let result = crate::archive_tools::materialize(&location, &path)
                    .map(|file| Location::Archive {
                        file,
                        prefix: PathBuf::new(),
                    })
                    .map_err(|error| format!("{error:#}"));
                let _ = sender.send(result);
                ctx.request_repaint();
            });
            return;
        }
        if let Some(location) = self.tab().location.enter(&entry) {
            self.tab_mut().navigate(location, ctx);
        } else if location_is_local_with_rule(
            &self.tab().location,
            &self.preferences.associations,
            &entry.path,
        ) {
            let application =
                crate::associations::application(&self.preferences.associations, &entry.path)
                    .unwrap();
            if let Err(e) = crate::native_actions::open_with(&entry.path, application) {
                self.fail(e);
            }
        } else {
            self.view(entry, false, ctx);
        }
    }
    fn remember_opened_file(&mut self, path: PathBuf) {
        self.recent_files.retain(|old| old != &path);
        self.recent_files.insert(0, path);
        self.recent_files.truncate(50);
    }
    fn view(&mut self, entry: Entry, editable: bool, ctx: &UiSignal) {
        let editable = editable
            && !matches!(&self.tab().location,Location::Remote{connection,..} if crate::remote::profile(connection).is_some_and(|c|c.service=="http"));
        if entry.is_dir {
            self.fail("Select a text file to view or edit.");
            return;
        }
        let location = self.tab().location.clone();
        if location.local().is_some() {
            self.remember_opened_file(entry.path.clone());
        }
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        self.pending_view = Some(receiver);
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<Editor> {
                let mut document = None;
                let mut text_page = None;
                let (text, image, kind, pdf) = match crate::viewer::load(&location, &entry.path)? {
                    crate::viewer::Content::LockedPdf { bytes } => (
                        String::new(),
                        Some(crate::viewer::RasterImage {
                            size: [1, 1],
                            pixels: vec![[255; 4]],
                        }),
                        "PDF viewer",
                        Some(PdfState::new(bytes, 0)),
                    ),
                    crate::viewer::Content::Text(text) => {
                        document =
                            crate::text_document::Document::detect(text.as_bytes().to_vec()).ok();
                        (text, None, "Text viewer", None)
                    }
                    crate::viewer::Content::EncodedText(doc) => {
                        let text = doc.text.clone();
                        document = Some(doc);
                        (text, None, "Text viewer", None)
                    }
                    crate::viewer::Content::LargeText(page) => {
                        let text = page.document.text.clone();
                        text_page = Some(page);
                        (text, None, "Paged text viewer", None)
                    }
                    crate::viewer::Content::Hex { text, truncated } => (
                        text,
                        None,
                        if truncated {
                            "Hex viewer · first 512 KiB"
                        } else {
                            "Hex viewer"
                        },
                        None,
                    ),
                    crate::viewer::Content::Image(image) => {
                        (String::new(), Some(image), "Image viewer", None)
                    }
                    crate::viewer::Content::Pdf {
                        bytes,
                        image,
                        pages,
                    } => (
                        String::new(),
                        Some(image),
                        "PDF viewer",
                        Some(PdfState::new(bytes, pages)),
                    ),
                };
                let path = location.local().map(|_| entry.path.clone());
                let remote = if let Location::Remote { connection, .. } = &location {
                    Some((connection.clone(), entry.path.clone()))
                } else {
                    None
                };
                Ok(Editor {
                    navigation: slint_ui::viewer_tools::Navigation::new(
                        location.clone(),
                        entry.path.clone(),
                    ),
                    image_revision: 0,
                    document,
                    text_page,
                    pdf,
                    image,
                    zoom: 1.0,
                    kind,
                    title: entry.path.display().to_string(),
                    can_edit: editable && (path.is_some() || remote.is_some()) && !entry.is_symlink,
                    editable: editable
                        && kind == "Text viewer"
                        && (path.is_some() || remote.is_some())
                        && !entry.is_symlink,
                    original: text.clone(),
                    text,
                    path,
                    remote,
                    error: None,
                    confirm_close: false,
                })
            })()
            .map_err(|e| format!("{e:#}"));
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }
    fn act(&mut self, action: Action, ctx: &UiSignal) {
        if !self.action_available(&format!("{action:?}")) {
            self.fail("This operation is unavailable for a read-only location.");
            return;
        }
        let chosen = self.tab().chosen(self.show_hidden);
        let location = self.tab().location.clone();
        let sources: Vec<_> = chosen.iter().map(|e| e.path.clone()).collect();
        let other = self.panes[1 - self.active]
            .tab()
            .location
            .local()
            .map(Path::to_owned);
        let local = location.local().map(Path::to_owned);
        let mut prompt = None;
        if let Location::Archive { file, .. } = &location
            && crate::archive_tools::editable(file)
            && crate::archive_tools::origin(file).is_none()
            && matches!(action, Action::Rename | Action::Trash)
        {
            let other = self.panes[1 - self.active].tab();
            let mut form = crate::feature_ui::ArchiveForm::new(
                &location,
                &chosen,
                &other.location,
                &other.chosen(self.show_hidden),
            );
            form.edit_mode = if matches!(action, Action::Rename) {
                2
            } else {
                1
            };
            self.archive_form = Some(form);
            return;
        }
        if matches!(action, Action::Copy) && location.local().is_some() {
            let other = self.panes[1 - self.active].tab();
            if let Location::Archive { file, .. } = &other.location
                && crate::archive_tools::editable(file)
                && crate::archive_tools::origin(file).is_none()
            {
                self.archive_form = Some(crate::feature_ui::ArchiveForm::new(
                    &other.location,
                    &other.chosen(self.show_hidden),
                    &location,
                    &chosen,
                ));
                return;
            }
        }
        match action {
            Action::ExternalEdit | Action::Associations => return,
            Action::DirectoryTree => {
                self.preferences.directory_tree = !self.preferences.directory_tree;
                self.save_settings();
                return;
            }
            Action::ToggleSidebar => {
                self.preferences.sidebar = !self.preferences.sidebar;
                self.save_settings();
                return;
            }
            Action::Undo | Action::Redo => {
                if self.job.is_some() || !self.queue.is_empty() {
                    self.fail("Finish or cancel queued operations before undoing.");
                    return;
                }
                if let Some(record) = self
                    .history
                    .iter()
                    .rev()
                    .find(|record| {
                        if matches!(action, Action::Redo) {
                            record.can_redo()
                        } else {
                            record.can_undo()
                        }
                    })
                    .cloned()
                {
                    self.start_job(
                        if matches!(action, Action::Redo) {
                            Operation::Redo(Box::new(record))
                        } else {
                            Operation::Undo(Box::new(record))
                        },
                        ctx,
                    );
                } else {
                    self.fail(if matches!(action, Action::Redo) {
                        "No operation is available to redo."
                    } else {
                        "No operation is available to undo."
                    });
                }
                return;
            }
            Action::Recovery => {
                self.features.recovery(ctx);
                return;
            }
            Action::Synchronize => {
                self.features.sync(PathBuf::new(), PathBuf::new());
                if let Some(form) = &mut self.features.sync {
                    form.left = self.panes[0].tab().location.label();
                    form.right = self.panes[1].tab().location.label();
                }
                return;
            }
            Action::Preview => {
                self.preferences.preview = !self.preferences.preview;
                self.save_settings();
                return;
            }
            Action::QuickLook => {
                #[cfg(target_os = "macos")]
                if local.is_some() {
                    if let Err(e) = crate::native_actions::quick_look(&sources) {
                        self.fail(e);
                    }
                } else {
                    self.preferences.preview = true;
                }
                #[cfg(not(target_os = "macos"))]
                {
                    self.preferences.preview = true;
                }
                return;
            }
            Action::Reveal => {
                if local.is_some() {
                    if let Some(path) = sources.first()
                        && let Err(e) = crate::native_actions::reveal(path)
                    {
                        self.fail(e);
                    }
                } else {
                    self.fail("Reveal requires a local file.");
                }
                return;
            }
            Action::OpenWith => {
                if local.is_some() && sources.len() == 1 {
                    self.features.open_with = Some((sources[0].clone(), String::new()));
                    self.features.error = None;
                } else {
                    self.fail("Select one local file.");
                }
                return;
            }
            Action::PackageContents => {
                if local.is_some() && chosen.len() == 1 && chosen[0].is_dir {
                    self.tab_mut()
                        .navigate(Location::Local(sources[0].clone()), ctx);
                }
                return;
            }
            Action::Workspaces => {
                self.features.workspaces = true;
                self.features.error = None;
                return;
            }
            Action::Queue => {
                self.show_history = true;
                return;
            }
            Action::ArchiveTools => {
                let other = self.panes[1 - self.active].tab();
                self.archive_form = Some(crate::feature_ui::ArchiveForm::new(
                    &location,
                    &chosen,
                    &other.location,
                    &other.chosen(self.show_hidden),
                ));
                return;
            }
            Action::NativeDrag => {
                if location.local().is_some() {
                    self.native_drag(sources);
                } else {
                    self.fail("Native drag-out requires local files.");
                }
                return;
            }
            Action::Quit => {
                ctx.request_close();
                return;
            }
            Action::ToggleHidden => {
                self.show_hidden = !self.show_hidden;
                return;
            }
            Action::Preferences => {
                self.preferences_form = Some(self.preferences.clone());
                self.preferences_error = None;
                return;
            }
            Action::Search => {
                if matches!(location, Location::Local(_) | Location::Remote { .. }) {
                    self.tools.search(location, self.show_hidden);
                } else {
                    self.fail("Search a local folder or a remote connection.");
                }
                return;
            }
            Action::Compare => {
                if let (Some(left), Some(right)) = (local, other) {
                    self.start_job(Operation::Compare { left, right }, ctx);
                } else {
                    self.fail("Choose two local folders to compare.");
                }
                return;
            }
            Action::Swap => {
                self.panes.swap(0, 1);
                self.active = 1 - self.active;
                return;
            }
            Action::SameFolder => {
                self.panes[1 - self.active]
                    .tab_mut()
                    .navigate(location, ctx);
                return;
            }
            Action::CloneTab => {
                let pane = &mut self.panes[1 - self.active];
                pane.tabs.push(Tab::new(location, ctx));
                pane.active = pane.tabs.len() - 1;
                return;
            }
            Action::NextTab | Action::PreviousTab => {
                let pane = &mut self.panes[self.active];
                pane.active = if matches!(action, Action::NextTab) {
                    (pane.active + 1) % pane.tabs.len()
                } else {
                    (pane.active + pane.tabs.len() - 1) % pane.tabs.len()
                };
                return;
            }
            Action::CloseOtherTabs => {
                let pane = &mut self.panes[self.active];
                let tab = pane.tabs.remove(pane.active);
                pane.tabs = vec![tab];
                pane.active = 0;
                return;
            }
            Action::CloseDuplicateTabs => {
                let pane = &mut self.panes[self.active];
                let current = pane.tab().id;
                let mut locations = Vec::new();
                pane.tabs.retain(|tab| {
                    if locations.contains(&tab.location) {
                        false
                    } else {
                        locations.push(tab.location.clone());
                        true
                    }
                });
                pane.active = pane.tabs.iter().position(|t| t.id == current).unwrap_or(0);
                return;
            }
            Action::InvertSelection | Action::MarkAll | Action::UnmarkAll => {
                let entries: Vec<_> = self
                    .tab()
                    .visible(self.show_hidden)
                    .into_iter()
                    .map(|i| self.tab().entries[i].path.clone())
                    .collect();
                for path in entries {
                    if matches!(action, Action::UnmarkAll)
                        || (matches!(action, Action::InvertSelection)
                            && self.tab().selected.contains(&path))
                    {
                        self.tab_mut().selected.remove(&path);
                    } else {
                        self.tab_mut().selected.insert(path);
                    }
                }
                return;
            }
            Action::PasteClipboard => {
                let native = self.native_files.paste().ok().and_then(|(paths, moving)| {
                    let entries = paths
                        .iter()
                        .map(|path| filesystem::entry_from_path(path))
                        .collect::<anyhow::Result<Vec<_>>>()
                        .ok()?;
                    let base = paths.first()?.parent()?.to_path_buf();
                    Some((Location::Local(base), entries, moving))
                });
                if let Some((from, entries, moving)) = native.or_else(|| {
                    self.clipboard.clone().filter(|(location, _, _)| {
                        location.local().is_none() && self.native_files.owns_selection()
                    })
                }) {
                    if let (Location::Local(base), Location::Local(destination)) =
                        (&from, &location)
                    {
                        self.prompt = Some(Prompt {
                            title: if moving { "Move files" } else { "Copy files" }.into(),
                            description: format!(
                                "{} selected item(s). Choose how to handle existing targets.",
                                entries.len()
                            ),
                            value: destination.display().to_string(),
                            kind: PromptKind::Transfer {
                                sources: entries.into_iter().map(|e| e.path).collect(),
                                moving,
                                base: base.clone(),
                            },
                            error: None,
                            resumable: false,
                            transfer_options: Default::default(),
                            overrides: Default::default(),
                            conflicts: Vec::new(),
                            conflict_key: String::new(),
                            pending_conflicts: None,
                        });
                    } else {
                        self.start_job(
                            Operation::RemoteTransfer {
                                from,
                                entries,
                                to: location,
                                moving,
                            },
                            ctx,
                        );
                    }
                } else {
                    self.fail("No files are available on the system clipboard.");
                }
                return;
            }
            Action::Terminal => {
                if self.terminal.is_some() {
                    self.terminal_visible = !self.terminal_visible;
                } else if let Some(path) = local {
                    match crate::terminal::Terminal::new(&path, ctx) {
                        Ok(terminal) => {
                            self.terminal = Some(terminal);
                            self.terminal_visible = true;
                        }
                        Err(error) => self.fail(error),
                    }
                } else {
                    self.fail("Open a local folder to launch a terminal.");
                }
                return;
            }
            Action::Mkfile => {
                if let Some(base) = local.clone() {
                    prompt = Some((
                        "New file",
                        "Create an empty file.".into(),
                        "New file.txt".into(),
                        PromptKind::Mkfile(base),
                    ));
                }
            }
            Action::Connections => {
                self.connection_form = Some(ConnectionForm::default());
                return;
            }
            Action::Refresh => {
                self.refresh(ctx);
                return;
            }
            Action::Help => {
                self.help = true;
                return;
            }
            Action::NewTab => {
                self.panes[self.active].add_tab(ctx);
                return;
            }
            Action::CloseTab => {
                let i = self.panes[self.active].active;
                self.panes[self.active].close_tab(i);
                return;
            }
            Action::Parent => {
                if let Some(parent) = location.parent() {
                    self.tab_mut().navigate(parent, ctx);
                }
                return;
            }
            Action::Bookmark => {
                prompt = Some((
                    "Add bookmark",
                    "Save this location in the sidebar.".into(),
                    location.title(),
                    PromptKind::Bookmark(location.clone()),
                ));
            }
            Action::Mkdir => {
                if let Location::Remote { connection, path } = &location {
                    prompt = Some((
                        "New remote folder",
                        "Enter a folder name.".into(),
                        "New folder".into(),
                        PromptKind::RemoteMkdir {
                            connection: connection.clone(),
                            path: path.clone(),
                        },
                    ));
                }
                if let Some(base) = local.clone() {
                    prompt = Some((
                        "New folder",
                        "Create a folder in the active pane.".into(),
                        "New folder".into(),
                        PromptKind::Mkdir(base),
                    ));
                }
            }
            _ if chosen.is_empty() => {
                self.fail("Select an item first.");
                return;
            }
            Action::BatchRename => {
                if local.is_some() {
                    self.tools.rename(sources);
                } else {
                    self.fail("Batch rename requires local files.");
                }
                return;
            }
            Action::Properties => {
                if local.is_some() {
                    self.start_job(Operation::Properties(sources), ctx);
                } else {
                    self.message = Some(
                        chosen
                            .iter()
                            .map(|e| {
                                format!(
                                    "{} · {} · {}",
                                    e.path.display(),
                                    e.kind(),
                                    filesystem::human_size(e.size)
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                    );
                }
                return;
            }
            Action::CopyNames | Action::CopyPaths => {
                ctx.copy_text(
                    chosen
                        .iter()
                        .map(|e| {
                            if matches!(action, Action::CopyNames) {
                                e.name.clone()
                            } else {
                                e.path.display().to_string()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
                return;
            }
            Action::CopyClipboard | Action::CutClipboard => {
                if location.local().is_none()
                    && let Err(error) = self.native_files.mark_remote()
                {
                    self.fail(error);
                    return;
                }
                if location.local().is_some()
                    && let Err(error) = self
                        .native_files
                        .copy(&sources, matches!(action, Action::CutClipboard))
                {
                    self.fail(error);
                    return;
                }
                self.clipboard = Some((location, chosen, matches!(action, Action::CutClipboard)));
                self.status = "File selection copied. Paste into the destination pane.".into();
                return;
            }
            Action::OpenOther | Action::OpenNewTab => {
                let target = location
                    .enter(&chosen[0])
                    .unwrap_or_else(|| location.clone());
                if matches!(action, Action::OpenOther) {
                    self.panes[1 - self.active].tab_mut().navigate(target, ctx);
                } else {
                    let pane = &mut self.panes[self.active];
                    pane.tabs.push(Tab::new(target, ctx));
                    pane.active = pane.tabs.len() - 1;
                }
                return;
            }
            Action::Split => {
                if let Some(base) = other.clone().or(local.clone())
                    && local.is_some()
                    && sources.len() == 1
                    && !chosen[0].is_dir
                {
                    prompt = Some((
                        "Split file",
                        "Part size in MiB. A new folder will contain numbered parts.".into(),
                        "64".into(),
                        PromptKind::Split {
                            source: sources[0].clone(),
                            base,
                        },
                    ));
                }
            }
            Action::Combine => {
                if let Some(base) = other.clone().or(local.clone())
                    && local.is_some()
                {
                    prompt = Some(("Combine files", "Combine selected files in the order shown in this pane. Choose a new output file.".into(), base.join("combined.bin").display().to_string(), PromptKind::Combine { sources, base }));
                }
            }
            Action::Permissions => {
                if local.is_some() {
                    prompt = Some((
                        "Change permissions",
                        "Enter POSIX permission bits in octal (for example 644 or 755).".into(),
                        "644".into(),
                        PromptKind::Permissions(sources),
                    ));
                }
            }
            Action::Modified => {
                if local.is_some() {
                    prompt = Some((
                        "Change modification time",
                        "Enter an RFC 3339 date/time including its timezone.".into(),
                        chrono::Local::now().to_rfc3339(),
                        PromptKind::Modified(sources),
                    ));
                }
            }
            Action::PermanentDelete => {
                if local.is_some() {
                    prompt = Some((
                        "Permanently delete",
                        format!(
                            "Permanently delete {} selected item(s)? This bypasses Trash and cannot be undone.",
                            chosen.len()
                        ),
                        String::new(),
                        PromptKind::PermanentDelete(sources),
                    ));
                }
            }
            Action::View | Action::Edit => {
                self.view(chosen[0].clone(), matches!(action, Action::Edit), ctx);
                return;
            }
            Action::External => {
                if local.is_some() {
                    self.remember_opened_file(sources[0].clone());
                    if let Err(e) = open::that_detached(&sources[0]) {
                        self.fail(e);
                    }
                } else {
                    self.fail("Extract this file before opening it externally.");
                }
                return;
            }
            Action::Copy | Action::Move => {
                let destination = self.panes[1 - self.active].tab().location.clone();
                if matches!(location, Location::Remote { .. })
                    || matches!(destination, Location::Remote { .. })
                {
                    prompt = Some((
                        "Transfer files",
                        format!(
                            "Transfer {} item(s). Choose how to handle existing targets.",
                            chosen.len()
                        ),
                        destination.label(),
                        PromptKind::RemoteTransfer {
                            from: location.clone(),
                            entries: chosen.clone(),
                            to: destination,
                            moving: matches!(action, Action::Move),
                        },
                    ));
                } else if let (Some(base), Some(destination)) = (local.clone(), other.clone()) {
                    let moving = matches!(action, Action::Move);
                    prompt = Some((
                        if moving { "Move files" } else { "Copy files" },
                        format!(
                            "{} selected item(s). Choose how to handle existing targets.",
                            sources.len()
                        ),
                        destination.display().to_string(),
                        PromptKind::Transfer {
                            sources,
                            moving,
                            base,
                        },
                    ));
                } else if matches!(action, Action::Copy)
                    && matches!(location, Location::Archive { .. })
                {
                    self.act(Action::Extract, ctx);
                    return;
                }
            }
            Action::Rename => {
                if let Location::Remote { connection, .. } = &location {
                    self.prompt = Some(Prompt {
                        title: "Rename remote item".into(),
                        description: "Choose a new name. Existing targets are refused.".into(),
                        value: chosen[0].name.clone(),
                        kind: PromptKind::RemoteRename {
                            connection: connection.clone(),
                            source: chosen[0].path.clone(),
                        },
                        error: None,
                        resumable: false,
                        transfer_options: Default::default(),
                        overrides: Default::default(),
                        conflicts: Vec::new(),
                        conflict_key: String::new(),
                        pending_conflicts: None,
                    });
                    return;
                }

                if local.is_some() && sources.len() == 1 {
                    prompt = Some((
                        "Rename",
                        "Enter the new name.".into(),
                        chosen[0].name.clone(),
                        PromptKind::Rename(sources[0].clone()),
                    ));
                } else if sources.len() > 1 {
                    self.fail("Select a single item to rename.");
                    return;
                }
            }
            Action::Trash => {
                if let Location::Remote { connection, .. } = &location {
                    prompt = Some((
                        "Permanently delete remote items",
                        format!(
                            "Permanently delete {} selected remote item(s)? This service has no system Trash. This cannot be undone by this app.",
                            chosen.len()
                        ),
                        String::new(),
                        PromptKind::RemoteDelete {
                            connection: connection.clone(),
                            entries: chosen.clone(),
                        },
                    ));
                }
                if local.is_some() {
                    prompt = Some((
                        "Move to Trash",
                        format!(
                            "Move {} item(s) to your system Trash?\n{}",
                            sources.len(),
                            chosen
                                .iter()
                                .take(5)
                                .map(|e| e.name.clone())
                                .collect::<Vec<_>>()
                                .join("\n")
                        ),
                        String::new(),
                        PromptKind::Trash(sources),
                    ));
                }
            }
            Action::Pack => {
                if let Some(base) = local.clone() {
                    let target = other
                        .clone()
                        .unwrap_or_else(|| base.clone())
                        .join("Archive.zip");
                    prompt = Some((
                        "Create archive",
                        "Choose a new ZIP, TAR, TAR.GZ, TAR.BZ2, TAR.XZ or 7z filename. GZ, BZ2 and XZ streams accept one file.".into(),
                        target.display().to_string(),
                        PromptKind::Pack { sources, base },
                    ));
                }
            }
            Action::Extract => {
                let (archive, prefix, selected) = match location.clone() {
                    Location::Archive { file, prefix } => (file, prefix, sources),
                    Location::Local(_) if crate::archive::format(&sources[0]).is_some() => {
                        (sources[0].clone(), PathBuf::new(), Vec::new())
                    }
                    _ => {
                        self.fail("Select a supported archive, or entries inside an archive.");
                        return;
                    }
                };
                let base = other.or(local).unwrap_or_else(filesystem::home);
                let stem = archive
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .trim_end_matches(".tar")
                    .to_owned();
                prompt = Some((
                    "Extract archive",
                    "Extract into a new folder. Links and unsafe archive paths are rejected."
                        .into(),
                    base.join(stem).display().to_string(),
                    PromptKind::Extract {
                        archive,
                        selected,
                        prefix,
                        base,
                    },
                ));
            }
            Action::FocusAddress
            | Action::FocusFilter
            | Action::SelectNextRow
            | Action::SelectPreviousRow
            | Action::SelectNextPage
            | Action::SelectPreviousPage
            | Action::SelectFirstRow
            | Action::SelectLastRow
            | Action::MarkNextRow
            | Action::MarkPreviousRow
            | Action::MarkNextPage
            | Action::MarkPreviousPage
            | Action::MarkToFirstRow
            | Action::MarkToLastRow
            | Action::MarkSelectedFile
            | Action::ClearSelection
            | Action::QuickFind
            | Action::QuickFindNext
            | Action::QuickFindPrevious
            | Action::SelectPattern
            | Action::UnselectPattern
            | Action::SelectExtension
            | Action::SelectNextBlock
            | Action::SelectPreviousBlock
            | Action::MarkNextBlock
            | Action::MarkPreviousBlock
            | Action::LockTab
            | Action::TabTitle
            | Action::MoveTabOther
            | Action::MoveTabLeft
            | Action::MoveTabRight
            | Action::SideBySide
            | Action::Stacked
            | Action::EqualPanes
            | Action::QuickLists
            | Action::RecentLocations
            | Action::RecentFiles
            | Action::BookmarksList
            | Action::TabsList
            | Action::RootsList
            | Action::OpenBoth
            | Action::ParentBoth
            | Action::ParentOther
            | Action::SwitchPane
            | Action::FocusNext
            | Action::FocusPrevious
            | Action::FileContextMenu
            | Action::AutoSizeColumns
            | Action::OpenTrash
            | Action::EmptyTrash
            | Action::NewWindow
            | Action::CloseWindow
            | Action::NextWindow
            | Action::PreviousWindow
            | Action::WindowsList
            | Action::ImportSettings
            | Action::ImportedCommands => return,
            Action::Checksum => {
                if local.is_some() && !chosen[0].is_dir {
                    self.start_job(Operation::Checksum(sources[0].clone()), ctx);
                } else {
                    self.fail("Select a local file to calculate its SHA-256 checksum.");
                }
                return;
            }
        }
        if let Some((title, description, value, kind)) = prompt {
            self.prompt = Some(Prompt {
                title: title.into(),
                description,
                value,
                kind,
                error: None,
                resumable: false,
                transfer_options: Default::default(),
                overrides: Default::default(),
                conflicts: Vec::new(),
                conflict_key: String::new(),
                pending_conflicts: None,
            });
        } else {
            self.fail("This operation requires local folders. Archive contents are read-only.");
        }
    }
    fn snapshot_settings(&self) -> Settings {
        Settings {
            window_sessions: self.saved_windows.clone(),
            tab_states: std::array::from_fn(|i| {
                self.panes[i]
                    .tabs
                    .iter()
                    .map(|t| t.state.persistent())
                    .collect()
            }),
            recent_locations: self.recent_locations.clone(),
            recent_files: self.recent_files.clone(),
            imported_commands: self.imported_commands.clone(),
            workspaces: self.workspaces.clone(),
            preferences: self.preferences.clone(),
            connections: self.connections.clone(),
            tabs: std::array::from_fn(|i| {
                self.panes[i]
                    .tabs
                    .iter()
                    .map(|t| crate::workspaces::persistent_location(&t.location))
                    .collect()
            }),
            active_tabs: [self.panes[0].active, self.panes[1].active],
            active_pane: self.active,
            bookmarks: self.bookmarks.clone(),
            show_hidden: self.show_hidden,
            dark_mode: self.preferences.color_scheme == crate::appearance::ColorScheme::Dark,
        }
    }
    fn save_settings(&self) {
        if !self.settings_valid {
            return;
        }
        let settings = self.snapshot_settings();
        if let Err(error) = settings::save(&settings) {
            eprintln!("Could not save settings: {error:#}");
        }
    }
    fn native_drag(&mut self, paths: Vec<PathBuf>) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            if let Some(window) = &self.native_window {
                if let Err(error) = crate::native_files::drag_out(window, paths) {
                    self.fail(error);
                }
            } else {
                self.fail("Native window handle unavailable.");
            }
        }
        #[cfg(target_os = "linux")]
        {
            if let Err(e) = crate::linux_drag::launch(&paths) {
                self.fail(e);
            } else {
                self.status =
                    "Drag files from the native GTK window into another application.".into();
            }
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            if let Err(e) = self.native_files.copy(&paths, false) {
                self.fail(e);
            }
        }
    }
    fn accept_prompt(&mut self, ctx: &UiSignal) {
        let Some(mut prompt) = self.prompt.take() else {
            return;
        };
        let result = (|| -> anyhow::Result<Option<Operation>> {
            Ok(Some(match &prompt.kind {
                PromptKind::DropTransfer {
                    selection,
                    to,
                    moving,
                } => Operation::DropTransfer {
                    selection: selection.clone(),
                    to: filesystem::parse_location(&prompt.value, to)?,
                    moving: *moving,
                    options: prompt.transfer_options,
                    overrides: prompt.overrides.clone(),
                },
                PromptKind::Mkfile(base) => {
                    filesystem::validate_name(&prompt.value)?;
                    Operation::Mkfile(base.join(&prompt.value))
                }
                PromptKind::Split { source, base } => Operation::Split {
                    source: source.clone(),
                    destination: base.join(format!(
                        "{}.parts",
                        source.file_name().unwrap_or_default().to_string_lossy()
                    )),
                    bytes: prompt
                        .value
                        .parse::<u64>()?
                        .checked_mul(1024 * 1024)
                        .ok_or_else(|| anyhow::anyhow!("Part size overflow"))?,
                },
                PromptKind::Combine { sources, base } => Operation::Combine {
                    sources: sources.clone(),
                    destination: filesystem::resolve_path(&prompt.value, base),
                },
                PromptKind::Permissions(paths) => Operation::Permissions {
                    paths: paths.clone(),
                    mode: u32::from_str_radix(prompt.value.trim_start_matches("0o"), 8)?,
                },
                PromptKind::Modified(paths) => Operation::Modified {
                    paths: paths.clone(),
                    time: chrono::DateTime::parse_from_rfc3339(&prompt.value)?.into(),
                },
                PromptKind::PermanentDelete(paths) => Operation::PermanentDelete(paths.clone()),
                PromptKind::RemoteTransfer {
                    from,
                    entries,
                    to,
                    moving,
                } if prompt.resumable => {
                    anyhow::ensure!(
                        !moving
                            && prompt.transfer_options.conflicts
                                == crate::transfer::ConflictPolicy::Refuse
                            && !prompt.transfer_options.merge_folders
                            && prompt.overrides.is_empty(),
                        "Resumable copies require Stop on conflicts, new target folders, and no per-file overrides"
                    );
                    Operation::ResumeRemote {
                        id: format!("batch-{:016x}", rand::random::<u64>()),
                        from: from.clone(),
                        entries: entries.clone(),
                        to: filesystem::parse_location(&prompt.value, to)?,
                    }
                }
                PromptKind::RemoteTransfer {
                    from,
                    entries,
                    to,
                    moving,
                } => Operation::RemoteTransferResolved {
                    from: from.clone(),
                    entries: entries.clone(),
                    to: filesystem::parse_location(&prompt.value, to)?,
                    moving: *moving,
                    options: prompt.transfer_options,
                    overrides: prompt.overrides.clone(),
                },
                PromptKind::RemoteMkdir { connection, path } => {
                    filesystem::validate_name(&prompt.value)?;
                    Operation::RemoteMkdir {
                        connection: connection.clone(),
                        path: path.join(&prompt.value),
                    }
                }
                PromptKind::RemoteDelete {
                    connection,
                    entries,
                } => Operation::RemoteDelete {
                    connection: connection.clone(),
                    entries: entries.clone(),
                },
                PromptKind::Transfer {
                    sources,
                    moving,
                    base,
                } => {
                    let destination = filesystem::resolve_path(&prompt.value, base);
                    Operation::ResolvedTransfer {
                        sources: sources.clone(),
                        destination,
                        moving: *moving,
                        options: prompt.transfer_options,
                        overrides: prompt.overrides.clone(),
                    }
                }
                PromptKind::RemoteRename { connection, source } => Operation::RemoteRename {
                    connection: connection.clone(),
                    source: source.clone(),
                    name: prompt.value.clone(),
                },
                PromptKind::Rename(source) => {
                    filesystem::validate_name(&prompt.value)?;
                    Operation::Rename {
                        source: source.clone(),
                        name: prompt.value.clone(),
                    }
                }
                PromptKind::Mkdir(base) => {
                    filesystem::validate_name(&prompt.value)?;
                    Operation::Mkdir(base.join(&prompt.value))
                }
                PromptKind::Trash(sources) => Operation::Trash(sources.clone()),
                PromptKind::Pack { sources, base } => Operation::Pack {
                    sources: sources.clone(),
                    destination: filesystem::resolve_path(&prompt.value, base),
                },
                PromptKind::Extract {
                    archive,
                    selected,
                    prefix,
                    base,
                } => Operation::ExtractResolved {
                    options: prompt.transfer_options,
                    overrides: prompt.overrides.clone(),
                    archive: archive.clone(),
                    selected: selected.clone(),
                    prefix: prefix.clone(),
                    destination: filesystem::resolve_path(&prompt.value, base),
                },
                PromptKind::Bookmark(location) => {
                    if prompt.value.trim().is_empty() {
                        anyhow::bail!("Enter a bookmark name.");
                    }
                    self.bookmarks.push(Bookmark {
                        name: prompt.value.clone(),
                        location: location.clone(),
                    });
                    self.status = "Bookmark added.".into();
                    self.status_error = false;
                    return Ok(None);
                }
            }))
        })();
        match result {
            Ok(Some(operation)) => {
                self.start_job(operation, ctx);
                return;
            }
            Ok(None) => return,
            Err(error) => prompt.error = Some(format!("{error:#}")),
        }
        self.prompt = Some(prompt);
    }
    fn poll_prompt(&mut self, ctx: &UiSignal) {
        let Some(mut prompt) = self.prompt.take() else {
            return;
        };
        if let Some(result) = crate::background::poll(&mut prompt.pending_conflicts) {
            match result {
                Ok(conflicts) => prompt.conflicts = conflicts,
                Err(error) => prompt.error = Some(error),
            }
        }
        if let PromptKind::Transfer { sources, base, .. } = &prompt.kind {
            let destination = filesystem::resolve_path(&prompt.value, base);
            let key = format!(
                "{}:{}",
                destination.display(),
                prompt.transfer_options.merge_folders
            );
            if key != prompt.conflict_key && prompt.pending_conflicts.is_none() {
                prompt.conflict_key = key;
                let sources = sources.clone();
                let merge = prompt.transfer_options.merge_folders;
                let (sender, receiver) = mpsc::channel();
                prompt.pending_conflicts = Some(receiver);
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let result = crate::transfer::conflicts(&sources, &destination, merge)
                        .map_err(|error| format!("{error:#}"));
                    let _ = sender.send(result);
                    ctx.request_repaint();
                });
            }
            if prompt.pending_conflicts.is_some() {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        if let PromptKind::Extract {
            archive,
            selected,
            prefix,
            base,
        } = &prompt.kind
        {
            let destination = filesystem::resolve_path(&prompt.value, base);
            let key = format!(
                "{}:{}",
                destination.display(),
                prompt.transfer_options.merge_folders
            );
            if key != prompt.conflict_key && prompt.pending_conflicts.is_none() {
                prompt.conflict_key = key;
                let (archive, selected, prefix) =
                    (archive.clone(), selected.clone(), prefix.clone());
                let merge = prompt.transfer_options.merge_folders;
                let (sender, receiver) = mpsc::channel();
                prompt.pending_conflicts = Some(receiver);
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let result = crate::archive::extraction_conflicts(
                        &archive,
                        &selected,
                        &prefix,
                        &destination,
                        merge,
                    )
                    .map_err(|e| format!("{e:#}"));
                    let _ = sender.send(result);
                    ctx.request_repaint();
                });
            }
        }
        if let PromptKind::DropTransfer { selection, to, .. } = &prompt.kind {
            let key = format!("{}:{}", prompt.value, prompt.transfer_options.merge_folders);
            if key != prompt.conflict_key && prompt.pending_conflicts.is_none() {
                prompt.conflict_key = key;
                let selection = selection.clone();
                let to = filesystem::parse_location(&prompt.value, to);
                let merge = prompt.transfer_options.merge_folders;
                let (sender, receiver) = mpsc::channel();
                prompt.pending_conflicts = Some(receiver);
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let result = to
                        .and_then(|to| crate::drag_drop::conflicts(&selection, &to, merge))
                        .map_err(|e| format!("{e:#}"));
                    let _ = sender.send(result);
                    ctx.request_repaint();
                });
            }
        }
        if let PromptKind::RemoteTransfer {
            from, entries, to, ..
        } = &prompt.kind
        {
            let key = format!("{}:{}", prompt.value, prompt.transfer_options.merge_folders);
            if key != prompt.conflict_key && prompt.pending_conflicts.is_none() {
                prompt.conflict_key = key;
                let from = from.clone();
                let merge = prompt.transfer_options.merge_folders;
                let entries = entries.clone();
                let to = filesystem::parse_location(&prompt.value, to);
                let (sender, receiver) = mpsc::channel();
                prompt.pending_conflicts = Some(receiver);
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let result = to
                        .and_then(|to| {
                            crate::remote::conflicts_with_merge(&from, &entries, &to, merge)
                        })
                        .map_err(|error| format!("{error:#}"));
                    let _ = sender.send(result);
                    ctx.request_repaint();
                });
            }
            if prompt.pending_conflicts.is_some() {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        self.prompt = Some(prompt);
    }
    fn apply_feature_effect(&mut self, effect: crate::next_ui::Effect, ctx: &UiSignal) {
        use crate::next_ui::Effect;
        match effect {
            Effect::Run(op) => self.start_job(*op, ctx),
            Effect::SaveHistory => {
                if let Err(e) = crate::history::save(&self.history) {
                    self.fail(e);
                }
            }
            Effect::ReloadHistory => match crate::history::load() {
                Ok(r) => self.history = r,
                Err(e) => self.fail(e),
            },
            Effect::SaveWorkspace(name) => {
                match crate::workspaces::Workspace::validate_name(&name) {
                    Ok(name) => {
                        if self.workspaces.iter().any(|w| w.name == name) {
                            self.features.error=Some("A workspace already uses that name. Rename it or choose another name.".into());
                        } else {
                            let w = crate::workspaces::Workspace {
                                name,
                                tab_states: std::array::from_fn(|i| {
                                    self.panes[i]
                                        .tabs
                                        .iter()
                                        .map(|t| t.state.persistent())
                                        .collect()
                                }),
                                tabs: std::array::from_fn(|i| {
                                    self.panes[i]
                                        .tabs
                                        .iter()
                                        .map(|t| {
                                            crate::workspaces::persistent_location(&t.location)
                                        })
                                        .collect()
                                }),
                                active_tabs: [self.panes[0].active, self.panes[1].active],
                                active_pane: self.active,
                                preferences: self.preferences.clone(),
                            };
                            self.workspaces.push(w);
                            self.features.error = None;
                        }
                    }
                    Err(e) => self.features.error = Some(e.to_string()),
                }
            }
            Effect::RenameWorkspace(i, name) => {
                match crate::workspaces::Workspace::validate_name(&name) {
                    Ok(name)
                        if !self
                            .workspaces
                            .iter()
                            .enumerate()
                            .any(|(n, w)| n != i && w.name == name) =>
                    {
                        if let Some(w) = self.workspaces.get_mut(i) {
                            w.name = name;
                        }
                        self.features.error = None;
                    }
                    _ => {
                        self.features.error =
                            Some("Choose a unique workspace name of 1–80 characters.".into())
                    }
                }
            }
            Effect::DeleteWorkspace(i) => {
                if i < self.workspaces.len() {
                    self.workspaces.remove(i);
                }
            }
            Effect::LoadWorkspace(i) => {
                if let Some(w) = self.workspaces.get(i).cloned() {
                    self.panes = std::array::from_fn(|side| {
                        let mut tabs = w.tabs[side]
                            .iter()
                            .take(32)
                            .cloned()
                            .enumerate()
                            .map(|(index, l)| {
                                let mut tab = Tab::new(l, ctx);
                                tab.state =
                                    w.tab_states[side].get(index).cloned().unwrap_or_default();
                                tab
                            })
                            .collect::<Vec<_>>();
                        if tabs.is_empty() {
                            tabs.push(Tab::new(Location::Local(filesystem::home()), ctx));
                        }
                        Pane {
                            active: w.active_tabs[side].min(tabs.len() - 1),
                            tabs,
                        }
                    });
                    self.active = w.active_pane.min(1);
                    self.preferences = w.preferences;
                    self.preferences.upgrade();
                    crate::i18n::set(self.preferences.language);
                    #[cfg(target_os = "macos")]
                    if let Some(menu) = &self.native_menu {
                        menu.localize();
                    }
                    self.pending_view = None;
                    self.pending_archive_target = None;
                    self.pending_archive = None;
                }
            }
        }
        self.save_settings();
    }
    fn apply_archive_action(&mut self, action: crate::feature_ui::ArchiveAction, ctx: &UiSignal) {
        match action {
            crate::feature_ui::ArchiveAction::Browse(file) => {
                self.tab_mut().navigate(
                    Location::Archive {
                        file,
                        prefix: PathBuf::new(),
                    },
                    ctx,
                );
            }
            crate::feature_ui::ArchiveAction::Run(operation) => self.start_job(*operation, ctx),
            crate::feature_ui::ArchiveAction::Unlock(file, password) => {
                let (sender, receiver) = mpsc::channel();
                self.pending_archive = Some(receiver);
                self.pending_archive_target = Some((self.active, self.tab().id));
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let result = crate::archive::unlock(&file, &password)
                        .map(|()| Location::Archive {
                            file,
                            prefix: PathBuf::new(),
                        })
                        .map_err(|error| format!("{error:#}"));
                    let _ = sender.send(result);
                    ctx.request_repaint();
                });
            }
        }
    }
    fn connect_form(&mut self, form: &mut ConnectionForm, authorize: bool, ctx: &UiSignal) {
        if form.connection.service == "sftp" && !form.secrets.password.is_empty() {
            form.connection.native_sftp = true;
        }
        let ticket = form
            .navigation
            .clone()
            .unwrap_or_else(|| self.uri_ticket(form.connection.initial_path.clone()));
        form.navigation = Some(ticket.clone());
        form.error = None;
        self.start_uri(
            ticket,
            form.connection.clone(),
            form.secrets.clone(),
            form.remember,
            authorize,
            ctx,
        );
    }
    fn save_editor(&mut self, editor: &Editor, ctx: &UiSignal) {
        if !editor.editable || !editor.dirty() || self.job.is_some() || !self.queue.is_empty() {
            return;
        }
        let bytes = match editor
            .document
            .as_ref()
            .map(|doc| doc.encode(&editor.text))
            .unwrap_or_else(|| Ok(editor.text.as_bytes().to_vec()))
        {
            Ok(bytes) => bytes,
            Err(error) => {
                self.fail(error);
                return;
            }
        };
        let original = editor
            .document
            .as_ref()
            .map(|doc| doc.original.clone())
            .unwrap_or_else(|| editor.original.as_bytes().to_vec());
        let operation = if let Some((connection, path)) = &editor.remote {
            Some(Operation::RemoteSaveBytes {
                connection: connection.clone(),
                path: path.clone(),
                original: original.clone(),
                bytes: bytes.clone(),
            })
        } else {
            editor.path.as_ref().map(|path| Operation::SaveBytes {
                path: path.clone(),
                original: original.clone(),
                bytes: bytes.clone(),
            })
        };
        if let Some(operation) = operation {
            self.start_job(operation, ctx);
            if self.job.is_some() {
                self.pending_save = Some((editor.title.clone(), editor.text.clone()));
            }
        }
    }
    fn drop_selection(
        &mut self,
        side: usize,
        selection: crate::drag_drop::Selection,
        to: Location,
        moving: bool,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.job.is_none() && self.prompt.is_none(),
            "Finish the current operation before dropping files."
        );
        selection.check_target(&to, moving)?;
        self.active = side.min(1);
        self.prompt = Some(Prompt {
            title: "Transfer dropped files".into(),
            description: format!(
                "{} item(s) from {}. Review the destination and conflict choices before transferring.",
                selection.entries.len(),
                selection.from.label()
            ),
            value: to.label(),
            kind: PromptKind::DropTransfer {
                selection,
                to,
                moving,
            },
            error: None,
            resumable: false,
            transfer_options: Default::default(),
            overrides: Default::default(),
            conflicts: Vec::new(),
            conflict_key: String::new(),
            pending_conflicts: None,
        });
        Ok(())
    }
    fn drop_files(
        &mut self,
        side: usize,
        sources: Vec<PathBuf>,
        destination: Location,
        _ctx: &UiSignal,
    ) {
        if self.job.is_none()
            && self.prompt.is_none()
            && self.editor.is_none()
            && !self.tools.active()
            && self.preferences_form.is_none()
            && self.archive_form.is_none()
            && !self.show_history
            && !self.features.active()
            && self.connection_form.is_none()
            && !self.help
            && self.message.is_none()
        {
            self.active = side;
            match destination {
                Location::Archive { file, prefix } => {
                    self.archive_form = Some(crate::feature_ui::ArchiveForm {
                        file: file.display().to_string(),
                        prefix,
                        additions: sources
                            .iter()
                            .map(|p| p.display().to_string())
                            .collect::<Vec<_>>()
                            .join("\n"),
                        ..Default::default()
                    });
                }
                Location::Remote { .. } => {
                    match sources
                        .iter()
                        .map(|p| filesystem::entry_from_path(p))
                        .collect::<anyhow::Result<Vec<_>>>()
                    {
                        Ok(entries) => {
                            self.prompt = Some(Prompt {
                                title: "Copy dropped files".into(),
                                description: format!(
                                    "Upload {} dropped item(s). Review conflicts before transferring.",
                                    entries.len()
                                ),
                                value: destination.label(),
                                kind: PromptKind::RemoteTransfer {
                                    from: Location::Local(filesystem::home()),
                                    entries,
                                    to: destination,
                                    moving: false,
                                },
                                error: None,
                                resumable: false,
                                transfer_options: Default::default(),
                                overrides: Default::default(),
                                conflicts: Vec::new(),
                                conflict_key: String::new(),
                                pending_conflicts: None,
                            });
                        }
                        Err(e) => self.fail(e),
                    }
                }
                Location::Local(destination) => {
                    self.prompt = Some(Prompt {
                        title: "Copy dropped files".into(),
                        description: format!(
                            "Copy {} dropped item(s) into this pane. Choose how to handle existing targets.",
                            sources.len()
                        ),
                        value: destination.display().to_string(),
                        kind: PromptKind::Transfer {
                            sources,
                            moving: false,
                            base: destination,
                        },
                        error: None,
                        resumable: false,
                        transfer_options: Default::default(),
                        overrides: Default::default(),
                        conflicts: Vec::new(),
                        conflict_key: String::new(),
                        pending_conflicts: None,
                    });
                }
            }
        } else {
            self.fail("Finish or close the current dialog before dropping files.");
        }
    }
}

pub mod slint_ui;

fn location_is_local_with_rule(
    location: &Location,
    rules: &[crate::associations::Rule],
    path: &Path,
) -> bool {
    matches!(location, Location::Local(_))
        && crate::associations::application(rules, path).is_some()
}
