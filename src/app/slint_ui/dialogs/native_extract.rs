//! An asynchronous AppKit sheet. Native callbacks only enqueue controller events;
//! they never borrow the adapter or run file operations inside Cocoa dispatch.
use super::{Dialog, Field};
use objc2::{
    DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained, runtime::ProtocolObject,
    sel,
};
use objc2_app_kit::{
    NSAccessibility, NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSBackingStoreType, NSBezelStyle, NSButton, NSButtonCell, NSButtonType, NSColor, NSControl,
    NSControlTextEditingDelegate, NSFont, NSPopUpButton, NSScrollView, NSSecureTextField,
    NSTextDelegate, NSTextField, NSTextFieldBezelStyle, NSTextFieldDelegate, NSTextView,
    NSTextViewDelegate, NSView, NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{cell::RefCell, rc::Rc};

type Events = Rc<RefCell<Vec<Event>>>;
/// An early Return may wait for preflight, but editing any operation input
/// invalidates that intent. It cannot approve a later, different extraction.
#[derive(PartialEq)]
pub(in super::super) struct Intent {
    identity: String,
    destination: String,
    resumable: bool,
    options: crate::transfer::TransferOptions,
    overrides: std::collections::BTreeMap<std::path::PathBuf, crate::transfer::ConflictPolicy>,
}
impl Intent {
    pub fn capture(adapter: &super::super::Adapter) -> Option<Self> {
        let prompt = adapter.app.prompt.as_ref()?;
        use crate::app::PromptKind;
        let identity = match &prompt.kind {
            PromptKind::Extract {
                archive,
                selected,
                prefix,
                base,
            } => serde_json::to_string(&("extract", archive, selected, prefix, base)),
            PromptKind::Transfer {
                sources,
                base,
                moving,
            } => serde_json::to_string(&("transfer", sources, base, moving)),
            PromptKind::RemoteTransfer {
                from,
                entries,
                to,
                moving,
            } => serde_json::to_string(&("remote-transfer", from, entries, to, moving)),
            PromptKind::DropTransfer {
                selection,
                to,
                moving,
            } => serde_json::to_string(&("drop-transfer", selection, to, moving)),
            _ => return None,
        };
        Some(Self {
            identity: identity.ok()?,
            destination: prompt.value.clone(),
            resumable: prompt.resumable,
            options: prompt.transfer_options,
            overrides: prompt.overrides.clone(),
        })
    }
}
pub(in super::super) struct Request {
    pub ui: slint::Weak<super::super::CommanderWindow>,
    pub spec: Option<Dialog>,
    pub scheme: crate::appearance::ColorScheme,
}
pub(in super::super) enum Event {
    Field(String, String),
    Action(String),
}

enum Widget {
    Text(Retained<NSTextField>),
    Choice(Retained<NSPopUpButton>, Vec<String>),
    Check(Retained<NSButton>),
    Multiline(Retained<NSTextView>, Retained<NSScrollView>),
}
impl Widget {
    fn control(&self) -> Option<&NSControl> {
        match self {
            Self::Text(w) => Some(w),
            Self::Choice(w, _) => Some(w),
            Self::Check(w) => Some(w),
            Self::Multiline(..) => None,
        }
    }
    fn view(&self) -> &NSView {
        match self {
            Self::Text(w) => w,
            Self::Choice(w, _) => w,
            Self::Check(w) => w,
            Self::Multiline(_, scroll) => scroll,
        }
    }
    fn value(&self) -> String {
        match self {
            Self::Text(w) => w.stringValue().to_string(),
            Self::Choice(w, choices) => usize::try_from(w.indexOfSelectedItem())
                .ok()
                .and_then(|i| choices.get(i))
                .cloned()
                .unwrap_or_default(),
            Self::Check(w) => (w.state() != 0).to_string(),
            Self::Multiline(w, _) => w.string().to_string(),
        }
    }
    fn update(&self, value: &str) {
        if self.value() == value {
            return;
        }
        match self {
            Self::Text(w) => w.setStringValue(&NSString::from_str(value)),
            Self::Choice(w, choices) => {
                if let Some(i) = choices.iter().position(|c| c == value) {
                    w.selectItemAtIndex(i as isize);
                }
            }
            Self::Check(w) => w.setState(isize::from(value == "true")),
            Self::Multiline(w, _) => w.setString(&NSString::from_str(value)),
        }
    }
}
#[derive(Default)]
struct TargetIvars {
    events: Events,
    fields: RefCell<Vec<(String, Widget)>>,
    actions: RefCell<Vec<String>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements. All objects and callbacks
    // are confined to the AppKit main thread, and Rust owns their lifetimes.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "RustCommanderExtractTarget"]
    #[ivars = TargetIvars]
    struct Target;
    unsafe impl NSObjectProtocol for Target {}
    unsafe impl NSControlTextEditingDelegate for Target {
        #[unsafe(method(controlTextDidChange:))]
        fn text_changed(&self, notification: &NSNotification) {
            if let Some(control) = notification.object().and_then(|object| object.downcast::<NSControl>().ok()) {
                self.collect_field(&control);
            }
        }
    }
    unsafe impl NSTextFieldDelegate for Target {}
    unsafe impl NSTextDelegate for Target {
        #[unsafe(method(textDidChange:))]
        fn multiline_changed(&self, notification:&NSNotification) {
            if let Some(text)=notification.object().and_then(|object|object.downcast::<NSTextView>().ok())
                && let Some((id,widget))=self.ivars().fields.borrow().iter().find(|(_,widget)|matches!(widget,Widget::Multiline(view,_) if std::ptr::eq(&**view,&*text))) {
                self.ivars().events.borrow_mut().push(Event::Field(id.clone(),widget.value()));
            }
        }
    }
    unsafe impl NSTextViewDelegate for Target {}
    unsafe impl NSWindowDelegate for Target {
        #[unsafe(method(windowShouldClose:))]
        fn should_close(&self, _window: &NSWindow) -> bool {
            self.ivars().events.borrow_mut().push(Event::Action("cancel".into()));
            false
        }
    }
    impl Target {
        #[unsafe(method(changed:))]
        fn changed(&self, sender: &NSControl) { self.collect_field(sender); }
        #[unsafe(method(perform:))]
        fn perform(&self, sender: &NSControl) {
            // Commit the field editor's current destination before the action.
            // Never materialize untouched per-file choices as overrides.
            if let Some((id, widget)) = self.ivars().fields.borrow().first() {
                self.ivars().events.borrow_mut().push(Event::Field(id.clone(), widget.value()));
            }
            if let Some(id) = self.ivars().actions.borrow().get(sender.tag() as usize) {
                self.ivars().events.borrow_mut().push(Event::Action(id.clone()));
            }
        }
    }
);
impl Target {
    fn new(mtm: MainThreadMarker, events: Events) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars {
            events,
            ..Default::default()
        });
        // SAFETY: NSObject's initializer has this signature.
        unsafe { msg_send![super(this), init] }
    }
    fn collect_field(&self, sender: &NSControl) {
        if let Some((id, widget)) = self.ivars().fields.borrow().iter().find(|(_, widget)| {
            widget
                .control()
                .is_some_and(|control| std::ptr::eq(control, sender))
        }) {
            self.ivars()
                .events
                .borrow_mut()
                .push(Event::Field(id.clone(), widget.value()));
        }
    }
}

