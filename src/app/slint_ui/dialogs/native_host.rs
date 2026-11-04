//! Native Windows/GTK forms run in an isolated UI process. Only private pipes
//! carry the live form (including passwords); no request is written to disk,
//! passed in arguments, logged or executed as a command by the helper.
use super::{Dialog, Picker, native_supported};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    time::{Duration, Instant},
};
#[cfg(target_os = "linux")]
mod gtk;
#[cfg(target_os = "windows")]
mod windows;

const MAX_MESSAGE: usize = 2 * 1024 * 1024;
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(super) enum Parent {
    Windows(usize),
    X11(u32),
    Wayland(String),
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct Model {
    pub form: Dialog,
    pub parent: Parent,
    pub scheme: crate::appearance::ColorScheme,
    pub language: crate::i18n::Language,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(super) enum Request {
    Update(Box<Model>),
    Close,
    #[cfg(feature = "screenshot")]
    Page(i32),
    #[cfg(feature = "screenshot")]
    Capture(String),
    #[cfg(feature = "screenshot")]
    Smoke(String, String),
}
#[derive(Serialize, Deserialize)]
pub(in super::super) enum Event {
    Editing,
    Ready,
    Field(String, String),
    Action(String),
    Select(String),
    Failed,
    #[cfg(feature = "screenshot")]
    Checked(bool),
    #[cfg(feature = "screenshot")]
    Captured(bool),
}

fn read_message<T: serde::de::DeserializeOwned>(
    reader: &mut impl BufRead,
) -> std::io::Result<Option<T>> {
    let mut bytes = Vec::new();
    let count = reader
        .take((MAX_MESSAGE + 1) as u64)
        .read_until(b'\n', &mut bytes)?;
    if count == 0 {
        return Ok(None);
    }
    if count > MAX_MESSAGE || bytes.last() != Some(&b'\n') {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| std::io::ErrorKind::InvalidData.into())
}
pub(super) fn emit(event: &Event) {
    let mut stdout = std::io::stdout().lock();
    if serde_json::to_writer(&mut stdout, event).is_ok() {
        let _ = stdout.write_all(b"\n");
        let _ = stdout.flush();
    }
}
pub(super) struct Incoming {
    commands: Receiver<Request>,
    latest: Arc<Mutex<Option<Model>>>,
}
impl Incoming {
    fn first_model(&self) -> Option<Model> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Some(model) = self.latest.lock().unwrap().take() {
                return Some(model);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        None
    }
    pub(super) fn try_iter(&self) -> std::vec::IntoIter<Request> {
        let mut requests = Vec::new();
        if let Some(model) = self.latest.lock().unwrap().take() {
            requests.push(Request::Update(Box::new(model)));
        }
        requests.extend(self.commands.try_iter());
        requests.into_iter()
    }
}
pub(super) fn incoming() -> Incoming {
    let (sender, receiver) = mpsc::sync_channel(8);
    let latest = Arc::new(Mutex::new(None));
    let updates = latest.clone();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        while let Ok(Some(request)) = read_message::<Request>(&mut stdin) {
            match request {
                Request::Update(model) => *updates.lock().unwrap() = Some(*model),
                Request::Close => exit_helper(),
                #[cfg(feature = "screenshot")]
                request => {
                    if sender.send(request).is_err() {
                        return;
                    }
                }
            }
        }
        // A blocked native file chooser must also disappear if its owner dies.
        // This process performs no file operations; the OS reclaims its windows.
        exit_helper();
    });
    #[cfg(not(feature = "screenshot"))]
    let _ = sender;
    Incoming {
        commands: receiver,
        latest,
    }
}

fn exit_helper() -> ! {
    // This disposable process has no application state or file operations to
    // flush. GTK's C exit handlers must not run on this reader thread while
    // the main thread is using GTK or displaying a native chooser.
    #[cfg(target_os = "linux")]
    unsafe {
        libc::_exit(0)
    }
    #[cfg(target_os = "windows")]
    std::process::exit(0)
}

pub(in super::super) fn run_helper() -> i32 {
    let incoming = incoming();
    let Some(model) = incoming.first_model() else {
        return 2;
    };
    if !native_supported(&model.form) {
        return 2;
    }
    #[cfg(target_os = "linux")]
    return gtk::run(model, incoming);
    #[cfg(target_os = "windows")]
    return windows::run(model, incoming);
}

