//! One menu order and grouping for AppKit, Win32 and the shared UI.
use super::Action;

pub(crate) type Command = (&'static str, Action);

pub(crate) struct ActionMenu {
    pub title: &'static str,
    pub groups: Vec<Vec<Command>>,
}

impl ActionMenu {
    /// Separators have no command ID and can never be activated.
    pub fn entries(&self) -> impl Iterator<Item = Option<Command>> + '_ {
        self.groups.iter().enumerate().flat_map(|(index, group)| {
            (index > 0)
                .then_some(None)
                .into_iter()
                .chain(group.iter().copied().map(Some))
        })
    }
}

pub(crate) fn action_menu_definitions() -> Vec<ActionMenu> {
    use Action::*;
    let mut menus = vec![
        ActionMenu {
            title: "File",
            groups: vec![
                vec![
                    ("View", View),
                    ("Edit", Edit),
                    ("Open externally", External),
                    ("Open With…", OpenWith),
                    ("Open As…", OpenAs),
                    ("Edit externally…", ExternalEdit),
                ],
                vec![("New file…", Mkfile), ("New folder…", Mkdir)],
                vec![
                    ("Copy to other pane…", Copy),
                    ("Move to other pane…", Move),
                    ("Rename…", Rename),
                    ("Batch rename…", BatchRename),
                ],
                vec![("Pack files…", Pack), ("Extract archive…", Extract)],
                vec![
                    ("Move to Trash", Trash),
                    ("Permanently delete…", PermanentDelete),
                ],
                vec![
                    ("Email files…", Email),
                    ("Reveal in file manager", Reveal),
                    ("Show package contents", PackageContents),
                ],
                vec![
                    ("Properties", Properties),
                    ("Permissions…", Permissions),
                    ("Ownership and ACL…", OwnershipAcl),
                    ("Modification time…", Modified),
                ],
            ],
        },
        ActionMenu {
            title: "Edit",
            groups: vec![
                vec![("Undo last operation", Undo), ("Redo last operation", Redo)],
                vec![
                    ("Cut files", CutClipboard),
                    ("Copy files", CopyClipboard),
                    ("Paste files", PasteClipboard),
                ],
                vec![
                    ("Copy names", CopyNames),
                    ("Copy names without extensions", CopyBaseNames),
                    ("Copy paths", CopyPaths),
                ],
                vec![("Prepare files for external drag…", NativeDrag)],
            ],
        },
        ActionMenu {
            title: "Mark",
            groups: vec![
                vec![
                    ("Mark / unmark file", MarkSelectedFile),
                    ("Mark all", MarkAll),
                    ("Unmark all", UnmarkAll),
                    ("Invert selection", InvertSelection),
                ],
                vec![
                    ("Select by pattern…", SelectPattern),
                    ("Unselect by pattern…", UnselectPattern),
                    ("Select same extension", SelectExtension),
                ],
                vec![
                    ("Next block", SelectNextBlock),
                    ("Previous block", SelectPreviousBlock),
                    ("Mark next block", MarkNextBlock),
                    ("Mark previous block", MarkPreviousBlock),
                ],
            ],
        },
        ActionMenu {
            title: "View",
            groups: vec![
                vec![("Refresh", Refresh)],
                vec![
                    ("Show hidden files", ToggleHidden),
                    ("Show parent directory row", ToggleParentEntry),
                ],
                vec![
                    ("Show sidebar", ToggleSidebar),
                    ("Directory tree", DirectoryTree),
                    ("Preview pane", Preview),
                    ("Quick Look", QuickLook),
                ],
                vec![
                    ("Side-by-side panes", SideBySide),
                    ("Stacked panes", Stacked),
                    ("Equal pane sizes", EqualPanes),
                ],
                vec![("Fit columns to content", AutoSizeColumns)],
            ],
        },
        ActionMenu {
            title: "Go",
            groups: vec![
                vec![("Back", Back), ("Forward", Forward)],
                vec![
                    ("Go to location…", FocusAddress),
                    ("Home", Home),
                    ("Parent folder", Parent),
                    ("Roots and drives…", RootsList),
                ],
                vec![
                    ("Connections…", Connections),
                    ("Recent locations…", RecentLocations),
                    ("Recently opened files…", RecentFiles),
                    ("Quick lists…", QuickLists),
                ],
                vec![
                    ("Open in new tab", OpenNewTab),
                    ("Open in other pane", OpenOther),
                    ("Open in both panes", OpenBoth),
                ],
                vec![
                    ("Switch pane", SwitchPane),
                    ("Swap panes", Swap),
                    ("Same folder in both panes", SameFolder),
                    ("Parent in other pane", ParentOther),
                    ("Parent in both panes", ParentBoth),
                ],
                vec![
                    ("Open Trash", OpenTrash),
                    ("Open URL in browser…", OpenURLInBrowser),
                ],
            ],
        },
        ActionMenu {
            title: "Bookmarks",
            groups: vec![
                vec![("Add bookmark", Bookmark)],
                vec![
                    ("Bookmarks…", BookmarksList),
                    ("Explore bookmarks…", ExploreBookmarks),
                ],
            ],
        },
        ActionMenu {
            title: "Tools",
            groups: vec![
                vec![
                    ("Find files…", Search),
                    ("Quick find", QuickFind),
                    ("Find next filename", QuickFindNext),
                    ("Find previous filename", QuickFindPrevious),
                ],
                vec![
                    ("Compare folders", Compare),
                    ("Synchronize folders…", Synchronize),
                    ("Checksums", Checksum),
                ],
                vec![
                    ("Archive tools…", ArchiveTools),
                    ("Split file…", Split),
                    ("Combine files…", Combine),
                ],
                vec![
                    ("Transfer queue and history…", Queue),
                    ("Recovery manager…", Recovery),
                    ("Recover editor drafts…", RecoverEditorDrafts),
                ],
                vec![
                    ("Open terminal", Terminal),
                    ("Imported commands…", ImportedCommands),
                ],
                vec![
                    ("Application associations…", Associations),
                    ("Import muCommander settings…", ImportSettings),
                ],
                vec![("Empty Trash…", EmptyTrash)],
            ],
        },
        ActionMenu {
            title: "Window",
            groups: vec![
                vec![
                    ("New tab", NewTab),
                    ("Duplicate tab", DuplicateTab),
                    ("Close tab", CloseTab),
                    ("Close other tabs", CloseOtherTabs),
                    ("Close duplicate tabs", CloseDuplicateTabs),
                ],
                vec![
                    ("Next tab", NextTab),
                    ("Previous tab", PreviousTab),
                    ("Open tabs…", TabsList),
                ],
                vec![
                    ("Lock/unlock tab", LockTab),
                    ("Name tab…", TabTitle),
                    ("Move tab left", MoveTabLeft),
                    ("Move tab right", MoveTabRight),
                    ("Move tab to other pane", MoveTabOther),
                    ("Clone tab to other pane", CloneTab),
                ],
                vec![("New window", NewWindow), ("Close window", CloseWindow)],
                vec![
                    ("Next window", NextWindow),
                    ("Previous window", PreviousWindow),
                    ("Windows…", WindowsList),
                ],
                vec![
                    ("Minimize window", MinimizeWindow),
                    ("Maximize / restore window", MaximizeWindow),
                    ("Bring all windows forward", BringAllToFront),
                ],
                vec![("Workspaces…", Workspaces)],
            ],
        },
        ActionMenu {
            title: "Help",
            groups: vec![
                vec![
                    ("Help", Help),
                    ("Documentation", GoToDocumentation),
                    ("Community support", GoToForums),
                ],
                vec![
                    ("Check for updates…", CheckForUpdates),
                    ("Project website", GoToWebsite),
                    ("Donate…", Donate),
                ],
                vec![
                    ("Report a bug…", ReportBug),
                    ("Diagnostics…", Diagnostics),
                    ("Debug console…", ShowDebugConsole),
                ],
            ],
        },
    ];
    // macOS keeps these in the application menu, as users expect.
    if !cfg!(target_os = "macos") {
        menus
            .iter_mut()
            .find(|menu| menu.title == "Edit")
            .unwrap()
            .groups
            .push(vec![("Settings…", Preferences)]);
        menus[0].groups.push(vec![("Quit Rust Commander", Quit)]);
    }
    menus
}

/// The command catalog needs labels, without presentation-only separators.
pub(crate) fn action_menus() -> Vec<(&'static str, Vec<Command>)> {
    action_menu_definitions()
        .into_iter()
        .map(|menu| (menu.title, menu.groups.into_iter().flatten().collect()))
        .collect()
}