#[derive(Default)]
pub(in super::super) struct Host {
    sheet: Option<Sheet>,
}
impl Host {
    pub fn available(ui: &super::super::CommanderWindow) -> bool {
        use slint::ComponentHandle;
        if MainThreadMarker::new().is_none() {
            return false;
        }
        let window_handle = ui.window().window_handle();
        let Ok(handle) = window_handle.window_handle() else {
            return false;
        };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return false;
        };
        // SAFETY: winit owns this view for the duration of this UI callback.
        unsafe { handle.ns_view.cast::<NSView>().as_ref() }
            .window()
            .is_some()
    }
    pub fn active(&self) -> bool {
        self.sheet.is_some()
    }
    pub fn focused(&self) -> bool {
        self.sheet
            .as_ref()
            .is_some_and(|sheet| sheet.window.isKeyWindow())
    }
    pub fn events(&self) -> Vec<Event> {
        self.sheet
            .as_ref()
            .map(|s| std::mem::take(&mut *s.target.ivars().events.borrow_mut()))
            .unwrap_or_default()
    }
    #[cfg(feature = "screenshot")]
    pub fn smoke(&self, command: &str, value: &str) {
        let sheet = self
            .sheet
            .as_ref()
            .expect("Native Extract sheet must be open");
        match command {
            "native-field" => {
                let (id, value): (String, String) = serde_json::from_str(value).unwrap();
                let fields = sheet.target.ivars().fields.borrow();
                let (_, widget) = fields
                    .iter()
                    .find(|(key, _)| key == &id)
                    .expect("Native field must exist");
                widget.update(&value);
                sheet
                    .target
                    .ivars()
                    .events
                    .borrow_mut()
                    .push(Event::Field(id, widget.value()));
            }
            "native-action" if value == "close" => sheet.window.performClose(None),
            "native-action" if value == "return" => {
                let fields = sheet.target.ivars().fields.borrow();
                let control = fields.first().unwrap().1.control().unwrap();
                unsafe {
                    control.sendAction_to(control.action(), control.target().as_deref());
                }
            }
            "native-action" => {
                let (_, button) = sheet
                    .buttons
                    .iter()
                    .find(|(id, _)| id == value)
                    .expect("Native button must exist");
                assert!(button.isEnabled(), "Native action must be enabled");
                unsafe {
                    // Dispatch Cocoa's target/action without performClick's nested
                    // animation loop inside the Slint screenshot timer.
                    button.sendAction_to(button.action(), button.target().as_deref());
                }
            }
            "native-check" => {
                assert_eq!(
                    sheet.parent.attachedSheet().as_deref(),
                    Some(&*sheet.window)
                );
                assert!(
                    sheet.error.stringValue().to_string().contains(value),
                    "Expected native error {value:?}, got {}",
                    sheet.error.stringValue()
                );
                assert!(
                    sheet.window.defaultButtonCell().is_some(),
                    "Native sheet must have a default action"
                );
                assert_eq!(
                    sheet
                        .buttons
                        .iter()
                        .find(|(id, _)| id == "cancel")
                        .unwrap()
                        .1
                        .keyEquivalent()
                        .to_string(),
                    "\u{1b}"
                );
            }
            _ => panic!("Unknown native smoke command"),
        }
    }
    #[cfg(feature = "screenshot")]
    pub fn snapshot(&self, path: &std::path::Path) -> bool {
        use objc2_app_kit::NSBitmapImageFileType;
        use objc2_foundation::NSDictionary;
        let Some(sheet) = &self.sheet else {
            return false;
        };
        let view = sheet.window.contentView().unwrap();
        view.displayIfNeeded();
        let bitmap = view
            .bitmapImageRepForCachingDisplayInRect(view.bounds())
            .unwrap();
        view.cacheDisplayInRect_toBitmapImageRep(view.bounds(), &bitmap);
        let png = unsafe {
            bitmap.representationUsingType_properties(
                NSBitmapImageFileType::PNG,
                &NSDictionary::new(),
            )
        }
        .unwrap();
        std::fs::write(path, png.to_vec()).expect("Save native sheet image");
        true
    }
    /// False means there is no usable AppKit parent (for example the test backend).
    /// Keep the existing Slint form in that case.
    pub fn sync(
        &mut self,
        ui: &super::super::CommanderWindow,
        spec: Option<&Dialog>,
        scheme: crate::appearance::ColorScheme,
    ) -> bool {
        use slint::ComponentHandle;
        let Some(spec) = spec else {
            self.sheet = None;
            return false;
        };
        if self.sheet.as_ref().is_some_and(|sheet| {
            sheet.kind != spec.kind
                || sheet.title != spec.title
                || sheet
                    .buttons
                    .iter()
                    .map(|(id, _)| id)
                    .collect::<std::collections::BTreeSet<_>>()
                    != spec
                        .buttons
                        .iter()
                        .map(|(id, _, _)| id)
                        .filter(|id| matches!(id.as_str(), "accept" | "cancel" | "apply-all"))
                        .collect::<std::collections::BTreeSet<_>>()
        }) {
            self.sheet = None;
        }
        if self.sheet.is_none() {
            let Some(mtm) = MainThreadMarker::new() else {
                return false;
            };
            let window_handle = ui.window().window_handle();
            let Ok(handle) = window_handle.window_handle() else {
                return false;
            };
            let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
                return false;
            };
            // SAFETY: winit retains the view while this UI-thread callback runs.
            let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
            let Some(parent) = view.window() else {
                return false;
            };
            // Don't replace an unrelated native sheet owned by another integration.
            if parent.attachedSheet().is_some() {
                return false;
            }
            self.sheet = Some(Sheet::new(mtm, parent, spec));
        }
        let sheet = self.sheet.as_mut().unwrap();
        sheet.update(spec, scheme);
        true
    }
}

