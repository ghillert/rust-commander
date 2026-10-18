//! Bounded muCommander XML import. Parsing never resolves external entities or runs commands.
use crate::{
    filesystem::Location,
    settings::{Bookmark, Settings},
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportedCommand {
    pub alias: String,
    pub arguments: Vec<String>,
}
impl ImportedCommand {
    pub fn parse(alias: &str, value: &str) -> Result<Self> {
        ensure!(
            !alias.trim().is_empty() && alias.len() <= 80,
            "Invalid command alias"
        );
        let arguments = shlex::split(value).context("Unbalanced command quotes")?;
        ensure!(
            !arguments.is_empty() && arguments.len() <= 128,
            "Invalid command arguments"
        );
        ensure!(
            !arguments.iter().any(|a| matches!(
                a.as_str(),
                "|" | "||" | "&&" | ";" | ">" | ">>" | "<"
            ) || a.contains(['`', '\n', '\0'])
                || a.contains("$(")),
            "Shell expressions are unsupported"
        );
        for arg in &arguments {
            let mut rest = arg.as_str();
            while let Some(i) = rest.find('$') {
                rest = &rest[i + 1..];
                let key = rest
                    .chars()
                    .next()
                    .context("Incomplete command placeholder")?;
                ensure!(
                    matches!(key, 'f' | 'p' | 'n' | 'b' | 'd'),
                    "Unsupported command placeholder"
                );
                rest = &rest[key.len_utf8()..];
            }
        }
        ensure!(
            !arguments[0].contains('$'),
            "Command program cannot contain placeholders"
        );
        Ok(Self {
            alias: alias.trim().into(),
            arguments,
        })
    }
    pub fn expand(&self, files: &[PathBuf], folder: &Path) -> Result<Vec<String>> {
        let mut output = Vec::new();
        for arg in &self.arguments {
            if arg == "$f" {
                output.extend(files.iter().map(|p| p.to_string_lossy().into_owned()));
                continue;
            }
            let file = files.first().map(PathBuf::as_path).unwrap_or(folder);
            let values = [
                ('p', file.to_string_lossy().into_owned()),
                (
                    'n',
                    file.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                ),
                (
                    'b',
                    file.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                ),
                ('d', folder.to_string_lossy().into_owned()),
            ];
            ensure!(
                !arg.contains("$f"),
                "$f must be a separate argument so multiple files stay separate"
            );
            let mut expanded = String::new();
            let mut chars = arg.chars();
            while let Some(c) = chars.next() {
                if c == '$' {
                    let key = chars.next().context("Incomplete command placeholder")?;
                    let (_, value) = values
                        .iter()
                        .find(|(k, _)| *k == key)
                        .context("Unknown command placeholder")?;
                    expanded.push_str(value);
                } else {
                    expanded.push(c);
                }
            }
            output.push(expanded);
        }
        Ok(output)
    }
}

#[derive(Clone, Default)]
pub struct Preview {
    pub bookmarks: Vec<Bookmark>,
    pub shortcuts: BTreeMap<String, String>,
    pub alternate_shortcuts: BTreeMap<String, String>,
    pub commands: Vec<ImportedCommand>,
    pub preferences: BTreeMap<String, String>,
    pub unsupported: Vec<String>,
}
#[derive(Default)]
struct Node {
    name: String,
    attributes: BTreeMap<String, String>,
    text: String,
    children: Vec<Node>,
}
fn parse(path: &Path) -> Result<Node> {
    let metadata = fs::metadata(path)?;
    ensure!(
        metadata.len() <= 8 * 1024 * 1024,
        "XML import is limited to 8 MiB per file"
    );
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 8 * 1024 * 1024,
        "XML input grew beyond the import limit"
    );
    let mut reader = quick_xml::Reader::from_reader(bytes.as_slice());
    let mut stack = vec![Node::default()];
    let mut count = 0;
    loop {
        use quick_xml::events::Event;
        match reader.read_event()? {
            Event::Start(e) => {
                count += 1;
                ensure!(
                    count <= 20_000 && stack.len() < 64,
                    "XML import structure is too large"
                );
                let node = Node {
                    name: String::from_utf8(e.name().as_ref().to_vec())?,
                    attributes: e
                        .attributes()
                        .map(|a| {
                            let a = a?;
                            Ok((
                                String::from_utf8(a.key.as_ref().to_vec())?,
                                a.decode_and_unescape_value(reader.decoder())?.into_owned(),
                            ))
                        })
                        .collect::<Result<_>>()?,
                    ..Default::default()
                };
                stack.push(node);
            }
            Event::Empty(e) => {
                count += 1;
                ensure!(
                    count <= 20_000 && stack.len() < 64,
                    "XML import structure is too large"
                );
                let node = Node {
                    name: String::from_utf8(e.name().as_ref().to_vec())?,
                    attributes: e
                        .attributes()
                        .map(|a| {
                            let a = a?;
                            Ok((
                                String::from_utf8(a.key.as_ref().to_vec())?,
                                a.decode_and_unescape_value(reader.decoder())?.into_owned(),
                            ))
                        })
                        .collect::<Result<_>>()?,
                    ..Default::default()
                };
                stack.last_mut().unwrap().children.push(node);
            }
            Event::End(_) => {
                ensure!(stack.len() > 1, "Malformed XML");
                let node = stack.pop().unwrap();
                stack.last_mut().unwrap().children.push(node);
            }
            Event::Text(e) => {
                let value = e.xml_content()?;
                stack
                    .last_mut()
                    .unwrap()
                    .text
                    .push_str(&quick_xml::escape::unescape(&value)?);
            }
            Event::CData(e) => stack.last_mut().unwrap().text.push_str(&e.decode()?),
            Event::DocType(_) => bail!("XML document types are unsupported"),
            Event::GeneralRef(e) => {
                let decoded = e.decode()?;
                let character = if let Some(c) = e.resolve_char_ref()? {
                    c
                } else {
                    match decoded.as_ref() {
                        "amp" => '&',
                        "lt" => '<',
                        "gt" => '>',
                        "quot" => '"',
                        "apos" => '\'',
                        _ => bail!("Custom XML entities are unsupported"),
                    }
                };
                stack.last_mut().unwrap().text.push(character);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    ensure!(stack.len() == 1, "Incomplete XML");
    Ok(stack.pop().unwrap())
}
fn child<'a>(node: &'a Node, name: &str) -> Option<&'a str> {
    node.children
        .iter()
        .find(|c| c.name == name)
        .map(|c| c.text.trim())
}
fn walk(node: &Node, prefix: &str, f: &mut impl FnMut(&Node, &str)) {
    let key = if prefix.is_empty() {
        node.name.clone()
    } else {
        format!("{prefix}.{}", node.name)
    };
    f(node, &key);
    for child in &node.children {
        walk(child, &key, f);
    }
}
fn action(id: &str) -> Option<&'static str> {
    Some(match id {
        "ToggleAutoSize" => "AutoSizeColumns",
        "OpenInBothPanels" => "OpenBoth",
        "GoToParentInBothPanels" => "ParentBoth",
        "GoToParentInOtherPanel" => "ParentOther",
        "MoveTabToOtherPanel" => "MoveTabOther",
        "ShowFilePopupMenu" => "FileContextMenu",
        "ShowBookmarksQL" => "BookmarksList",
        "ShowRecentExecutedFilesQL" => "RecentFiles",
        "ShowRootFoldersQL" => "RootsList",
        "ShowTabsQL" => "TabsList",
        "GoToParent" => "Parent",
        "GoBack" => "Back",
        "GoForward" => "Forward",
        "Mkdir" => "Mkdir",
        "Delete" => "Trash",
        "InternalView" => "View",
        "InternalEdit" => "Edit",
        "AddBookmark" => "Bookmark",
        "ShowPreferences" => "Preferences",
        "ShowServerConnections" => "Connections",
        "SwitchActiveTable" => "SwitchPane",
        "ToggleLockTab" => "LockTab",
        "SetTabTitle" => "TabTitle",
        "MarkGroup" => "SelectPattern",
        "UnmarkGroup" => "UnselectPattern",
        "MarkExtension" => "SelectExtension",
        "ShowRecentLocationsQL" => "RecentLocations",
        "SplitEqually" => "EqualPanes",
        "SplitHorizontally" => "Stacked",
        "SplitVertically" => "SideBySide",
        "Copy" => "Copy",
        "Move" => "Move",
        "Rename" => "Rename",
        "NewTab" => "NewTab",
        "CloseTab" => "CloseTab",
        "Undo" => "Undo",
        "Redo" => "Redo",
        "Find" => "Search",
        "QuickFind" => "QuickFind",
        "OpenTrash" => "OpenTrash",
        "EmptyTrash" => "EmptyTrash",
        "NewWindow" => "NewWindow",
        "Quit" => "Quit",
        _ => return None,
    })
}
pub fn preview(directory: &Path) -> Result<Preview> {
    ensure!(
        directory.is_dir(),
        "Choose the muCommander configuration directory"
    );
    let mut result = Preview::default();
    let mut found = false;
    for file in [
        "bookmarks.xml",
        "preferences.xml",
        "action_keymap.xml",
        "keymap.xml",
        "shortcuts.xml",
        "commands.xml",
    ] {
        let path = directory.join(file);
        if !path.exists() {
            continue;
        }
        found = true;
        let tree = parse(&path).with_context(|| format!("Cannot import {file}"))?;
        walk(&tree, "", &mut |node, key| {
            if node.name == "bookmark" {
                let name =
                    child(node, "name").or_else(|| node.attributes.get("name").map(String::as_str));
                let value = child(node, "location")
                    .or_else(|| child(node, "url"))
                    .or_else(|| node.attributes.get("location").map(String::as_str));
                if let (Some(name), Some(value)) = (name, value) {
                    let location = if value.starts_with("file:") {
                        url::Url::parse(value)
                            .ok()
                            .filter(|u| u.username().is_empty() && u.password().is_none())
                            .and_then(|u| u.to_file_path().ok())
                    } else {
                        let p = PathBuf::from(value);
                        p.is_absolute().then_some(p)
                    };
                    if let Some(path) = location {
                        result.bookmarks.push(Bookmark {
                            name: name.into(),
                            location: Location::Local(path),
                        });
                    } else {
                        result.unsupported.push(format!("Bookmark {name}: remote or credential-bearing URL requires manual connection mapping"));
                    }
                }
            } else if node.name == "command" {
                if let (Some(alias), Some(value)) =
                    (node.attributes.get("alias"), node.attributes.get("value"))
                {
                    if node.attributes.get("type").is_some_and(|t| t != "system") {
                        result.unsupported.push(format!(
                            "Command {alias}: only argument-based system commands are supported"
                        ));
                        return;
                    }
                    match ImportedCommand::parse(alias, value) {
                        Ok(c) => result.commands.push(c),
                        Err(e) => result.unsupported.push(format!("Command {alias}: {e}")),
                    }
                }
            } else if matches!(node.name.as_str(), "action" | "shortcut" | "mapping") {
                let legacy = node
                    .attributes
                    .get("class")
                    .and_then(|s| s.rsplit('.').next())
                    .and_then(|s| s.strip_suffix("Action"));
                let id = node
                    .attributes
                    .get("id")
                    .or_else(|| node.attributes.get("action"))
                    .map(String::as_str)
                    .or(legacy);
                if let Some(id) = id {
                    let target = action(id).map(str::to_owned).or_else(|| {
                        crate::app::slint_ui::supported_commands()
                            .into_iter()
                            .find(|name| name == id)
                    });
                    if let Some(target) = target {
                        for (attribute, alternate) in [
                            ("keystroke", false),
                            ("alt_keystroke", true),
                            ("key", false),
                            ("shortcut", false),
                        ] {
                            if let Some(key) = node.attributes.get(attribute) {
                                let binding = key
                                    .replace("pressed ", "")
                                    .replace("ctrl ", "Command+")
                                    .replace("meta ", "Command+")
                                    .replace("shift ", "Shift+")
                                    .replace("alt ", "Alt+")
                                    .replace(' ', "+");
                                if crate::preferences::shortcut(&binding).is_ok() {
                                    if alternate {
                                        result.alternate_shortcuts.insert(target.clone(), binding);
                                    } else {
                                        result.shortcuts.insert(target.clone(), binding);
                                    }
                                } else {
                                    result
                                        .unsupported
                                        .push(format!("Shortcut {id}: unsupported binding"));
                                }
                            }
                        }
                    } else {
                        result
                            .unsupported
                            .push(format!("Shortcut {id}: unsupported action"));
                    }
                }
            } else if node.children.is_empty()
                && !node.text.trim().is_empty()
                && file == "preferences.xml"
            {
                let key = key.trim_start_matches('.').to_owned();
                let canonical = if key.ends_with("command_bar.visible") {
                    Some("show_command_bar")
                } else if key.ends_with("status_bar.visible") {
                    Some("show_status_bar")
                } else if key.ends_with("toolbar.visible") {
                    Some("show_toolbar")
                } else if node.name == "show_folders_first" {
                    Some("folders_first")
                } else if matches!(
                    node.name.as_str(),
                    "show_hidden_files"
                        | "show_hidden"
                        | "show_toolbar"
                        | "show_status_bar"
                        | "show_command_bar"
                        | "folders_first"
                        | "font_size"
                ) {
                    Some(node.name.as_str())
                } else {
                    None
                };
                if let Some(name) = canonical {
                    result
                        .preferences
                        .insert(name.into(), node.text.trim().into());
                } else {
                    result
                        .unsupported
                        .push(format!("Preference {key}: unsupported"));
                }
            }
        });
    }
    ensure!(found, "No supported muCommander XML files found");
    ensure!(
        result.bookmarks.len() + result.commands.len() + result.shortcuts.len() <= 5000,
        "Too many imported items"
    );
    Ok(result)
}
impl Preview {
    pub fn apply(&self, settings: &mut Settings, import_preferences: bool) -> Result<()> {
        let mut candidate: Settings = serde_json::from_slice(&serde_json::to_vec(settings)?)?;
        for b in &self.bookmarks {
            if !candidate
                .bookmarks
                .iter()
                .any(|old| old.name == b.name || old.location == b.location)
            {
                candidate.bookmarks.push(b.clone());
            }
        }
        for (id, key) in &self.shortcuts {
            if !candidate.preferences.shortcuts.contains_key(id)
                || candidate.preferences.shortcuts.get(id)
                    == crate::preferences::Preferences::default().shortcuts.get(id)
            {
                candidate
                    .preferences
                    .shortcuts
                    .insert(id.clone(), key.clone());
            }
        }
        for (id, key) in &self.alternate_shortcuts {
            if !candidate.preferences.alternate_shortcuts.contains_key(id)
                || candidate.preferences.alternate_shortcuts.get(id)
                    == crate::preferences::Preferences::default()
                        .alternate_shortcuts
                        .get(id)
            {
                candidate
                    .preferences
                    .alternate_shortcuts
                    .insert(id.clone(), key.clone());
            }
        }
        for command in &self.commands {
            if !candidate
                .imported_commands
                .iter()
                .any(|c| c.alias == command.alias)
            {
                candidate.imported_commands.push(command.clone());
            }
        }
        if import_preferences {
            for (key, value) in &self.preferences {
                let boolean = || -> Result<bool> {
                    match value.as_str() {
                        "true" => Ok(true),
                        "false" => Ok(false),
                        _ => bail!("Invalid imported boolean"),
                    }
                };
                match key.as_str() {
                    "show_hidden_files" | "show_hidden" => candidate.show_hidden = boolean()?,
                    "show_toolbar" => candidate.preferences.toolbar = boolean()?,
                    "show_status_bar" => candidate.preferences.status_bar = boolean()?,
                    "show_command_bar" => candidate.preferences.command_bar = boolean()?,
                    "folders_first" => candidate.preferences.folders_first = boolean()?,
                    "font_size" => candidate.preferences.font_size = value.parse()?,
                    _ => {}
                }
            }
        }
        crate::preferences::validate(&candidate.preferences)?;
        *settings = candidate;
        Ok(())
    }
}