struct Process {
    child: Option<Child>,
    sender: SyncSender<Request>,
    events: Receiver<Event>,
    model: Model,
    ready: bool,
    started: Instant,
}
impl Process {
    fn start(model: Model) -> std::io::Result<Self> {
        let mut child = Command::new(std::env::current_exe()?)
            .arg("--native-forms-helper")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child.stdout.take().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let (sender, incoming) = mpsc::sync_channel::<Request>(1);
        std::thread::spawn(move || {
            for request in incoming {
                if serde_json::to_writer(&mut stdin, &request).is_err()
                    || stdin.write_all(b"\n").is_err()
                    || stdin.flush().is_err()
                {
                    break;
                }
            }
        });
        let (reply, events) = mpsc::sync_channel(512);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Ok(Some(event)) = read_message::<Event>(&mut reader) {
                if reply.send(event).is_err() {
                    return;
                }
            }
            let _ = reply.send(Event::Failed);
        });
        sender
            .send(Request::Update(Box::new(model.clone())))
            .map_err(|_| std::io::ErrorKind::BrokenPipe)?;
        Ok(Self {
            child: Some(child),
            sender,
            events,
            model,
            ready: false,
            started: Instant::now(),
        })
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // The disposable helper has no application state to flush. Kill it
            // before closing its pipes, then reap away from the UI thread:
            // Win32 window teardown can synchronously message its owner.
            let _ = child.kill();
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}
#[derive(Default)]
pub(in super::super) struct Host {
    process: Option<Process>,
    failed_kind: Option<String>,
    #[cfg(feature = "screenshot")]
    pending: Vec<Event>,
    #[cfg(feature = "screenshot")]
    fixture: Option<Fixture>,
}
#[cfg(feature = "screenshot")]
struct Fixture {
    request: Request,
    capture: bool,
    started: Instant,
    reply: Option<bool>,
}
impl Host {
    pub fn active(&self) -> bool {
        self.process.is_some()
    }
    #[cfg(feature = "screenshot")]
    pub fn fixture_active(&self) -> bool {
        self.active() || self.fixture.is_some()
    }
    pub fn events(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        #[cfg(feature = "screenshot")]
        events.append(&mut self.pending);
        let mut failed = false;
        if let Some(process) = &mut self.process {
            for event in process.events.try_iter() {
                match event {
                    Event::Ready => process.ready = true,
                    Event::Failed => failed = true,
                    #[cfg(feature = "screenshot")]
                    Event::Checked(ok) if self.fixture.as_ref().is_some_and(|f| !f.capture) => {
                        self.fixture.as_mut().unwrap().reply = Some(ok);
                    }
                    #[cfg(feature = "screenshot")]
                    Event::Captured(ok) if self.fixture.as_ref().is_some_and(|f| f.capture) => {
                        self.fixture.as_mut().unwrap().reply = Some(ok);
                    }
                    event => events.push(event),
                }
            }
            if process
                .child
                .as_mut()
                .unwrap()
                .try_wait()
                .ok()
                .flatten()
                .is_some()
                || (!process.ready && process.started.elapsed() > Duration::from_secs(4))
            {
                failed = true;
            }
        }
        if failed {
            self.failed_kind = self.process.as_ref().map(|p| p.model.form.kind.clone());
            self.process = None;
        }
        events
    }
    #[cfg(feature = "screenshot")]
    fn test_request(&mut self, request: Request, capture: bool) -> bool {
        // Never wait on the UI thread: the helper can synchronously message its
        // Win32 owner while rebuilding controls, changing focus or capturing.
        // Preserve normal field/action events for the next application tick.
        let events = self.events();
        self.pending.extend(events);
        if let Some(fixture) = self.fixture.as_ref() {
            assert!(fixture.request == request && fixture.capture == capture);
            if let Some(ok) = fixture.reply {
                self.fixture = None;
                assert!(ok, "Native control fixture failed");
                return true;
            }
            assert!(
                fixture.started.elapsed() < Duration::from_secs(4),
                "Native helper fixture response timed out"
            );
            assert!(self.active(), "Native helper must stay active");
            return false;
        }
        let process = self
            .process
            .as_mut()
            .expect("Native helper must stay active");
        match process.sender.try_send(request.clone()) {
            Ok(()) => {
                self.fixture = Some(Fixture {
                    request,
                    capture,
                    started: Instant::now(),
                    reply: None,
                });
            }
            Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => panic!("Native helper pipe closed"),
        }
        false
    }
    #[cfg(feature = "screenshot")]
    pub fn snapshot(&mut self, path: &std::path::Path) -> Option<bool> {
        self.fixture_active()
            .then(|| self.test_request(Request::Capture(path.to_string_lossy().into()), true))
    }
    #[cfg(feature = "screenshot")]
    pub fn smoke(&mut self, callback: &str, value: &str) -> bool {
        let request = if callback == "native-page" {
            Request::Page(value.parse().unwrap())
        } else {
            Request::Smoke(callback.into(), value.into())
        };
        self.test_request(request, false)
    }
    pub fn sync(
        &mut self,
        ui: &super::super::CommanderWindow,
        form: Option<&Dialog>,
        scheme: crate::appearance::ColorScheme,
        language: crate::i18n::Language,
        _wayland_parent: Option<&str>,
    ) -> bool {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use slint::ComponentHandle;
        let Some(form) = form.filter(|f| native_supported(f)) else {
            self.process = None;
            self.failed_kind = None;
            return false;
        };
        if self.failed_kind.as_deref() == Some(&form.kind) {
            return false;
        }
        let Ok(handle) = ui
            .window()
            .window_handle()
            .window_handle()
            .map(|h| h.as_raw())
        else {
            return false;
        };
        let parent = match handle {
            #[cfg(target_os = "windows")]
            RawWindowHandle::Win32(h) => Parent::Windows(h.hwnd.get() as usize),
            #[cfg(target_os = "linux")]
            RawWindowHandle::Xlib(h) => match u32::try_from(h.window) {
                Ok(id) => Parent::X11(id),
                Err(_) => return false,
            },
            #[cfg(target_os = "linux")]
            RawWindowHandle::Xcb(h) => Parent::X11(h.window.get()),
            #[cfg(target_os = "linux")]
            RawWindowHandle::Wayland(_) => match _wayland_parent {
                Some(handle) => Parent::Wayland(handle.into()),
                None => return false,
            },
            _ => return false,
        };
        let model = Model {
            form: form.clone(),
            parent,
            scheme,
            language,
        };
        if self.process.as_ref().is_some_and(|p| p.model == model) {
            return true;
        }
        if serde_json::to_vec(&model).map_or(true, |bytes| bytes.len() > MAX_MESSAGE - 32) {
            self.process = None;
            return false;
        }
        if self
            .process
            .as_ref()
            .is_some_and(|p| p.model.form.kind != model.form.kind)
        {
            self.process = None;
        }
        if self.process.is_none() {
            match Process::start(model.clone()) {
                Ok(process) => self.process = Some(process),
                Err(_) => {
                    self.failed_kind = Some(form.kind.clone());
                    return false;
                }
            }
        }
        let process = self.process.as_mut().unwrap();
        if process.model != model {
            match process
                .sender
                .try_send(Request::Update(Box::new(model.clone())))
            {
                Ok(()) => process.model = model,
                Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => {
                    self.failed_kind = Some(form.kind.clone());
                    self.process = None;
                    return false;
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "screenshot")]
    #[test]
    fn completed_action_is_acknowledged_after_the_form_closes() {
        // The application can close the helper on Action before the screenshot
        // timer next polls the Checked reply. Do not send the action twice.
        let mut host = Host {
            fixture: Some(Fixture {
                request: Request::Smoke("native-action".into(), "cancel".into()),
                capture: false,
                started: Instant::now(),
                reply: Some(true),
            }),
            ..Default::default()
        };
        assert!(!host.active());
        assert!(host.fixture_active());
        assert!(host.smoke("native-action", "cancel"));
        assert!(!host.fixture_active());
    }
    #[test]
    fn protocol_rejects_unterminated_and_oversized_messages() {
        assert!(read_message::<Event>(&mut &b"{\"Action\":\"accept\"}"[..]).is_err());
        assert!(read_message::<Event>(&mut &vec![b'x'; MAX_MESSAGE + 1][..]).is_err());
        assert!(
            matches!(read_message::<Event>(&mut &b"{\"Action\":\"cancel\"}\n"[..]).unwrap(),Some(Event::Action(id)) if id=="cancel")
        );
    }
}
