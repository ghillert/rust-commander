use super::Key;
use objc2::{AllocAnyThread, rc::autoreleasepool};
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};

pub(super) struct Provider;

impl Provider {
    pub fn new() -> Option<Self> {
        Some(Self)
    }

    pub fn load(&self, key: &Key) -> Option<image::RgbaImage> {
        autoreleasepool(|_| {
            // NSString cannot represent non-Unicode filesystem paths. Keep the
            // embedded fallback for these instead of looking up a different path.
            let path = NSString::from_str(key.path.to_str()?);
            let icon = NSWorkspace::sharedWorkspace().iconForFile(&path);
            let size = super::ICON_SIZE as f64;
            let mut rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(size, size));
            // The rectangle lives through this call; no graphics context or
            // hints are supplied. Each request owns its image on this thread.
            let cg = unsafe { icon.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
            let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
            // The empty dictionary contains no values with mismatched types.
            let png = unsafe {
                bitmap.representationUsingType_properties(
                    NSBitmapImageFileType::PNG,
                    &NSDictionary::new(),
                )
            }?;
            image::load_from_memory_with_format(&png.to_vec(), image::ImageFormat::Png)
                .ok()
                .map(|i| i.into_rgba8())
        })
    }
}
