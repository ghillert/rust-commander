//! AppKit asks for names while hovering and writes only after an accepted drop.
use super::*;
use objc2_app_kit::{NSFilePromiseProvider, NSFilePromiseProviderDelegate};
use objc2_foundation::{NSError, NSOperationQueue};

struct Ivars {
    files: crate::PromisedFiles,
    index: usize,
    queue: Retained<NSOperationQueue>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = AnyThread]
    #[name = "RustCommanderFilePromiseDelegate"]
    #[ivars = Ivars]
    struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl NSFilePromiseProviderDelegate for Delegate {
        #[unsafe(method_id(filePromiseProvider:fileNameForType:))]
        fn name(&self, _: &NSFilePromiseProvider, _: &NSString) -> Retained<NSString> {
            NSString::from_str(
                &self.ivars().files.entries[self.ivars().index]
                    .path
                    .to_string_lossy(),
            )
        }
        #[unsafe(method_id(operationQueueForFilePromiseProvider:))]
        fn queue(&self, _: &NSFilePromiseProvider) -> Retained<NSOperationQueue> {
            self.ivars().queue.clone()
        }
        #[unsafe(method(filePromiseProvider:writePromiseToURL:completionHandler:))]
        fn write(
            &self,
            _: &NSFilePromiseProvider,
            url: &NSURL,
            completion: &block2::DynBlock<dyn Fn(*mut NSError)>,
        ) {
            let result = if url.isFileURL() {
                url.path()
                    .ok_or_else(|| "Invalid promise file URL".to_string())
                    .and_then(|path| {
                        (self.ivars().files.write)(
                            self.ivars().index,
                            std::path::Path::new(&path.to_string()),
                        )
                    })
            } else {
                Err("The promised destination is not a file URL".into())
            };
            let error = result.err().map(|_| unsafe {
                NSError::errorWithDomain_code_userInfo(
                    &NSString::from_str("RustCommanderFilePromise"),
                    1,
                    None,
                )
            });
            completion.call((error
                .as_ref()
                .map_or(std::ptr::null_mut(), |e| Retained::as_ptr(e).cast_mut()),));
        }
    }
);
pub(super) fn provider(
    files: &crate::PromisedFiles,
    index: usize,
    directory: bool,
) -> Retained<NSFilePromiseProvider> {
    let queue = NSOperationQueue::new();
    queue.setMaxConcurrentOperationCount(1);
    let delegate = Delegate::alloc().set_ivars(Ivars {
        files: files.clone(),
        index,
        queue,
    });
    let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
    let provider = NSFilePromiseProvider::initWithFileType_delegate(
        NSFilePromiseProvider::alloc(),
        &NSString::from_str(if directory {
            "public.folder"
        } else {
            "public.data"
        }),
        ProtocolObject::from_ref(&*delegate),
    );
    // AppKit's delegate property is weak. The provider (retained by the session
    // and receiver) owns its delegate until delayed writes have completed.
    unsafe {
        provider.setUserInfo(Some(&delegate));
    }
    provider
}
