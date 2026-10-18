use crate::{
    filesystem::{self, Entry, Location},
    model::{Pane, Tab},
    operations::{self, Operation, Progress, Task},
    settings::{self, Bookmark, Settings},
};
use eframe::egui::{self, Color32, Key, RichText, Stroke, Vec2};
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

fn focus_text(response: &egui::Response, text: &str) {
    response.request_focus();
    if let Some(mut state) = egui::TextEdit::load_state(&response.ctx, response.id) {
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(text.chars().count()),
            )));
        state.store(&response.ctx, response.id);
    }
}

fn file_icon(
    painter: &egui::Painter,
    position: egui::Pos2,
    entry: &Entry,
    marked: bool,
    color: Color32,
) {
    let point = |x, y| position + Vec2::new(x, y);
    let stroke = Stroke::new(1.2_f32, color);
    if marked {
        painter.line_segment([point(0.0, 7.0), point(4.0, 11.0)], stroke);
        painter.line_segment([point(4.0, 11.0), point(12.0, 1.0)], stroke);
    } else if entry.is_symlink {
        painter.line_segment([point(1.0, 12.0), point(11.0, 2.0)], stroke);
        painter.line_segment([point(4.0, 2.0), point(11.0, 2.0)], stroke);
        painter.line_segment([point(11.0, 2.0), point(11.0, 9.0)], stroke);
    } else if entry.is_dir {
        painter.rect_filled(
            egui::Rect::from_min_size(point(0.0, 2.0), Vec2::new(7.0, 4.0)),
            1.0,
            color,
        );
        painter.rect_filled(
            egui::Rect::from_min_size(point(0.0, 5.0), Vec2::new(14.0, 10.0)),
            1.5,
            color,
        );
    } else {
        painter.rect_stroke(
            egui::Rect::from_min_size(point(2.0, 0.0), Vec2::new(10.0, 15.0)),
            1.0,
            stroke,
            egui::StrokeKind::Inside,
        );
        if entry.kind() == "Archive" {
            for y in [3.0, 6.0, 9.0, 12.0] {
                painter.line_segment([point(6.0, y), point(9.0, y)], stroke);
            }
        } else {
            painter.line_segment([point(4.0, 5.0), point(10.0, 5.0)], stroke);
            painter.line_segment([point(4.0, 8.0), point(10.0, 8.0)], stroke);
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Action {
    Undo,
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

pub(crate) fn action_menus() -> Vec<(&'static str, Vec<(&'static str, Action)>)> {
    Vec::from([
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
                ("Native drag-out", Action::NativeDrag),
            ],
        ),
    ])
}

struct Prompt {
    title: String,
    description: String,
    value: String,
    kind: PromptKind,
    focus: bool,
    error: Option<String>,
    transfer_options: crate::transfer::TransferOptions,
    overrides: std::collections::BTreeMap<PathBuf, crate::transfer::ConflictPolicy>,
    conflicts: Vec<crate::transfer::Conflict>,
    conflict_key: String,
    pending_conflicts: Option<Receiver<Result<Vec<crate::transfer::Conflict>, String>>>,
}
enum PromptKind {
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
    pdf: Option<PdfState>,
    image: Option<egui::ColorImage>,
    texture: Option<egui::TextureHandle>,
    zoom: f32,
    kind: &'static str,
    title: String,
    path: Option<PathBuf>,
    text: String,
    original: String,
    editable: bool,
    error: Option<String>,
    confirm_close: bool,
}
struct PdfState {
    bytes: Arc<Vec<u8>>,
    page: usize,
    pages: usize,
}
type PdfResult = Result<(String, usize, egui::ColorImage), String>;
impl Editor {
    fn dirty(&self) -> bool {
        self.editable && self.text != self.original
    }
}

enum JobEvent {
    Progress(Progress),
    Done(Result<crate::history::Record, String>),
}
struct Job {
    label: String,
    undo_of: Option<String>,
    receiver: Receiver<JobEvent>,
    cancel: Arc<AtomicBool>,
    progress: Progress,
}

#[derive(Clone)]
struct FileDrag {
    sources: Vec<PathBuf>,
}

#[derive(Default)]
struct ConnectionForm {
    connection: crate::remote::Connection,
    secrets: crate::remote::Secrets,
    remember: bool,
    error: Option<String>,
}

pub struct Commander {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    native_window: Option<crate::native_files::Window>,
    native_files: crate::native_files::NativeFiles,
    queue: std::collections::VecDeque<Operation>,
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
    pending_connection: Option<Receiver<Result<crate::remote::Connection, String>>>,
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
    pub fn new(cc: &eframe::CreationContext<'_>, paths: &[PathBuf]) -> Self {
        let app = Self::with_settings(&cc.egui_ctx, paths, settings::load());
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        let mut app = app;
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            use raw_window_handle::HasWindowHandle;
            app.native_window = cc
                .window_handle()
                .ok()
                .map(|handle| crate::native_files::Window(handle.as_raw()));
        }
        #[cfg(target_os = "macos")]
        match crate::native_menu::NativeMenu::new(&cc.egui_ctx) {
            Ok(menu) => app.native_menu = Some(menu),
            Err(error) => app.fail(format!("Native menu unavailable: {error}")),
        }
        app
    }

    fn with_settings(
        ctx: &egui::Context,
        paths: &[PathBuf],
        loaded: anyhow::Result<Settings>,
    ) -> Self {
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
                .map(|l| Tab::new(l, ctx))
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
        crate::appearance::install_fonts(ctx, settings.preferences.desktop_style);
        crate::i18n::set(settings.preferences.language);
        let app = Self {
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            native_window: None,
            native_files: Default::default(),
            queue: Default::default(),
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
            pending_connection: None,
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
        };
        app.theme(ctx);
        app
    }

    fn theme(&self, ctx: &egui::Context) {
        crate::appearance::apply(
            ctx,
            self.preferences.desktop_style,
            self.preferences.color_scheme,
            self.preferences.font_size,
        );
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

    fn refresh(&mut self, ctx: &egui::Context) {
        for pane in &mut self.panes {
            for tab in &mut pane.tabs {
                tab.refresh(ctx);
            }
        }
    }

    fn start_job(&mut self, operation: Operation, ctx: &egui::Context) {
        if self.job.is_some() {
            if self.queue.len() >= 1000 {
                self.fail("The transfer queue is full. Finish or cancel queued operations.");
                return;
            }
            self.queue.push_back(operation);
            self.status = format!("Queued {} operation(s).", self.queue.len());
            return;
        }
        let label = operation.label().to_owned();
        let undo_of = if let Operation::Undo(record) = &operation {
            Some(record.id.clone())
        } else {
            None
        };
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
            let _ = sender.send(JobEvent::Done(result));
            worker_ctx.request_repaint();
        });
        self.job = Some(Job {
            label,
            undo_of,
            receiver,
            cancel,
            progress: Progress::default(),
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        #[cfg(target_os = "macos")]
        if let Some(menu) = &self.native_menu {
            let blocked = self.archive_form.is_some()
                || self.show_history
                || self.prompt.is_some()
                || self.editor.is_some()
                || self.help
                || self.message.is_some()
                || self.confirm_exit
                || self.tools.active()
                || self.preferences_form.is_some()
                || self.connection_form.is_some()
                || (self.terminal_visible && self.terminal.as_ref().is_some_and(|t| t.focused));
            for action in menu.poll(blocked) {
                self.act(action, ctx);
            }
        }
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
                Ok((title, page, image)) => {
                    if let Some(editor) = &mut self.editor
                        && editor.title == title
                        && let Some(pdf) = &mut editor.pdf
                    {
                        pdf.page = page;
                        editor.image = Some(image);
                    }
                }
                Err(error) => self.fail(error),
            }
        }
        if let Some(result) = crate::background::poll(&mut self.pending_connection) {
            match result {
                Ok(connection) => {
                    self.connections.retain(|c| c.id != connection.id);
                    self.connections.push(connection.clone());
                    self.save_settings();
                    self.connection_form = None;
                    self.tab_mut().navigate(
                        Location::Remote {
                            connection: connection.id,
                            path: PathBuf::new(),
                        },
                        ctx,
                    );
                }
                Err(error) => {
                    if let Some(form) = &mut self.connection_form {
                        form.error = Some(error);
                    } else {
                        self.fail(error);
                    }
                }
            }
        }
        for pane in &mut self.panes {
            for tab in &mut pane.tabs {
                tab.folders_first = self.preferences.folders_first;
                if self.preferences.auto_refresh {
                    tab.poll_changes(ctx);
                }
                tab.poll(self.show_hidden);
            }
        }
        let mut completed = None;
        if let Some(job) = &mut self.job {
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
            let undo_of = self.job.as_ref().and_then(|job| job.undo_of.clone());
            self.job = None;
            match result {
                Ok(record) => {
                    if record.success
                        && let Some(id) = undo_of
                        && let Some(original) =
                            self.history.iter_mut().find(|record| record.id == id)
                    {
                        original.undone = true;
                    }
                    self.status = record.result.clone();
                    self.status_error = !record.success;
                    if record.result.contains("Replaced originals are kept")
                        || record.result.starts_with("Checksums")
                        || record.result.starts_with("Properties")
                        || record.result.starts_with("Folder comparison")
                    {
                        self.message = Some(record.result.clone());
                    }
                    self.history.push(record);
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
            && !self.exiting
            && let Some(operation) = self.queue.pop_front()
        {
            self.start_job(operation, ctx);
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
            || self.pending_connection.is_some()
            || self.pending_archive.is_some()
            || self.panes.iter().any(|p| p.tabs.iter().any(|t| t.loading))
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if self.exiting && self.job.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn open_entry(&mut self, entry: Entry, ctx: &egui::Context) {
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
        } else {
            self.view(entry, false, ctx);
        }
    }

    fn view(&mut self, entry: Entry, editable: bool, ctx: &egui::Context) {
        if entry.is_dir {
            self.fail("Select a text file to view or edit.");
            return;
        }
        let location = self.tab().location.clone();
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        self.pending_view = Some(receiver);
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<Editor> {
                let (text, image, kind, pdf) = match crate::viewer::load(&location, &entry.path)? {
                    crate::viewer::Content::Text(text) => (text, None, "Text viewer", None),
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
                        Some(PdfState {
                            bytes,
                            page: 0,
                            pages,
                        }),
                    ),
                };
                let path = location.local().map(|_| entry.path.clone());
                Ok(Editor {
                    pdf,
                    image,
                    texture: None,
                    zoom: 1.0,
                    kind,
                    title: entry.path.display().to_string(),
                    editable: editable
                        && kind == "Text viewer"
                        && path.is_some()
                        && !entry.is_symlink,
                    original: text.clone(),
                    text,
                    path,
                    error: None,
                    confirm_close: false,
                })
            })()
            .map_err(|e| format!("{e:#}"));
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }

    fn act(&mut self, action: Action, ctx: &egui::Context) {
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
            && crate::archive::format(file) == Some(crate::archive::Format::Zip)
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
                && crate::archive::format(file) == Some(crate::archive::Format::Zip)
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
            Action::Undo => {
                if self.job.is_some() || !self.queue.is_empty() {
                    self.fail("Finish or cancel queued operations before undoing.");
                    return;
                }
                if let Some(record) = self
                    .history
                    .iter()
                    .rev()
                    .find(|record| !record.undone && !record.changes.is_empty())
                    .cloned()
                {
                    self.start_job(Operation::Undo(Box::new(record)), ctx);
                } else {
                    self.fail("No operation is available to undo.");
                }
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
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
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
                if let Some(path) = local {
                    self.tools.search(path, self.show_hidden);
                } else {
                    self.fail("Recursive search currently requires a local folder.");
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
                            focus: true,
                            error: None,
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
                        focus: true,
                        error: None,
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
                focus: true,
                error: None,
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

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.prompt.is_some()
            || self.editor.is_some()
            || self.help
            || self.message.is_some()
            || self.confirm_exit
            || self.connection_form.is_some()
            || self.archive_form.is_some()
            || self.show_history
            || self.tools.active()
            || self.preferences_form.is_some()
            || (self.terminal_visible && self.terminal.as_ref().is_some_and(|t| t.focused))
        {
            return;
        }
        let typing = ctx.wants_keyboard_input();
        let mut action = None;
        ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::COMMAND, Key::L) {
                self.tab_mut().focus_address = true;
            }
            if input.consume_key(egui::Modifiers::COMMAND, Key::F) {
                self.tab_mut().focus_filter = true;
            }
            if typing {
                return;
            }
            let command = input.modifiers.command;
            let shift = input.modifiers.shift;
            if input.key_pressed(Key::Tab) {
                self.active = 1 - self.active;
            }
            if input.key_pressed(Key::ArrowUp) {
                let h = self.show_hidden;
                self.tab_mut().move_cursor(-1, shift, h);
            }
            if input.key_pressed(Key::ArrowDown) {
                let h = self.show_hidden;
                self.tab_mut().move_cursor(1, shift, h);
            }
            if input.key_pressed(Key::PageUp) {
                let h = self.show_hidden;
                self.tab_mut().move_cursor(-15, shift, h);
            }
            if input.key_pressed(Key::PageDown) {
                let h = self.show_hidden;
                self.tab_mut().move_cursor(15, shift, h);
            }
            if input.key_pressed(Key::Home) {
                let h = self.show_hidden;
                self.tab_mut().move_cursor(isize::MIN, shift, h);
            }
            if input.key_pressed(Key::End) {
                let h = self.show_hidden;
                self.tab_mut().move_cursor(isize::MAX, shift, h);
            }
            if (input.key_pressed(Key::Space) || input.key_pressed(Key::Insert))
                && let Some(path) = self.tab().cursor.clone()
                && !self.tab_mut().selected.remove(&path)
            {
                self.tab_mut().selected.insert(path);
            }
            if input.key_pressed(Key::Escape) {
                self.tab_mut().selected.clear();
                self.tab_mut().filter.clear();
            }
            if command && input.key_pressed(Key::A) {
                let entries: Vec<_> = self
                    .tab()
                    .visible(self.show_hidden)
                    .iter()
                    .map(|&i| self.tab().entries[i].path.clone())
                    .collect();
                self.tab_mut().selected = entries.into_iter().collect();
            }
            if input.key_pressed(Key::Enter)
                && let Some(entry) = self
                    .tab()
                    .entries
                    .iter()
                    .find(|e| Some(&e.path) == self.tab().cursor.as_ref())
                    .cloned()
            {
                self.open_entry(entry, ctx);
            }
            if input.modifiers.alt && input.key_pressed(Key::ArrowLeft) {
                self.tab_mut().history(false, ctx);
            }
            if input.modifiers.alt && input.key_pressed(Key::ArrowRight) {
                self.tab_mut().history(true, ctx);
            }
            for candidate in [
                Action::Help,
                Action::Rename,
                Action::View,
                Action::Edit,
                Action::Copy,
                Action::Move,
                Action::Mkdir,
                Action::Trash,
                Action::NewTab,
                Action::CloseTab,
                Action::Refresh,
                Action::Bookmark,
                Action::Search,
                Action::CopyClipboard,
                Action::CutClipboard,
                Action::PasteClipboard,
                Action::Undo,
                Action::Queue,
            ] {
                if let Some(binding) = self.preferences.shortcuts.get(&format!("{candidate:?}"))
                    && let Ok(shortcut) = crate::preferences::shortcut(binding)
                    && input.consume_shortcut(&shortcut)
                {
                    action = Some(candidate);
                }
            }
            if input.key_pressed(Key::Delete) {
                action = Some(Action::Trash);
            }
            if input.key_pressed(Key::Backspace) {
                action = Some(Action::Parent);
            }
            if (cfg!(target_os = "macos") && command && shift && input.key_pressed(Key::Period))
                || (!cfg!(target_os = "macos") && command && input.key_pressed(Key::H))
            {
                self.show_hidden = !self.show_hidden;
            }
            if command && input.key_pressed(Key::Comma) {
                action = Some(Action::Preferences);
            }
            if cfg!(target_os = "macos") && command && input.key_pressed(Key::Backspace) {
                action = Some(Action::Trash);
            }
        });
        if let Some(action) = action {
            self.act(action, ctx);
        }
    }

    fn top(&mut self, ctx: &egui::Context) {
        let palette = crate::appearance::Palette::new(
            self.preferences.desktop_style,
            ctx.style().visuals.dark_mode,
        );
        let mut action = None;
        let mut appearance_changed = false;
        #[cfg(target_os = "macos")]
        let native = self.native_menu.is_some();
        #[cfg(not(target_os = "macos"))]
        let native = false;
        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(palette.toolbar)
                    .inner_margin(egui::Margin::symmetric(12, 8)),
            )
            .show(ctx, |ui| {
                #[cfg(target_os = "macos")]
                {
                    // Leave the real AppKit traffic lights above the content.
                    // Only the toolbar background acts as our drag surface.
                    let titlebar = egui::Rect::from_min_size(
                        ui.max_rect().min,
                        Vec2::new(ui.available_width(), 26.0),
                    );
                    let drag =
                        ui.interact(titlebar, ui.id().with("titlebar_drag"), egui::Sense::drag());
                    if drag.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                }
                if !native {
                    egui::MenuBar::new().ui(ui, |ui| {
                        for (title, entries) in action_menus() {
                            ui.menu_button(tr(title), |ui| {
                                for (label, a) in entries {
                                    if ui.button(tr(label)).clicked() {
                                        action = Some(a);
                                        ui.close();
                                    }
                                }
                                if title == "File" && ui.button(tr("Quit")).clicked() {
                                    action = Some(Action::Quit);
                                    ui.close();
                                }
                            });
                        }
                        if ui.button(tr("Help")).clicked() {
                            action = Some(Action::Help);
                        }
                    });
                    ui.add_space(4.0);
                }
                ui.horizontal(|ui| {
                    if cfg!(target_os = "macos") {
                        ui.add_space(80.0);
                    }
                    ui.label(RichText::new(crate::APP_NAME).strong());
                    ui.separator();
                    if self.preferences.toolbar {
                        for item in &self.preferences.toolbar_actions {
                            let label = tr(item.label());
                            let a = item.action();
                            if ui
                                .add_enabled(!self.exiting, egui::Button::new(tr(label.as_ref())))
                                .clicked()
                            {
                                action = Some(a);
                            }
                        }
                        ui.menu_button(tr("More"), |ui| {
                            for (label, a) in [
                                ("Rename…", Action::Rename),
                                ("Create archive…", Action::Pack),
                                ("Extract archive…", Action::Extract),
                                ("Checksums", Action::Checksum),
                                ("Batch rename…", Action::BatchRename),
                                ("Open terminal", Action::Terminal),
                                ("Archive tools…", Action::ArchiveTools),
                                ("Undo", Action::Undo),
                                ("Transfer queue and history…", Action::Queue),
                            ] {
                                if ui.button(tr(label)).clicked() {
                                    action = Some(a);
                                    ui.close();
                                }
                            }
                        });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(tr("Search")).clicked() {
                            action = Some(Action::Search);
                        }
                        ui.menu_button(tr("View"), |ui| {
                            let mut hidden = self.show_hidden;
                            if ui.checkbox(&mut hidden, tr("Show hidden files")).changed() {
                                action = Some(Action::ToggleHidden);
                            }
                            ui.separator();
                            for scheme in [
                                crate::appearance::ColorScheme::System,
                                crate::appearance::ColorScheme::Light,
                                crate::appearance::ColorScheme::Dark,
                            ] {
                                appearance_changed |= ui
                                    .radio_value(
                                        &mut self.preferences.color_scheme,
                                        scheme,
                                        tr(scheme.label()),
                                    )
                                    .changed();
                            }
                            ui.separator();
                            if ui.button(tr("Settings…")).clicked() {
                                action = Some(Action::Preferences);
                                ui.close();
                            }
                        });
                        if ui.button(tr("Refresh")).clicked() {
                            action = Some(Action::Refresh);
                        }
                        if ui.button(tr("Bookmark")).clicked() {
                            action = Some(Action::Bookmark);
                        }
                    });
                });
            });
        if appearance_changed {
            self.theme(ctx);
            self.save_settings();
        }
        if let Some(action) = action {
            self.act(action, ctx);
        }
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        let palette = crate::appearance::Palette::new(
            self.preferences.desktop_style,
            ctx.style().visuals.dark_mode,
        );
        let mut location = None;
        let mut remove = None;
        let mut add_bookmark = false;
        egui::SidePanel::left("places")
            .resizable(false)
            .exact_width(180.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.sidebar)
                    .inner_margin(egui::Margin::symmetric(12, 8)),
            )
            .show(ctx, |ui| {
                ui.add_space(12.0);
                ui.label(RichText::new(tr("Favorites")).small().strong().weak());
                let home = filesystem::home();
                use crate::appearance::SidebarIcon;
                for (label, path, icon) in [
                    ("Home", home.clone(), SidebarIcon::Home),
                    ("Desktop", home.join("Desktop"), SidebarIcon::Desktop),
                    ("Documents", home.join("Documents"), SidebarIcon::Documents),
                    ("Downloads", home.join("Downloads"), SidebarIcon::Downloads),
                    (
                        "Computer",
                        PathBuf::from(std::path::MAIN_SEPARATOR.to_string()),
                        SidebarIcon::Computer,
                    ),
                ] {
                    if path.is_dir()
                        && crate::appearance::sidebar_button(
                            ui,
                            tr(label).as_ref(),
                            self.tab().location == Location::Local(path.clone()),
                            icon,
                            &palette,
                        )
                        .clicked()
                    {
                        location = Some(Location::Local(path));
                    }
                }
                ui.collapsing(tr("Mounted drives"), |ui| {
                    for (label, path) in crate::native_files::mounted_drives() {
                        if ui.button(label).clicked() {
                            location = Some(Location::Local(path));
                        }
                    }
                });
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr("Bookmarks")).small().strong().weak());
                    if ui.small_button(tr("+")).clicked() {
                        add_bookmark = true;
                    }
                });
                for (i, bookmark) in self.bookmarks.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::selectable(
                                    self.tab().location == bookmark.location,
                                    &bookmark.name,
                                )
                                .min_size(Vec2::new(104.0, 28.0)),
                            )
                            .on_hover_text(bookmark.location.label())
                            .clicked()
                        {
                            location = Some(bookmark.location.clone());
                        }
                        if ui
                            .small_button(tr("×"))
                            .on_hover_text(tr("Remove bookmark"))
                            .clicked()
                        {
                            remove = Some(i);
                        }
                    });
                }
                if self.bookmarks.is_empty() {
                    ui.label(
                        RichText::new(if cfg!(target_os = "macos") {
                            "Add favorites with ⌘D."
                        } else {
                            "Add favorites with Ctrl+D."
                        })
                        .small()
                        .weak(),
                    );
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(tr("Tab switches panes\nSpace marks files\nF1 opens help"))
                            .small()
                            .weak(),
                    );
                });
            });
        if let Some(location) = location {
            self.tab_mut().navigate(location, ctx);
        }
        if let Some(i) = remove {
            self.bookmarks.remove(i);
        }
        if add_bookmark {
            self.act(Action::Bookmark, ctx);
        }
    }

    fn bottom(&mut self, ctx: &egui::Context) {
        let mut action = None;
        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.add_space(5.0);
            if let Some(job) = &self.job {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!(
                        "{} files · {}",
                        job.progress.files,
                        filesystem::human_size(job.progress.bytes)
                    ));
                    if ui.button(tr("Cancel")).clicked() {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                    ui.add(egui::Label::new(&job.progress.current).truncate());
                });
            } else if self.pending_view.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tr("Opening file…"));
                });
            } else {
                let color = if self.status_error {
                    Color32::from_rgb(190, 65, 55)
                } else {
                    ui.visuals().weak_text_color()
                };
                if self.preferences.status_bar {
                    ui.add(
                        egui::Label::new(RichText::new(tr(&self.status)).color(color)).truncate(),
                    )
                    .on_hover_text(tr(&self.status));
                }
            }
            if !self.queue.is_empty()
                && ui
                    .button(format!(
                        "{} ({})",
                        tr("Queued operations"),
                        self.queue.len()
                    ))
                    .clicked()
            {
                action = Some(Action::Queue);
            }
            ui.separator();
            if self.preferences.command_bar {
                ui.horizontal_wrapped(|ui| {
                    for (label, a) in [
                        ("F2  Rename", Action::Rename),
                        ("F3  View", Action::View),
                        ("F4  Edit", Action::Edit),
                        ("F5  Copy", Action::Copy),
                        ("F6  Move", Action::Move),
                        ("F7  New folder", Action::Mkdir),
                        ("F8  Trash", Action::Trash),
                    ] {
                        if ui
                            .add_enabled(!self.exiting, egui::Button::new(tr(label)).frame(false))
                            .clicked()
                        {
                            action = Some(a);
                        }
                    }
                });
            }
            ui.add_space(3.0);
        });
        if let Some(action) = action {
            self.act(action, ctx);
        }
    }

    fn panes(&mut self, ctx: &egui::Context) {
        let mut column_layout = self.preferences.columns.clone();
        let mut column_resized = false;
        let mut native_drag = None;
        let mut unlock_archive = None;
        let palette = crate::appearance::Palette::new(
            self.preferences.desktop_style,
            ctx.style().visuals.dark_mode,
        );
        let font_size = self.preferences.font_size;
        let row_height = (font_size + 14.0).max(26.0);
        let mut open = None;
        let mut action = None;
        let mut dropped = None;
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.columns(
                if self.preferences.single_pane { 1 } else { 2 },
                |columns| {
                    for (index, ui) in columns.iter_mut().enumerate() {
                        let side = if self.preferences.single_pane {
                            self.active
                        } else {
                            index
                        };
                        let active = self.active == side;
                        let frame = egui::Frame::new()
                            .fill(ui.visuals().window_fill)
                            .stroke(Stroke::new(1.0_f32, palette.line))
                            .corner_radius(palette.radius)
                            .inner_margin(8)
                            .show(ui, |ui| {
                                let mut close = None;
                                let pane = &mut self.panes[side];
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(if side == 0 { "Left" } else { "Right" })
                                            .small()
                                            .strong()
                                            .color(if active {
                                                ui.visuals().hyperlink_color
                                            } else {
                                                ui.visuals().weak_text_color()
                                            }),
                                    );
                                    egui::ScrollArea::horizontal().id_salt(("tabs", side)).show(
                                        ui,
                                        |ui| {
                                            ui.horizontal(|ui| {
                                                for (index, tab) in pane.tabs.iter().enumerate() {
                                                    if ui
                                                        .selectable_label(
                                                            index == pane.active,
                                                            tab.location.title(),
                                                        )
                                                        .on_hover_text(tab.location.label())
                                                        .clicked()
                                                    {
                                                        pane.active = index;
                                                        self.active = side;
                                                    }
                                                    if pane.tabs.len() > 1
                                                        && ui.small_button(tr("×")).clicked()
                                                    {
                                                        close = Some(index);
                                                    }
                                                }
                                                if ui
                                                    .small_button(tr("+"))
                                                    .on_hover_text(tr("New tab"))
                                                    .clicked()
                                                {
                                                    pane.add_tab(ctx);
                                                    self.active = side;
                                                }
                                            });
                                        },
                                    );
                                });
                                if let Some(index) = close {
                                    pane.close_tab(index);
                                }
                                let tab = pane.tab_mut();
                                ui.push_id(tab.id, |ui| {
                                    ui.horizontal(|ui| {
                                        if ui
                                            .small_button(tr("<"))
                                            .on_hover_text(tr("Back"))
                                            .clicked()
                                        {
                                            tab.history(false, ctx);
                                            self.active = side;
                                        }
                                        if ui
                                            .small_button(tr(">"))
                                            .on_hover_text(tr("Forward"))
                                            .clicked()
                                        {
                                            tab.history(true, ctx);
                                            self.active = side;
                                        }
                                        if ui
                                            .small_button(tr("Up"))
                                            .on_hover_text(tr("Parent folder"))
                                            .clicked()
                                        {
                                            if let Some(parent) = tab.location.parent() {
                                                tab.navigate(parent, ctx);
                                            }
                                            self.active = side;
                                        }
                                        let address = ui.add(
                                            egui::TextEdit::singleline(&mut tab.address)
                                                .desired_width(ui.available_width())
                                                .hint_text(tr("Path to folder")),
                                        );
                                        if tab.focus_address {
                                            focus_text(&address, &tab.address);
                                            tab.focus_address = false;
                                        }
                                        if address.has_focus() {
                                            self.active = side;
                                        }
                                        if address.lost_focus()
                                            && ui.input(|i| i.key_pressed(Key::Enter))
                                        {
                                            match filesystem::parse_location(
                                                &tab.address,
                                                &tab.location,
                                            ) {
                                                Ok(location) => tab.navigate(location, ctx),
                                                Err(error) => tab.error = Some(error.to_string()),
                                            }
                                        }
                                    });
                                    if let Some(path) = tab.location.local().map(Path::to_path_buf)
                                    {
                                        let mut target = None;
                                        ui.horizontal_wrapped(|ui| {
                                            for (label, path) in
                                                crate::native_files::breadcrumbs(&path)
                                            {
                                                if ui.small_button(label).clicked() {
                                                    target = Some(path);
                                                }
                                                ui.weak(tr("›"));
                                            }
                                        });
                                        if let Some(path) = target {
                                            tab.navigate(Location::Local(path), ctx);
                                            self.active = side;
                                        }
                                    }
                                    ui.horizontal(|ui| {
                                        if matches!(tab.location, Location::Archive { .. }) {
                                            ui.label(
                                                RichText::new(tr("ARCHIVE"))
                                                    .size(10.0)
                                                    .color(ui.visuals().hyperlink_color),
                                            );
                                        }
                                        let filter = ui.add(
                                            egui::TextEdit::singleline(&mut tab.filter)
                                                .hint_text(tr("Filter files…"))
                                                .desired_width(ui.available_width()),
                                        );
                                        if tab.focus_filter {
                                            focus_text(&filter, &tab.filter);
                                            tab.focus_filter = false;
                                        }
                                        if filter.has_focus() {
                                            self.active = side;
                                        }
                                        if filter.changed() {
                                            tab.selected.clear();
                                            tab.cursor = tab
                                                .visible(self.show_hidden)
                                                .first()
                                                .map(|&i| tab.entries[i].path.clone());
                                        }
                                    });
                                    let width = ui.available_width();
                                    ui.horizontal(|ui| {
                                        let total: f32 =
                                            column_layout.iter().map(|column| column.width).sum();
                                        for (index, column) in column_layout.iter_mut().enumerate()
                                        {
                                            let sort = column.kind.sort();
                                            let w = width * column.width / total;
                                            let label = format!(
                                                "{}{}",
                                                tr(column.kind.label()),
                                                if sort == Some(tab.sort) {
                                                    if tab.descending { " v" } else { " ^" }
                                                } else {
                                                    ""
                                                }
                                            );
                                            let response = ui.add_sized(
                                                [w - 6.0, 24.0],
                                                egui::Button::new(
                                                    RichText::new(label).small().strong(),
                                                )
                                                .frame(false),
                                            );
                                            if response.clicked()
                                                && let Some(sort) = sort
                                            {
                                                if tab.sort == sort {
                                                    tab.descending = !tab.descending;
                                                } else {
                                                    tab.sort = sort;
                                                    tab.descending = false;
                                                }
                                                filesystem::sort_entries_with_options(
                                                    &mut tab.entries,
                                                    tab.sort,
                                                    tab.descending,
                                                    tab.folders_first,
                                                );
                                            }
                                            let handle = egui::Rect::from_min_size(
                                                response.rect.right_top(),
                                                Vec2::new(6.0, 24.0),
                                            );
                                            let resize = ui
                                                .interact(
                                                    handle,
                                                    ui.id().with(("column_resize", index)),
                                                    egui::Sense::drag(),
                                                )
                                                .on_hover_cursor(
                                                    egui::CursorIcon::ResizeHorizontal,
                                                );
                                            if resize.dragged() {
                                                column.width = (column.width
                                                    + resize.drag_delta().x * total / width)
                                                    .clamp(40.0, 800.0);
                                                column_resized = true;
                                            }
                                        }
                                    });
                                    ui.separator();
                                    let visible = tab.visible(self.show_hidden);
                                    let height = (ui.available_height() - 38.0).max(100.0);
                                    if let Some(error) = &tab.error {
                                        ui.colored_label(Color32::from_rgb(190, 65, 55), tr(error));
                                        if let Location::Archive { file, .. } = &tab.location
                                            && ui.button(tr("Unlock archive")).clicked()
                                        {
                                            unlock_archive = Some(file.clone());
                                            self.active = side;
                                        }
                                    }
                                    if tab.loading {
                                        ui.horizontal(|ui| {
                                            ui.spinner();
                                            ui.label(tr("Loading folder…"));
                                        });
                                    }
                                    let scroll_to = if tab.scroll_to_cursor {
                                        visible.iter().position(|&i| {
                                            Some(&tab.entries[i].path) == tab.cursor.as_ref()
                                        })
                                    } else {
                                        None
                                    };
                                    let mut scroll = egui::ScrollArea::vertical()
                                        .id_salt("files")
                                        .auto_shrink([false, false])
                                        .max_height(height)
                                        .min_scrolled_height(height);
                                    if let Some(index) = scroll_to {
                                        scroll = scroll.vertical_scroll_offset(
                                            index.saturating_sub(4) as f32 * row_height,
                                        );
                                        tab.scroll_to_cursor = false;
                                    }
                                    scroll.show_rows(ui, row_height, visible.len(), |ui, range| {
                                        for position in range {
                                            let index = visible[position];
                                            let entry = tab.entries[index].clone();
                                            let marked = tab.selected.contains(&entry.path);
                                            let cursor = tab.cursor.as_ref() == Some(&entry.path);
                                            let (rect, response) = ui.allocate_exact_size(
                                                Vec2::new(ui.available_width(), row_height),
                                                egui::Sense::click_and_drag(),
                                            );
                                            if tab.location.local().is_some() {
                                                let sources = if tab.selected.contains(&entry.path)
                                                {
                                                    tab.chosen(self.show_hidden)
                                                        .into_iter()
                                                        .map(|e| e.path)
                                                        .collect()
                                                } else {
                                                    vec![entry.path.clone()]
                                                };
                                                if response.drag_started()
                                                    && ui.input(|input| input.modifiers.alt)
                                                {
                                                    native_drag = Some(sources.clone());
                                                } else {
                                                    response
                                                        .dnd_set_drag_payload(FileDrag { sources });
                                                }
                                            }
                                            response.widget_info(|| {
                                                egui::WidgetInfo::selected(
                                                    egui::WidgetType::SelectableLabel,
                                                    true,
                                                    marked || cursor,
                                                    &entry.name,
                                                )
                                            });
                                            let painter = ui.painter_at(rect);
                                            let highlighted = marked || cursor;
                                            let selected = highlighted && active;
                                            if highlighted {
                                                painter.rect_filled(
                                                    rect,
                                                    4.0,
                                                    if active {
                                                        palette.selection
                                                    } else {
                                                        palette.inactive_selection
                                                    },
                                                );
                                            } else if response.hovered() {
                                                painter.rect_filled(
                                                    rect,
                                                    4.0,
                                                    ui.visuals().widgets.hovered.bg_fill,
                                                );
                                            } else if position % 2 == 1 {
                                                painter.rect_filled(rect, 0.0, palette.stripe);
                                            }
                                            let text_color = if selected {
                                                palette.selection_text
                                            } else {
                                                ui.visuals().text_color()
                                            };
                                            let detail_color = if selected {
                                                palette.selection_text.gamma_multiply(0.9)
                                            } else if highlighted {
                                                ui.visuals().text_color()
                                            } else {
                                                ui.visuals().weak_text_color()
                                            };
                                            let icon_color = if selected {
                                                palette.selection_text
                                            } else if entry.is_dir {
                                                palette.folder
                                            } else if entry.kind() == "Archive" {
                                                Color32::from_rgb(144, 98, 187)
                                            } else {
                                                ui.visuals().weak_text_color()
                                            };
                                            file_icon(
                                                &painter,
                                                rect.min
                                                    + Vec2::new(10.0, (row_height - 15.0) / 2.0),
                                                &entry,
                                                marked,
                                                icon_color,
                                            );
                                            let total: f32 = column_layout
                                                .iter()
                                                .map(|column| column.width)
                                                .sum();
                                            let mut x = rect.left();
                                            for column in &column_layout {
                                                let w = width * column.width / total;
                                                let name =
                                                    column.kind == crate::preferences::Column::Name;
                                                let cell = egui::Rect::from_min_max(
                                                    egui::pos2(
                                                        x + if name { 31.0 } else { 4.0 },
                                                        rect.top(),
                                                    ),
                                                    egui::pos2(x + w - 5.0, rect.bottom()),
                                                );
                                                painter.with_clip_rect(cell).text(
                                                    cell.left_center(),
                                                    egui::Align2::LEFT_CENTER,
                                                    column.kind.value(
                                                        &entry,
                                                        tab.location.local().is_some(),
                                                    ),
                                                    egui::FontId::proportional(
                                                        font_size - if name { 1.0 } else { 2.0 },
                                                    ),
                                                    if name { text_color } else { detail_color },
                                                );
                                                x += w;
                                            }
                                            if response.clicked() {
                                                self.active = side;
                                                let modifiers = ui.input(|i| i.modifiers);
                                                if modifiers.shift {
                                                    let start = visible
                                                        .iter()
                                                        .position(|&i| {
                                                            Some(&tab.entries[i].path)
                                                                == tab.cursor.as_ref()
                                                        })
                                                        .unwrap_or(position);
                                                    for &i in &visible
                                                        [start.min(position)..=start.max(position)]
                                                    {
                                                        tab.selected
                                                            .insert(tab.entries[i].path.clone());
                                                    }
                                                } else if modifiers.command {
                                                    if !tab.selected.remove(&entry.path) {
                                                        tab.selected.insert(entry.path.clone());
                                                    }
                                                } else {
                                                    tab.selected.clear();
                                                }
                                                tab.cursor = Some(entry.path.clone());
                                            }
                                            if response.double_clicked() {
                                                self.active = side;
                                                open = Some(entry.clone());
                                            }
                                            if response.secondary_clicked() {
                                                self.active = side;
                                                tab.cursor = Some(entry.path.clone());
                                                if !tab.selected.contains(&entry.path) {
                                                    tab.selected.clear();
                                                }
                                            }
                                            response
                                                .on_hover_text(format!(
                                                    "{}\n{}",
                                                    entry.name,
                                                    entry.kind()
                                                ))
                                                .context_menu(|ui| {
                                                    for (label, a) in [
                                                        ("View", Action::View),
                                                        ("Edit", Action::Edit),
                                                        ("Open externally", Action::External),
                                                        ("Copy", Action::Copy),
                                                        ("Move", Action::Move),
                                                        ("Rename", Action::Rename),
                                                        ("Create archive", Action::Pack),
                                                        ("Extract", Action::Extract),
                                                        ("Move to Trash", Action::Trash),
                                                    ] {
                                                        if ui.button(label).clicked() {
                                                            action = Some(a);
                                                            ui.close();
                                                        }
                                                    }
                                                });
                                        }
                                        if visible.is_empty() && !tab.loading && tab.error.is_none()
                                        {
                                            ui.label(
                                                RichText::new(if tab.filter.is_empty() {
                                                    "This folder is empty"
                                                } else {
                                                    "No files match your filter"
                                                })
                                                .weak(),
                                            );
                                        }
                                    });
                                    ui.separator();
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(format!("{} items", visible.len()))
                                                .small()
                                                .weak(),
                                        );
                                        let selected: Vec<_> = visible
                                            .iter()
                                            .map(|&i| &tab.entries[i])
                                            .filter(|e| tab.selected.contains(&e.path))
                                            .collect();
                                        if !selected.is_empty() {
                                            ui.label(
                                                RichText::new(format!(
                                                    "· {} marked · {}",
                                                    selected.len(),
                                                    filesystem::human_size(
                                                        selected
                                                            .iter()
                                                            .filter(|e| !e.is_dir)
                                                            .map(|e| e.size)
                                                            .sum()
                                                    )
                                                ))
                                                .small()
                                                .color(ui.visuals().hyperlink_color),
                                            );
                                        }
                                    });
                                });
                            });
                        if let Some(destination) = self.panes[side].tab().location.local() {
                            let pointer_inside = ctx.input(|i| {
                                i.pointer
                                    .latest_pos()
                                    .is_some_and(|position| frame.response.rect.contains(position))
                            });
                            let dragging = egui::DragAndDrop::has_payload_of_type::<FileDrag>(ctx)
                                || ctx.input(|i| !i.raw.hovered_files.is_empty());
                            if pointer_inside && dragging {
                                ui.painter().rect_stroke(
                                    frame.response.rect,
                                    7.0,
                                    Stroke::new(3.0_f32, ui.visuals().hyperlink_color),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            if pointer_inside
                                && ctx.input(|i| i.pointer.any_released())
                                && let Some(payload) =
                                    egui::DragAndDrop::take_payload::<FileDrag>(ctx)
                            {
                                dropped = Some((
                                    side,
                                    payload.sources.clone(),
                                    destination.to_path_buf(),
                                ));
                            } else if pointer_inside {
                                let sources: Vec<_> = ctx.input(|i| {
                                    i.raw
                                        .dropped_files
                                        .iter()
                                        .filter_map(|file| file.path.clone())
                                        .collect()
                                });
                                if !sources.is_empty() {
                                    dropped = Some((side, sources, destination.to_path_buf()));
                                }
                            }
                        }
                    }
                },
            );
        });
        if let Some(entry) = open {
            self.open_entry(entry, ctx);
        }
        if let Some(action) = action {
            self.act(action, ctx);
        }
        if let Some((side, sources, destination)) = dropped {
            if self.job.is_none()
                && self.prompt.is_none()
                && self.editor.is_none()
                && !self.tools.active()
                && self.preferences_form.is_none()
                && self.archive_form.is_none()
                && !self.show_history
                && self.connection_form.is_none()
                && !self.help
                && self.message.is_none()
            {
                self.active = side;
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
                    focus: true,
                    error: None,
                    transfer_options: Default::default(),
                    overrides: Default::default(),
                    conflicts: Vec::new(),
                    conflict_key: String::new(),
                    pending_conflicts: None,
                });
            } else {
                self.fail("Finish or close the current dialog before dropping files.");
            }
        }
        if column_resized {
            self.preferences.columns = column_layout;
            self.save_settings();
        }
        if let Some(paths) = native_drag {
            self.native_drag(paths);
        }
        if let Some(file) = unlock_archive {
            self.archive_form = Some(crate::feature_ui::ArchiveForm {
                file: file.display().to_string(),
                mode: 2,
                ..Default::default()
            });
        }
    }

    fn prompt(&mut self, ctx: &egui::Context) {
        let Some(mut prompt) = self.prompt.take() else {
            return;
        };
        let mut accept = false;
        let mut cancel = false;
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
        if let PromptKind::RemoteTransfer {
            from, entries, to, ..
        } = &prompt.kind
        {
            let key = prompt.value.clone();
            if key != prompt.conflict_key && prompt.pending_conflicts.is_none() {
                prompt.conflict_key = key;
                let from = from.clone();
                let entries = entries.clone();
                let to = filesystem::parse_location(&prompt.value, to);
                let (sender, receiver) = mpsc::channel();
                prompt.pending_conflicts = Some(receiver);
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let result = to
                        .and_then(|to| crate::remote::conflicts(&from, &entries, &to))
                        .map_err(|error| format!("{error:#}"));
                    let _ = sender.send(result);
                    ctx.request_repaint();
                });
            }
            if prompt.pending_conflicts.is_some() {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        egui::Modal::new(egui::Id::new("operation_prompt")).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.heading(tr(&prompt.title));
            ui.label(tr(&prompt.description));
            ui.add_space(8.0);
            if !matches!(
                prompt.kind,
                PromptKind::Trash(_)
                    | PromptKind::RemoteDelete { .. }
                    | PromptKind::PermanentDelete(_)
            ) {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut prompt.value).desired_width(f32::INFINITY),
                );
                if prompt.focus {
                    focus_text(&response, &prompt.value);
                    prompt.focus = false;
                }
            }
            if let Some(error) = &prompt.error {
                ui.colored_label(Color32::from_rgb(190, 65, 55), tr(error));
            }
            if matches!(prompt.kind, PromptKind::Transfer { .. }|PromptKind::RemoteTransfer{..}) {
                use crate::transfer::ConflictPolicy;
                ui.collapsing(tr("Per-file conflicts"),|ui|{
                    if prompt.pending_conflicts.is_some(){ui.spinner();}
                    egui::ScrollArea::vertical().max_height(180.0).show(ui,|ui|{
                        for conflict in &prompt.conflicts {
                            ui.push_id(&conflict.target,|ui|{
                                ui.label(conflict.target.display().to_string());
                                ui.small(format!("{} → {}",filesystem::human_size(conflict.source_size),filesystem::human_size(conflict.target_size)));
                                let mut policy=prompt.overrides.get(&conflict.target).copied().unwrap_or(prompt.transfer_options.conflicts);let before=policy;
                                ui.horizontal(|ui|{for(value,label)in[(ConflictPolicy::Refuse,"Stop on conflict"),(ConflictPolicy::Skip,"Skip"),(ConflictPolicy::Rename,"Keep both"),(ConflictPolicy::Replace,"Replace")]{ui.selectable_value(&mut policy,value,tr(label));}});if policy!=before{prompt.overrides.insert(conflict.target.clone(),policy);}
                            });
                        }
                    });
                    if ui.button(tr("Apply to all")).clicked(){prompt.overrides.clear();}
                });
                ui.label(tr("When a target exists:"));
                for (policy, label) in [
                    (ConflictPolicy::Refuse, "Stop before copying"),
                    (ConflictPolicy::Skip, "Skip existing items"),
                    (ConflictPolicy::Rename, "Keep both, with a numbered name"),
                    (ConflictPolicy::Replace, "Replace, keeping originals in a recovery folder"),
                ] {
                    ui.radio_value(&mut prompt.transfer_options.conflicts, policy, tr(label));
                }
                if matches!(prompt.kind, PromptKind::Transfer{..}) {
                    ui.add_enabled_ui(prompt.transfer_options.conflicts != ConflictPolicy::Rename, |ui| {
                        ui.checkbox(&mut prompt.transfer_options.merge_folders,tr("Merge existing folders"));
                    });
                }
                if prompt.transfer_options.conflicts == ConflictPolicy::Replace {
                    ui.label(tr("Replaced items stay in a hidden .rust-commander-replaced-* folder in the destination. The result reports its path."));
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tr("Cancel")).clicked() {
                    cancel = true;
                }
                if ui
                    .add(
                        egui::Button::new(if matches!(prompt.kind, PromptKind::Trash(_)) {
                            "Move to Trash"
                        } else {
                            "Confirm"
                        })
                        .fill(ui.visuals().hyperlink_color)
                        .stroke(Stroke::NONE),
                    )
                    .clicked()
                {
                    accept = true;
                }
            });
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                cancel = true;
            }
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                accept = true;
            }
        });
        if cancel {
            return;
        }
        if accept {
            let result = (|| -> anyhow::Result<Option<Operation>> {
                Ok(Some(match &prompt.kind {
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
                    } => Operation::Extract {
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
        }
        self.prompt = Some(prompt);
    }

    fn editor(&mut self, ctx: &egui::Context) {
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        let mut close = false;
        let mut page_request = None;
        egui::Modal::new(egui::Id::new("text_editor")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 120.0).clamp(550.0, 1000.0));
            ui.horizontal(|ui| {
                ui.heading(tr(if editor.editable {
                    "Text editor"
                } else {
                    editor.kind
                }));
                if editor.dirty() {
                    ui.label(
                        RichText::new(tr("Unsaved changes")).color(ui.visuals().hyperlink_color),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tr("Close")).clicked() {
                        close = true;
                    }
                });
            });
            ui.add(egui::Label::new(&editor.title).truncate());
            if let Some(pdf) = &editor.pdf {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            pdf.page > 0 && self.pending_pdf.is_none(),
                            egui::Button::new(tr("Previous page")),
                        )
                        .clicked()
                    {
                        page_request = Some(pdf.page - 1);
                    }
                    ui.label(format!("Page {} of {}", pdf.page + 1, pdf.pages));
                    if ui
                        .add_enabled(
                            pdf.page + 1 < pdf.pages && self.pending_pdf.is_none(),
                            egui::Button::new(tr("Next page")),
                        )
                        .clicked()
                    {
                        page_request = Some(pdf.page + 1);
                    }
                    if self.pending_pdf.is_some() {
                        ui.spinner();
                    }
                });
            }
            if let Some(image) = editor.image.take() {
                editor.texture =
                    Some(ctx.load_texture(&editor.title, image, egui::TextureOptions::LINEAR));
            }
            if editor.texture.is_some() {
                ui.add(egui::Slider::new(&mut editor.zoom, 0.1..=4.0).text(tr("Zoom")));
            }
            egui::ScrollArea::both()
                .max_height((ctx.content_rect().height() - 260.0).max(180.0))
                .show(ui, |ui| {
                    if let Some(texture) = &editor.texture {
                        ui.image((texture.id(), texture.size_vec2() * editor.zoom));
                        return;
                    }
                    let widget = |text| {
                        egui::TextEdit::multiline(text)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .desired_rows(22)
                            .code_editor()
                    };
                    if editor.editable {
                        ui.add(widget(&mut editor.text as &mut dyn egui::TextBuffer));
                    } else {
                        let mut text = editor.text.as_str();
                        ui.add(widget(&mut text as &mut dyn egui::TextBuffer));
                    }
                });
            if let Some(error) = &editor.error {
                ui.colored_label(Color32::from_rgb(190, 65, 55), tr(error));
            }
            if editor.editable
                && (ui
                    .add_enabled(editor.dirty(), egui::Button::new(tr("Save  ⌘/Ctrl+S")))
                    .clicked()
                    || ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::S)))
                && let Some(path) = &editor.path
            {
                match operations::save_text(path, editor.original.as_bytes(), &editor.text) {
                    Ok(()) => {
                        editor.original = editor.text.clone();
                        editor.error = None;
                        self.status = "File saved.".into();
                        self.status_error = false;
                    }
                    Err(error) => editor.error = Some(format!("{error:#}")),
                }
            }
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                close = true;
            }
            if editor.confirm_close {
                ui.separator();
                ui.label(tr("Discard unsaved changes?"));
                ui.horizontal(|ui| {
                    if ui.button(tr("Keep editing")).clicked() {
                        editor.confirm_close = false;
                    }
                    if ui.button(tr("Discard changes")).clicked() {
                        editor.text = editor.original.clone();
                        close = true;
                    }
                });
            }
        });
        if let Some(page) = page_request
            && let Some(pdf) = &editor.pdf
        {
            let bytes = pdf.bytes.clone();
            let title = editor.title.clone();
            let ctx = ctx.clone();
            let (sender, receiver) = mpsc::channel();
            self.pending_pdf = Some(receiver);
            std::thread::spawn(move || {
                let result = crate::viewer::render_pdf(&bytes, page)
                    .map(|(image, _)| (title, page, image))
                    .map_err(|e| e.to_string());
                let _ = sender.send(result);
                ctx.request_repaint();
            });
        }
        if close && editor.dirty() {
            editor.confirm_close = true;
        } else if close {
            self.refresh(ctx);
            return;
        }
        self.editor = Some(editor);
    }

    fn preferences_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut preferences) = self.preferences_form.take() else {
            return;
        };
        let mut close = false;
        let mut apply = false;
        egui::Modal::new(egui::Id::new("preferences")).show(ctx, |ui| {
            ui.set_width(550.0);
            ui.heading(tr("Preferences"));
            egui::ScrollArea::vertical()
                .max_height(ctx.content_rect().height() - 180.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(tr("Desktop style"));
                        egui::ComboBox::from_id_salt("desktop_style")
                            .selected_text(tr(preferences.desktop_style.label()))
                            .show_ui(ui, |ui| {
                                for desktop in crate::appearance::DesktopStyle::ALL {
                                    ui.selectable_value(
                                        &mut preferences.desktop_style,
                                        desktop,
                                        tr(desktop.label()),
                                    );
                                }
                            });
                    });
                    ui.horizontal(|ui| {
                        ui.label(tr("Appearance"));
                        for scheme in [
                            crate::appearance::ColorScheme::System,
                            crate::appearance::ColorScheme::Light,
                            crate::appearance::ColorScheme::Dark,
                        ] {
                            ui.selectable_value(
                                &mut preferences.color_scheme,
                                scheme,
                                tr(scheme.label()),
                            );
                        }
                    });
                    ui.add(
                        egui::Slider::new(&mut preferences.font_size, 10.0..=24.0)
                            .text(tr("Font size")),
                    );
                    crate::feature_ui::preferences(ui, &mut preferences);
                    for (label, value) in [
                        ("Show toolbar", &mut preferences.toolbar),
                        ("Show command bar", &mut preferences.command_bar),
                        ("Show status bar", &mut preferences.status_bar),
                        ("Single pane", &mut preferences.single_pane),
                        (
                            "Automatically refresh changed folders",
                            &mut preferences.auto_refresh,
                        ),
                        ("Folders first", &mut preferences.folders_first),
                    ] {
                        ui.checkbox(value, tr(label));
                    }
                    ui.separator();
                    ui.label(tr(
                        "Keyboard shortcuts · Command means Cmd on macOS and Ctrl elsewhere",
                    ));
                    egui::Grid::new("shortcut_settings")
                        .num_columns(2)
                        .show(ui, |ui| {
                            for (action, binding) in &mut preferences.shortcuts {
                                ui.label(tr(action));
                                ui.text_edit_singleline(binding);
                                ui.end_row();
                            }
                        });
                });
            ui.separator();
            if let Some(error) = &self.preferences_error {
                ui.colored_label(Color32::RED, tr(error));
            }
            ui.horizontal(|ui| {
                if ui.button(tr("Cancel")).clicked() {
                    close = true;
                }
                if ui.button(tr("Apply")).clicked() {
                    apply = true;
                }
                if ui.button(tr("Restore defaults")).clicked() {
                    preferences = crate::preferences::Preferences::default();
                }
            });
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                close = true;
            }
        });
        if apply {
            match crate::preferences::validate(&preferences) {
                Ok(()) => {
                    self.preferences = preferences.clone();
                    crate::i18n::set(self.preferences.language);
                    #[cfg(target_os = "macos")]
                    if let Some(menu) = &self.native_menu {
                        menu.localize();
                    }
                    crate::appearance::install_fonts(ctx, self.preferences.desktop_style);
                    self.theme(ctx);
                    self.refresh(ctx);
                    close = true;
                    self.save_settings();
                }
                Err(error) => self.preferences_error = Some(error.to_string()),
            }
        }
        if !close {
            self.preferences_form = Some(preferences);
        }
    }

    fn connection_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut form) = self.connection_form.take() else {
            return;
        };
        let mut close = false;
        let mut connect = false;
        let mut authorize = false;
        let mut open = None;
        egui::Modal::new(egui::Id::new("connections")).show(ctx, |ui| {
            ui.set_width(570.0); ui.heading(tr("Connections"));
            egui::ScrollArea::vertical().max_height(ctx.content_rect().height() - 160.0).show(ui, |ui| {
                for connection in &self.connections {
                    ui.horizontal(|ui| {
                        ui.label(format!("{} · {}", connection.name, connection.service));
                        if ui.button(tr("Open")).clicked() { open = Some(connection.id.clone()); }
                        if ui.button(tr("Edit")).clicked() { form.connection = connection.clone(); }
                        if ui.button(tr("Forget credentials")).clicked()
                            && let Err(error) = crate::remote::forget_credentials(&connection.id) { form.error = Some(error.to_string()); }
                    });
                }
                ui.separator();
                egui::Grid::new("connection_fields").num_columns(2).show(ui, |ui| {
                    ui.label(tr("Name")); ui.text_edit_singleline(&mut form.connection.name); ui.end_row();
                    ui.label(tr("Service")); egui::ComboBox::from_id_salt("remote_service").selected_text(&form.connection.service).show_ui(ui, |ui| {
                        for service in crate::remote::SERVICES { ui.selectable_value(&mut form.connection.service, (*service).into(), *service); }
                    }); ui.end_row();
                    for (label, value) in [("Endpoint", &mut form.connection.endpoint), ("Root folder", &mut form.connection.root),
                        ("Bucket", &mut form.connection.bucket), ("Region", &mut form.connection.region), ("Username", &mut form.connection.username), ("Private key file", &mut form.connection.key_file)] {
                        ui.label(tr(label)); ui.text_edit_singleline(value); ui.end_row();
                    }
                    if form.connection.service != "sftp" { ui.label(tr("Password")); ui.add(egui::TextEdit::singleline(&mut form.secrets.password).password(true)); ui.end_row(); }
                    for (label, value) in [("Access key ID", &mut form.secrets.access_key_id), ("Secret access key", &mut form.secrets.secret_access_key), ("Access token / base64 GCS credentials", &mut form.secrets.access_token)] {
                        ui.label(tr(label)); ui.add(egui::TextEdit::singleline(value).password(true)); ui.end_row();
                    }
                });
                if matches!(form.connection.service.as_str(),"gdrive"|"dropbox"|"onedrive") {
                    ui.separator();
                    ui.label(tr("OAuth client ID"));ui.text_edit_singleline(&mut form.connection.oauth_client_id);
                    ui.label(tr("OAuth client secret"));ui.add(egui::TextEdit::singleline(&mut form.secrets.oauth_client_secret).password(true));
                    if form.connection.service=="onedrive"{ui.label(tr("Microsoft tenant"));ui.text_edit_singleline(&mut form.connection.oauth_tenant);}
                    ui.horizontal(|ui|{ui.label(tr("Callback port"));ui.add(egui::DragValue::new(&mut form.connection.oauth_port).range(1024..=65535));});
                    let host=if form.connection.service=="onedrive"{"localhost"}else{"127.0.0.1"};
                    ui.small(format!("Register redirect URI: http://{host}:{}/callback",form.connection.oauth_port));
                    if ui.add_enabled(self.pending_connection.is_none(),egui::Button::new(tr("Sign in with browser"))).clicked(){authorize=true;}
                }
                if form.connection.service == "sftp" { ui.label(tr("SFTP uses OpenSSH, your SSH agent or private key, and strict known-host verification. Add a trusted server to your OpenSSH known_hosts before connecting.")); }
                ui.checkbox(&mut form.remember, tr("Remember credentials in the operating system credential store"));
                if let Some(error) = &form.error { ui.colored_label(Color32::from_rgb(190, 65, 55), tr(error)); }
                ui.horizontal(|ui| {
                    if ui.button(tr("Close")).clicked() { close = true; }
                    if ui.add_enabled(self.pending_connection.is_none(), egui::Button::new(tr("Connect"))).clicked() { connect = true; }
                    if self.pending_connection.is_some() { ui.spinner(); ui.label(tr("Connecting…")); }
                });
            });
        });
        if connect || authorize {
            let cancel = Arc::new(AtomicBool::new(false));
            self.oauth_cancel = Some(cancel.clone());
            let connection = form.connection.clone();
            let secrets = form.secrets.clone();
            let remember = form.remember;
            let (sender, receiver) = mpsc::channel();
            let ctx = ctx.clone();
            self.pending_connection = Some(receiver);
            form.error = None;
            std::thread::spawn(move || {
                let result = (|| -> anyhow::Result<_> {
                    let secrets = if authorize {
                        crate::oauth::authorize(&connection, secrets, cancel)?
                    } else {
                        secrets
                    };
                    crate::remote::connect(&connection, secrets, remember)?;
                    Ok(connection)
                })()
                .map_err(|e| format!("{e:#}"));
                let _ = sender.send(result);
                ctx.request_repaint();
            });
        }
        if let Some(connection) = open {
            self.tab_mut().navigate(
                Location::Remote {
                    connection,
                    path: PathBuf::new(),
                },
                ctx,
            );
            close = true;
        }
        if !close {
            self.connection_form = Some(form);
        } else if let Some(cancel) = self.oauth_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        self.feature_dialogs(ctx);
        self.preferences_dialog(ctx);
        if let Some(effect) = self.tools.show(ctx) {
            match effect {
                crate::tools_ui::Effect::Run(operation) => self.start_job(operation, ctx),
                crate::tools_ui::Effect::Navigate(location) => {
                    self.tab_mut().navigate(location, ctx)
                }
            }
        }
        self.connection_dialog(ctx);
        self.prompt(ctx);
        self.editor(ctx);
        if self.help {
            egui::Modal::new(egui::Id::new("help")).show(ctx, |ui| {
                ui.set_width(510.0); ui.heading(crate::APP_NAME);
                egui::ScrollArea::vertical().max_height((ctx.content_rect().height()-200.0).max(180.0)).show(ui, |ui| {
                ui.label(format!("{} · {}", tr("Native dual-pane file manager"), env!("CARGO_PKG_VERSION")));
                ui.separator();
                egui::Grid::new("shortcuts").num_columns(2).spacing([28.0, 7.0]).show(ui, |ui| {
                    for (key, description) in [("Tab", "Switch active pane"), ("↑ / ↓ · Enter · Backspace", "Navigate files · Open · Parent folder"), ("Space / Insert", "Mark or unmark current file"), ("Shift / ⌘ / Ctrl + click", "Range / multiple selection"), ("F2 / F3 / F4", "Rename / View / Edit"), ("F5 / F6 / F7 / F8", "Copy / Move / New folder / Trash"), ("⌘ / Ctrl + L / F", "Focus address / filter"), ("⌘ / Ctrl + T / W", "New tab / close tab"), ("⌘ / Ctrl + A / D / R", "Select all / bookmark / refresh"), ("⌘+Shift+. / Ctrl+H", "Show hidden files"), ("⌘ / Ctrl + C / X / V", "System file copy / cut / paste"), ("⌘ / Ctrl + Z / J", "Undo / queue and history"), ("Alt + drag", "Native drag-out on macOS/Windows"), ("Alt + ← / →", "Back / forward"), ("Escape", "Clear marks and filter / close dialog")] { ui.label(RichText::new(key).monospace()); ui.label(tr(description)); ui.end_row(); }
                });
                ui.separator();
                ui.label(tr("Double-click folders and supported archives to browse, or files to view text, images, hex and PDFs. Use Archive tools to edit local ZIPs or unlock encrypted archives. F5 extracts entries; nested and remote archives open read-only. Text editing supports local UTF-8 files up to 2 MiB."));
                ui.label(tr("Inspired by muCommander. Independent Rust implementation; not an official release. GPL-3.0-only. You may redistribute this program under that license. No warranty."));
                ui.hyperlink_to(tr("Original muCommander project"), "https://github.com/mucommander/mucommander");
                ui.collapsing(tr("GNU General Public License v3"), |ui| { egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| { ui.label(include_str!("../LICENSE")); }); });
                });
                if ui.button(tr("Close")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) { self.help = false; }
            });
        }
        if let Some(message) = self.message.clone() {
            egui::Modal::new(egui::Id::new("result")).show(ctx, |ui| {
                ui.set_width(550.0);
                ui.heading(tr("Results"));
                egui::ScrollArea::both()
                    .max_height(ctx.content_rect().height() - 240.0)
                    .show(ui, |ui| {
                        let mut text = message.as_str();
                        ui.add(
                            egui::TextEdit::multiline(&mut text)
                                .font(egui::TextStyle::Monospace)
                                .desired_rows(16)
                                .desired_width(780.0),
                        );
                    });
                ui.horizontal(|ui| {
                    if ui.button(tr("Copy result")).clicked() {
                        ctx.copy_text(message.clone());
                    }
                    if ui.button(tr("Close")).clicked() || ui.input(|i| i.key_pressed(Key::Escape))
                    {
                        self.message = None;
                    }
                });
            });
        }
        if self.confirm_exit {
            egui::Modal::new(egui::Id::new("exit")).show(ctx, |ui| {
                ui.heading(format!("Close {}?", crate::APP_NAME)); ui.label(tr("There is a running operation or unsaved text. Closing cancels the operation and discards unsaved text."));
                ui.horizontal(|ui| {
                    if ui.button(tr("Keep working")).clicked() { self.confirm_exit = false; }
                    if ui.button(tr("Cancel work and close")).clicked() {
                        if let Some(job) = &self.job { job.cancel.store(true, Ordering::Relaxed); }
                        self.editor = None; self.confirm_exit = false; self.exiting = true;
                    }
                });
            });
        }
    }

    fn save_settings(&self) {
        if !self.settings_valid {
            return;
        }
        let settings = Settings {
            preferences: self.preferences.clone(),
            connections: self.connections.clone(),
            tabs: std::array::from_fn(|i| {
                self.panes[i]
                    .tabs
                    .iter()
                    .map(|t| t.location.clone())
                    .collect()
            }),
            active_tabs: [self.panes[0].active, self.panes[1].active],
            active_pane: self.active,
            bookmarks: self.bookmarks.clone(),
            show_hidden: self.show_hidden,
            dark_mode: self.preferences.color_scheme == crate::appearance::ColorScheme::Dark,
        };
        if let Err(error) = settings::save(&settings) {
            eprintln!("Could not save settings: {error:#}");
        }
    }
}

