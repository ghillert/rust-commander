# Upstream parity

Baseline: `3504a51e9af9f69a67dc616db821a320ec780f79`

Completion requires implemented behavior and validation evidence. A menu entry,
adapter dependency, or untested backend does not establish parity. Runtime-only
Java actions require an explicit rationale if they are inapplicable to Rust.

| Upstream action or module | Status | Evidence |
| --- | --- | --- |
| AddBookmark | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| BatchRename | implemented | src/desktop.rs, tests/desktop_tools.rs::batch_rename_supports_cycles_without_overwriting_contents, tests/desktop_tools.rs::batch_rename_previews_numbering_and_regex |
| BringAllToFront | missing | — |
| CalculateChecksum | implemented | src/operations.rs, tests/desktop_tools.rs::all_checksum_algorithms_match_known_values |
| ChangeDate | partial | src/operations.rs, src/app.rs — Core and UI exist; full platform behavior is not validated. |
| ChangeLocation | partial | src/app.rs, parity/VALIDATION.md — Core local navigation/open workflow exists; full upstream action semantics and cross-platform behavior remain incomplete. |
| ChangePermissions | partial | src/operations.rs, src/app.rs — Core and UI exist; full platform behavior is not validated. |
| CheckForUpdates | missing | — |
| CloneTabToOtherPanel | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| CloseDuplicateTabs | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| CloseOtherTabs | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| CloseTab | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| CloseWindow | partial | src/app.rs — Implemented UI and close protection; full lifecycle validation pending. |
| CombineFiles | implemented | src/desktop.rs, tests/desktop_tools.rs::split_and_combine_round_trip_every_chunk_boundary |
| CompareFolders | partial | src/desktop.rs, tests/desktop_tools.rs — Local-only; search content has a size limit and comparison skips symlinks. |
| ConnectToServer | partial | src/remote.rs, tests/webdav_backend.rs — Saved profiles, keychain integration and partial adapters; live services and keychain UI need validation. |
| Copy | partial | src/transfer.rs, src/remote.rs, src/app.rs, tests/transfer_conflicts.rs, tests/features.rs — Local recursive conflict preview and per-file choices, recoverable replacements and optional folder merge. Remote top-level choices tested with filesystem adapter; live services and cross-platform behavior pending. |
| CopyFileBaseNames | missing | — |
| CopyFileNames | partial | src/app.rs — UI implemented; native smoke validation pending. |
| CopyFilePaths | partial | src/app.rs — UI implemented; native smoke validation pending. |
| CopyFilesToClipboard | partial | src/native_files.rs, src/app.rs, tests/features.rs — Native local file clipboard plus remote pane selections. Mac copy/cut, Unicode paths and external clipboard changes pass a real desktop round-trip. Windows/Linux interoperability requires validation. |
| CustomizeCommandBar | missing | — |
| Delete | partial | src/operations.rs, src/app.rs — Core and UI exist; full platform behavior is not validated. |
| Donate | missing | — |
| DuplicateTab | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| Edit | partial | src/app.rs, src/remote.rs, tests/features_05.rs::remote_search_and_guarded_save_use_actual_storage_operations — Local and guarded remote UTF-8 editing up to 2 MiB. Native macOS Save/Undo and disposable Samba checks pass; broader encoding/editor compatibility remains open. |
| EditBookmarks | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| EditCredentials | partial | src/remote.rs, tests/webdav_backend.rs — Saved profiles, keychain integration and partial adapters; live services and keychain UI need validation. |
| Email | missing | — |
| EmptyTrash | missing | — |
| ExploreBookmarks | missing | — |
| Find | partial | src/desktop.rs, src/remote.rs, tests/features_05.rs::remote_search_and_guarded_save_use_actual_storage_operations, tests/features_05.rs::smb_live_list_stream_upload_download_rename_edit_search_delete — Local and cancellable remote filename/content search; filesystem and live disposable Samba coverage. Full upstream search options remain open. |
| FocusNext | missing | — |
| FocusPrevious | missing | — |
| GarbageCollect | inapplicable | src/main.rs — This upstream action explicitly triggers the JVM garbage collector; the Rust application does not run a JVM. |
| GoBack | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| GoForward | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| GoToDocumentation | missing | — |
| GoToForums | missing | — |
| GoToHome | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| GoToParent | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| GoToParentInBothPanels | missing | — |
| GoToParentInOtherPanel | missing | — |
| GoToRoot | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| GoToWebsite | missing | — |
| InternalEdit | partial | src/app.rs, tests/file_manager.rs::text_editor_refuses_external_changes_and_preserves_content — Local UTF-8 text editing only. |
| InternalView | partial | src/viewer.rs, tests/desktop_tools.rs::image_and_hex_viewers_load_real_files, tests/desktop_tools.rs::pdf_viewer_renders_and_navigates_pages — Bounded input; format edge cases, text encodings, image animation and full PDF compatibility remain open. |
| InvertSelection | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| LocalCopy | partial | src/transfer.rs, src/remote.rs, src/app.rs, tests/transfer_conflicts.rs, tests/features.rs — Local recursive conflict preview and per-file choices, recoverable replacements and optional folder merge. Remote top-level choices tested with filesystem adapter; live services and cross-platform behavior pending. |
| MarkAll | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| MarkExtension | missing | — |
| MarkGroup | missing | — |
| MarkNextBlock | missing | — |
| MarkNextPage | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| MarkNextRow | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| MarkPreviousBlock | missing | — |
| MarkPreviousPage | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| MarkPreviousRow | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| MarkSelectedFile | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| MarkToFirstRow | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| MarkToLastRow | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| MaximizeWindow | missing | — |
| MinimizeWindow | missing | — |
| Mkdir | partial | src/operations.rs, src/app.rs — Core and UI exist; full platform behavior is not validated. |
| Mkfile | partial | src/operations.rs, src/app.rs — Core and UI exist; full platform behavior is not validated. |
| Move | partial | src/transfer.rs, src/remote.rs, src/app.rs, tests/transfer_conflicts.rs, tests/features.rs — Local recursive conflict preview and per-file choices, recoverable replacements and optional folder merge. Remote top-level choices tested with filesystem adapter; live services and cross-platform behavior pending. |
| MoveTabToOtherPanel | missing | — |
| NewTab | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| NewWindow | missing | — |
| NextTab | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| Open | partial | src/app.rs, parity/VALIDATION.md — Core local navigation/open workflow exists; full upstream action semantics and cross-platform behavior remain incomplete. |
| OpenAs | missing | — |
| OpenCommandPrompt | partial | src/terminal.rs, src/app.rs — PTY/VT100 screen ran a shell command in the packaged macOS app; full terminal compatibility and other platforms remain unverified. |
| OpenInBothPanels | missing | — |
| OpenInNewTab | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| OpenInOtherPanel | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| OpenNatively | partial | src/app.rs, parity/VALIDATION.md — Core local navigation/open workflow exists; full upstream action semantics and cross-platform behavior remain incomplete. |
| OpenTrash | missing | — |
| OpenURLInBrowser | missing | — |
| Pack | partial | src/archive.rs, src/archive_tools.rs, src/app.rs, tests/features.rs, tests/file_manager.rs, tests/extended_archives.rs — ZIP and 7z AES encryption, local ZIP editing and nested/remote archive browsing. Extract to new folders; archive links, non-ZIP mutation and broad format fixtures remain open. |
| PasteClipboardFiles | partial | src/native_files.rs, src/app.rs, tests/features.rs — Native local file clipboard plus remote pane selections. Mac copy/cut, Unicode paths and external clipboard changes pass a real desktop round-trip. Windows/Linux interoperability requires validation. |
| PermanentDelete | partial | src/operations.rs, src/app.rs — Core and UI exist; full platform behavior is not validated. |
| PopupLeftDriveButton | partial | src/native_files.rs, src/app.rs — Mounted-drive sidebar and clickable local breadcrumbs provide navigation. Exact upstream quick-list behavior and cross-platform interactions remain open. |
| PopupRightDriveButton | partial | src/native_files.rs, src/app.rs — Mounted-drive sidebar and clickable local breadcrumbs provide navigation. Exact upstream quick-list behavior and cross-platform interactions remain open. |
| PreviousTab | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| QuickFind | missing | — |
| Quit | partial | src/app.rs — Implemented UI and close protection; full lifecycle validation pending. |
| RecallNextWindow | missing | — |
| RecallPreviousWindow | missing | — |
| RecallWindow10 | missing | — |
| RecallWindow1 | missing | — |
| RecallWindow2 | missing | — |
| RecallWindow3 | missing | — |
| RecallWindow4 | missing | — |
| RecallWindow5 | missing | — |
| RecallWindow6 | missing | — |
| RecallWindow7 | missing | — |
| RecallWindow8 | missing | — |
| RecallWindow9 | missing | — |
| Redo | partial | src/history.rs, tests/features_05.rs::text_save_undo_redo_survives_history_reload, tests/features_05.rs::redo_refuses_external_edits — Guarded Redo for journaled local operations; macOS native menu smoke check. Remote/universal Redo and Windows interaction remain open. |
| Refresh | partial | src/app.rs, src/model.rs — Manual refresh and automatic macOS file-change notifications smoke tested; other platforms and remote refresh behavior remain unverified. |
| Rename | partial | src/operations.rs, src/remote.rs, src/archive_tools.rs, src/app.rs, tests/features.rs — Local, ZIP and remote rename implemented. Remote copy/check/delete validated through filesystem adapter; live services and full platform behavior pending. |
| ReportBug | missing | — |
| RevealInDesktop | missing | — |
| ReverseSortOrder | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| Save | partial | src/app.rs, tests/file_manager.rs::text_editor_refuses_external_changes_and_preserves_content — Local UTF-8 text editing only. |
| SelectFirstRow | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| SelectLastRow | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| SelectNextBlock | missing | — |
| SelectNextPage | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| SelectNextRow | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| SelectPreviousBlock | missing | — |
| SelectPreviousPage | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| SelectPreviousRow | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| SetSameFolder | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| SetTabTitle | missing | — |
| ShowAbout | partial | src/app.rs — Implemented UI and close protection; full lifecycle validation pending. |
| ShowBookmarksQL | missing | — |
| ShowDebugConsole | missing | — |
| ShowFilePopupMenu | missing | — |
| ShowFileProperties | partial | src/operations.rs, src/app.rs — Core and UI exist; full platform behavior is not validated. |
| ShowInEnclosingFolder | partial | src/native_actions.rs, src/app.rs — Native Reveal actions are wired for local files; platform-specific launch behavior needs broader interactive validation. |
| ShowKeyboardShortcuts | partial | src/preferences.rs, src/app.rs, tests/desktop_tools.rs::preferences_validate_custom_shortcuts_and_detect_collisions — Common preferences only; all upstream settings and shortcut actions are not ported. |
| ShowParentFoldersQL | partial | src/native_files.rs, src/app.rs — Mounted-drive sidebar and clickable local breadcrumbs provide navigation. Exact upstream quick-list behavior and cross-platform interactions remain open. |
| ShowPreferences | partial | src/preferences.rs, src/feature_ui.rs, src/app.rs, tests/features.rs — Desktop preferences now include language, persistent column customization and toolbar action order. Full upstream preferences and cross-platform behavior remain incomplete. |
| ShowRecentExecutedFilesQL | missing | — |
| ShowRecentLocationsQL | missing | — |
| ShowRootFoldersQL | missing | — |
| ShowServerConnections | partial | src/remote.rs, tests/webdav_backend.rs — Saved profiles, keychain integration and partial adapters; live services and keychain UI need validation. |
| ShowTabsQL | missing | — |
| SortByDate | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| SortByExtension | partial | src/preferences.rs, src/app.rs, tests/features.rs — Configurable persistent columns and sorting through headers/settings; exact upstream actions and platform interaction validation remain open. |
| SortByGroup | missing | — |
| SortByName | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| SortByOwner | missing | — |
| SortByPermissions | missing | — |
| SortBySize | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| SplitEqually | missing | — |
| SplitFile | implemented | src/desktop.rs, tests/desktop_tools.rs::split_and_combine_round_trip_every_chunk_boundary |
| SplitHorizontally | missing | — |
| SplitVertically | missing | — |
| Stop | partial | src/app.rs, src/history.rs, tests/features.rs — Queue and active operation cancellation with persistent outcomes. Cancellation is cooperative; not all native/provider calls are interruptible. |
| SwapFolders | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| SwitchActiveTable | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| ToggleAutoSize | missing | — |
| ToggleCommandBar | partial | src/preferences.rs, src/app.rs, tests/desktop_tools.rs::preferences_validate_custom_shortcuts_and_detect_collisions — Common preferences only; all upstream settings and shortcut actions are not ported. |
| ToggleDateColumn | partial | src/preferences.rs, src/app.rs, tests/features.rs — Configurable persistent columns and sorting through headers/settings; exact upstream actions and platform interaction validation remain open. |
| ToggleExtensionColumn | partial | src/preferences.rs, src/app.rs, tests/features.rs — Configurable persistent columns and sorting through headers/settings; exact upstream actions and platform interaction validation remain open. |
| ToggleGroupColumn | missing | — |
| ToggleHiddenFiles | partial | src/preferences.rs, src/app.rs, tests/desktop_tools.rs::preferences_validate_custom_shortcuts_and_detect_collisions — Common preferences only; all upstream settings and shortcut actions are not ported. |
| ToggleLockTab | missing | — |
| ToggleOwnerColumn | missing | — |
| ToggleShowParentFolder | missing | — |
| TogglePermissionsColumn | partial | src/preferences.rs, src/app.rs, tests/features.rs — Configurable persistent columns and sorting through headers/settings; exact upstream actions and platform interaction validation remain open. |
| ToggleShowFoldersFirst | partial | src/preferences.rs, src/app.rs, tests/desktop_tools.rs::preferences_validate_custom_shortcuts_and_detect_collisions — Common preferences only; all upstream settings and shortcut actions are not ported. |
| ToggleSizeColumn | partial | src/preferences.rs, src/app.rs, tests/features.rs — Configurable persistent columns and sorting through headers/settings; exact upstream actions and platform interaction validation remain open. |
| ToggleStatusBar | partial | src/preferences.rs, src/app.rs, tests/desktop_tools.rs::preferences_validate_custom_shortcuts_and_detect_collisions — Common preferences only; all upstream settings and shortcut actions are not ported. |
| ToggleTerminal | partial | src/terminal.rs, src/app.rs — PTY and VT100 screen implemented; full terminal compatibility needs validation. |
| ToggleToolBar | partial | src/preferences.rs, src/feature_ui.rs, src/app.rs, tests/features.rs — Desktop preferences now include language, persistent column customization and toolbar action order. Full upstream preferences and cross-platform behavior remain incomplete. |
| ToggleTree | missing | — |
| ToggleUseSinglePanel | partial | src/preferences.rs, src/app.rs, tests/desktop_tools.rs::preferences_validate_custom_shortcuts_and_detect_collisions — Common preferences only; all upstream settings and shortcut actions are not ported. |
| UnmarkAll | partial | src/app.rs, src/model.rs — Core workflow implemented; exact upstream behavior and full keyboard coverage remain to be validated. |
| UnmarkGroup | missing | — |
| Unpack | partial | src/archive.rs, src/archive_tools.rs, src/app.rs, tests/features.rs, tests/file_manager.rs, tests/extended_archives.rs — ZIP and 7z AES encryption, local ZIP editing and nested/remote archive browsing. Extract to new folders; archive links, non-ZIP mutation and broad format fixtures remain open. |
| Undo | partial | src/history.rs, tests/features_05.rs, tests/features.rs — Guarded local Undo/Redo includes text saves, permissions, timestamps and synchronization. Remote Undo, permanent deletion and universal upstream coverage remain open. |
| View | partial | src/viewer.rs, tests/desktop_tools.rs::image_and_hex_viewers_load_real_files, tests/desktop_tools.rs::pdf_viewer_renders_and_navigates_pages — Bounded input; format edge cases, text encodings, image animation and full PDF compatibility remain open. |
| mucommander-protocol-api | missing | — |
| mucommander-viewer-api | missing | — |
| mucommander-format-ar | partial | src/archive.rs, src/archive_tools.rs, tests/desktop_tools.rs, tests/extended_archives.rs, tests/features.rs — Format-dependent read/write support; encrypted ZIP/7z, nested browsing and local ZIP mutation implemented. NRG/CPIO/RPM and additional format edge cases still need fixtures. |
| mucommander-format-bzip2 | partial | src/archive.rs, src/archive_tools.rs, tests/desktop_tools.rs, tests/extended_archives.rs, tests/features.rs — Format-dependent read/write support; encrypted ZIP/7z, nested browsing and local ZIP mutation implemented. NRG/CPIO/RPM and additional format edge cases still need fixtures. |
| mucommander-format-cpio | partial | src/archive.rs, src/archive_tools.rs, tests/desktop_tools.rs, tests/extended_archives.rs, tests/features.rs — Format-dependent read/write support; encrypted ZIP/7z, nested browsing and local ZIP mutation implemented. NRG/CPIO/RPM and additional format edge cases still need fixtures. |
| mucommander-format-gzip | partial | src/archive.rs, src/archive_tools.rs, tests/desktop_tools.rs, tests/extended_archives.rs, tests/features.rs — Format-dependent read/write support; encrypted ZIP/7z, nested browsing and local ZIP mutation implemented. NRG/CPIO/RPM and additional format edge cases still need fixtures. |
| mucommander-format-iso | partial | src/archive.rs, src/archive_tools.rs, tests/desktop_tools.rs, tests/extended_archives.rs, tests/features.rs — Format-dependent read/write support; encrypted ZIP/7z, nested browsing and local ZIP mutation implemented. NRG/CPIO/RPM and additional format edge cases still need fixtures. |
| mucommander-format-libguestfs | missing | — |
| mucommander-format-lst | missing | — |
| mucommander-format-rar | partial | src/archive.rs, src/archive_tools.rs, tests/desktop_tools.rs, tests/extended_archives.rs, tests/features.rs — Format-dependent read/write support; encrypted ZIP/7z, nested browsing and local ZIP mutation implemented. NRG/CPIO/RPM and additional format edge cases still need fixtures. |
| mucommander-format-sevenzip | partial | src/archive.rs, src/archive_tools.rs, tests/desktop_tools.rs, tests/extended_archives.rs, tests/features.rs — Format-dependent read/write support; encrypted ZIP/7z, nested browsing and local ZIP mutation implemented. NRG/CPIO/RPM and additional format edge cases still need fixtures. |
| mucommander-format-tar | partial | src/archive.rs, src/archive_tools.rs, tests/desktop_tools.rs, tests/extended_archives.rs, tests/features.rs — Format-dependent read/write support; encrypted ZIP/7z, nested browsing and local ZIP mutation implemented. NRG/CPIO/RPM and additional format edge cases still need fixtures. |
| mucommander-format-xz | partial | src/archive.rs, src/archive_tools.rs, tests/desktop_tools.rs, tests/extended_archives.rs, tests/features.rs — Format-dependent read/write support; encrypted ZIP/7z, nested browsing and local ZIP mutation implemented. NRG/CPIO/RPM and additional format edge cases still need fixtures. |
| mucommander-format-zip | partial | src/archive.rs, src/archive_tools.rs, tests/desktop_tools.rs, tests/extended_archives.rs, tests/features.rs — Format-dependent read/write support; encrypted ZIP/7z, nested browsing and local ZIP mutation implemented. NRG/CPIO/RPM and additional format edge cases still need fixtures. |
| mucommander-format-rpm | partial | src/archive.rs, src/archive_tools.rs, tests/desktop_tools.rs, tests/extended_archives.rs, tests/features.rs — Format-dependent read/write support; encrypted ZIP/7z, nested browsing and local ZIP mutation implemented. NRG/CPIO/RPM and additional format edge cases still need fixtures. |
| mucommander-protocol-adb | missing | — |
| mucommander-protocol-ftp | partial | src/remote.rs, tests/desktop_tools.rs::remote_backend_contract_upload_download_list_and_conflicts — Adapter implemented; not validated against a live instance of this service. HDFS uses WebHDFS; SFTP uses key/agent authentication. |
| mucommander-protocol-dropbox | partial | src/oauth.rs, src/remote.rs, tests/desktop_tools.rs::remote_backend_contract_upload_download_list_and_conflicts, tests/features.rs — Browser OAuth with PKCE, refresh/reconnect and remote file workflows implemented. Protocol tests use loopback fixtures; live provider registration/account validation pending. |
| mucommander-protocol-gcs | partial | src/remote.rs, tests/desktop_tools.rs::remote_backend_contract_upload_download_list_and_conflicts — Adapter implemented; not validated against a live instance of this service. HDFS uses WebHDFS; SFTP uses key/agent authentication. |
| mucommander-protocol-gdrive | partial | src/oauth.rs, src/remote.rs, tests/desktop_tools.rs::remote_backend_contract_upload_download_list_and_conflicts, tests/features.rs — Browser OAuth with PKCE, refresh/reconnect and remote file workflows implemented. Protocol tests use loopback fixtures; live provider registration/account validation pending. |
| mucommander-protocol-onedrive | partial | src/oauth.rs, src/remote.rs, tests/desktop_tools.rs::remote_backend_contract_upload_download_list_and_conflicts, tests/features.rs — Browser OAuth with PKCE, refresh/reconnect and remote file workflows implemented. Protocol tests use loopback fixtures; live provider registration/account validation pending. |
| mucommander-protocol-hadoop | partial | src/remote.rs, tests/desktop_tools.rs::remote_backend_contract_upload_download_list_and_conflicts — Adapter implemented; not validated against a live instance of this service. HDFS uses WebHDFS; SFTP uses key/agent authentication. |
| mucommander-protocol-nfs | missing | — |
| mucommander-protocol-http | partial | src/remote.rs, tests/desktop_tools.rs::remote_backend_contract_upload_download_list_and_conflicts — Adapter implemented; not validated against a live instance of this service. HDFS uses WebHDFS; SFTP uses key/agent authentication. |
| mucommander-protocol-ovirt | missing | — |
| mucommander-protocol-registry | missing | — |
| mucommander-protocol-s3 | partial | src/remote.rs, tests/desktop_tools.rs::remote_backend_contract_upload_download_list_and_conflicts — Adapter implemented; not validated against a live instance of this service. HDFS uses WebHDFS; SFTP uses key/agent authentication. |
| mucommander-protocol-sftp | partial | src/remote.rs, tests/desktop_tools.rs::remote_backend_contract_upload_download_list_and_conflicts, tests/desktop_tools.rs::remote_service_choices_match_platform_support — SFTP uses OpenSSH key/agent authentication on macOS/Linux; the transport is Unix-only and Windows reports this limitation. Platform service choices are tested; live SFTP remains unvalidated. |
| mucommander-protocol-smb | partial | src/smb.rs, tests/features_05.rs::smb_live_list_stream_upload_download_rename_edit_search_delete — Direct pure Rust SMB2/3 listing, streaming transfers, rename, editing, search and deletion validated against disposable Samba from macOS/Linux. Broader server/platform interoperability remains open. |
| mucommander-protocol-vsphere | missing | — |
| mucommander-viewer-text | partial | src/viewer.rs, tests/desktop_tools.rs — Bounded renderer/viewer implemented; complete format and interaction compatibility remains unverified. |
| mucommander-viewer-image | partial | src/viewer.rs, tests/desktop_tools.rs — Bounded renderer/viewer implemented; complete format and interaction compatibility remains unverified. |
| mucommander-viewer-pdf | partial | src/viewer.rs, tests/desktop_tools.rs — Bounded renderer/viewer implemented; complete format and interaction compatibility remains unverified. |
| mucommander-viewer-binary | partial | src/viewer.rs, tests/desktop_tools.rs — Bounded renderer/viewer implemented; complete format and interaction compatibility remains unverified. |
