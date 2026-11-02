//! AppKit subviews above the Slint document. The transparent host passes document
//! mouse events through and scales its bounds with Slint's logical coordinates.
use super::*;
use objc2::{
    ClassType, DefinedClass, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
    sel,
};
use objc2_app_kit::*;
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::Cell;

fn rect(r: [f32; 4]) -> NSRect {
    NSRect::new(
        NSPoint::new(r[0] as f64, r[1] as f64),
        NSSize::new(r[2] as f64, r[3] as f64),
    )
}
define_class!(
    // SAFETY: NSView's drawing and hit testing stay on the AppKit main thread.
    #[unsafe(super=NSView)] #[thread_kind=MainThreadOnly] #[name="RustCommanderViewerOverlay"]
    #[ivars=bool]
    struct Overlay;
    impl Overlay {
        #[unsafe(method(isFlipped))] fn flipped(&self)->bool {true}
        #[unsafe(method_id(hitTest:))] fn hit(&self,point:NSPoint)->Option<Retained<NSView>> {
            let hit:Option<Retained<NSView>>=unsafe{msg_send![super(self),hitTest:point]};
            // winit does not reclaim its Cocoa responder after a native field.
            if *self.ivars() && hit.as_ref().is_some_and(|view|std::ptr::eq(&**view,self.as_super()))
                && NSApplication::sharedApplication(self.mtm()).currentEvent().is_some_and(|event|matches!(event.r#type(),NSEventType::LeftMouseDown|NSEventType::RightMouseDown))
                && let Some(parent)=unsafe{self.superview()} && let Some(window)=parent.window(){window.makeFirstResponder(Some(&parent));}
            hit.filter(|view|!(*self.ivars()&&std::ptr::eq(&**view,self.as_super())))
        }
    }
);
impl Overlay {
    fn new(mtm: MainThreadMarker, pass: bool) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(pass);
        unsafe { msg_send![super(this),initWithFrame:rect([0.0;4])] }
    }
}
enum Widget {
    Button(Retained<NSButton>),
    Text(Retained<NSTextField>),
    Choice(Retained<NSPopUpButton>),
    Modes(Retained<NSSegmentedControl>),
}
impl Widget {
    fn control(&self) -> &NSControl {
        match self {
            Self::Button(w) => w,
            Self::Text(w) => w,
            Self::Choice(w) => w,
            Self::Modes(w) => w,
        }
    }
    fn view(&self) -> &NSView {
        self.control()
    }
    fn value(&self, spec: &Control) -> String {
        match self {
            Self::Text(w) => w.stringValue().to_string(),
            Self::Choice(w) => spec
                .choices
                .get(w.indexOfSelectedItem() as usize)
                .cloned()
                .unwrap_or_default(),
            Self::Modes(w) => spec
                .choices
                .get(w.selectedSegment() as usize)
                .cloned()
                .unwrap_or_default(),
            _ => String::new(),
        }
    }
}
struct Item {
    spec: Control,
    widget: Widget,
    tool: bool,
}
struct TargetState {
    document: u64,
    events: Events,
    items: Rc<RefCell<Vec<Item>>>,
    popup_number: Cell<isize>,
}
define_class!(
    // SAFETY: NSObject has no subclassing requirements; callbacks only enqueue.
    #[unsafe(super=NSObject)] #[thread_kind=MainThreadOnly] #[name="RustCommanderViewerTarget"]
    #[ivars=TargetState] struct Target;
    unsafe impl NSObjectProtocol for Target {}
    unsafe impl NSControlTextEditingDelegate for Target {
        #[unsafe(method(controlTextDidChange:))] fn edited(&self,note:&NSNotification) {
            if let Some(control)=note.object().and_then(|o|o.downcast::<NSControl>().ok()){self.collect_field(&control);}
        }
    }
    unsafe impl NSTextFieldDelegate for Target {}
    unsafe impl NSPopoverDelegate for Target {
        #[unsafe(method(popoverDidClose:))] fn popup_closed(&self,_:&NSNotification){self.send(Action::Activate("tools-closed".into()));self.ivars().popup_number.set(-1);}
    }
    impl Target {
        #[unsafe(method(changed:))] fn changed(&self,sender:&NSControl) {
            self.collect_field(sender);
        }
        #[unsafe(method(perform:))] fn perform(&self,sender:&NSControl) {
            let items=self.ivars().items.borrow();
            for item in items.iter().filter(|i|matches!(i.spec.kind,Kind::Field|Kind::Search)){self.send(Action::Change(item.spec.id.clone(),item.widget.value(&item.spec)));}
            if let Some(item)=items.iter().find(|i|std::ptr::eq(i.widget.control(),sender)){
                let id=match item.spec.id.as_str(){"query"=>"find-next","hex"=>"hex-find","offset"=>"jump","page"=>"page-go",id=>id};self.send(Action::Activate(id.into()));
            }
        }
    }
);
impl Target {
    fn collect_field(&self, sender: &NSControl) {
        if let Some(item) = self
            .ivars()
            .items
            .borrow()
            .iter()
            .find(|i| std::ptr::eq(i.widget.control(), sender))
        {
            self.send(Action::Change(
                item.spec.id.clone(),
                item.widget.value(&item.spec),
            ));
        }
    }
    fn new(mtm: MainThreadMarker, document: u64, events: Events) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetState {
            document,
            events,
            items: Default::default(),
            popup_number: Cell::new(-1),
        });
        unsafe { msg_send![super(this), init] }
    }
    fn send(&self, action: Action) {
        self.ivars().events.borrow_mut().push(Event {
            document: self.ivars().document,
            action,
        });
    }
}
pub(super) struct Platform {
    parent: Retained<NSView>,
    overlay: Retained<Overlay>,
    target: Retained<Target>,
    popup: Retained<NSPopover>,
    body: Retained<Overlay>,
    _controller: Retained<NSViewController>,
    monitor: Option<Retained<AnyObject>>,
    shortcuts: Rc<RefCell<Vec<crate::preferences::Shortcut>>>,
    find_focus: i32,
    document_focus: i32,
    native_focused: bool,
}
impl Platform {
    pub fn new(ui: &CommanderWindow, document: u64, events: Events) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let handle = ui.window().window_handle();
        let handle = handle.window_handle().ok()?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return None;
        };
        // SAFETY: winit owns a live NSView on this thread. Retaining it keeps the
        // parent alive until all viewer subviews and the event monitor detach.
        let parent = unsafe { Retained::retain(handle.ns_view.cast::<NSView>().as_ptr()) }?;
        let window = parent.window()?;
        let overlay = Overlay::new(mtm, true);
        parent.addSubview(&overlay);
        let target = Target::new(mtm, document, events);
        let body = Overlay::new(mtm, false);
        let controller = NSViewController::new(mtm);
        controller.setView(&body);
        let popup = NSPopover::new(mtm);
        popup.setContentViewController(Some(&controller));
        popup.setBehavior(NSPopoverBehavior::Transient);
        popup.setAnimates(false);
        popup.setDelegate(Some(ProtocolObject::from_ref(&*target)));
        let shortcuts = Rc::new(RefCell::new(vec![]));
        let keys = shortcuts.clone();
        let tab_ui = ui.as_weak();
        let native_parent = parent.clone();
        let dispatch = target.clone();
        let number = window.windowNumber();
        let monitor = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(
                NSEventMask::KeyDown,
                &block2::RcBlock::new(move |event: std::ptr::NonNull<NSEvent>| {
                    let event = event.as_ref();
                    let event_window = event.windowNumber();
                    if event_window != number && event_window != dispatch.ivars().popup_number.get()
                    {
                        return event as *const NSEvent as *mut NSEvent;
                    }
                    let mut key = slint::language::KeyEvent::default();
                    let flags = event.modifierFlags();
                    key.modifiers.control = flags.contains(NSEventModifierFlags::Command);
                    key.modifiers.shift = flags.contains(NSEventModifierFlags::Shift);
                    key.modifiers.alt = flags.contains(NSEventModifierFlags::Option);
                    key.text = if event.keyCode() == 53 {
                        slint::platform::Key::Escape.into()
                    } else {
                        event
                            .charactersIgnoringModifiers()
                            .map(|s| s.to_string())
                            .unwrap_or_default()
                            .into()
                    };
                    if event.keyCode() == 48
                        && event_window == number
                        && !key.modifiers.control
                        && !key.modifiers.alt
                        && tab_ui.upgrade().is_some_and(|ui| {
                            !ui.get_viewer_editable() && ui.get_viewer_document_focused()
                        })
                        && !intercept(&key, &keys.borrow())
                        && let Some(window) = native_parent.window()
                        && window
                            .firstResponder()
                            .is_some_and(|r| std::ptr::eq(&*r, &*native_parent as &NSResponder))
                    {
                        let items = dispatch.ivars().items.borrow();
                        let mut controls = items
                            .iter()
                            .filter(|i| !i.tool && i.spec.enabled && i.spec.kind != Kind::Label)
                            .collect::<Vec<_>>();
                        if key.modifiers.shift {
                            controls.reverse();
                        }
                        for control in controls {
                            if control.widget.view().canBecomeKeyView()
                                && window.makeFirstResponder(Some(control.widget.view()))
                            {
                                return std::ptr::null_mut();
                            }
                        }
                    }
                    if intercept(&key, &keys.borrow()) {
                        dispatch.send(Action::Key(key));
                        std::ptr::null_mut()
                    } else {
                        event as *const NSEvent as *mut NSEvent
                    }
                }),
            )
        };
        Some(Self {
            parent,
            overlay,
            target,
            popup,
            body,
            _controller: controller,
            monitor,
            shortcuts,
            find_focus: ui.get_viewer_native_find_focus(),
            document_focus: ui.get_viewer_native_document_focus(),
            native_focused: false,
        })
    }
    fn reconcile(&self, specs: &[Control], tool: bool) {
        let mut items = self.target.ivars().items.borrow_mut();
        items.retain(|item| {
            let keep = item.tool != tool
                || specs
                    .iter()
                    .any(|s| s.id == item.spec.id && s.kind == item.spec.kind);
            if !keep {
                item.widget.view().removeFromSuperview();
            }
            keep
        });
        let mtm = self.target.mtm();
        let parent: &NSView = if tool { &self.body } else { &self.overlay };
        for spec in specs {
            if !items.iter().any(|i| i.spec.id == spec.id) {
                let widget = match spec.kind {
                    Kind::Button => {
                        let button = unsafe {
                            NSButton::buttonWithTitle_target_action(
                                &NSString::from_str(&spec.label),
                                Some(&self.target),
                                Some(sel!(perform:)),
                                mtm,
                            )
                        };
                        button.setBezelStyle(NSBezelStyle::Push);
                        Widget::Button(button)
                    }
                    Kind::Modes => {
                        let control = NSSegmentedControl::initWithFrame(
                            NSSegmentedControl::alloc(mtm),
                            rect(spec.frame),
                        );
                        control.setTrackingMode(NSSegmentSwitchTracking::SelectOne);
                        unsafe {
                            control.setTarget(Some(&self.target));
                            control.setAction(Some(sel!(changed:)));
                        }
                        Widget::Modes(control)
                    }
                    Kind::Choice => {
                        let control = NSPopUpButton::initWithFrame_pullsDown(
                            NSPopUpButton::alloc(mtm),
                            rect(spec.frame),
                            false,
                        );
                        unsafe {
                            control.setTarget(Some(&self.target));
                            control.setAction(Some(sel!(changed:)));
                        }
                        Widget::Choice(control)
                    }
                    Kind::Search | Kind::Field => {
                        let field = if spec.kind == Kind::Search {
                            let field = NSSearchField::initWithFrame(
                                NSSearchField::alloc(mtm),
                                rect(spec.frame),
                            );
                            field.setSendsWholeSearchString(true);
                            field.setSendsSearchStringImmediately(false);
                            field.into_super()
                        } else {
                            NSTextField::initWithFrame(NSTextField::alloc(mtm), rect(spec.frame))
                        };
                        field.setBezeled(true);
                        field.setBezelStyle(NSTextFieldBezelStyle::RoundedBezel);
                        field.setEditable(true);
                        field.setSelectable(true);
                        unsafe {
                            field.setDelegate(Some(ProtocolObject::from_ref(&*self.target)));
                        }
                        unsafe {
                            field.setTarget(Some(&self.target));
                            field.setAction(Some(sel!(perform:)));
                        }
                        Widget::Text(field)
                    }
                    Kind::Label => Widget::Text(NSTextField::labelWithString(
                        &NSString::from_str(&spec.label),
                        mtm,
                    )),
                };
                parent.addSubview(widget.view());
                items.push(Item {
                    spec: spec.clone(),
                    widget,
                    tool,
                });
            }
            let item = items.iter_mut().find(|i| i.spec.id == spec.id).unwrap();
            item.widget
                .view()
                .setAccessibilityLabel(Some(&NSString::from_str(match spec.id.as_str() {
                    "tools" => "More viewer tools",
                    "previous" => "Previous page",
                    "next" => "Next page",
                    _ => &spec.label,
                })));
            item.widget.view().setFrame(rect(spec.frame));
            item.widget.control().setEnabled(spec.enabled);
            match &item.widget {
                Widget::Button(w) => w.setTitle(&NSString::from_str(&spec.label)),
                Widget::Text(w) => {
                    if spec.kind == Kind::Label {
                        w.setStringValue(&NSString::from_str(&spec.label));
                        w.setFont(Some(&NSFont::systemFontOfSize(12.0)));
                    } else {
                        w.setPlaceholderString(Some(&NSString::from_str(&spec.label)));
                        if w.stringValue().to_string() != spec.value {
                            w.setStringValue(&NSString::from_str(&spec.value));
                        }
                    }
                }
                Widget::Choice(w) => {
                    if item.spec.choices != spec.choices || w.numberOfItems() == 0 {
                        w.removeAllItems();
                        for choice in &spec.choices {
                            w.addItemWithTitle(&NSString::from_str(choice));
                        }
                    }
                    if let Some(index) = spec.choices.iter().position(|s| s == &spec.value) {
                        w.selectItemAtIndex(index as isize);
                    }
                }
                Widget::Modes(w) => {
                    w.setSegmentCount(spec.choices.len() as isize);
                    for (i, choice) in spec.choices.iter().enumerate() {
                        w.setLabel_forSegment(
                            &NSString::from_str(match choice.as_str() {
                                "Plain original" => "Original",
                                "Highlighted original" => "Highlighted",
                                _ => "Pretty",
                            }),
                            i as isize,
                        );
                        w.setWidth_forSegment(
                            spec.frame[2] as f64 / spec.choices.len() as f64,
                            i as isize,
                        );
                    }
                    if let Some(index) = spec.choices.iter().position(|s| s == &spec.value) {
                        w.setSelectedSegment(index as isize);
                    }
                }
            }
            item.spec = spec.clone();
        }
        let visible = items
            .iter()
            .filter(|i| i.tool == tool && !matches!(i.spec.kind, Kind::Label))
            .collect::<Vec<_>>();
        for pair in visible.windows(2) {
            unsafe {
                pair[0]
                    .widget
                    .view()
                    .setNextKeyView(Some(pair[1].widget.view()));
            }
        }
        if let Some(last) = visible.last() {
            unsafe {
                last.widget.view().setNextKeyView(Some(&self.parent));
            }
        }
    }
    pub fn sync(
        &mut self,
        ui: &CommanderWindow,
        controls: &[Control],
        tools: &[Control],
        height: f32,
        dark: bool,
        shortcuts: &[crate::preferences::Shortcut],
    ) {
        *self.shortcuts.borrow_mut() = shortcuts.to_vec();
        self.overlay.setFrame(self.parent.bounds());
        self.overlay.setBounds(rect([
            0.0,
            0.0,
            ui.get_viewer_window_width(),
            ui.get_viewer_window_height(),
        ]));
        let appearance = NSAppearance::appearanceNamed(if dark {
            unsafe { NSAppearanceNameDarkAqua }
        } else {
            unsafe { NSAppearanceNameAqua }
        });
        self.overlay.setAppearance(appearance.as_deref());
        self.body.setAppearance(appearance.as_deref());
        self.reconcile(controls, false);
        if ui.get_viewer_native_tools_open() {
            self.body.setFrame(rect([0.0, 0.0, 352.0, height]));
            self.reconcile(tools, true);
            self.popup.setContentSize(NSSize::new(352.0, height as f64));
            if !self.popup.isShown()
                && let Some(button) = self
                    .target
                    .ivars()
                    .items
                    .borrow()
                    .iter()
                    .find(|i| i.spec.id == "tools")
            {
                self.popup.showRelativeToRect_ofView_preferredEdge(
                    button.widget.view().bounds(),
                    button.widget.view(),
                    objc2_foundation::NSRectEdge::MinY,
                );
                self.target
                    .ivars()
                    .popup_number
                    .set(self.body.window().map(|w| w.windowNumber()).unwrap_or(-1));
            }
        } else if self.popup.isShown() {
            self.popup.close();
        }
        if self.find_focus != ui.get_viewer_native_find_focus() {
            self.find_focus = ui.get_viewer_native_find_focus();
            if let Some(item) = self
                .target
                .ivars()
                .items
                .borrow()
                .iter()
                .find(|i| i.spec.id == "query")
                && let Widget::Text(field) = &item.widget
            {
                if let Some(window) = field.window() {
                    window.makeFirstResponder(Some(field));
                    unsafe {
                        field.selectText(None);
                    }
                }
            }
        }
        if self.document_focus != ui.get_viewer_native_document_focus() {
            self.document_focus = ui.get_viewer_native_document_focus();
            if let Some(window) = self.parent.window() {
                window.makeFirstResponder(Some(&self.parent));
            }
        }
        let native_focused = self.popup.isShown()
            || self
                .parent
                .window()
                .and_then(|w| w.firstResponder())
                .is_some_and(|r| !std::ptr::eq(&*r, &*self.parent as &NSResponder));
        if native_focused {
            ui.invoke_focus_viewer_chrome();
        } else if self.native_focused {
            ui.invoke_restore_viewer_focus();
        }
        self.native_focused = native_focused;
    }
    #[cfg(feature = "screenshot")]
    pub fn smoke(&self, id: &str, value: &str) {
        if id == "assert-document-focus" {
            let window = self.parent.window().unwrap();
            assert!(
                window
                    .firstResponder()
                    .is_some_and(|r| std::ptr::eq(&*r, &*self.parent as &NSResponder))
            );
            return;
        }
        if id == "assert-query-focus" {
            let items = self.target.ivars().items.borrow();
            let item = items.iter().find(|i| i.spec.id == "query").unwrap();
            let Widget::Text(field) = &item.widget else {
                panic!("Search must be a text field");
            };
            let responder = self.parent.window().unwrap().firstResponder().unwrap();
            assert!(
                std::ptr::eq(&*responder, &**field as &NSResponder)
                    || field
                        .currentEditor()
                        .is_some_and(|editor| std::ptr::eq(&*responder, &*editor as &NSResponder))
            );
            return;
        }
        if id == "document-click" {
            let y = self
                .target
                .ivars()
                .items
                .borrow()
                .iter()
                .find(|i| i.spec.id == "query")
                .map_or(110.0, |i| i.spec.frame[1] + i.spec.frame[3] + 30.0);
            let point = self
                .overlay
                .convertPoint_toView(NSPoint::new(80.0, y as f64), None);
            let window = self.parent.window().unwrap();
            for kind in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
                let event=NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(kind,point,NSEventModifierFlags::empty(),0.0,window.windowNumber(),None,0,1,1.0).unwrap();
                NSApplication::sharedApplication(self.target.mtm())
                    .postEvent_atStart(&event, false);
            }
            return;
        }
        if id == "key" {
            let (chars, code, flags) = match value {
                "Escape" => ("\u{1b}", 53, NSEventModifierFlags::empty()),
                "Return" => ("\r", 36, NSEventModifierFlags::empty()),
                "X" => ("x", 7, NSEventModifierFlags::empty()),
                _ => ("f", 3, NSEventModifierFlags::Command),
            };
            let chars = NSString::from_str(chars);
            let window = if self.popup.isShown() {
                self.body.window()
            } else {
                self.parent.window()
            }
            .unwrap();
            let event=NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(NSEventType::KeyDown,NSPoint::new(0.0,0.0),flags,0.0,window.windowNumber(),None,&chars,&chars,false,code).unwrap();
            NSApplication::sharedApplication(self.target.mtm()).postEvent_atStart(&event, false);
            return;
        }
        let items = self.target.ivars().items.borrow();
        let item = items
            .iter()
            .find(|i| i.spec.id == id)
            .expect("Native viewer control must exist");
        match &item.widget {
            Widget::Button(w) => unsafe { w.performClick(None) },
            Widget::Modes(w) => {
                let index = item.spec.choices.iter().position(|s| s == value).unwrap();
                w.setSelectedSegment(index as isize);
                self.target.collect_field(w);
            }
            Widget::Text(w) => {
                w.setStringValue(&NSString::from_str(value));
                if let Some(editor) = w.currentEditor() {
                    editor.setString(&NSString::from_str(value));
                }
                self.target.collect_field(w);
            }
            Widget::Choice(w) => {
                w.selectItemAtIndex(
                    item.spec.choices.iter().position(|s| s == value).unwrap() as isize
                );
                self.target.collect_field(w);
            }
        }
    }
    #[cfg(feature = "screenshot")]
    pub fn snapshot(&self, ui: &CommanderWindow, path: &Path) -> bool {
        use objc2_foundation::NSDictionary;
        fn capture(view: &NSView) -> Option<::image::RgbaImage> {
            view.displayIfNeeded();
            let bitmap = view.bitmapImageRepForCachingDisplayInRect(view.bounds())?;
            view.cacheDisplayInRect_toBitmapImageRep(view.bounds(), &bitmap);
            let data = unsafe {
                bitmap.representationUsingType_properties(
                    NSBitmapImageFileType::PNG,
                    &NSDictionary::new(),
                )
            }?;
            Some(::image::load_from_memory(&data.to_vec()).ok()?.into_rgba8())
        }
        let Ok(buffer) = ui.window().take_snapshot() else {
            return false;
        };
        let Some(mut image) = ::image::RgbaImage::from_raw(
            buffer.width(),
            buffer.height(),
            buffer.as_bytes().to_vec(),
        ) else {
            return false;
        };
        let Some(native) = capture(&self.overlay) else {
            return false;
        };
        eprintln!(
            "Native viewer snapshot: {} controls, {}x{}",
            self.target.ivars().items.borrow().len(),
            image.width(),
            image.height()
        );
        let native = ::image::imageops::resize(
            &native,
            image.width(),
            image.height(),
            ::image::imageops::FilterType::Lanczos3,
        );
        ::image::imageops::overlay(&mut image, &native, 0, 0);
        if self.popup.isShown()
            && let (Some(popup), Some(owner), Some(body)) = (
                self.body.window(),
                self.parent.window(),
                capture(&self.body),
            )
        {
            let frame =
                popup.convertRectToScreen(self.body.convertRect_toView(self.body.bounds(), None));
            let parent = owner
                .convertRectToScreen(self.parent.convertRect_toView(self.parent.bounds(), None));
            let scale = image.width() as f64 / parent.size.width;
            let body = ::image::imageops::resize(
                &body,
                (frame.size.width * scale).round() as u32,
                (frame.size.height * scale).round() as u32,
                ::image::imageops::FilterType::Lanczos3,
            );
            ::image::imageops::overlay(
                &mut image,
                &body,
                ((frame.origin.x - parent.origin.x) * scale).round() as i64,
                ((parent.origin.y + parent.size.height - frame.origin.y - frame.size.height)
                    * scale)
                    .round() as i64,
            );
        }
        image.save(path).is_ok()
    }
}
impl Drop for Platform {
    fn drop(&mut self) {
        if let Some(monitor) = self.monitor.take() {
            unsafe {
                NSEvent::removeMonitor(&monitor);
            }
        }
        self.popup.close();
        if let Some(window) = self.parent.window() {
            window.makeFirstResponder(Some(&self.parent));
        }
        self.overlay.removeFromSuperview();
    }
}