struct Sheet {
    kind: String,
    title: String,
    parent: Retained<NSWindow>,
    window: Retained<NSWindow>,
    target: Retained<Target>,
    body: Retained<NSScrollView>,
    error: Retained<NSTextField>,
    buttons: Vec<(String, Retained<NSButton>)>,
    schema: Vec<(String, String, i32, Vec<String>)>,
}
fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}
fn label(mtm: MainThreadMarker, value: &str, frame: NSRect) -> Retained<NSTextField> {
    let label = NSTextField::wrappingLabelWithString(&NSString::from_str(value), mtm);
    label.setFrame(frame);
    label.setFont(Some(&NSFont::systemFontOfSize(13.0)));
    label
}
fn translated(value: &str) -> Retained<NSString> {
    NSString::from_str(&crate::app::tr(value))
}
fn field_height(field: &Field) -> f64 {
    match field.kind {
        2 => 34.0,
        4 => 124.0,
        8 => 34.0,
        9 => 28.0,
        6 => 68.0,
        _ => 62.0 + (field.title.chars().count() / 80).min(3) as f64 * 16.0,
    }
}
impl Sheet {
    fn new(mtm: MainThreadMarker, parent: Retained<NSWindow>, spec: &Dialog) -> Self {
        // SAFETY: The window is retained below and auto-release on close is disabled.
        let window = unsafe {
            let w = NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect(0.0, 0.0, 620.0, 360.0),
                NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
                NSBackingStoreType::Buffered,
                false,
            );
            w.setReleasedWhenClosed(false);
            w
        };
        window.setTitle(&NSString::from_str(&spec.title));
        let target = Target::new(mtm, Default::default());
        window.setDelegate(Some(ProtocolObject::from_ref(&*target)));
        let content = window.contentView().unwrap();
        let body =
            NSScrollView::initWithFrame(NSScrollView::alloc(mtm), rect(16.0, 100.0, 588.0, 244.0));
        body.setHasVerticalScroller(true);
        body.setAutohidesScrollers(true);
        body.setDrawsBackground(false);
        let error = label(mtm, "", rect(24.0, 52.0, 572.0, 42.0));
        error.setTextColor(Some(&NSColor::systemRedColor()));
        // SAFETY: Native subviews are retained by the content view.
        content.addSubview(&body);
        content.addSubview(&error);
        let mut buttons = Vec::new();
        let mut x = 596.0;
        // Cocoa places the default action on the trailing edge, Cancel next to it.
        for id in ["accept", "cancel", "apply-all"] {
            if let Some((_, title, _)) = spec.buttons.iter().find(|(key, _, _)| key == id) {
                let width = (title.chars().count() as f64 * 8.0 + 28.0).max(84.0);
                let button = unsafe {
                    NSButton::buttonWithTitle_target_action(
                        &translated(title),
                        Some(&target),
                        Some(sel!(perform:)),
                        mtm,
                    )
                };
                button.setBezelStyle(NSBezelStyle::Push);
                button.setFrame(rect(x - width, 14.0, width, 32.0));
                button.setTag(buttons.len() as isize);
                if id == "accept" {
                    let cell = button.cell().unwrap().downcast::<NSButtonCell>().unwrap();
                    window.setDefaultButtonCell(Some(&cell));
                }
                button.setKeyEquivalent(&NSString::from_str(match id {
                    "accept" => "\r",
                    "cancel" => "\u{1b}",
                    _ => "",
                }));
                content.addSubview(&button);
                target.ivars().actions.borrow_mut().push(id.into());
                buttons.push((id.into(), button));
                x -= width + 8.0;
            }
        }
        let mut sheet = Self {
            kind: spec.kind.clone(),
            title: spec.title.clone(),
            parent,
            window,
            target,
            body,
            error,
            buttons,
            schema: Vec::new(),
        };
        sheet.layout(spec);
        sheet
            .parent
            .beginSheet_completionHandler(&sheet.window, None);
        if let Some((_, Widget::Text(field))) = sheet.target.ivars().fields.borrow().first() {
            sheet.window.makeFirstResponder(Some(field));
            unsafe {
                field.selectText(None);
            }
        }
        sheet
    }
    fn layout(&mut self, spec: &Dialog) {
        let editing = self
            .target
            .ivars()
            .fields
            .borrow()
            .iter()
            .find_map(|(id, widget)| match widget {
                Widget::Multiline(view, _)
                    if self.window.firstResponder().is_some_and(|r| {
                        std::ptr::eq(
                            &*r as *const _ as *const (),
                            &**view as *const _ as *const (),
                        )
                    }) =>
                {
                    Some((id.clone(), view.selectedRange()))
                }
                _ => widget
                    .control()
                    .and_then(|control| control.currentEditor())
                    .map(|editor| (id.clone(), editor.selectedRange())),
            });
        let mtm = self.window.mtm();
        let description_height = 74.0;
        let total = description_height + spec.fields.iter().map(field_height).sum::<f64>() + 16.0;
        let viewport = total.clamp(180.0, 480.0);
        self.window
            .setContentSize(NSSize::new(620.0, viewport + 116.0));
        self.body.setFrame(rect(16.0, 100.0, 588.0, viewport));
        let doc = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, 568.0, total));
        let title = label(mtm, &spec.title, rect(8.0, total - 26.0, 552.0, 24.0));
        title.setFont(Some(&NSFont::boldSystemFontOfSize(17.0)));
        doc.addSubview(&title);
        let description = label(
            mtm,
            &spec.description,
            rect(8.0, total - description_height, 552.0, 42.0),
        );
        doc.addSubview(&description);
        let mut y = total - description_height - 8.0;
        let mut widgets = Vec::new();
        for field in &spec.fields {
            let height = field_height(field);
            y -= height;
            let title = translated(&field.title);
            if !matches!(field.kind, 2 | 8 | 9) {
                let title_height = height
                    - if field.kind == 6 {
                        42.0
                    } else if field.kind == 4 {
                        100.0
                    } else {
                        34.0
                    };
                let heading = label(
                    mtm,
                    &title.to_string(),
                    rect(8.0, y + height - title_height, 552.0, title_height),
                );
                doc.addSubview(&heading);
            }
            let widget = match field.kind {
                0 | 1 => {
                    let text: Retained<NSTextField> = if field.kind == 1 {
                        NSSecureTextField::initWithFrame(
                            NSSecureTextField::alloc(mtm),
                            rect(8.0, y + 6.0, 552.0, 24.0),
                        )
                        .into_super()
                    } else {
                        NSTextField::initWithFrame(
                            NSTextField::alloc(mtm),
                            rect(8.0, y + 6.0, 552.0, 24.0),
                        )
                    };
                    text.setBezeled(true);
                    text.setBezelStyle(NSTextFieldBezelStyle::RoundedBezel);
                    text.setMaximumNumberOfLines(1);
                    if let Some(cell) = text.cell() {
                        cell.setSendsActionOnEndEditing(false);
                        cell.setWraps(false);
                        cell.setScrollable(true);
                        cell.setUsesSingleLineMode(true);
                    }
                    unsafe {
                        // Return submits; leaving the field never submits.
                        text.setTarget(Some(&self.target));
                        text.setAction(Some(sel!(perform:)));
                        text.setTag(0);
                        text.setDelegate(Some(ProtocolObject::from_ref(&*self.target)));
                    }
                    Widget::Text(text)
                }
                2 | 8 => {
                    let check = if field.kind == 8 {
                        let button = NSButton::initWithFrame(
                            NSButton::alloc(mtm),
                            rect(8.0, y + 6.0, 20.0, 24.0),
                        );
                        button.setTitle(&NSString::from_str(""));
                        button.setButtonType(NSButtonType::PushOnPushOff);
                        button.setBezelStyle(NSBezelStyle::Disclosure);
                        button.setBordered(true);
                        unsafe {
                            button.setTarget(Some(&self.target));
                            button.setAction(Some(sel!(changed:)));
                        }
                        doc.addSubview(&label(
                            mtm,
                            &title.to_string(),
                            rect(34.0, y + 6.0, 520.0, 24.0),
                        ));
                        button
                    } else {
                        let button = unsafe {
                            NSButton::checkboxWithTitle_target_action(
                                &title,
                                Some(&self.target),
                                Some(sel!(changed:)),
                                mtm,
                            )
                        };
                        button.setFrame(rect(8.0, y + 6.0, 552.0, 24.0));
                        button
                    };
                    Widget::Check(check)
                }
                4 => {
                    let scroll = NSScrollView::initWithFrame(
                        NSScrollView::alloc(mtm),
                        rect(8.0, y + 6.0, 552.0, 84.0),
                    );
                    scroll.setHasVerticalScroller(true);
                    scroll.setAutohidesScrollers(true);
                    let text = NSTextView::initWithFrame(
                        NSTextView::alloc(mtm),
                        rect(0.0, 0.0, 532.0, 84.0),
                    );
                    text.setRichText(false);
                    text.setVerticallyResizable(true);
                    text.setHorizontallyResizable(false);
                    text.setMaxSize(NSSize::new(532.0, f64::MAX));
                    text.setFont(Some(&NSFont::systemFontOfSize(13.0)));
                    text.setDelegate(Some(ProtocolObject::from_ref(&*self.target)));
                    text.setAccessibilityLabel(Some(&title));
                    scroll.setDocumentView(Some(&text));
                    Widget::Multiline(text, scroll)
                }
                3 => {
                    let choice = NSPopUpButton::initWithFrame_pullsDown(
                        NSPopUpButton::alloc(mtm),
                        rect(6.0, y + 4.0, 556.0, 28.0),
                        false,
                    );
                    for value in &field.choices {
                        choice.addItemWithTitle(&translated(value));
                    }
                    unsafe {
                        choice.setTarget(Some(&self.target));
                        choice.setAction(Some(sel!(changed:)));
                    }
                    Widget::Choice(choice, field.choices.clone())
                }
                _ => {
                    let note = label(
                        mtm,
                        &crate::app::tr(&field.value),
                        rect(
                            8.0,
                            y + 4.0,
                            552.0,
                            if field.kind == 9 { 24.0 } else { 38.0 },
                        ),
                    );
                    note.setTextColor(Some(&NSColor::secondaryLabelColor()));
                    doc.addSubview(&note);
                    continue;
                }
            };
            widget.view().setAccessibilityLabel(Some(&title));
            widget.update(&field.value);
            doc.addSubview(widget.view());
            widgets.push((field.id.clone(), widget));
        }
        *self.target.ivars().fields.borrow_mut() = widgets;
        self.body.setDocumentView(Some(&doc));
        // NSView coordinates run from the bottom. Open long conflict lists at top.
        self.body
            .contentView()
            .scrollToPoint(NSPoint::new(0.0, (total - viewport).max(0.0)));
        self.window.recalculateKeyViewLoop();
        if let Some((id, range)) = editing {
            let fields = self.target.ivars().fields.borrow();
            if let Some((_, widget)) = fields.iter().find(|(key, _)| key == &id) {
                if let Widget::Multiline(view, _) = widget {
                    self.window.makeFirstResponder(Some(view));
                    view.setSelectedRange(range);
                } else {
                    self.window.makeFirstResponder(Some(widget.view()));
                    if let Some(editor) =
                        widget.control().and_then(|control| control.currentEditor())
                    {
                        editor.setSelectedRange(range);
                    }
                }
            }
        }
        self.schema = spec
            .fields
            .iter()
            .map(|f| (f.id.clone(), f.title.clone(), f.kind, f.choices.clone()))
            .collect();
    }
    fn update(&mut self, spec: &Dialog, scheme: crate::appearance::ColorScheme) {
        if self.schema.len() != spec.fields.len()
            || self
                .schema
                .iter()
                .zip(&spec.fields)
                .any(|((id, title, kind, choices), field)| {
                    id != &field.id
                        || title != &field.title
                        || kind != &field.kind
                        || choices != &field.choices
                })
        {
            self.layout(spec);
        }
        // Inherit the user's explicit light/dark setting, or the OS appearance.
        let appearance = unsafe {
            match scheme {
                crate::appearance::ColorScheme::System => None,
                crate::appearance::ColorScheme::Light => {
                    NSAppearance::appearanceNamed(NSAppearanceNameAqua)
                }
                crate::appearance::ColorScheme::Dark => {
                    NSAppearance::appearanceNamed(NSAppearanceNameDarkAqua)
                }
            }
        };
        unsafe {
            let _: () = msg_send![&*self.window, setAppearance: appearance.as_deref()];
        }
        for ((id, widget), field) in self.target.ivars().fields.borrow().iter().zip(
            spec.fields
                .iter()
                .filter(|f| matches!(f.kind, 0 | 1 | 2 | 3 | 4 | 8)),
        ) {
            debug_assert_eq!(id, &field.id);
            widget.update(&field.value);
        }
        self.error.setStringValue(&NSString::from_str(&spec.error));
        self.error
            .setToolTip(Some(&NSString::from_str(&spec.error)));
        self.error
            .setAccessibilityLabel(Some(&NSString::from_str(&spec.error)));
        for (id, button) in &self.buttons {
            button.setEnabled(
                spec.buttons
                    .iter()
                    .any(|(key, _, enabled)| key == id && *enabled),
            );
        }
    }
}
impl Drop for Sheet {
    fn drop(&mut self) {
        // Disconnect weak Cocoa delegates before releasing the retained target.
        self.window.setDelegate(None);
        for (_, widget) in self.target.ivars().fields.borrow().iter() {
            if let Widget::Text(text) = widget {
                unsafe {
                    text.setDelegate(None);
                }
            }
            if let Widget::Multiline(text, _) = widget {
                text.setDelegate(None);
            }
        }
        self.parent.endSheet(&self.window);
        self.window.orderOut(None);
    }
}
