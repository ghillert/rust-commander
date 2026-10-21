use super::{IconKind, Key, NativeIcon};
use objc2::{AllocAnyThread, rc::autoreleasepool, runtime::AnyObject};
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage, NSWorkspace};
use objc2_foundation::{
    NSArray, NSBundle, NSDictionary, NSPoint, NSRect, NSSize, NSString, NSURL, NSURLCustomIconKey,
};
use objc2_uniform_type_identifiers::{UTType, UTTypeContent, UTTypeData, UTTypeItem};
use std::{ffi::CString, os::unix::ffi::OsStrExt, path::Path};

pub(super) struct Provider {
    generic: Vec<image::RgbaImage>,
}

impl Provider {
    pub fn new() -> Option<Self> {
        Some(autoreleasepool(|_| {
            let workspace = NSWorkspace::sharedWorkspace();
            // These immutable framework constants have process lifetime.
            let generic = unsafe { [UTTypeData, UTTypeItem, UTTypeContent] }
                .into_iter()
                .filter_map(|kind| render(&workspace.iconForContentType(kind)))
                .map(super::normalize)
                .collect();
            Self { generic }
        }))
    }

    pub fn load(&self, key: &Key) -> Option<NativeIcon> {
        autoreleasepool(|_| {
            // NSString cannot represent non-Unicode filesystem paths. Keep the
            // embedded fallback for these instead of looking up a different path.
            let path = NSString::from_str(key.path.to_str()?);
            let url = NSURL::fileURLWithPath(&path);
            let mut custom = None;
            // The output is an owned Objective-C object, used only for presence.
            let custom_status =
                unsafe { url.getResourceValue_forKey_error(&mut custom, NSURLCustomIconKey) };
            let workspace = NSWorkspace::sharedWorkspace();
            let icon = workspace.iconForFile(&path);
            let mut result = NativeIcon::new(render(&icon)?, IconKind::Unknown);
            result.kind = if custom.is_some() || has_custom_finder_icon(&key.path) || key.directory
            {
                IconKind::Specific
            } else if custom_status.is_ok()
                && (self.generic.contains(&result.pixels)
                    || system_document_template(&workspace, &key.path, &result.pixels))
            {
                IconKind::Generic
            } else {
                IconKind::Unknown
            };
            Some(result)
        })
    }
}

fn system_document_template(
    workspace: &NSWorkspace,
    path: &Path,
    pixels: &image::RgbaImage,
) -> bool {
    let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    let Some(kind) = UTType::typeWithFilenameExtension(&NSString::from_str(extension)) else {
        return false;
    };
    let Some(application) = workspace.URLForApplicationToOpenContentType(&kind) else {
        return false;
    };
    let Some(application_path) = application.path() else {
        return false;
    };
    if !system_application(Path::new(&application_path.to_string())) {
        return false;
    }
    let Some(bundle) = NSBundle::bundleWithURL(&application) else {
        return false;
    };
    let Some(documents) =
        bundle.objectForInfoDictionaryKey(&NSString::from_str("CFBundleDocumentTypes"))
    else {
        return false;
    };
    // Apple's stock handlers (Preview, TextEdit, etc.) can inherit a document
    // template with only a faint type label. Recognize it by provenance and
    // exact artwork, never by being white, gray, or monochrome. Keep supplied
    // document artwork, third-party handlers, and uncertain metadata native.
    // Resolve associations per lookup so normal cache expiry picks up changes.
    document_artwork(&documents, &kind, extension) == Some(false)
        && render(&workspace.iconForContentType(&kind))
            .map(super::normalize)
            .is_some_and(|reference| reference == *pixels)
}

fn system_application(path: &Path) -> bool {
    path.starts_with("/System/Applications") || path.starts_with("/System/Library/CoreServices")
}

