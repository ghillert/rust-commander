//! A private event queue on the host window's Wayland connection.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};

use sctk::data_device_manager::data_device::{DataDevice, DataDeviceHandler};
use sctk::data_device_manager::data_offer::{DataOfferHandler, DragOffer};
use sctk::data_device_manager::data_source::{DataSourceHandler, DragSource};
use sctk::data_device_manager::{DataDeviceManagerState, WritePipe};
use sctk::reexports::calloop::channel::{self, Channel, Sender};
use sctk::reexports::calloop::EventLoop;
use sctk::reexports::calloop_wayland_source::WaylandSource;
use sctk::reexports::client::globals::registry_queue_init;
use sctk::reexports::client::protocol::wl_data_device::WlDataDevice;
use sctk::reexports::client::protocol::wl_data_device_manager::DndAction;
use sctk::reexports::client::protocol::wl_data_source::WlDataSource;
use sctk::reexports::client::protocol::wl_pointer::WlPointer;
use sctk::reexports::client::protocol::wl_seat::WlSeat;
use sctk::reexports::client::protocol::wl_surface::WlSurface;
use sctk::reexports::client::{Connection, Proxy, QueueHandle};
use sctk::registry::{ProvidesRegistryState, RegistryState};
use sctk::seat::pointer::{PointerData, PointerEvent, PointerEventKind, PointerHandler};
use sctk::seat::{Capability, SeatHandler, SeatState};
use sctk::{
    delegate_data_device, delegate_pointer, delegate_registry, delegate_seat, registry_handlers,
};
use wayland_backend::client::{Backend, ObjectId};

use crate::DndEvent;

const MIME: &str = "text/uri-list";

enum Command {
    StartDrag(Vec<u8>, bool),
    Received { x: f64, y: f64, data: Vec<u8> },
    Exit,
}

/// The thread that owns the Wayland data device for one window. Dropping it stops and joins
/// the thread.
pub struct Worker {
    pub events: mpsc::Receiver<DndEvent>,
    commands: Sender<Command>,
    thread: Option<std::thread::JoinHandle<()>>,
    pressed: Arc<AtomicBool>,
}