impl Commander {
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
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            match self.native_files.copy(&paths,false){Ok(())=>self.status="Files copied to the system clipboard. Native drag-out is not supported by this Linux window backend.".into(),Err(error)=>self.fail(error)}
        }
    }

    fn feature_dialogs(&mut self, ctx: &egui::Context) {
        if let Some(mut form) = self.archive_form.take() {
            let (close, action) = form.show(ctx);
            if let Some(action) = action {
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
                    crate::feature_ui::ArchiveAction::Run(operation) => {
                        self.start_job(operation, ctx)
                    }
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
            if !close {
                self.archive_form = Some(form);
            }
        }
        if self.show_history {
            let mut close = false;
            let mut remove = None;
            let mut undo = None;
            egui::Modal::new(egui::Id::new("queue_history")).show(ctx, |ui| {
                ui.set_width(660.0);
                ui.heading(tr("Transfer queue and history"));
                if let Some(job) = &self.job {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(tr(&job.label));
                        if ui.button(tr("Cancel")).clicked() {
                            job.cancel.store(true, Ordering::Relaxed);
                        }
                    });
                }
                ui.separator();
                ui.strong(tr("Queued operations"));
                if self.queue.is_empty() {
                    ui.label(tr("No queued operations"));
                }
                egui::ScrollArea::vertical()
                    .id_salt("queued_list")
                    .max_height(120.0)
                    .show(ui, |ui| {
                        for (index, operation) in self.queue.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(format!("{}. {}", index + 1, tr(operation.label())));
                                if ui.button(tr("Cancel")).clicked() {
                                    remove = Some(index);
                                }
                            });
                        }
                    });
                ui.separator();
                ui.strong(tr("Operation history"));
                egui::ScrollArea::vertical()
                    .max_height((ctx.content_rect().height() - 330.0).clamp(100.0, 350.0))
                    .show(ui, |ui| {
                        for record in self.history.iter().rev() {
                            ui.push_id(&record.id, |ui| {
                                let state = if record.undone {
                                    "Undone"
                                } else if record.cancelled {
                                    "Cancelled"
                                } else if record.success {
                                    "Completed"
                                } else {
                                    "Failed"
                                };
                                ui.collapsing(
                                    format!("{} · {}", tr(&record.label), tr(state)),
                                    |ui| {
                                        ui.label(tr(&record.result));
                                        if ui
                                            .add_enabled(
                                                !record.undone
                                                    && !record.changes.is_empty()
                                                    && self.job.is_none()
                                                    && self.queue.is_empty(),
                                                egui::Button::new(tr("Undo")),
                                            )
                                            .clicked()
                                        {
                                            undo = Some(record.clone());
                                        }
                                    },
                                );
                            });
                        }
                    });
                if ui.button(tr("Close")).clicked()
                    || ui.input(|input| input.key_pressed(Key::Escape))
                {
                    close = true;
                }
            });
            if let Some(index) = remove
                && let Some(operation) = self.queue.remove(index)
            {
                self.history
                    .push(crate::history::cancelled(operation.label()));
                if let Err(error) = crate::history::save(&self.history) {
                    self.fail(error);
                }
            }
            if let Some(record) = undo {
                self.start_job(Operation::Undo(Box::new(record)), ctx);
            }
            if close {
                self.show_history = false;
            }
        }
    }
}