// None means malformed metadata or no matching declaration; preserve it.
fn document_artwork(documents: &AnyObject, kind: &UTType, extension: &str) -> Option<bool> {
    let documents = documents.downcast_ref::<NSArray<AnyObject>>()?;
    let mut matched = false;
    for document in documents {
        let document = document.downcast_ref::<NSDictionary<AnyObject, AnyObject>>()?;
        let mut applies = false;
        if let Some(types) = document.objectForKey(&NSString::from_str("LSItemContentTypes")) {
            for identifier in metadata_strings(&types)? {
                applies |= UTType::typeWithIdentifier(&NSString::from_str(&identifier))
                    .is_some_and(|declared| kind.conformsToType(&declared));
            }
        }
        if let Some(extensions) =
            document.objectForKey(&NSString::from_str("CFBundleTypeExtensions"))
        {
            applies |= metadata_strings(&extensions)?
                .iter()
                .any(|ext| ext == "*" || ext.eq_ignore_ascii_case(extension));
        }
        if applies {
            matched = true;
            if [
                "CFBundleTypeIconFile",
                "CFBundleTypeIconFiles",
                "CFBundleTypeIconName",
            ]
            .iter()
            .any(|key| document.objectForKey(&NSString::from_str(key)).is_some())
            {
                return Some(true);
            }
        }
    }
    matched.then_some(false)
}

fn metadata_strings(value: &AnyObject) -> Option<Vec<String>> {
    if let Some(string) = value.downcast_ref::<NSString>() {
        return Some(vec![string.to_string()]);
    }
    value
        .downcast_ref::<NSArray<AnyObject>>()?
        .iter()
        .map(|item| item.downcast_ref::<NSString>().map(|s| s.to_string()))
        .collect()
}

fn has_custom_finder_icon(path: &Path) -> bool {
    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut info = [0u8; 32];
    // FinderInfo is a 32-byte record. Its big-endian flags at offset 8 contain
    // kHasCustomIcon (0x0400). NSURLCustomIconKey may return nil even when this
    // flag and the resource-fork icon are present. All reads stay on the worker.
    let length = unsafe {
        libc::getxattr(
            path.as_ptr(),
            c"com.apple.FinderInfo".as_ptr(),
            info.as_mut_ptr().cast(),
            info.len(),
            0,
            0,
        )
    };
    length >= 10 && u16::from_be_bytes([info[8], info[9]]) & 0x0400 != 0
}