/// Apply only a reviewed preview, preserve the current file in a private backup,
/// and refuse an intervening settings change. Source XML is never modified.
pub fn commit(
    path: &Path,
    current: &Settings,
    preview: &Preview,
    import_preferences: bool,
    expected: Option<&[u8]>,
) -> Result<(Settings, Option<PathBuf>)> {
    use std::io::Write;
    let actual = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    ensure!(
        actual.as_deref() == expected,
        "Settings changed since the preview. Preview the import again."
    );
    if let Some(bytes) = &actual {
        let _: Settings = serde_json::from_slice(bytes)
            .context("Current settings are invalid; repair them before import")?;
    }
    let mut candidate = current.clone();
    preview.apply(&mut candidate, import_preferences)?;
    let directory = path.parent().context("Settings directory is missing")?;
    fs::create_dir_all(directory)?;
    let backup = if let Some(bytes) = &actual {
        let backup = directory.join(format!(
            "settings.before-import-{:016x}.json",
            rand::random::<u64>()
        ));
        let mut options = fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&backup)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Some(backup)
    } else {
        None
    };
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&serde_json::to_vec_pretty(&candidate)?)?;
    file.as_file().sync_all()?;
    ensure!(
        match fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        }
        .as_deref()
            == expected,
        "Settings changed while preparing the import; backup retained"
    );
    file.persist(path).map_err(|e| e.error)?;
    Ok((candidate, backup))
}
