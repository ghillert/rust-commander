//! XDND v5 source. Polling avoids stealing winit's implicit pointer grab.
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};
use x11rb::{
    connection::Connection,
    protocol::{Event, xproto::*},
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

x11rb::atom_manager! { Atoms:AtomsCookie {
    XdndAware,XdndProxy,XdndEnter,XdndLeave,XdndPosition,XdndStatus,XdndDrop,XdndFinished,
    XdndSelection,XdndActionCopy,XdndActionMove,XdndActionList,TARGETS,INCR,
    uri_list:b"text/uri-list",
}}
pub(super) struct Pointer {
    connection: RustConnection,
    root: u32,
    window: u32,
}
impl Pointer {
    pub fn new(window: u32) -> Result<Self> {
        let (connection, screen) = x11rb::connect(None)?;
        let root = connection.setup().roots[screen].root;
        Ok(Self {
            connection,
            root,
            window,
        })
    }
    pub fn position(&self, scale: f64) -> Option<(f32, f32)> {
        let reply = self
            .connection
            .query_pointer(self.window)
            .ok()?
            .reply()
            .ok()?;
        reply.same_screen.then_some((
            (f64::from(reply.win_x) / scale) as f32,
            (f64::from(reply.win_y) / scale) as f32,
        ))
    }
    pub fn buttons(&self) -> (bool, bool) {
        self.connection
            .query_pointer(self.root)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|p| {
                let mask = u16::from(p.mask);
                (
                    mask & u16::from(KeyButMask::BUTTON1) != 0,
                    mask & u16::from(KeyButMask::SHIFT) != 0
                        && mask & u16::from(KeyButMask::CONTROL | KeyButMask::MOD1) == 0,
                )
            })
            .unwrap_or_default()
    }
}
fn property(connection: &RustConnection, window: u32, atom: u32) -> Option<u32> {
    connection
        .get_property(false, window, atom, AtomEnum::ANY, 0, 1)
        .ok()?
        .reply()
        .ok()?
        .value32()?
        .next()
}
fn target(connection: &RustConnection, root: u32, atoms: &Atoms) -> Option<(u32, u32)> {
    let mut window = root;
    let mut result = None;
    for _ in 0..64 {
        if property(connection, window, atoms.XdndAware).is_some_and(|v| v >= 3) {
            let proxy = property(connection, window, atoms.XdndProxy)
                .filter(|&proxy| property(connection, proxy, atoms.XdndProxy) == Some(proxy))
                .unwrap_or(window);
            result = Some((window, proxy));
        }
        let child = connection.query_pointer(window).ok()?.reply().ok()?.child;
        if child == 0 || child == window {
            break;
        }
        window = child;
    }
    result
}
fn message(
    connection: &RustConnection,
    target: (u32, u32),
    kind: u32,
    data: [u32; 5],
) -> Result<()> {
    connection.send_event(
        false,
        target.1,
        EventMask::NO_EVENT,
        ClientMessageEvent::new(32, target.0, kind, data),
    )?;
    connection.flush()?;
    Ok(())
}
pub(super) type Deferred = Arc<dyn Fn() -> std::result::Result<Vec<PathBuf>, String> + Send + Sync>;
#[cfg(test)]
pub(super) fn start(paths: Vec<PathBuf>, finished: impl Fn() + Send + 'static) -> Result<()> {
    start_offer(Some(encode(&paths)?), None, false, move |_| finished())
}
fn encode(paths: &[PathBuf]) -> Result<Vec<u8>> {
    let data = paths
        .iter()
        .map(|path| url::Url::from_file_path(path).map(|url| url.to_string()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| anyhow::anyhow!("Native file drags require absolute local paths"))?
        .join("\r\n");
    let data = format!("{data}\r\n").into_bytes();
    ensure!(
        !data.is_empty() && data.len() <= 16 * 1024 * 1024,
        "The native file list exceeds the 16 MiB limit"
    );
    Ok(data)
}
pub(super) fn start_files(
    paths: Vec<PathBuf>,
    moving: bool,
    finished: impl Fn(bool) + Send + 'static,
) -> Result<()> {
    start_offer(Some(encode(&paths)?), None, moving, finished)
}
pub(super) fn start_deferred(
    provider: Deferred,
    finished: impl Fn(bool) + Send + 'static,
) -> Result<()> {
    start_offer(None, Some(provider), false, finished)
}
fn start_offer(
    data: Option<Vec<u8>>,
    provider: Option<Deferred>,
    moving: bool,
    finished: impl Fn(bool) + Send + 'static,
) -> Result<()> {
    std::thread::Builder::new()
        .name("rust-commander-xdnd".into())
        .spawn(move || {
            let accepted = match run(data, provider, moving) {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("Native X11 drag failed: {error:#}");
                    false
                }
            };
            finished(accepted);
        })?;
    Ok(())
}
fn run(
    mut data: Option<Vec<u8>>,
    mut provider: Option<Deferred>,
    allow_move: bool,
) -> Result<bool> {
    let (connection, screen) = x11rb::connect(None)?;
    let root = connection.setup().roots[screen].root;
    let atoms = Atoms::new(&connection)?.reply()?;
    let source = connection.generate_id()?;
    connection
        .create_window(
            0,
            source,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )?
        .check()?;
    connection
        .set_selection_owner(source, atoms.XdndSelection, x11rb::CURRENT_TIME)?
        .check()?;
    let actions = if allow_move {
        vec![atoms.XdndActionCopy, atoms.XdndActionMove]
    } else {
        vec![atoms.XdndActionCopy]
    };
    connection.change_property32(
        PropMode::REPLACE,
        source,
        atoms.XdndActionList,
        AtomEnum::ATOM,
        &actions,
    )?;
    connection.flush()?;
    let setup = connection.setup();
    let keycodes = setup.max_keycode - setup.min_keycode + 1;
    let mapping = connection
        .get_keyboard_mapping(setup.min_keycode, keycodes)?
        .reply()?;
    let escape = mapping
        .keysyms
        .chunks(usize::from(mapping.keysyms_per_keycode))
        .position(|keys| keys.contains(&0xff1b))
        .map(|offset| usize::from(setup.min_keycode) + offset);
    let mut current = None;
    let mut accepted = false;
    let mut dropped = None;
    let mut increments: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    let began = Instant::now();
    let (result_tx, result_rx) = mpsc::channel();
    let mut waiting = Vec::new();
    let mut preferred = atoms.XdndActionCopy;
    loop {
        for _ in 0..512 {
            let queued = if data.is_some() {
                waiting.pop().map(Event::SelectionRequest)
            } else {
                None
            };
            let event = match queued {
                Some(event) => Some(event),
                None => connection.poll_for_event()?,
            };
            let Some(event) = event else {
                break;
            };
            match event {
                Event::ClientMessage(event)
                    if event.type_ == atoms.XdndStatus
                        && event.format == 32
                        && event.window == source
                        && current
                            .is_some_and(|(window, _)| event.data.as_data32()[0] == window) =>
                {
                    accepted = event.data.as_data32()[1] & 1 != 0
                        && (event.data.as_data32()[4] == atoms.XdndActionCopy
                            || (allow_move && event.data.as_data32()[4] == atoms.XdndActionMove));
                }
                Event::ClientMessage(event)
                    if event.type_ == atoms.XdndFinished
                        && event.format == 32
                        && event.window == source
                        && dropped.is_some()
                        && current
                            .is_some_and(|(window, _)| event.data.as_data32()[0] == window) =>
                {
                    connection.destroy_window(source)?;
                    connection.flush()?;
                    return Ok(event.data.as_data32()[1] & 1 != 0
                        || current.is_some_and(|(window, _)| {
                            property(&connection, window, atoms.XdndAware).unwrap_or(5) < 5
                        }));
                }
                Event::SelectionRequest(request) if request.selection == atoms.XdndSelection => {
                    let property = if request.property == 0 {
                        request.target
                    } else {
                        request.property
                    };
                    let mut response_property = property;
                    if request.target == atoms.TARGETS {
                        connection.change_property32(
                            PropMode::REPLACE,
                            request.requestor,
                            property,
                            AtomEnum::ATOM,
                            &[atoms.uri_list, atoms.TARGETS],
                        )?;
                    } else if request.target == atoms.uri_list {
                        let Some(data) = data.as_ref() else {
                            if waiting.len() < 32 {
                                waiting.push(request);
                            } else {
                                reject_request(&connection, &request)?;
                            }
                            continue;
                        };
                        if data.len() <= 48 * 1024 {
                            connection.change_property8(
                                PropMode::REPLACE,
                                request.requestor,
                                property,
                                atoms.uri_list,
                                data,
                            )?;
                        } else {
                            connection.change_window_attributes(
                                request.requestor,
                                &ChangeWindowAttributesAux::new()
                                    .event_mask(EventMask::PROPERTY_CHANGE),
                            )?;
                            connection.change_property32(
                                PropMode::REPLACE,
                                request.requestor,
                                property,
                                atoms.INCR,
                                &[u32::try_from(data.len())?],
                            )?;
                            increments.insert((request.requestor, property), 0);
                        }
                    } else {
                        response_property = 0;
                    }
                    connection.send_event(
                        false,
                        request.requestor,
                        EventMask::NO_EVENT,
                        SelectionNotifyEvent {
                            response_type: SELECTION_NOTIFY_EVENT,
                            sequence: 0,
                            time: request.time,
                            requestor: request.requestor,
                            selection: request.selection,
                            target: request.target,
                            property: response_property,
                        },
                    )?;
                    connection.flush()?;
                }
                Event::PropertyNotify(event) if event.state == Property::DELETE => {
                    if let Some(offset) = increments.get_mut(&(event.window, event.atom)) {
                        let data = data.as_ref().context("Missing incremental payload")?;
                        let end = (*offset + 48 * 1024).min(data.len());
                        connection.change_property8(
                            PropMode::REPLACE,
                            event.window,
                            event.atom,
                            atoms.uri_list,
                            &data[*offset..end],
                        )?;
                        let complete = *offset == data.len();
                        *offset = end;
                        if complete {
                            increments.remove(&(event.window, event.atom));
                        }
                        connection.flush()?;
                    }
                }
                _ => {}
            }
        }
        if dropped.is_some()
            && data.is_none()
            && !waiting.is_empty()
            && let Some(provider) = provider.take()
        {
            let sender = result_tx.clone();
            std::thread::spawn(move || {
                let value = provider()
                    .map_err(anyhow::Error::msg)
                    .and_then(|paths| encode(&paths))
                    .map_err(|e| e.to_string());
                let _ = sender.send(value);
            });
        }
        if let Ok(value) = result_rx.try_recv() {
            data = Some(value.map_err(anyhow::Error::msg)?);
        }
        if dropped.is_some_and(|when: Instant| when.elapsed() > Duration::from_secs(30 * 60)) {
            break;
        }
        if dropped.is_none() {
            let pointer = connection.query_pointer(root)?.reply()?;
            let escaped = if let Some(key) = escape {
                let mask = connection.query_keymap()?.reply()?.keys;
                mask[key / 8] & (1 << (key % 8)) != 0
            } else {
                false
            };
            let requested = if allow_move
                && pointer.mask.contains(KeyButMask::SHIFT)
                && !pointer.mask.contains(KeyButMask::CONTROL)
            {
                atoms.XdndActionMove
            } else {
                atoms.XdndActionCopy
            };
            if requested != preferred {
                preferred = requested;
                accepted = false;
            }
            let next = target(&connection, root, &atoms);
            if next != current {
                if let Some(previous) = current {
                    message(&connection, previous, atoms.XdndLeave, [source, 0, 0, 0, 0])?;
                }
                current = next;
                accepted = false;
                if let Some(next) = current {
                    message(
                        &connection,
                        next,
                        atoms.XdndEnter,
                        [
                            source,
                            property(&connection, next.0, atoms.XdndAware)
                                .unwrap_or(5)
                                .min(5)
                                << 24,
                            atoms.uri_list,
                            0,
                            0,
                        ],
                    )?;
                }
            }
            if escaped || began.elapsed() > Duration::from_secs(5 * 60) {
                if let Some(current) = current {
                    message(&connection, current, atoms.XdndLeave, [source, 0, 0, 0, 0])?;
                }
                break;
            }
            if u16::from(pointer.mask) & u16::from(KeyButMask::BUTTON1) == 0 {
                if let Some(current) = current.filter(|_| accepted) {
                    message(
                        &connection,
                        current,
                        atoms.XdndDrop,
                        [source, 0, x11rb::CURRENT_TIME, 0, 0],
                    )?;
                    dropped = Some(Instant::now());
                } else {
                    if let Some(current) = current {
                        message(&connection, current, atoms.XdndLeave, [source, 0, 0, 0, 0])?;
                    }
                    break;
                }
            } else if let Some(current) = current {
                let position =
                    (u32::from(pointer.root_x as u16) << 16) | u32::from(pointer.root_y as u16);
                message(
                    &connection,
                    current,
                    atoms.XdndPosition,
                    [source, 0, position, x11rb::CURRENT_TIME, preferred],
                )?;
            }
        }
        std::thread::sleep(Duration::from_millis(12));
    }
    for request in waiting {
        reject_request(&connection, &request)?;
    }
    connection
        .destroy_window(source)
        .context("Finish native file drag")?;
    connection.flush()?;
    Ok(false)
}

