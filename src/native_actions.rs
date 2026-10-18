//! Native launch commands use argument arrays; filenames never pass through a shell.
use anyhow::{Result, bail};
use std::{
    path::{Path, PathBuf},
    process::Command,
};
pub fn is_application(path: &Path) -> bool {
    cfg!(target_os = "macos")
        && path.is_dir()
        && path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("app"))
}
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