impl Worker {
    /// # Safety
    ///
    /// `display` and `surface` must be valid `wl_display` and `wl_surface`
    /// pointers that outlive the worker.
    pub unsafe fn spawn(
        display: *mut c_void,
        surface: *mut c_void,
        wake: Box<dyn Fn() + Send>,
    ) -> Option<Self> {
        let connection =
            Connection::from_backend(unsafe { Backend::from_foreign_display(display.cast()) });
        let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.cast()) }.ok()?;
        let surface = WlSurface::from_id(&connection, id).ok()?;
        let (commands, receiver) = channel::channel();
        let (sender, events) = mpsc::channel();
        let replies = commands.clone();
        let pressed = Arc::new(AtomicBool::new(false));
        let worker_pressed = pressed.clone();
        let thread = std::thread::Builder::new()
            .name("limn-dnd".into())
            .spawn(move || {
                run(
                    connection,
                    surface,
                    receiver,
                    replies,
                    sender,
                    wake,
                    worker_pressed,
                )
            })
            .ok()?;
        Some(Self {
            events,
            commands,
            thread: Some(thread),
            pressed,
        })
    }

    /// Ask the thread to start a drag that offers this `text/uri-list` payload. An empty payload
    /// is ignored.
    pub fn left_pressed(&self) -> bool {
        self.pressed.load(Ordering::Relaxed)
    }
    pub fn start_drag(&self, uri_list: Vec<u8>, allow_move: bool) {
        if !uri_list.is_empty() {
            let _ = self.commands.send(Command::StartDrag(uri_list, allow_move));
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Exit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(
    connection: Connection,
    surface: WlSurface,
    commands: Channel<Command>,
    replies: Sender<Command>,
    events: mpsc::Sender<DndEvent>,
    wake: Box<dyn Fn() + Send>,
    pressed: Arc<AtomicBool>,
) {
    let Ok((globals, queue)) = registry_queue_init::<State>(&connection) else {
        return;
    };
    let Ok(mut event_loop) = EventLoop::<State>::try_new() else {
        return;
    };
    let queue_handle = queue.handle();
    let Ok(manager) = DataDeviceManagerState::bind(&globals, &queue_handle) else {
        return;
    };
    let mut state = State {
        registry: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &queue_handle),
        manager,
        seats: HashMap::new(),
        latest_seat: None,
        surface,
        queue_handle,
        drag: None,
        hovering: false,
        action: DndAction::None,
        dropped: None,
        replies,
        events,
        wake,
        exit: false,
        pressed,
    };
    let handle = event_loop.handle();
    if handle
        .insert_source(commands, |event, _, state| {
            if let channel::Event::Msg(command) = event {
                state.command(command);
            }
        })
        .is_err()
        || WaylandSource::new(connection, queue)
            .insert(handle)
            .is_err()
    {
        return;
    }
    while !state.exit && event_loop.dispatch(None, &mut state).is_ok() {}
}

#[derive(Default)]
struct Seat {
    pointer: Option<WlPointer>,
    device: Option<DataDevice>,
    /// Serial of the latest button press on our surface; a drag needs it.
    serial: u32,
}

struct State {
    registry: RegistryState,
    seat_state: SeatState,
    manager: DataDeviceManagerState,
    seats: HashMap<ObjectId, Seat>,
    latest_seat: Option<ObjectId>,
    surface: WlSurface,
    queue_handle: QueueHandle<Self>,
    /// Our outgoing drag and the URI list it serves.
    drag: Option<(DragSource, Vec<u8>)>,
    /// An acceptable drag is over our surface.
    hovering: bool,
    action: DndAction,
    /// Kept until its data arrives, then finished and destroyed.
    dropped: Option<DragOffer>,
    replies: Sender<Command>,
    events: mpsc::Sender<DndEvent>,
    wake: Box<dyn Fn() + Send>,
    exit: bool,
    pressed: Arc<AtomicBool>,
}

impl State {
    fn emit(&self, event: DndEvent) {
        let _ = self.events.send(event);
        (self.wake)();
    }

    fn offer(&self, device: &WlDataDevice) -> Option<DragOffer> {
        self.seats
            .values()
            .filter_map(|seat| seat.device.as_ref())
            .find(|candidate| candidate.inner() == device)?
            .data()
            .drag_offer()
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Exit => self.exit = true,
            Command::StartDrag(uri_list, allow_move) => {
                let seat = self.latest_seat.as_ref().and_then(|id| self.seats.get(id));
                let Some((seat, device)) =
                    seat.and_then(|seat| Some((seat, seat.device.as_ref()?)))
                else {
                    self.emit(DndEvent::DragEnded);
                    return;
                };
                let source = self.manager.create_drag_and_drop_source(
                    &self.queue_handle,
                    [MIME],
                    if allow_move {
                        DndAction::Copy | DndAction::Move
                    } else {
                        DndAction::Copy
                    },
                );
                source.start_drag(device, &self.surface, None, seat.serial);
                self.drag = Some((source, uri_list));
            }
            Command::Received { x, y, data } => {
                if let Some(offer) = self.dropped.take() {
                    // Finishing without a negotiated action is a protocol error.
                    if self.action.intersects(DndAction::Copy | DndAction::Move) {
                        offer.finish();
                    }
                    offer.destroy();
                }
                let paths = crate::uri_list::decode(&data);
                if !paths.is_empty() {
                    self.emit(DndEvent::Dropped {
                        x: x as f32,
                        y: y as f32,
                        paths,
                    });
                }
            }
        }
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        queue_handle: &QueueHandle<Self>,
        wl_seat: WlSeat,
        capability: Capability,
    ) {
        if capability != Capability::Pointer {
            return;
        }
        let seat = self.seats.entry(wl_seat.id()).or_default();
        seat.pointer = self.seat_state.get_pointer(queue_handle, &wl_seat).ok();
        if seat.device.is_none() {
            seat.device = Some(self.manager.get_data_device(queue_handle, &wl_seat));
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        wl_seat: WlSeat,
        capability: Capability,
    ) {
        if capability != Capability::Pointer {
            return;
        }
        if let Some(pointer) = self
            .seats
            .get_mut(&wl_seat.id())
            .and_then(|seat| seat.pointer.take())
        {
            if pointer.version() >= 3 {
                pointer.release();
            }
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, wl_seat: WlSeat) {
        self.seats.remove(&wl_seat.id());
    }
}

impl PointerHandler for State {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        pointer: &WlPointer,
        events: &[PointerEvent],
    ) {
        let Some(id) = pointer.data::<PointerData>().map(|data| data.seat().id()) else {
            return;
        };
        for event in events {
            match event.kind {
                PointerEventKind::Press { button: 0x110, .. } if event.surface == self.surface => {
                    self.pressed.store(true, Ordering::Relaxed)
                }
                PointerEventKind::Release { button: 0x110, .. } => {
                    self.pressed.store(false, Ordering::Relaxed)
                }
                _ => {}
            }
            if let PointerEventKind::Press { serial, .. } = event.kind {
                if event.surface == self.surface {
                    if let Some(seat) = self.seats.get_mut(&id) {
                        seat.serial = serial;
                        self.latest_seat = Some(id.clone());
                    }
                }
            }
        }
    }
}

impl DataDeviceHandler for State {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        device: &WlDataDevice,
        x: f64,
        y: f64,
        surface: &WlSurface,
    ) {
        self.hovering = false;
        self.action = DndAction::None;
        if *surface != self.surface {
            return;
        }
        let Some(offer) = self.offer(device) else {
            return;
        };
        if !offer.with_mime_types(|types| types.iter().any(|mime| mime == MIME)) {
            offer.accept_mime_type(offer.serial, None);
            return;
        }
        offer.accept_mime_type(offer.serial, Some(MIME.into()));
        // Our transfer confirmation is asynchronous. Never acknowledge an external
        // move and make its source delete files before the user has accepted.
        offer.set_actions(DndAction::Copy, DndAction::Copy);
        self.hovering = true;
        self.emit(DndEvent::Hovered {
            x: x as f32,
            y: y as f32,
        });
    }

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        if std::mem::take(&mut self.hovering) {
            self.emit(DndEvent::Left);
        }
    }

    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, x: f64, y: f64) {
        if self.hovering {
            self.emit(DndEvent::Hovered {
                x: x as f32,
                y: y as f32,
            });
        }
    }

    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}

    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, device: &WlDataDevice) {
        if !std::mem::take(&mut self.hovering) {
            return;
        }
        let Some(offer) = self.offer(device) else {
            return;
        };
        let Ok(pipe) = offer.receive(MIME.into()) else {
            offer.destroy();
            return;
        };
        let (x, y) = (offer.x, offer.y);
        self.dropped = Some(offer);
        let replies = self.replies.clone();
        // The source writes after this queue flushes, and it may be this very
        // worker when a drag returns to its own window, so never read here.
        let _ = std::thread::Builder::new()
            .name("limn-dnd-read".into())
            .spawn(move || {
                let data = crate::pipe::receive(pipe).unwrap_or_default();
                let _ = replies.send(Command::Received { x, y, data });
            });
    }
}

