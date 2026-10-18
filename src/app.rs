use crate::{
    filesystem::{self, Entry, Location, Sort},
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
    Done(Result<String, String>),
}
struct Job {
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
        #[cfg(target_os = "macos")]
        let mut app = app;
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
        let app = Self {
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
            self.fail("Wait for the current operation to finish.");
            return;
        }
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
            let result = operations::run(operation, &mut task).map_err(|e| format!("{e:#}"));
            let _ = sender.send(JobEvent::Done(result));
            worker_ctx.request_repaint();
        });
        self.job = Some(Job {
            receiver,
            cancel,
            progress: Progress::default(),
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        #[cfg(target_os = "macos")]
        if let Some(menu) = &self.native_menu {
            let blocked = self.job.is_some()
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
            self.job = None;
            match result {
                Ok(message) => {
                    if message.starts_with("Checksums")
                        || message.starts_with("Properties")
                        || message.starts_with("Folder comparison")
                        || message.contains("Replaced originals are kept")
                    {
                        self.message = Some(message.clone());
                    }
                    self.status = message;
                    self.status_error = false;
                }
                Err(error) => self.fail(error),
            }
            self.refresh(ctx);
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
            || self.panes.iter().any(|p| p.tabs.iter().any(|t| t.loading))
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if self.exiting && self.job.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn open_entry(&mut self, entry: Entry, ctx: &egui::Context) {
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
        match action {
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
                if let Some((from, entries, moving)) = self.clipboard.clone() {
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
                    self.fail("Copy or cut files in this app first.");
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
                            "Transfer {} item(s) to the destination? Existing targets are refused.",
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
        if self.job.is_some() && !matches!(action, Action::Bookmark) {
            self.fail("Wait for the current operation to finish.");
            return;
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
                            ui.menu_button(title, |ui| {
                                for (label, a) in entries {
                                    if ui.button(label).clicked() {
                                        action = Some(a);
                                        ui.close();
                                    }
                                }
                                if title == "File" && ui.button("Quit").clicked() {
                                    action = Some(Action::Quit);
                                    ui.close();
                                }
                            });
                        }
                        if ui.button("Help").clicked() {
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
                        for (label, a) in [
                            ("New folder", Action::Mkdir),
                            ("Copy", Action::Copy),
                            ("Move", Action::Move),
                        ] {
                            if ui
                                .add_enabled(self.job.is_none(), egui::Button::new(label))
                                .clicked()
                            {
                                action = Some(a);
                            }
                        }
                        ui.menu_button("More", |ui| {
                            for (label, a) in [
                                ("Rename…", Action::Rename),
                                ("Create archive…", Action::Pack),
                                ("Extract archive…", Action::Extract),
                                ("Checksums", Action::Checksum),
                                ("Batch rename…", Action::BatchRename),
                                ("Open terminal", Action::Terminal),
                            ] {
                                if ui.button(label).clicked() {
                                    action = Some(a);
                                    ui.close();
                                }
                            }
                        });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Search").clicked() {
                            action = Some(Action::Search);
                        }
                        ui.menu_button("View", |ui| {
                            let mut hidden = self.show_hidden;
                            if ui.checkbox(&mut hidden, "Show hidden files").changed() {
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
                                        scheme.label(),
                                    )
                                    .changed();
                            }
                            ui.separator();
                            if ui.button("Settings…").clicked() {
                                action = Some(Action::Preferences);
                                ui.close();
                            }
                        });
                        if ui.button("Refresh").clicked() {
                            action = Some(Action::Refresh);
                        }
                        if ui.button("Bookmark").clicked() {
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
                ui.label(RichText::new("Favorites").small().strong().weak());
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
                            label,
                            self.tab().location == Location::Local(path.clone()),
                            icon,
                            &palette,
                        )
                        .clicked()
                    {
                        location = Some(Location::Local(path));
                    }
                }
                #[cfg(target_os = "windows")]
                for drive in b'A'..=b'Z' {
                    let path = PathBuf::from(format!("{}:\\", drive as char));
                    if path.is_dir() && ui.button(path.display().to_string()).clicked() {
                        location = Some(Location::Local(path));
                    }
                }
                #[cfg(target_os = "macos")]
                if let Ok(volumes) = std::fs::read_dir("/Volumes") {
                    ui.collapsing("Volumes", |ui| {
                        for volume in volumes.flatten() {
                            if ui.button(volume.file_name().to_string_lossy()).clicked() {
                                location = Some(Location::Local(volume.path()));
                            }
                        }
                    });
                }
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Bookmarks").small().strong().weak());
                    if ui.small_button("+").clicked() {
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
                            .small_button("×")
                            .on_hover_text("Remove bookmark")
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
                        RichText::new("Tab switches panes\nSpace marks files\nF1 opens help")
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
                    if ui.button("Cancel").clicked() {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                    ui.add(egui::Label::new(&job.progress.current).truncate());
                });
            } else if self.pending_view.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Opening file…");
                });
            } else {
                let color = if self.status_error {
                    Color32::from_rgb(190, 65, 55)
                } else {
                    ui.visuals().weak_text_color()
                };
                if self.preferences.status_bar {
                    ui.add(egui::Label::new(RichText::new(&self.status).color(color)).truncate())
                        .on_hover_text(&self.status);
                }
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
                            .add_enabled(self.job.is_none(), egui::Button::new(label).frame(false))
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
                                                        && ui.small_button("×").clicked()
                                                    {
                                                        close = Some(index);
                                                    }
                                                }
                                                if ui
                                                    .small_button("+")
                                                    .on_hover_text("New tab")
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
                                        if ui.small_button("<").on_hover_text("Back").clicked() {
                                            tab.history(false, ctx);
                                            self.active = side;
                                        }
                                        if ui.small_button(">").on_hover_text("Forward").clicked() {
                                            tab.history(true, ctx);
                                            self.active = side;
                                        }
                                        if ui
                                            .small_button("Up")
                                            .on_hover_text("Parent folder")
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
                                                .hint_text("Path to folder"),
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
                                    ui.horizontal(|ui| {
                                        if matches!(tab.location, Location::Archive { .. }) {
                                            ui.label(
                                                RichText::new("ARCHIVE · READ ONLY")
                                                    .size(10.0)
                                                    .color(ui.visuals().hyperlink_color),
                                            );
                                        }
                                        let filter = ui.add(
                                            egui::TextEdit::singleline(&mut tab.filter)
                                                .hint_text("Filter files…")
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
                                        for (label, sort, w) in [
                                            ("Name", Sort::Name, width * 0.49),
                                            ("Size", Sort::Size, width * 0.16),
                                            ("Modified", Sort::Modified, width * 0.27),
                                        ] {
                                            let label = if tab.sort == sort {
                                                format!(
                                                    "{label} {}",
                                                    if tab.descending { "v" } else { "^" }
                                                )
                                            } else {
                                                label.into()
                                            };
                                            if ui
                                                .add_sized(
                                                    [w, 24.0],
                                                    egui::Button::new(
                                                        RichText::new(label).small().strong(),
                                                    )
                                                    .frame(false),
                                                )
                                                .clicked()
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
                                        }
                                    });
                                    ui.separator();
                                    let visible = tab.visible(self.show_hidden);
                                    let height = (ui.available_height() - 38.0).max(100.0);
                                    if let Some(error) = &tab.error {
                                        ui.colored_label(Color32::from_rgb(190, 65, 55), error);
                                    }
                                    if tab.loading {
                                        ui.horizontal(|ui| {
                                            ui.spinner();
                                            ui.label("Loading folder…");
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
                                                response.dnd_set_drag_payload(FileDrag { sources });
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
                                            let name_rect = egui::Rect::from_min_max(
                                                rect.min + Vec2::new(31.0, 0.0),
                                                egui::pos2(
                                                    rect.left() + width * 0.51 - 6.0,
                                                    rect.bottom(),
                                                ),
                                            );
                                            painter.with_clip_rect(name_rect).text(
                                                name_rect.left_center(),
                                                egui::Align2::LEFT_CENTER,
                                                &entry.name,
                                                egui::FontId::proportional(font_size - 1.0),
                                                text_color,
                                            );
                                            let size = if entry.is_dir {
                                                "—".into()
                                            } else {
                                                filesystem::human_size(entry.size)
                                            };
                                            painter.text(
                                                egui::pos2(
                                                    rect.left() + width * 0.51,
                                                    rect.center().y,
                                                ),
                                                egui::Align2::LEFT_CENTER,
                                                size,
                                                egui::FontId::proportional(font_size - 2.0),
                                                detail_color,
                                            );
                                            let modified = entry
                                                .modified
                                                .map(|t| {
                                                    chrono::DateTime::<chrono::Local>::from(t)
                                                        .format("%Y-%m-%d %H:%M")
                                                        .to_string()
                                                })
                                                .unwrap_or_else(|| "—".into());
                                            painter.text(
                                                egui::pos2(
                                                    rect.left() + width * 0.70,
                                                    rect.center().y,
                                                ),
                                                egui::Align2::LEFT_CENTER,
                                                modified,
                                                egui::FontId::proportional(font_size - 3.0),
                                                detail_color,
                                            );
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
                });
            } else {
                self.fail("Finish or close the current dialog before dropping files.");
            }
        }
    }

    fn prompt(&mut self, ctx: &egui::Context) {
        let Some(mut prompt) = self.prompt.take() else {
            return;
        };
        let mut accept = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("operation_prompt")).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.heading(&prompt.title);
            ui.label(&prompt.description);
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
                ui.colored_label(Color32::from_rgb(190, 65, 55), error);
            }
            if matches!(prompt.kind, PromptKind::Transfer { .. }) {
                use crate::transfer::ConflictPolicy;
                ui.label("When a target exists:");
                for (policy, label) in [
                    (ConflictPolicy::Refuse, "Stop before copying"),
                    (ConflictPolicy::Skip, "Skip existing items"),
                    (ConflictPolicy::Rename, "Keep both, with a numbered name"),
                    (ConflictPolicy::Replace, "Replace, keeping originals in a recovery folder"),
                ] {
                    ui.radio_value(&mut prompt.transfer_options.conflicts, policy, label);
                }
                ui.add_enabled_ui(prompt.transfer_options.conflicts != ConflictPolicy::Rename, |ui| {
                    ui.checkbox(&mut prompt.transfer_options.merge_folders, "Merge existing folders");
                });
                if prompt.transfer_options.conflicts == ConflictPolicy::Replace {
                    ui.label("Replaced items stay in a hidden .rust-commander-replaced-* folder in the destination. The result reports its path.");
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
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
                    } => Operation::RemoteTransfer {
                        from: from.clone(),
                        entries: entries.clone(),
                        to: filesystem::parse_location(&prompt.value, to)?,
                        moving: *moving,
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
                        Operation::Transfer {
                            sources: sources.clone(),
                            destination,
                            moving: *moving,
                            options: prompt.transfer_options,
                        }
                    }
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
                ui.heading(if editor.editable {
                    "Text editor"
                } else {
                    editor.kind
                });
                if editor.dirty() {
                    ui.label(RichText::new("Unsaved changes").color(ui.visuals().hyperlink_color));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Close").clicked() {
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
                            egui::Button::new("Previous page"),
                        )
                        .clicked()
                    {
                        page_request = Some(pdf.page - 1);
                    }
                    ui.label(format!("Page {} of {}", pdf.page + 1, pdf.pages));
                    if ui
                        .add_enabled(
                            pdf.page + 1 < pdf.pages && self.pending_pdf.is_none(),
                            egui::Button::new("Next page"),
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
                ui.add(egui::Slider::new(&mut editor.zoom, 0.1..=4.0).text("Zoom"));
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
                ui.colored_label(Color32::from_rgb(190, 65, 55), error);
            }
            if editor.editable
                && (ui
                    .add_enabled(editor.dirty(), egui::Button::new("Save  ⌘/Ctrl+S"))
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
                ui.label("Discard unsaved changes?");
                ui.horizontal(|ui| {
                    if ui.button("Keep editing").clicked() {
                        editor.confirm_close = false;
                    }
                    if ui.button("Discard changes").clicked() {
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
            ui.heading("Preferences");
            egui::ScrollArea::vertical()
                .max_height(ctx.content_rect().height() - 180.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Desktop style");
                        egui::ComboBox::from_id_salt("desktop_style")
                            .selected_text(preferences.desktop_style.label())
                            .show_ui(ui, |ui| {
                                for desktop in crate::appearance::DesktopStyle::ALL {
                                    ui.selectable_value(
                                        &mut preferences.desktop_style,
                                        desktop,
                                        desktop.label(),
                                    );
                                }
                            });
                    });
                    ui.horizontal(|ui| {
                        ui.label("Appearance");
                        for scheme in [
                            crate::appearance::ColorScheme::System,
                            crate::appearance::ColorScheme::Light,
                            crate::appearance::ColorScheme::Dark,
                        ] {
                            ui.selectable_value(
                                &mut preferences.color_scheme,
                                scheme,
                                scheme.label(),
                            );
                        }
                    });
                    ui.add(
                        egui::Slider::new(&mut preferences.font_size, 10.0..=24.0)
                            .text("Font size"),
                    );
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
                        ui.checkbox(value, label);
                    }
                    ui.separator();
                    ui.label("Keyboard shortcuts · Command means Cmd on macOS and Ctrl elsewhere");
                    egui::Grid::new("shortcut_settings")
                        .num_columns(2)
                        .show(ui, |ui| {
                            for (action, binding) in &mut preferences.shortcuts {
                                ui.label(action);
                                ui.text_edit_singleline(binding);
                                ui.end_row();
                            }
                        });
                });
            ui.separator();
            if let Some(error) = &self.preferences_error {
                ui.colored_label(Color32::RED, error);
            }
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    close = true;
                }
                if ui.button("Apply").clicked() {
                    apply = true;
                }
                if ui.button("Restore defaults").clicked() {
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
        let mut open = None;
        egui::Modal::new(egui::Id::new("connections")).show(ctx, |ui| {
            ui.set_width(570.0); ui.heading("Connections");
            egui::ScrollArea::vertical().max_height(ctx.content_rect().height() - 160.0).show(ui, |ui| {
                for connection in &self.connections {
                    ui.horizontal(|ui| {
                        ui.label(format!("{} · {}", connection.name, connection.service));
                        if ui.button("Open").clicked() { open = Some(connection.id.clone()); }
                        if ui.button("Edit").clicked() { form.connection = connection.clone(); }
                        if ui.button("Forget credentials").clicked()
                            && let Err(error) = crate::remote::forget_credentials(&connection.id) { form.error = Some(error.to_string()); }
                    });
                }
                ui.separator();
                egui::Grid::new("connection_fields").num_columns(2).show(ui, |ui| {
                    ui.label("Name"); ui.text_edit_singleline(&mut form.connection.name); ui.end_row();
                    ui.label("Service"); egui::ComboBox::from_id_salt("remote_service").selected_text(&form.connection.service).show_ui(ui, |ui| {
                        for service in crate::remote::SERVICES { ui.selectable_value(&mut form.connection.service, (*service).into(), *service); }
                    }); ui.end_row();
                    for (label, value) in [("Endpoint", &mut form.connection.endpoint), ("Root folder", &mut form.connection.root),
                        ("Bucket", &mut form.connection.bucket), ("Region", &mut form.connection.region), ("Username", &mut form.connection.username), ("Private key file", &mut form.connection.key_file)] {
                        ui.label(label); ui.text_edit_singleline(value); ui.end_row();
                    }
                    if form.connection.service != "sftp" { ui.label("Password"); ui.add(egui::TextEdit::singleline(&mut form.secrets.password).password(true)); ui.end_row(); }
                    for (label, value) in [("Access key ID", &mut form.secrets.access_key_id), ("Secret access key", &mut form.secrets.secret_access_key), ("Access token / base64 GCS credentials", &mut form.secrets.access_token)] {
                        ui.label(label); ui.add(egui::TextEdit::singleline(value).password(true)); ui.end_row();
                    }
                });
                if form.connection.service == "sftp" { ui.label("SFTP uses OpenSSH, your SSH agent or private key, and strict known-host verification. Add a trusted server to your OpenSSH known_hosts before connecting."); }
                ui.checkbox(&mut form.remember, "Remember credentials in the operating system credential store");
                if let Some(error) = &form.error { ui.colored_label(Color32::from_rgb(190, 65, 55), error); }
                ui.horizontal(|ui| {
                    if ui.button("Close").clicked() { close = true; }
                    if ui.add_enabled(self.pending_connection.is_none(), egui::Button::new("Connect")).clicked() { connect = true; }
                    if self.pending_connection.is_some() { ui.spinner(); ui.label("Connecting…"); }
                });
            });
        });
        if connect {
            let connection = form.connection.clone();
            let secrets = std::mem::take(&mut form.secrets);
            let remember = form.remember;
            let (sender, receiver) = mpsc::channel();
            let ctx = ctx.clone();
            self.pending_connection = Some(receiver);
            form.error = None;
            std::thread::spawn(move || {
                let result = crate::remote::connect(&connection, secrets, remember)
                    .map(|()| connection)
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
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
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
                ui.label(format!("Native dual-pane file manager · {}", env!("CARGO_PKG_VERSION")));
                ui.separator();
                egui::Grid::new("shortcuts").num_columns(2).spacing([28.0, 7.0]).show(ui, |ui| {
                    for (key, description) in [("Tab", "Switch active pane"), ("↑ / ↓ · Enter · Backspace", "Navigate files · Open · Parent folder"), ("Space / Insert", "Mark or unmark current file"), ("Shift / ⌘ / Ctrl + click", "Range / multiple selection"), ("F2 / F3 / F4", "Rename / View / Edit"), ("F5 / F6 / F7 / F8", "Copy / Move / New folder / Trash"), ("⌘ / Ctrl + L / F", "Focus address / filter"), ("⌘ / Ctrl + T / W", "New tab / close tab"), ("⌘ / Ctrl + A / D / R / H", "Select all / bookmark / refresh / hidden files"), ("Alt + ← / →", "Back / forward"), ("Escape", "Clear marks and filter / close dialog")] { ui.label(RichText::new(key).monospace()); ui.label(description); ui.end_row(); }
                });
                ui.separator();
                ui.label("Double-click folders and supported archives to browse, or files to view text, images, hex and PDFs. Archive contents are read-only; use Extract or F5 to copy them out. Text editing supports local UTF-8 files up to 2 MiB.");
                ui.label("Inspired by muCommander. Independent Rust implementation; not an official release. GPL-3.0-only. You may redistribute this program under that license. No warranty.");
                ui.hyperlink_to("Original muCommander project", "https://github.com/mucommander/mucommander");
                ui.collapsing("GNU General Public License v3", |ui| { egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| { ui.label(include_str!("../LICENSE")); }); });
                if ui.button("Close").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) { self.help = false; }
            });
        }
        if let Some(message) = self.message.clone() {
            egui::Modal::new(egui::Id::new("result")).show(ctx, |ui| {
                ui.set_width(550.0);
                ui.heading("Results");
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
                    if ui.button("Copy result").clicked() {
                        ctx.copy_text(message.clone());
                    }
                    if ui.button("Close").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        self.message = None;
                    }
                });
            });
        }
        if self.confirm_exit {
            egui::Modal::new(egui::Id::new("exit")).show(ctx, |ui| {
                ui.heading(format!("Close {}?", crate::APP_NAME)); ui.label("There is a running operation or unsaved text. Closing cancels the operation and discards unsaved text.");
                ui.horizontal(|ui| {
                    if ui.button("Keep working").clicked() { self.confirm_exit = false; }
                    if ui.button("Cancel work and close").clicked() {
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
                        ui.label("Terminal · click the screen to type");
                        if ui.button("Hide terminal").clicked() {
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
