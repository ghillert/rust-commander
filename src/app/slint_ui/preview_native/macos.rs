use super::*;
use objc2::{ClassType, DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained, sel};
use objc2_app_kit::*;
use objc2_foundation::{
    MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

fn rect(x: f32, y: f32, width: f32, height: f32) -> NSRect {
    NSRect::new(
        NSPoint::new(x as f64, y as f64),
        NSSize::new(width as f64, height as f64),
    )
}
define_class!(
    // SAFETY: AppKit owns all view calls on the main UI thread.
    #[unsafe(super=NSView)] #[thread_kind=MainThreadOnly] #[name="RustCommanderPreviewOverlay"]
    struct Overlay;
    impl Overlay {
        #[unsafe(method(isFlipped))] fn flipped(&self)->bool {true}
        #[unsafe(method_id(hitTest:))] fn hit(&self, point:NSPoint)->Option<Retained<NSView>> {
            let hit:Option<Retained<NSView>>=unsafe{msg_send![super(self),hitTest:point]};
            if hit.as_ref().is_some_and(|view| std::ptr::eq(&**view, self.as_super()))
                && NSApplication::sharedApplication(self.mtm()).currentEvent().is_some_and(|event| matches!(event.r#type(), NSEventType::LeftMouseDown | NSEventType::RightMouseDown))
                && let Some(parent) = unsafe { self.superview() }
                && let Some(window) = parent.window()
            { window.makeFirstResponder(Some(&parent)); }
            hit.filter(|view|!std::ptr::eq(&**view,self.as_super()))
        }
    }
);
define_class!(
    // SAFETY: callbacks only set a main-thread event flag, never reenter Slint.
    #[unsafe(super=NSObject)] #[thread_kind=MainThreadOnly] #[name="RustCommanderPreviewTarget"]
    #[ivars=Rc<Cell<bool>>] struct Target;
    unsafe impl NSObjectProtocol for Target {}
    impl Target {
        #[unsafe(method(perform:))] fn perform(&self, _sender:&NSButton) {self.ivars().set(true);}
    }
);
pub(super) struct Platform {
    parent: Retained<NSView>,
    overlay: Retained<Overlay>,
    button: Retained<NSButton>,
    _target: Retained<Target>,
}
impl Platform {
    pub fn new(ui: &CommanderWindow, clicked: Rc<Cell<bool>>) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let handle = ui.window().window_handle();
        let handle = handle.window_handle().ok()?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return None;
        };
        // SAFETY: the window owns this live NSView; retain until the button detaches.
        let parent = unsafe { Retained::retain(handle.ns_view.cast::<NSView>().as_ptr()) }?;
        let overlay: Retained<Overlay> =
            unsafe { msg_send![Overlay::alloc(mtm), initWithFrame:rect(0.0,0.0,0.0,0.0)] };
        let target = Target::alloc(mtm).set_ivars(clicked);
        let target: Retained<Target> = unsafe { msg_send![super(target), init] };
        let button = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str(&tr("Open externally")),
                Some(&target),
                Some(sel!(perform:)),
                mtm,
            )
        };
        button.setBezelStyle(NSBezelStyle::Push);
        unsafe {
            button.setNextKeyView(Some(&parent));
        }
        overlay.addSubview(&button);
        parent.addSubview(&overlay);
        Some(Self {
            parent,
            overlay,
            button,
            _target: target,
        })
    }
    pub fn sync(&self, ui: &CommanderWindow, dark: bool) {
        self.overlay.setFrame(self.parent.bounds());
        self.overlay.setBounds(rect(
            0.0,
            0.0,
            ui.get_viewer_window_width(),
            ui.get_viewer_window_height(),
        ));
        let appearance = NSAppearance::appearanceNamed(if dark {
            unsafe { NSAppearanceNameDarkAqua }
        } else {
            unsafe { NSAppearanceNameAqua }
        });
        self.overlay.setAppearance(appearance.as_deref());
        let label = NSString::from_str(&tr("Open externally"));
        self.button.setTitle(&label);
        self.button.setAccessibilityLabel(Some(&label));
        self.button.setEnabled(ui.get_preview_can_open());
        self.button.setFrame(rect(
            ui.get_preview_button_x(),
            ui.get_preview_button_y(),
            ui.get_preview_button_width(),
            32.0,
        ));
    }
    pub fn focus(&self) {
        if let Some(window) = self.parent.window() {
            window.makeFirstResponder(Some(&self.button));
        }
    }
    #[cfg(feature = "screenshot")]
    pub fn snapshot(&self, ui: &CommanderWindow, path: &Path) -> bool {
        use objc2_foundation::NSDictionary;
        // The smoke test verifies the actual AppKit control and its placement.
        assert_eq!(self.button.title().to_string(), tr("Open externally"));
        assert!(self.button.frame().size.width > 0.0);
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
        self.overlay.displayIfNeeded();
        let Some(bitmap) = self
            .overlay
            .bitmapImageRepForCachingDisplayInRect(self.overlay.bounds())
        else {
            return false;
        };
        self.overlay
            .cacheDisplayInRect_toBitmapImageRep(self.overlay.bounds(), &bitmap);
        let Some(data) = (unsafe {
            bitmap.representationUsingType_properties(
                NSBitmapImageFileType::PNG,
                &NSDictionary::new(),
            )
        }) else {
            return false;
        };
        let Ok(native) = ::image::load_from_memory(&data.to_vec()) else {
            return false;
        };
        let native = ::image::imageops::resize(
            &native.to_rgba8(),
            image.width(),
            image.height(),
            ::image::imageops::FilterType::Lanczos3,
        );
        ::image::imageops::overlay(&mut image, &native, 0, 0);
        image.save(path).is_ok()
    }
}
impl Drop for Platform {
    fn drop(&mut self) {
        if let Some(window) = self.parent.window()
            && window
                .firstResponder()
                .is_some_and(|r| std::ptr::eq(&*r, &*self.button as &NSResponder))
        {
            window.makeFirstResponder(Some(&self.parent));
        }
        self.overlay.removeFromSuperview();
    }
}
