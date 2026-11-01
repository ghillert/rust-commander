//! Immutable input identity for deferred native Return across platforms.
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