fn reject_request(connection: &RustConnection, request: &SelectionRequestEvent) -> Result<()> {
    connection.send_event(
        false,
        request.requestor,
        EventMask::NO_EVENT,
        SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: request.time,
            requestor: request.requestor,
            selection: request.selection,
            target: request.target,
            property: 0,
        },
    )?;
    connection.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use x11rb::protocol::xtest::ConnectionExt as _;
    #[test]
    #[ignore = "requires an isolated X server (xvfb-run)"]
    fn xdnd_round_trips_unicode_file_lists_and_incremental_payloads() {
        for count in [2, 1600] {
            let (connection, screen) = x11rb::connect(None).unwrap();
            let root = connection.setup().roots[screen].root;
            let atoms = Atoms::new(&connection).unwrap().reply().unwrap();
            let destination = connection.generate_id().unwrap();
            connection
                .create_window(
                    0,
                    destination,
                    root,
                    200,
                    200,
                    300,
                    300,
                    0,
                    WindowClass::INPUT_OUTPUT,
                    0,
                    &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
                )
                .unwrap()
                .check()
                .unwrap();
            connection
                .change_property32(
                    PropMode::REPLACE,
                    destination,
                    atoms.XdndAware,
                    AtomEnum::ATOM,
                    &[5],
                )
                .unwrap();
            connection.map_window(destination).unwrap();
            connection
                .warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, 10, 10)
                .unwrap();
            connection
                .xtest_fake_input(BUTTON_PRESS_EVENT, 1, x11rb::CURRENT_TIME, root, 10, 10, 0)
                .unwrap();
            connection.flush().unwrap();
            connection.get_input_focus().unwrap().reply().unwrap();
            let paths = (0..count)
                .map(|i| PathBuf::from(format!("/tmp/drag-fixture/{i:05} café #100%.txt")))
                .collect::<Vec<_>>();
            let expected = format!(
                "{}\r\n",
                paths
                    .iter()
                    .map(|p| url::Url::from_file_path(p).unwrap().to_string())
                    .collect::<Vec<_>>()
                    .join("\r\n")
            )
            .into_bytes();
            let (done, finished) = std::sync::mpsc::channel();
            start(paths, move || {
                let _ = done.send(());
            })
            .unwrap();
            connection
                .warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, 250, 250)
                .unwrap();
            connection.flush().unwrap();
            let pointer = Pointer::new(destination).unwrap();
            assert_eq!(pointer.position(1.0), Some((50.0, 50.0)));
            assert!(pointer.buttons().0);
            let property = connection
                .intern_atom(false, b"RUST_COMMANDER_DROP_FIXTURE")
                .unwrap()
                .reply()
                .unwrap()
                .atom;
            let mut source = 0;
            let mut released = false;
            let mut received = Vec::new();
            let mut incremental = false;
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                assert!(
                    Instant::now() < deadline,
                    "XDND fixture timed out; received {} bytes",
                    received.len()
                );
                if let Some(event) = connection.poll_for_event().unwrap() {
                    match event {
                        Event::ClientMessage(event) if event.type_ == atoms.XdndEnter => {
                            source = event.data.as_data32()[0];
                            assert_eq!(event.data.as_data32()[2], atoms.uri_list);
                        }
                        Event::ClientMessage(event) if event.type_ == atoms.XdndPosition => {
                            message(
                                &connection,
                                (source, source),
                                atoms.XdndStatus,
                                [destination, 3, 0, 0, atoms.XdndActionCopy],
                            )
                            .unwrap();
                            if !released {
                                connection
                                    .xtest_fake_input(
                                        BUTTON_RELEASE_EVENT,
                                        1,
                                        x11rb::CURRENT_TIME,
                                        root,
                                        250,
                                        250,
                                        0,
                                    )
                                    .unwrap();
                                connection.flush().unwrap();
                                released = true;
                            }
                        }
                        Event::ClientMessage(event) if event.type_ == atoms.XdndDrop => {
                            assert!(released);
                            connection
                                .convert_selection(
                                    destination,
                                    atoms.XdndSelection,
                                    atoms.uri_list,
                                    property,
                                    x11rb::CURRENT_TIME,
                                )
                                .unwrap();
                            connection.flush().unwrap();
                        }
                        Event::SelectionNotify(event) => {
                            assert_ne!(event.property, 0);
                            let value = connection
                                .get_property(
                                    true,
                                    destination,
                                    property,
                                    AtomEnum::ANY,
                                    0,
                                    u32::MAX,
                                )
                                .unwrap()
                                .reply()
                                .unwrap();
                            if value.type_ == atoms.INCR {
                                incremental = true;
                            } else {
                                received = value.value;
                                break;
                            }
                        }
                        Event::PropertyNotify(event)
                            if incremental
                                && event.atom == property
                                && event.state == Property::NEW_VALUE =>
                        {
                            let value = connection
                                .get_property(
                                    true,
                                    destination,
                                    property,
                                    AtomEnum::ANY,
                                    0,
                                    u32::MAX,
                                )
                                .unwrap()
                                .reply()
                                .unwrap();
                            if value.value.is_empty() {
                                break;
                            }
                            received.extend(value.value);
                        }
                        _ => {}
                    }
                } else {
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
            assert_eq!(received, expected);
            assert_eq!(incremental, count > 2);
            message(
                &connection,
                (source, source),
                atoms.XdndFinished,
                [destination, 1, atoms.XdndActionCopy, 0, 0],
            )
            .unwrap();
            finished.recv_timeout(Duration::from_secs(2)).unwrap();
            connection.destroy_window(destination).unwrap();
            connection.flush().unwrap();
        }
    }
    #[test]
    #[ignore = "requires an isolated X server (xvfb-run)"]
    fn xdnd_release_without_an_accepting_target_cancels_cleanly() {
        let (connection, screen) = x11rb::connect(None).unwrap();
        let root = connection.setup().roots[screen].root;
        connection
            .warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, 20, 20)
            .unwrap();
        connection
            .xtest_fake_input(BUTTON_PRESS_EVENT, 1, x11rb::CURRENT_TIME, root, 20, 20, 0)
            .unwrap();
        connection.flush().unwrap();
        let (done, finished) = std::sync::mpsc::channel();
        start(vec!["/tmp/cancel-fixture".into()], move || {
            let _ = done.send(());
        })
        .unwrap();
        connection
            .xtest_fake_input(
                BUTTON_RELEASE_EVENT,
                1,
                x11rb::CURRENT_TIME,
                root,
                20,
                20,
                0,
            )
            .unwrap();
        connection.flush().unwrap();
        finished.recv_timeout(Duration::from_secs(2)).unwrap();
    }
    #[test]
    #[ignore = "requires an isolated X server (xvfb-run)"]
    fn xdnd_deferred_probes_wait_for_drop_and_native_move_is_negotiated() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        for (deferred, accept) in [(true, true), (true, false), (false, true)] {
            let (connection, screen) = x11rb::connect(None).unwrap();
            let root = connection.setup().roots[screen].root;
            let atoms = Atoms::new(&connection).unwrap().reply().unwrap();
            let window = connection.generate_id().unwrap();
            connection
                .create_window(
                    0,
                    window,
                    root,
                    200,
                    200,
                    300,
                    300,
                    0,
                    WindowClass::INPUT_OUTPUT,
                    0,
                    &CreateWindowAux::new(),
                )
                .unwrap()
                .check()
                .unwrap();
            connection
                .change_property32(
                    PropMode::REPLACE,
                    window,
                    atoms.XdndAware,
                    AtomEnum::ATOM,
                    &[5],
                )
                .unwrap();
            connection.map_window(window).unwrap();
            connection
                .warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, 250, 250)
                .unwrap();
            connection
                .xtest_fake_input(
                    BUTTON_PRESS_EVENT,
                    1,
                    x11rb::CURRENT_TIME,
                    root,
                    250,
                    250,
                    0,
                )
                .unwrap();
            connection.flush().unwrap();
            connection.get_input_focus().unwrap().reply().unwrap();
            let calls = Arc::new(AtomicUsize::new(0));
            let counter = calls.clone();
            let (tx, rx) = mpsc::channel();
            if deferred {
                start_deferred(
                    Arc::new(move || {
                        counter.fetch_add(1, Ordering::Relaxed);
                        Ok(vec!["/tmp/deferred café.txt".into()])
                    }),
                    move |ok| {
                        let _ = tx.send(ok);
                    },
                )
                .unwrap();
            } else {
                start_files(vec!["/tmp/local-move.txt".into()], true, move |ok| {
                    let _ = tx.send(ok);
                })
                .unwrap();
            }
            let property = connection
                .intern_atom(false, b"DEFERRED_DROP_TEST")
                .unwrap()
                .reply()
                .unwrap()
                .atom;
            let mut source = 0;
            let mut released = false;
            let mut notified = false;
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                assert!(Instant::now() < deadline, "deferred XDND fixture timed out");
                if !accept
                    && released
                    && let Ok(ok) = rx.try_recv()
                {
                    assert!(!ok);
                    break;
                }
                if let Some(event) = connection.poll_for_event().unwrap() {
                    match event {
                        Event::ClientMessage(event) if event.type_ == atoms.XdndEnter => {
                            source = event.data.as_data32()[0];
                        }
                        Event::ClientMessage(event)
                            if event.type_ == atoms.XdndPosition && !released =>
                        {
                            message(
                                &connection,
                                (source, source),
                                atoms.XdndStatus,
                                [
                                    window,
                                    if accept { 3 } else { 0 },
                                    0,
                                    0,
                                    if deferred {
                                        atoms.XdndActionCopy
                                    } else {
                                        atoms.XdndActionMove
                                    },
                                ],
                            )
                            .unwrap();
                            if deferred {
                                connection
                                    .convert_selection(
                                        window,
                                        atoms.XdndSelection,
                                        atoms.uri_list,
                                        property,
                                        x11rb::CURRENT_TIME,
                                    )
                                    .unwrap();
                                connection.flush().unwrap();
                                std::thread::sleep(Duration::from_millis(80));
                                assert_eq!(
                                    calls.load(Ordering::Relaxed),
                                    0,
                                    "a hover probe must never materialize files"
                                );
                            } else {
                                let actions = connection
                                    .get_property(
                                        false,
                                        source,
                                        atoms.XdndActionList,
                                        AtomEnum::ATOM,
                                        0,
                                        16,
                                    )
                                    .unwrap()
                                    .reply()
                                    .unwrap();
                                assert!(
                                    actions
                                        .value32()
                                        .unwrap()
                                        .any(|action| action == atoms.XdndActionMove)
                                );
                            }
                            connection
                                .xtest_fake_input(
                                    BUTTON_RELEASE_EVENT,
                                    1,
                                    x11rb::CURRENT_TIME,
                                    root,
                                    250,
                                    250,
                                    0,
                                )
                                .unwrap();
                            connection.flush().unwrap();
                            released = true;
                        }
                        Event::ClientMessage(event) if event.type_ == atoms.XdndDrop => {
                            assert!(accept);
                            if !deferred {
                                connection
                                    .convert_selection(
                                        window,
                                        atoms.XdndSelection,
                                        atoms.uri_list,
                                        property,
                                        x11rb::CURRENT_TIME,
                                    )
                                    .unwrap();
                                connection.flush().unwrap();
                            }
                        }
                        Event::SelectionNotify(event) if event.property == property => {
                            notified = true;
                            break;
                        }
                        _ => {}
                    }
                } else {
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
            assert_eq!(
                calls.load(Ordering::Relaxed),
                usize::from(deferred && accept)
            );
            if accept {
                assert!(notified);
                message(
                    &connection,
                    (source, source),
                    atoms.XdndFinished,
                    [
                        window,
                        1,
                        if deferred {
                            atoms.XdndActionCopy
                        } else {
                            atoms.XdndActionMove
                        },
                        0,
                        0,
                    ],
                )
                .unwrap();
                assert!(rx.recv_timeout(Duration::from_secs(2)).unwrap());
            }
            connection.destroy_window(window).unwrap();
            connection.flush().unwrap();
        }
    }
}