fn render(icon: &NSImage) -> Option<image::RgbaImage> {
    let size = super::ICON_SIZE as f64;
    let mut rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(size, size));
    // The rectangle lives through this call; no graphics context or
    // hints are supplied. Each request owns its image on this thread.
    let cg = unsafe { icon.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
    let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
    // The empty dictionary contains no values with mismatched types.
    let png = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }?;
    image::load_from_memory_with_format(&png.to_vec(), image::ImageFormat::Png)
        .ok()
        .map(|i| i.into_rgba8())
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_uniform_type_identifiers::UTTypePDF;

    #[test]
    fn macos_explicit_custom_icons_survive_generic_detection() {
        std::thread::spawn(|| {
            autoreleasepool(|_| {
                let root = tempfile::tempdir().unwrap();
                let file = root.path().join("custom.pdf");
                std::fs::write(&file, "<sample/>").unwrap();
                let workspace = NSWorkspace::sharedWorkspace();
                // Even explicitly assigning the stock artwork must keep the
                // user's custom icon rather than replacing it with a fallback.
                let custom = workspace.iconForContentType(unsafe { UTTypePDF });
                let path = NSString::from_str(file.to_str().unwrap());
                assert!(workspace.setIcon_forFile_options(
                    Some(&custom),
                    &path,
                    objc2_app_kit::NSWorkspaceIconCreationOptions::empty()
                ));
                let provider = Provider::new().unwrap();
                assert!(!provider.generic.is_empty());
                let result = provider
                    .load(&Key {
                        path: file,
                        directory: false,
                        modified: None,
                    })
                    .unwrap();
                assert_eq!(result.kind, IconKind::Specific);
            });
        })
        .join()
        .unwrap();
    }

    #[test]
    fn macos_stock_document_templates_yield_to_builtin_type_icons() {
        std::thread::spawn(|| {
            autoreleasepool(|_| {
                let root = tempfile::tempdir().unwrap();
                let workspace = NSWorkspace::sharedWorkspace();
                let provider = Provider::new().unwrap();
                let different = image::RgbaImage::from_pixel(64, 64, image::Rgba([0, 0, 0, 255]));
                for extension in ["PDF", "jpeg", "png", "txt", "rtf", "zip"] {
                    let file = root.path().join(format!("Document.{extension}"));
                    std::fs::write(&file, b"fixture").unwrap();
                    let kind =
                        UTType::typeWithFilenameExtension(&NSString::from_str(extension)).unwrap();
                    let reference = super::super::normalize(
                        render(&workspace.iconForContentType(&kind)).unwrap(),
                    );
                    let template = system_document_template(&workspace, &file, &reference);
                    assert!(
                        !system_document_template(&workspace, &file, &different),
                        "{extension}: different artwork must survive"
                    );
                    let icon = provider
                        .load(&Key {
                            path: file,
                            directory: false,
                            modified: None,
                        })
                        .unwrap();
                    if template {
                        assert_eq!(icon.kind, IconKind::Generic, "{extension}");
                    } else {
                        assert!(
                            provider.generic.contains(&icon.pixels)
                                || icon.kind == IconKind::Unknown,
                            "{extension}: non-template icon must survive"
                        );
                    }
                    eprintln!(
                        "{extension}: stock template = {template}, native classification = {:?}",
                        icon.kind
                    );
                }
                assert!(!system_document_template(
                    &workspace,
                    &root.path().join("Makefile"),
                    &different
                ));
            });
        })
        .join()
        .unwrap();
    }

    #[test]
    fn document_declarations_preserve_supplied_artwork_and_uncertain_metadata() {
        autoreleasepool(|_| {
            let kind = unsafe { UTTypePDF };
            let type_key = NSString::from_str("LSItemContentTypes");
            let types = NSArray::from_retained_slice(&[NSString::from_str("public.data")]);
            let plain = NSDictionary::<NSString, AnyObject>::from_slices(&[&*type_key], &[&types]);
            assert_eq!(
                document_artwork(&NSArray::from_slice(&[&*plain]), kind, "pdf"),
                Some(false)
            );
            for key in [
                "CFBundleTypeIconFile",
                "CFBundleTypeIconFiles",
                "CFBundleTypeIconName",
            ] {
                let icon_key = NSString::from_str(key);
                let icon = NSString::from_str("MonochromeDocument");
                let supplied = NSDictionary::<NSString, AnyObject>::from_slices(
                    &[&*type_key, &*icon_key],
                    &[&types, &icon],
                );
                assert_eq!(
                    document_artwork(&NSArray::from_slice(&[&*supplied]), kind, "pdf"),
                    Some(true)
                );
            }
            let legacy_key = NSString::from_str("CFBundleTypeExtensions");
            let extension = NSString::from_str("PDF");
            let legacy =
                NSDictionary::<NSString, AnyObject>::from_slices(&[&*legacy_key], &[&extension]);
            assert_eq!(
                document_artwork(&NSArray::from_slice(&[&*legacy]), kind, "pdf"),
                Some(false)
            );
            assert_eq!(
                document_artwork(&NSArray::from_slice(&[&*legacy]), kind, "png"),
                None
            );
            assert_eq!(document_artwork(&extension, kind, "pdf"), None);
            assert!(!system_application(Path::new("/Applications/Preview.app")));
            assert!(!system_application(Path::new(
                "/System/ApplicationsBackup/Example.app"
            )));
        });
    }
}
