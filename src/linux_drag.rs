//! A separate GTK event loop keeps native Wayland/X11 drag-and-drop independent of winit.
use anyhow::{Result, bail};
use std::path::PathBuf;
pub fn launch(paths: &[PathBuf]) -> Result<()> {
    if paths.is_empty() {
        bail!("Select files to drag");
    }
    std::process::Command::new(std::env::current_exe()?)
        .arg("--drag-helper")
        .args(paths)
        .spawn()?;
    Ok(())
}
fn file_source(files: &[gtk4::gio::File]) -> gtk4::DragSource {
    use gtk4::{gdk, prelude::*};
    let source = gtk4::DragSource::new();
    source.set_actions(gdk::DragAction::COPY);
    let list = gdk::FileList::from_array(files);
    source.set_content(Some(&gdk::ContentProvider::for_value(&list.to_value())));
    source
}
pub fn run(paths: Vec<PathBuf>) -> i32 {
    use gtk4::{gio, prelude::*};
    let app = gtk4::Application::builder()
        .application_id("io.github.ghillert.rust-commander.drag")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |app| {
        let files = paths.iter().map(gio::File::for_path).collect::<Vec<_>>();
        let names = paths
            .iter()
            .map(|p| p.file_name().unwrap_or_default().to_string_lossy())
            .collect::<Vec<_>>()
            .join("\n");
        let label = gtk4::Label::new(Some(&format!(
            "Drag these files into another application\n\n{names}\n\nClose this window to cancel."
        )));
        label.set_margin_top(24);
        label.set_margin_bottom(24);
        label.set_margin_start(24);
        label.set_margin_end(24);
        label.set_wrap(true);
        let window = gtk4::ApplicationWindow::builder()
            .application(app)
            .title("Rust Commander — Drag files")
            .default_width(440)
            .default_height(220)
            .child(&label)
            .build();
        let source = file_source(&files);
        let weak = window.downgrade();
        source.connect_drag_end(move |_, _, _| {
            if let Some(window) = weak.upgrade() {
                window.close();
            }
        });
        label.add_controller(source);
        window.present();
    });
    app.run_with_args::<&str>(&[]).into()
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires GTK display; run under Xvfb or a Linux desktop"]
    fn gtk_native_drag_source_exposes_file_list() {
        use gtk4::{gdk, gio, prelude::*};
        gtk4::init().unwrap();
        let source = super::file_source(&[gio::File::for_path("/tmp/file with spaces.txt")]);
        assert_eq!(source.actions(), gdk::DragAction::COPY);
        assert!(
            source
                .content()
                .unwrap()
                .formats()
                .contains_type(gdk::FileList::static_type())
        );
        let label = gtk4::Label::new(Some("Drag files"));
        label.add_controller(source);
        let window = gtk4::Window::builder().child(&label).build();
        window.present();
        while gtk4::glib::MainContext::default().pending() {
            gtk4::glib::MainContext::default().iteration(false);
        }
        assert!(window.is_visible());
        assert!(window.is_realized());
        window.close();
    }
}
