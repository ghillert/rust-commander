//! Extend the existing winit delegate for in-process file promises, preserving its behavior.
use super::*;
use objc2::{
    runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel},
    sel, ClassType, Message,
};
use objc2_app_kit::NSFilePromiseReceiver;
use std::{cell::RefCell, collections::BTreeMap, sync::OnceLock};

#[derive(Clone)]
struct Receiver {
    view: Retained<NSView>,
    callback: crate::DropCallback,
}
thread_local! {static RECEIVERS: RefCell<BTreeMap<usize, Receiver>> = const {RefCell::new(BTreeMap::new())};}
static SUBCLASS: OnceLock<&'static AnyClass> = OnceLock::new();

pub struct DropTarget {
    delegate: Retained<AnyObject>,
    parent: &'static AnyClass,
}
impl Drop for DropTarget {
    fn drop(&mut self) {
        RECEIVERS.with(|map| {
            map.borrow_mut()
                .remove(&(Retained::as_ptr(&self.delegate) as usize))
        });
        // No ivars were added; restore the original delegate class before releasing our retain.
        unsafe {
            objc2::ffi::object_setClass(Retained::as_ptr(&self.delegate).cast_mut(), self.parent);
        }
    }
}
fn token(sender: &AnyObject) -> Option<String> {
    let source: Option<Retained<AnyObject>> = unsafe { msg_send![sender, draggingSource] };
    let source = source?;
    if source.class() != DragRsSource::class() {
        return None;
    }
    INTERNAL_TOKEN.with(|token| token.borrow().clone())
}
fn emit(this: &AnyObject, sender: &AnyObject, dropped: bool) -> bool {
    let token = token(sender);
    let paths = if token.is_none() {
        let board: Retained<objc2_app_kit::NSPasteboard> =
            unsafe { msg_send![sender, draggingPasteboard] };
        board
            .pasteboardItems()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let text = item.stringForType(unsafe { NSPasteboardTypeFileURL })?;
                        let url = NSURL::URLWithString(&text)?;
                        url.isFileURL()
                            .then(|| {
                                url.path()
                                    .map(|path| std::path::PathBuf::from(path.to_string()))
                            })
                            .flatten()
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    if token.is_none() && paths.is_empty() {
        return false;
    }
    let receiver = RECEIVERS.with(|map| map.borrow().get(&(this as *const _ as usize)).cloned());
    let Some(receiver) = receiver else {
        return false;
    };
    let point: NSPoint = unsafe { msg_send![sender, draggingLocation] };
    let point = receiver.view.convertPoint_fromView(point, None);
    let y = if receiver.view.isFlipped() {
        point.y
    } else {
        receiver.view.bounds().size.height - point.y
    };
    let event = if dropped {
        crate::DropEvent::Drop {
            x: point.x as f32,
            y: y as f32,
            token,
            paths,
        }
    } else {
        crate::DropEvent::Hover {
            x: point.x as f32,
            y: y as f32,
            token,
            paths,
        }
    };
    (receiver.callback)(event);
    true
}
fn parent(this: &AnyObject) -> &'static AnyClass {
    this.class().superclass().expect("native delegate parent")
}
extern "C-unwind" fn entered(this: &AnyObject, _: Sel, sender: &AnyObject) -> Bool {
    if emit(this, sender, false) {
        Bool::YES
    } else {
        unsafe { msg_send![super(this, parent(this)), draggingEntered: sender] }
    }
}
extern "C-unwind" fn updated(this: &AnyObject, _: Sel, sender: &AnyObject) -> usize {
    if emit(this, sender, false) {
        1
    } else {
        let allowed: bool =
            unsafe { msg_send![super(this, parent(this)), draggingEntered: sender] };
        usize::from(allowed)
    }
}
extern "C-unwind" fn prepare(this: &AnyObject, _: Sel, sender: &AnyObject) -> Bool {
    if token(sender).is_some() {
        Bool::YES
    } else {
        unsafe { msg_send![super(this, parent(this)), prepareForDragOperation: sender] }
    }
}
extern "C-unwind" fn perform(this: &AnyObject, _: Sel, sender: &AnyObject) -> Bool {
    if emit(this, sender, true) {
        Bool::YES
    } else {
        unsafe { msg_send![super(this, parent(this)), performDragOperation: sender] }
    }
}
extern "C-unwind" fn exited(this: &AnyObject, _: Sel, sender: &AnyObject) {
    if let Some(receiver) =
        RECEIVERS.with(|map| map.borrow().get(&(this as *const _ as usize)).cloned())
    {
        (receiver.callback)(crate::DropEvent::Leave);
    }
    if token(sender).is_none() {
        unsafe {
            let _: () = msg_send![super(this, parent(this)), draggingExited: sender];
        }
    }
}

pub fn attach_drop_target(
    window: &impl HasWindowHandle,
    callback: crate::DropCallback,
) -> crate::Result<DropTarget> {
    let RawWindowHandle::AppKit(handle) = window
        .window_handle()
        .map_err(|_| crate::Error::UnsupportedWindowHandle)?
        .as_raw()
    else {
        return Err(crate::Error::UnsupportedWindowHandle);
    };
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let window = view.window().ok_or(crate::Error::UnsupportedWindowHandle)?;
    let delegate: Retained<AnyObject> = unsafe { msg_send![&window, delegate] };
    let parent = delegate.class();
    let subclass = SUBCLASS.get_or_init(|| {
        let mut builder = ClassBuilder::new(c"RustCommanderDropDelegate", parent)
            .expect("unique native receiver class");
        unsafe {
            builder.add_method(
                sel!(draggingEntered:),
                entered as extern "C-unwind" fn(_, _, _) -> _,
            );
            builder.add_method(
                sel!(draggingUpdated:),
                updated as extern "C-unwind" fn(_, _, _) -> _,
            );
            builder.add_method(
                sel!(prepareForDragOperation:),
                prepare as extern "C-unwind" fn(_, _, _) -> _,
            );
            builder.add_method(
                sel!(performDragOperation:),
                perform as extern "C-unwind" fn(_, _, _) -> _,
            );
            builder.add_method(
                sel!(draggingExited:),
                exited as extern "C-unwind" fn(_, _, _),
            );
        }
        builder.register()
    });
    // Subclass only this app-owned winit delegate; its layout and all other methods stay inherited.
    unsafe {
        objc2::ffi::object_setClass(Retained::as_ptr(&delegate).cast_mut(), *subclass);
    }
    RECEIVERS.with(|map| {
        map.borrow_mut().insert(
            Retained::as_ptr(&delegate) as usize,
            Receiver {
                view: view.retain(),
                callback,
            },
        );
    });
    let mut types = NSFilePromiseReceiver::readableDraggedTypes().to_vec();
    types.push(NSString::from_str("NSFilenamesPboardType"));
    types.push(unsafe { NSPasteboardTypeFileURL.retain() });
    window.registerForDraggedTypes(&NSArray::from_retained_slice(&types));
    Ok(DropTarget { delegate, parent })
}
