//! Native launch commands use argument arrays; filenames never pass through a shell.
use anyhow::{Result, bail};
use std::{
    path::{Path, PathBuf},
    process::Command,
};
/// Returns whether `path` is a macOS application bundle directory with an `.app` extension.
/// Other platforms return `false`.
pub fn is_application(path: &Path) -> bool {
    cfg!(target_os = "macos")
        && path.is_dir()
        && path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("app"))
}
/// Asks the native file manager to reveal the path. Finder and Explorer select the entry,
/// while other desktops open its containing directory.
///
/// # Errors
///
/// Returns an error if the platform opener cannot be launched.
pub fn reveal(path: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        Command::new("/usr/bin/open").arg("-R").arg(path).spawn()?;
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer.exe")
            .arg(format!("/select,{}", path.display()))
            .spawn()?;
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        open::that(path.parent().unwrap_or(path))?;
    }
    Ok(())
}
/// Launches the selected application with the file path as an argument. Supports macOS app
/// bundles and desktop launchers on systems using `gio`.
///
/// # Errors
///
/// Returns an error if the application path is relative or missing, or the launch fails.
pub fn open_with(path: &Path, application: &Path) -> Result<()> {
    if !application.is_absolute() || !application.exists() {
        bail!("Choose an existing application with an absolute path");
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("/usr/bin/open")
            .arg("-a")
            .arg(application)
            .arg(path)
            .spawn()?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        if application.extension().is_some_and(|s| s == "desktop") {
            Command::new("gio")
                .arg("launch")
                .arg(application)
                .arg(path)
                .spawn()?;
        } else {
            Command::new(application).arg(path).spawn()?;
        }
    }
    Ok(())
}
/// Starts macOS Quick Look for one or more local files.
///
/// # Errors
///
/// Returns an error for an empty selection, an unsupported platform, or a launcher failure.
pub fn quick_look(paths: &[PathBuf]) -> Result<()> {
    if paths.is_empty() {
        bail!("Select a local file to preview");
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("/usr/bin/qlmanage")
            .arg("-p")
            .args(paths)
            .spawn()?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        bail!("Use the built-in preview pane on this platform")
    }
}

/// Native system Trash actions. Emptying is invoked only after explicit UI confirmation.
pub fn trash_action(empty: bool) -> Result<()> {
    #[cfg(target_os = "macos")]
    let status = Command::new("/usr/bin/osascript")
        .args([
            "-e",
            if empty {
                "tell application \"Finder\" to empty trash"
            } else {
                "tell application \"Finder\" to open trash"
            },
        ])
        .status()?;
    #[cfg(target_os = "windows")]
    {
        if empty {
            let result = unsafe {
                windows_sys::Win32::UI::Shell::SHEmptyRecycleBinW(
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    0,
                )
            };
            if result < 0 {
                bail!("Windows could not empty the Recycle Bin: {result:#x}");
            }
            return Ok(());
        }
        Command::new("explorer.exe")
            .arg("shell:RecycleBinFolder")
            .spawn()?;
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let status = if empty {
        Command::new("gio").args(["trash", "--empty"]).status()?
    } else {
        Command::new("gio").args(["open", "trash:///"]).status()?
    };
    #[cfg(not(target_os = "windows"))]
    if !status.success() {
        bail!("The native Trash action was cancelled or denied (status {status}).");
    }
    #[cfg(not(target_os = "windows"))]
    Ok(())
}