impl eframe::App for Commander {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        if ctx.input(|i| i.viewport().close_requested())
            && !self.exiting
            && (self.job.is_some() || self.editor.as_ref().is_some_and(Editor::dirty))
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_exit = true;
        }
        self.shortcuts(ctx);
        self.top(ctx);
        self.bottom(ctx);
        if self.terminal_visible {
            egui::TopBottomPanel::bottom("terminal_panel")
                .resizable(true)
                .default_height(240.0)
                .min_height(100.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(tr("Terminal · click the screen to type"));
                        if ui.button(tr("Hide terminal")).clicked() {
                            self.terminal_visible = false;
                        }
                    });
                    if let Some(terminal) = &mut self.terminal {
                        terminal.show(ui);
                    }
                });
        }
        self.sidebar(ctx);
        self.panes(ctx);
        self.dialogs(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_settings();
        crate::archive_tools::clear_cache();
        if let Some(cancel) = &self.oauth_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod ui_tests {
    use super::*;

    fn fixture() -> (
        tempfile::TempDir,
        egui::Context,
        Commander,
        PathBuf,
        PathBuf,
    ) {
        let root = tempfile::tempdir().unwrap();
        let left = root.path().join("left");
        let right = root.path().join("right");
        std::fs::create_dir(&left).unwrap();
        std::fs::create_dir(&right).unwrap();
        std::fs::write(left.join("drop.txt"), "keep me").unwrap();
        let ctx = egui::Context::default();
        let mut app = Commander::with_settings(
            &ctx,
            &[left.clone(), right.clone()],
            Ok(Settings::default()),
        );
        app.panes[0].tab_mut().entries = filesystem::list(&Location::Local(left.clone())).unwrap();
        (root, ctx, app, left, right)
    }

    fn frame(
        app: &mut Commander,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        dropped_files: Vec<egui::DroppedFile>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1240.0, 780.0),
                )),
                events,
                dropped_files,
                ..Default::default()
            },
            |ctx| app.panes(ctx),
        )
    }

    #[test]
    fn pane_drag_opens_destination_confirmation_without_writing() {
        let (_root, ctx, mut app, left, right) = fixture();
        let output = frame(&mut app, &ctx, vec![], vec![]);
        let source = output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape
                    && text.galley.job.text == "drop.txt"
                {
                    Some(text.pos + Vec2::new(3.0, 3.0))
                } else {
                    None
                }
            })
            .expect("file row is visible");
        let target = egui::pos2(950.0, 400.0);
        frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(source),
                egui::Event::PointerButton {
                    pos: source,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
            vec![],
        );
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(target)],
            vec![],
        );
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: target,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
            vec![],
        );
        let prompt = app.prompt.expect("drop opens confirmation");
        assert_eq!(prompt.value, right.display().to_string());
        assert!(
            matches!(prompt.kind, PromptKind::Transfer { sources, moving: false, .. } if sources == vec![left.join("drop.txt")])
        );
        assert!(!right.join("drop.txt").exists());
        assert_eq!(
            std::fs::read_to_string(left.join("drop.txt")).unwrap(),
            "keep me"
        );
    }

    #[test]
    fn external_drop_targets_hovered_pane_and_respects_modal_state() {
        let (_root, ctx, mut app, left, right) = fixture();
        frame(&mut app, &ctx, vec![], vec![]);
        let files = vec![egui::DroppedFile {
            path: Some(left.join("drop.txt")),
            ..Default::default()
        }];
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(egui::pos2(950.0, 400.0))],
            files.clone(),
        );
        assert_eq!(
            app.prompt.take().unwrap().value,
            right.display().to_string()
        );
        app.help = true;
        frame(&mut app, &ctx, vec![], files);
        assert!(app.prompt.is_none());
        assert!(!right.join("drop.txt").exists());
    }
}
