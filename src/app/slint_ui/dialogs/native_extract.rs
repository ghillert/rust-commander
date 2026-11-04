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

mod preferences;

type Events = Rc<RefCell<Vec<Event>>>;
pub(in super::super) use super::native_intent::Intent;
pub(in super::super) struct Request {
    pub ui: slint::Weak<super::super::CommanderWindow>,
    pub spec: Option<Dialog>,
    pub scheme: crate::appearance::ColorScheme,
}
pub(in super::super) enum Event {
    Editing,
    Review(i32),
    Field(String, String),
    Action(String),
    Select(String),
}

#[derive(Clone)]
enum Widget {
    Text(Retained<NSTextField>),
    Note(Retained<NSTextField>),
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
            Self::Multiline(..) | Self::Note(_) => None,
        }
    }
    fn view(&self) -> &NSView {
        match self {
            Self::Text(w) | Self::Note(w) => w,
            Self::Choice(w, _) => w,
            Self::Check(w) => w,
            Self::Multiline(_, scroll) => scroll,
        }
    }
    fn value(&self) -> String {
        match self {
            Self::Text(w) | Self::Note(w) => w.stringValue().to_string(),
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
            Self::Text(w) | Self::Note(w) => w.setStringValue(&NSString::from_str(value)),
            Self::Choice(w, choices) => {
                if let Some(i) = choices.iter().position(|c| c == value) {
                    w.selectItemAtIndex(i as isize);
                } else {
                    w.selectItem(None);
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
    pickers: RefCell<Vec<(String, super::Picker)>>,
    review_page: std::cell::Cell<i32>,
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
        #[unsafe(method(review:))]
        fn review(&self, _sender: &NSControl) { self.ivars().events.borrow_mut().push(Event::Review(self.ivars().review_page.get())); }
        #[unsafe(method(browse:))]
        fn browse(&self,sender:&NSControl) {
            self.ivars().events.borrow_mut().push(Event::Editing);
            use objc2_app_kit::{NSOpenPanel,NSSavePanel};
            use objc2_foundation::NSURL;
            let Some((id,picker))=self.ivars().pickers.borrow().get(sender.tag() as usize).cloned() else{return;};
            let current=self.ivars().fields.borrow().iter().find(|(key,_)|key==&id).map(|(_,widget)|widget.value()).unwrap_or_default();
            let Some(owner)=sender.window() else{return;};let mtm=self.mtm();
            let panel:Retained<NSSavePanel>=if picker==super::Picker::SaveFile {NSSavePanel::savePanel(mtm)}else{
                let panel=NSOpenPanel::openPanel(mtm);
                panel.setCanChooseFiles(picker==super::Picker::File);
                panel.setCanChooseDirectories(matches!(picker,super::Picker::Directory|super::Picker::NewDirectory));
                panel.setAllowsMultipleSelection(false);panel.into_super()
            };
            panel.setTitle(Some(&translated(if picker==super::Picker::NewDirectory{"Choose parent folder"}else{"Browse"})));
            let path=std::path::Path::new(&current);
            let folder=if picker==super::Picker::Directory {Some(path)}else{path.parent()};
            if let Some(folder)=folder.filter(|path|path.is_absolute()){panel.setDirectoryURL(Some(&NSURL::fileURLWithPath(&NSString::from_str(&folder.to_string_lossy()))));}
            if picker==super::Picker::SaveFile&& let Some(name)=path.file_name(){panel.setNameFieldStringValue(&NSString::from_str(&name.to_string_lossy()));}
            let selected=panel.clone();let events=self.ivars().events.clone();
            let completion=block2::RcBlock::new(move|response:isize|{
                if response==1&& let Some(path)=selected.URL().and_then(|url|url.path()) {
                    events.borrow_mut().push(Event::Field(id.clone(),picker.selected(std::path::PathBuf::from(path.to_string()),&current)));
                }
            });
            panel.beginSheetModalForWindow_completionHandler(&owner,&completion);
        }
        #[unsafe(method(changed:))]
        fn changed(&self, sender: &NSControl) { self.collect_field(sender); }
        #[unsafe(method(perform:))]
        fn perform(&self, sender: &NSControl) {
            // Commit the field editor's current destination before the action.
            // Never materialize untouched per-file choices as overrides.
            if let Some((id, widget)) = self.ivars().fields.borrow().first() {
                self.ivars().events.borrow_mut().push(Event::Field(id.clone(), widget.value()));
            }
            if let Some(id) = self.ivars().actions.borrow().get(sender.tag() as usize).filter(|id| !id.is_empty()) {
                self.ivars().events.borrow_mut().push(if let Some(row) = id.strip_prefix("select:") {Event::Select(row.into())} else {Event::Action(id.clone())});
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
        self.sheet.as_ref().is_some_and(|sheet| {
            sheet.window.isKeyWindow()
                || sheet
                    .window
                    .attachedSheet()
                    .is_some_and(|panel| panel.isKeyWindow())
        })
    }
    pub fn show_page(&self, page: i32) {
        if let Some(view) = self
            .sheet
            .as_ref()
            .and_then(|sheet| sheet.preferences.as_ref())
        {
            view.show_page(page);
        }
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
            "native-page" => {
                sheet
                    .preferences
                    .as_ref()
                    .expect("Native Settings must be open")
                    .show_page(value.parse().unwrap());
            }
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
                // AppKit temporarily clears the default key equivalent when
                // another button has focus. Verify the default cell's identity
                // instead of requiring a key equivalent throughout focus changes.
                let default_cell = sheet.window.defaultButtonCell();
                let default_button = default_cell.as_ref().map(|cell| {
                    let (id, button) = sheet
                        .buttons
                        .iter()
                        .find(|(_, button)| {
                            button.cell().is_some_and(|candidate| {
                                std::ptr::eq(
                                    &*candidate as *const _ as *const (),
                                    &**cell as *const _ as *const (),
                                )
                            })
                        })
                        .expect("Native default cell must belong to a form button");
                    assert_eq!(id, "accept", "Native default action must be affirmative");
                    button
                });
                for (_, button) in &sheet.buttons {
                    if button.keyEquivalent().to_string() == "\r" {
                        assert!(
                            default_button
                                .is_some_and(|default| std::ptr::eq(&**default, &**button)),
                            "Return must belong to the native default action"
                        );
                    }
                }
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
        use objc2::AnyThread;
        use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
        use objc2_foundation::NSDictionary;
        let Some(sheet) = &self.sheet else {
            return false;
        };
        let view = sheet.window.contentView().unwrap();
        view.displayIfNeeded();
        // Layer-backed native tabs cannot be captured with NSView's vector
        // cache. Read only our own window's composited image when available.
        #[allow(deprecated)]
        let captured = objc2_core_graphics::CGWindowListCreateImage(
            rect(f64::INFINITY, f64::INFINITY, 0.0, 0.0),
            objc2_core_graphics::CGWindowListOption::OptionIncludingWindow,
            sheet.window.windowNumber() as u32,
            objc2_core_graphics::CGWindowImageOption::BoundsIgnoreFraming,
        );
        let bitmap = if let Some(image) = captured {
            NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &image)
        } else {
            let bitmap = view
                .bitmapImageRepForCachingDisplayInRect(view.bounds())
                .unwrap();
            view.cacheDisplayInRect_toBitmapImageRep(view.bounds(), &bitmap);
            bitmap
        };
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
    description: String,
    parent: Retained<NSWindow>,
    window: Retained<NSWindow>,
    target: Retained<Target>,
    body: Retained<NSScrollView>,
    error: Retained<NSTextField>,
    review: Retained<NSButton>,
    busy: Retained<objc2_app_kit::NSProgressIndicator>,
    buttons: Vec<(String, Retained<NSButton>)>,
    schema: Vec<Field>,
    preferences: Option<preferences::View>,
    row_schema: Vec<super::Row>,
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
        6 => {
            if field.value.len() > 300 {
                220.0
            } else {
                68.0
            }
        }
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
        let review = unsafe {
            NSButton::buttonWithTitle_target_action(
                &translated("Review invalid setting"),
                Some(&target),
                Some(sel!(review:)),
                mtm,
            )
        };
        review.setBezelStyle(NSBezelStyle::Push);
        review.setFrame(rect(612.0, 54.0, 184.0, 28.0));
        review.setHidden(true);
        content.addSubview(&review);
        let busy = objc2_app_kit::NSProgressIndicator::initWithFrame(
            objc2_app_kit::NSProgressIndicator::alloc(mtm),
            rect(24.0, 62.0, 16.0, 16.0),
        );
        busy.setStyle(objc2_app_kit::NSProgressIndicatorStyle::Spinning);
        busy.setIndeterminate(true);
        busy.setDisplayedWhenStopped(false);
        busy.setAccessibilityLabel(Some(&translated("Working…")));
        content.addSubview(&busy);
        let mut buttons = Vec::new();
        let mut x = if spec.kind == "preferences" {
            796.0
        } else {
            596.0
        };
        // Cocoa places the default action on the trailing edge, Cancel next to it.
        let mut footer = spec.buttons.iter().collect::<Vec<_>>();
        footer.sort_by_key(|(id, _, _)| match id.as_str() {
            "accept" => 0,
            "cancel" => 1,
            "apply" => 2,
            "apply-all" => 3,
            "defaults" => 99,
            _ => 4,
        });
        for (id, title, _) in footer {
            let id = id.as_str();
            {
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
                button.setFrame(rect(
                    if id == "defaults" { 24.0 } else { x - width },
                    14.0,
                    width,
                    32.0,
                ));
                button.setTag(buttons.len() as isize);
                if id == "accept" && spec.default_action {
                    let cell = button.cell().unwrap().downcast::<NSButtonCell>().unwrap();
                    window.setDefaultButtonCell(Some(&cell));
                }
                button.setKeyEquivalent(&NSString::from_str(match id {
                    "accept" if spec.default_action => "\r",
                    "cancel" => "\u{1b}",
                    _ => "",
                }));
                content.addSubview(&button);
                target.ivars().actions.borrow_mut().push(id.into());
                buttons.push((id.into(), button));
                if id != "defaults" {
                    x -= width + 8.0;
                }
            }
        }
        let mut sheet = Self {
            kind: spec.kind.clone(),
            title: spec.title.clone(),
            description: String::new(),
            parent,
            window,
            target,
            body,
            error,
            review,
            busy,
            buttons,
            schema: Vec::new(),
            preferences: None,
            row_schema: Vec::new(),
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
        self.description = spec.description.clone();
        if spec.kind == "preferences" {
            self.window.setContentSize(NSSize::new(820.0, 640.0));
            self.body.setHidden(true);
            self.error.setFrame(rect(24.0, 50.0, 772.0, 40.0));
            if self.preferences.is_none() {
                let view = preferences::View::new(self.window.mtm(), &self.target);
                self.window.contentView().unwrap().addSubview(view.view());
                self.preferences = Some(view);
            }
            self.preferences
                .as_mut()
                .unwrap()
                .update(spec, &self.target, &self.window);
            return;
        }
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
        let text_height = (spec
            .description
            .lines()
            .map(|line| line.chars().count().div_ceil(75).max(1))
            .sum::<usize>() as f64
            * 17.0)
            .max(24.0);
        let description_height = 32.0 + text_height;
        let total = description_height
            + spec.fields.iter().map(field_height).sum::<f64>()
            + spec.rows.len() as f64 * 74.0
            + 16.0;
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
            rect(8.0, total - description_height, 552.0, text_height),
        );
        doc.addSubview(&description);
        let mut y = total - description_height - 8.0;
        let mut widgets = Vec::new();
        self.target.ivars().pickers.borrow_mut().clear();
        self.target
            .ivars()
            .actions
            .borrow_mut()
            .truncate(self.buttons.len());
        for field in &spec.fields {
            let height = field_height(field);
            y -= height;
            let title = translated(&field.title);
            if !matches!(field.kind, 2 | 8 | 9) {
                let title_height = height
                    - if field.kind == 6 {
                        height - 24.0
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
                0 | 1 | 5 => {
                    let text: Retained<NSTextField> = if field.kind == 1 {
                        NSSecureTextField::initWithFrame(
                            NSSecureTextField::alloc(mtm),
                            rect(
                                8.0,
                                y + 6.0,
                                if field.picker.is_some() { 448.0 } else { 552.0 },
                                24.0,
                            ),
                        )
                        .into_super()
                    } else {
                        NSTextField::initWithFrame(
                            NSTextField::alloc(mtm),
                            rect(
                                8.0,
                                y + 6.0,
                                if field.picker.is_some() { 448.0 } else { 552.0 },
                                24.0,
                            ),
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
                        if spec.default_action
                            && let Some(index) =
                                self.buttons.iter().position(|(id, _)| id == "accept")
                        {
                            text.setTarget(Some(&self.target));
                            text.setAction(Some(sel!(perform:)));
                            text.setTag(index as isize);
                        }
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
                4 | 6 if field.kind == 4 || field.value.len() > 300 => {
                    let scroll = NSScrollView::initWithFrame(
                        NSScrollView::alloc(mtm),
                        rect(
                            8.0,
                            y + 6.0,
                            552.0,
                            if field.kind == 6 { height - 36.0 } else { 84.0 },
                        ),
                    );
                    scroll.setHasVerticalScroller(true);
                    scroll.setAutohidesScrollers(true);
                    let text = NSTextView::initWithFrame(
                        NSTextView::alloc(mtm),
                        rect(0.0, 0.0, 532.0, 84.0),
                    );
                    text.setRichText(false);
                    text.setEditable(field.kind == 4);
                    text.setVerticallyResizable(true);
                    text.setHorizontallyResizable(false);
                    text.setMaxSize(NSSize::new(532.0, f64::MAX));
                    text.setFont(Some(&NSFont::systemFontOfSize(13.0)));
                    if field.kind == 4 {
                        text.setDelegate(Some(ProtocolObject::from_ref(&*self.target)));
                    }
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
                    Widget::Note(note)
                }
            };
            widget.view().setAccessibilityLabel(Some(&title));
            widget.update(&field.value);
            doc.addSubview(widget.view());
            widgets.push((field.id.clone(), widget));
            if let Some(picker) = field.picker {
                let button = unsafe {
                    NSButton::buttonWithTitle_target_action(
                        &translated("Browse…"),
                        Some(&self.target),
                        Some(sel!(browse:)),
                        mtm,
                    )
                };
                button.setBezelStyle(NSBezelStyle::Push);
                button.setFrame(rect(464.0, y + 4.0, 96.0, 28.0));
                let mut pickers = self.target.ivars().pickers.borrow_mut();
                button.setTag(pickers.len() as isize);
                pickers.push((field.id.clone(), picker));
                doc.addSubview(&button);
            }
        }
        for row in &spec.rows {
            y -= 74.0;
            let heading = label(mtm, &row.title, rect(8.0, y + 42.0, 552.0, 24.0));
            heading.setFont(Some(&NSFont::boldSystemFontOfSize(13.0)));
            doc.addSubview(&heading);
            let mut x = 8.0;
            for (id, title, enabled) in &row.actions {
                let button = unsafe {
                    NSButton::buttonWithTitle_target_action(
                        &translated(title),
                        Some(&self.target),
                        Some(sel!(perform:)),
                        mtm,
                    )
                };
                let width = (title.chars().count() as f64 * 7.0 + 28.0).max(84.0);
                button.setBezelStyle(NSBezelStyle::Push);
                button.setFrame(rect(x, y + 14.0, width, 28.0));
                button.setEnabled(*enabled);
                let mut actions = self.target.ivars().actions.borrow_mut();
                button.setTag(actions.len() as isize);
                actions.push(id.clone());
                doc.addSubview(&button);
                x += width + 8.0;
            }
            if !row.detail.is_empty() {
                doc.addSubview(&label(
                    mtm,
                    &row.detail,
                    rect(x, y + 14.0, (552.0 - x).max(0.0), 28.0),
                ));
            }
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
            .cloned()
            .map(|mut field| {
                field.value.clear();
                field
            })
            .collect();
        self.row_schema = spec.rows.clone();
    }
    fn update(&mut self, spec: &Dialog, scheme: crate::appearance::ColorScheme) {
        if spec.kind == "preferences"
            || self.description != spec.description
            || self.row_schema != spec.rows
            || self.schema.len() != spec.fields.len()
            || self.schema.iter().zip(&spec.fields).any(|(old, field)| {
                old.id != field.id
                    || old.title != field.title
                    || old.kind != field.kind
                    || old.choices != field.choices
                    || old.picker != field.picker
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
        let widgets = self.target.ivars().fields.borrow();
        if spec.kind != "preferences" {
            for ((_, widget), field) in widgets.iter().zip(&spec.fields) {
                widget.update(&field.value);
            }
        } else {
            for (id, widget) in widgets.iter() {
                if let Some(field) = spec.fields.iter().find(|f| &f.id == id) {
                    widget.update(&field.value);
                } else if let Some(field) = spec
                    .fields
                    .iter()
                    .find(|f| f.kind == 11 && f.choices.get(1) == Some(id))
                {
                    widget.update(&field.choices[0]);
                }
            }
        }
        // SAFETY: nil sender is valid; this indicator is owned on the AppKit thread.
        unsafe {
            if spec.busy && spec.error.is_empty() {
                self.busy.startAnimation(None);
            } else {
                self.busy.stopAnimation(None);
            }
        }
        self.review.setHidden(spec.error_page.is_none());
        self.target
            .ivars()
            .review_page
            .set(spec.error_page.unwrap_or(-1));
        if self.preferences.is_some() {
            self.error.setFrame(rect(
                24.0,
                50.0,
                if spec.error_page.is_some() {
                    572.0
                } else {
                    772.0
                },
                40.0,
            ));
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
        if let Some(panel) = self.window.attachedSheet() {
            self.window.endSheet(&panel);
            panel.orderOut(None);
        }
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