impl DataOfferHandler for State {
    fn source_actions(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }

    fn selected_action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        action: DndAction,
    ) {
        self.action = action;
    }
}

impl DataSourceHandler for State {
    fn accept_mime(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: Option<String>,
    ) {
    }

    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        mime: String,
        pipe: WritePipe,
    ) {
        if let Some((_, uri_list)) = self.drag.as_ref().filter(|_| mime == MIME) {
            let data = uri_list.clone();
            let _ = std::thread::Builder::new()
                .name("rust-commander-dnd-write".into())
                .spawn(move || {
                    let _ = crate::pipe::send(pipe, &data);
                });
        }
    }

    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        self.end_drag(source);
    }

    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}

    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        self.end_drag(source);
    }

    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
}

impl State {
    fn end_drag(&mut self, source: &WlDataSource) {
        if self
            .drag
            .as_ref()
            .is_some_and(|(drag, _)| drag.inner() == source)
        {
            self.drag = None;
            self.emit(DndEvent::DragEnded);
        }
    }
}

impl ProvidesRegistryState for State {
    registry_handlers![SeatState];

    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
}

delegate_seat!(State);
delegate_pointer!(State);
delegate_data_device!(State);
delegate_registry!(State);

#[cfg(test)]
mod tests {
    use super::*;
    use sctk::reexports::client::globals::GlobalListContents;
    use sctk::reexports::client::protocol::wl_compositor::WlCompositor;
    use sctk::reexports::client::protocol::wl_registry::WlRegistry;
    use sctk::reexports::client::{delegate_noop, Dispatch};
    use std::time::Duration;

    struct Host;
    delegate_noop!(Host: WlCompositor);
    delegate_noop!(Host: ignore WlSurface);
    impl Dispatch<WlRegistry, GlobalListContents> for Host {
        fn event(
            _: &mut Self,
            _: &WlRegistry,
            _: <WlRegistry as Proxy>::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    /// Plays the host window: its own queue, display and surface, with the
    /// worker attached through raw pointers exactly as from winit. Skipped
    /// where no compositor is running.
    #[test]
    #[ignore = "requires a live Wayland compositor"]
    fn worker_shares_a_live_connection_without_protocol_errors() {
        let connection = Connection::connect_to_env().expect("connect to the fixture compositor");
        let (globals, mut queue) = registry_queue_init::<Host>(&connection).unwrap();
        let handle = queue.handle();
        let compositor: WlCompositor = globals.bind(&handle, 1..=4, ()).unwrap();
        let surface = compositor.create_surface(&handle, ());
        queue.roundtrip(&mut Host).unwrap();

        let worker = unsafe {
            Worker::spawn(
                connection.backend().display_ptr().cast(),
                surface.id().as_ptr().cast(),
                Box::new(|| {}),
            )
        }
        .expect("worker attaches");
        // No button was pressed on the surface, so the drag is refused locally.
        worker.start_drag(b"file:///tmp/a\r\n".to_vec(), false);
        assert_eq!(
            worker.events.recv_timeout(Duration::from_secs(2)),
            Ok(DndEvent::DragEnded)
        );
        // Let seats, pointers and data devices bind on the worker's queue.
        std::thread::sleep(Duration::from_millis(300));
        queue.roundtrip(&mut Host).unwrap();
        drop(worker);
        queue.roundtrip(&mut Host).unwrap();
        assert!(connection.protocol_error().is_none());
    }
}
