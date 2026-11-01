//! A GTK4-only process owns this thread and its GLib main loop. It never shares
//! GTK objects with the main app's icon worker or nests a loop in a Slint callback.
use super::*;
use gtk4::{self as gtk, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

enum Widget {
    Text(gtk::Entry),
    Check(gtk::CheckButton),
    Disclosure(gtk::Expander),
    Choice(gtk::DropDown, Vec<String>),
    Multiline(gtk::TextView),
}
impl Widget {
    fn value(&self) -> String {
        match self {
            Self::Text(w) => w.text().to_string(),
            Self::Check(w) => w.is_active().to_string(),
            Self::Disclosure(w) => w.is_expanded().to_string(),
            Self::Choice(w, values) => values
                .get(w.selected() as usize)
                .cloned()
                .unwrap_or_default(),
            Self::Multiline(w) => {
                let b = w.buffer();
                b.text(&b.start_iter(), &b.end_iter(), true).to_string()
            }
        }
    }
    fn update(&self, value: &str) {
        if self.value() == value {
            return;
        }
        match self {
            Self::Text(w) => w.set_text(value),
            Self::Check(w) => w.set_active(value == "true"),
            Self::Disclosure(w) => w.set_expanded(value == "true"),
            Self::Choice(w, values) => {
                w.set_selected(
                    values
                        .iter()
                        .position(|v| v == value)
                        .map_or(gtk::INVALID_LIST_POSITION, |index| index as u32),
                );
            }
            Self::Multiline(w) => w.buffer().set_text(value),
        }
    }
}
struct View {
    window: gtk::Window,
    widgets: Vec<(String, Widget)>,
    buttons: HashMap<String, gtk::Button>,
    error: gtk::Label,
    busy: gtk::Spinner,
    review: gtk::Button,
    review_page: Rc<Cell<i32>>,
    tabs: Option<(gtk::Notebook, gtk::Notebook, gtk::Notebook)>,
    updating: Rc<Cell<bool>>,
    model: Model,
    system_dark: bool,
}
fn tr(s: &str) -> String {
    crate::app::tr(s).into_owned()
}
fn structure(form: &Dialog) -> Dialog {
    let mut f = form.clone();
    f.error.clear();
    f.error_page = None;
    f.busy = false;
    for field in &mut f.fields {
        if !matches!(field.kind, 6 | 9) {
            field.value.clear();
        }
        if field.kind == 11 {
            field.choices[0].clear();
        }
    }
    for (_, _, enabled) in &mut f.buttons {
        *enabled = true;
    }
    for row in &mut f.rows {
        row.selected = false;
        for (_, _, enabled) in &mut row.actions {
            *enabled = true;
        }
    }
    f
}
fn label(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l.set_wrap(true);
    l
}
fn page_box() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 10);
    for setter in [
        gtk::Widget::set_margin_top,
        gtk::Widget::set_margin_bottom,
        gtk::Widget::set_margin_start,
        gtk::Widget::set_margin_end,
    ] {
        setter(&b.clone().upcast(), 16);
    }
    b
}
fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .hexpand(true)
        .child(child)
        .build()
}
impl View {
    fn build(window: gtk::Window, model: Model, selected: [u32; 3]) -> Self {
        crate::i18n::set(model.language);
        let updating = Rc::new(Cell::new(true));
        let root = page_box();
        let mut view = Self {
            window,
            widgets: Vec::new(),
            buttons: HashMap::new(),
            error: label(""),
            busy: gtk::Spinner::new(),
            review: gtk::Button::with_label(&tr("Review invalid setting")),
            review_page: Rc::new(Cell::new(-1)),
            tabs: None,
            updating,
            model,
            system_dark: gtk::Settings::default()
                .is_some_and(|s| s.is_gtk_application_prefer_dark_theme()),
        };
        if view.model.form.kind == "preferences" {
            let pages = (0..9)
                .map(|index| {
                    let body = page_box();
                    let fields = view
                        .model
                        .form
                        .fields
                        .iter()
                        .filter(|f| f.page == index)
                        .cloned()
                        .collect::<Vec<_>>();
                    for field in &fields {
                        view.field(&body, field);
                    }
                    let rows = view
                        .model
                        .form
                        .rows
                        .iter()
                        .filter(|r| r.page == index)
                        .cloned()
                        .collect::<Vec<_>>();
                    for row in &rows {
                        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                        let title = label(&row.title);
                        title.set_hexpand(true);
                        line.append(&title);
                        for (id, title, enabled) in &row.actions {
                            let button = view.button(id, title, *enabled);
                            line.append(&button);
                        }
                        body.append(&line);
                    }
                    scrolled(&body)
                })
                .collect::<Vec<_>>();
            let tabs = gtk::Notebook::new();
            let folders = gtk::Notebook::new();
            let appearance = gtk::Notebook::new();
            for (name, page) in [("Browsing", 1), ("Columns", 8)] {
                folders.append_page(&pages[page], Some(&gtk::Label::new(Some(&tr(name)))));
            }
            for (name, page) in [("Theme and icons", 2), ("Toolbar", 6), ("Command bar", 7)] {
                appearance.append_page(&pages[page], Some(&gtk::Label::new(Some(&tr(name)))));
            }
            for (name, child) in [
                ("General", pages[0].clone().upcast::<gtk::Widget>()),
                ("Folders", folders.clone().upcast()),
                ("Appearance", appearance.clone().upcast()),
                ("Shortcuts", pages[3].clone().upcast()),
                ("Mail", pages[4].clone().upcast()),
                ("Advanced", pages[5].clone().upcast()),
            ] {
                tabs.append_page(&child, Some(&gtk::Label::new(Some(&tr(name)))));
            }
            tabs.set_vexpand(true);
            root.append(&tabs);
            tabs.set_current_page(Some(selected[0]));
            folders.set_current_page(Some(selected[1]));
            appearance.set_current_page(Some(selected[2]));
            view.tabs = Some((tabs, folders, appearance));
        } else {
            if !view.model.form.description.is_empty() {
                root.append(&label(&view.model.form.description));
            }
            let body = page_box();
            let fields = view.model.form.fields.clone();
            for field in &fields {
                view.field(&body, field);
            }
            for row in view.model.form.rows.clone() {
                body.append(&label(&row.title));
                let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                for (id, title, enabled) in row.actions {
                    actions.append(&view.button(&id, &title, enabled));
                }
                body.append(&actions);
                if !row.detail.is_empty() {
                    body.append(&label(&row.detail));
                }
            }
            root.append(&scrolled(&body));
        }
        view.busy.set_halign(gtk::Align::Start);
        view.busy
            .update_property(&[gtk::accessible::Property::Label(&tr("Working…"))]);
        root.append(&view.busy);
        view.error.add_css_class("error");
        root.append(&view.error);
        root.append(&view.review);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let buttons = view.model.form.buttons.clone();
        if let Some((id, title, enabled)) = buttons.iter().find(|(id, _, _)| id == "defaults") {
            footer.append(&view.button(id, title, *enabled));
        }
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        footer.append(&spacer);
        for (id, title, enabled) in buttons.iter().filter(|(id, _, _)| id != "defaults") {
            let button = view.button(id, title, *enabled);
            if id == "accept" && view.model.form.default_action {
                button.add_css_class("suggested-action");
                view.window.set_default_widget(Some(&button));
            }
            footer.append(&button);
        }
        root.append(&footer);
        view.window.set_child(Some(&root));
        let tabs = view.tabs.clone();
        let page = view.review_page.clone();
        view.review.connect_clicked(move |_| {
            if let Some(tabs) = &tabs {
                show_page(tabs, page.get());
            }
        });
        view.updating.set(false);
        view.update_values();
        view
    }
    fn button(&mut self, id: &str, title: &str, enabled: bool) -> gtk::Button {
        let button = gtk::Button::with_label(&tr(title));
        button.set_sensitive(enabled);
        let action = id.to_owned();
        button.connect_clicked(move |_| emit(&Event::Action(action.clone())));
        self.buttons.insert(id.into(), button.clone());
        button
    }
    fn entry(&mut self, id: &str, title: &str, value: &str, password: bool) -> gtk::Entry {
        let entry = gtk::Entry::new();
        entry.set_hexpand(true);
        entry.set_text(value);
        entry.set_visibility(!password);
        entry.set_activates_default(id != "shortcut-filter" && self.model.form.default_action);
        entry.update_property(&[gtk::accessible::Property::Label(title)]);
        let guard = self.updating.clone();
        let key = id.to_owned();
        entry.connect_changed(move |w| {
            if !guard.get() {
                emit(&Event::Field(key.clone(), w.text().into()));
            }
        });
        self.widgets.push((id.into(), Widget::Text(entry.clone())));
        entry
    }
    fn field(&mut self, body: &gtk::Box, field: &super::super::Field) {
        let title = tr(&field.title);
        match field.kind {
            10 => {
                let heading = label(&title);
                heading.add_css_class("heading");
                heading.set_margin_top(8);
                body.append(&heading);
            }
            6 | 9 => {
                if !field.title.is_empty() {
                    body.append(&label(&title));
                }
                if field.kind == 6 && field.value.len() > 300 {
                    let text = gtk::TextView::new();
                    text.set_editable(false);
                    text.set_wrap_mode(gtk::WrapMode::WordChar);
                    text.buffer().set_text(&field.value);
                    text.update_property(&[gtk::accessible::Property::Label(&title)]);
                    let scroll = scrolled(&text);
                    scroll.set_min_content_height(220);
                    body.append(&scroll);
                } else {
                    body.append(&label(&field.value));
                }
            }
            12 => {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                for text in [&field.title, &field.value, &field.choices[0]] {
                    let l = label(text);
                    l.set_hexpand(true);
                    l.add_css_class("dim-label");
                    row.append(&l);
                }
                body.append(&row);
            }
            8 => {
                let disclosure = gtk::Expander::new(Some(&title));
                disclosure.set_child(Some(&gtk::Box::new(gtk::Orientation::Vertical, 0)));
                disclosure.set_expanded(field.value == "true");
                let guard = self.updating.clone();
                let id = field.id.clone();
                disclosure.connect_expanded_notify(move |w| {
                    if !guard.get() {
                        emit(&Event::Field(id.clone(), w.is_expanded().to_string()));
                    }
                });
                self.widgets
                    .push((field.id.clone(), Widget::Disclosure(disclosure.clone())));
                body.append(&disclosure);
            }
            2 => {
                let check = gtk::CheckButton::with_label(&title);
                check.set_active(field.value == "true");
                check.set_sensitive(field.id != "column-Name");
                let guard = self.updating.clone();
                let id = field.id.clone();
                check.connect_toggled(move |w| {
                    if !guard.get() {
                        emit(&Event::Field(id.clone(), w.is_active().to_string()));
                    }
                });
                self.widgets
                    .push((field.id.clone(), Widget::Check(check.clone())));
                body.append(&check);
            }
            0 | 1 | 3 | 4 | 5 | 11 => {
                let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                let caption = label(&title);
                caption.set_width_chars(23);
                caption.set_max_width_chars(28);
                caption.set_hexpand(false);
                line.append(&caption);
                match field.kind {
                    0 | 1 | 5 | 11 => {
                        let entry = self.entry(&field.id, &title, &field.value, field.kind == 1);
                        line.append(&entry);
                        if let Some(picker) = field.picker {
                            let browse = gtk::Button::with_label(&tr("Browse…"));
                            let owner = self.window.clone();
                            let id = field.id.clone();
                            browse.connect_clicked(move |_| {
                                emit(&Event::Editing);
                                let current = entry.text().to_string();
                                let dialog = gtk::FileChooserNative::builder()
                                    .transient_for(&owner)
                                    .modal(true)
                                    .title(tr(if picker == Picker::NewDirectory {
                                        "Choose parent folder"
                                    } else {
                                        "Browse"
                                    }))
                                    .action(match picker {
                                        Picker::Directory | Picker::NewDirectory => {
                                            gtk::FileChooserAction::SelectFolder
                                        }
                                        Picker::SaveFile => gtk::FileChooserAction::Save,
                                        _ => gtk::FileChooserAction::Open,
                                    })
                                    .build();
                                let path = std::path::Path::new(&current);
                                let folder = if picker == Picker::Directory {
                                    Some(path)
                                } else {
                                    path.parent()
                                };
                                if let Some(folder) = folder.filter(|p| p.is_absolute()) {
                                    let _ = dialog.set_current_folder(Some(
                                        &gtk::gio::File::for_path(folder),
                                    ));
                                }
                                if picker == Picker::SaveFile
                                    && let Some(name) = path.file_name()
                                {
                                    dialog.set_current_name(&name.to_string_lossy());
                                }
                                let id = id.clone();
                                dialog.run_async(move |dialog, response| {
                                    if response == gtk::ResponseType::Accept
                                        && let Some(path) =
                                            dialog.file().and_then(|file| file.path())
                                    {
                                        emit(&Event::Field(
                                            id.clone(),
                                            picker.selected(path, &current),
                                        ));
                                    }
                                    dialog.destroy();
                                });
                            });
                            self.buttons
                                .insert(format!("browse:{}", field.id), browse.clone());
                            line.append(&browse);
                        }
                        if field.kind == 11 {
                            let alt = format!("{title} — {}", tr("Alternate shortcut"));
                            line.append(&self.entry(
                                &field.choices[1],
                                &alt,
                                &field.choices[0],
                                false,
                            ));
                        }
                    }
                    3 => {
                        let labels = field.choices.iter().map(|s| tr(s)).collect::<Vec<_>>();
                        let options = labels.iter().map(String::as_str).collect::<Vec<_>>();
                        let choice = gtk::DropDown::from_strings(&options);
                        choice.set_hexpand(true);
                        choice.set_selected(
                            field
                                .choices
                                .iter()
                                .position(|v| v == &field.value)
                                .map_or(gtk::INVALID_LIST_POSITION, |index| index as u32),
                        );
                        choice.update_property(&[gtk::accessible::Property::Label(&title)]);
                        let id = field.id.clone();
                        let values = field.choices.clone();
                        let guard = self.updating.clone();
                        choice.connect_selected_notify(move |w| {
                            if !guard.get()
                                && let Some(value) = values.get(w.selected() as usize)
                            {
                                emit(&Event::Field(id.clone(), value.clone()));
                            }
                        });
                        self.widgets.push((
                            field.id.clone(),
                            Widget::Choice(choice.clone(), field.choices.clone()),
                        ));
                        line.append(&choice);
                    }
                    4 => {
                        let text = gtk::TextView::new();
                        text.set_wrap_mode(gtk::WrapMode::WordChar);
                        text.buffer().set_text(&field.value);
                        text.update_property(&[gtk::accessible::Property::Label(&title)]);
                        let id = field.id.clone();
                        let guard = self.updating.clone();
                        text.buffer().connect_changed(move |b| {
                            if !guard.get() {
                                emit(&Event::Field(
                                    id.clone(),
                                    b.text(&b.start_iter(), &b.end_iter(), true).into(),
                                ));
                            }
                        });
                        let scroll = scrolled(&text);
                        scroll.set_min_content_height(100);
                        line.append(&scroll);
                        self.widgets
                            .push((field.id.clone(), Widget::Multiline(text)));
                    }
                    _ => {}
                }
                body.append(&line);
            }
            _ => {}
        }
    }
    fn update_values(&self) {
        self.updating.set(true);
        self.window.set_title(Some(&self.model.form.title));
        self.busy
            .set_visible(self.model.form.busy && self.model.form.error.is_empty());
        self.busy.set_spinning(self.model.form.busy);
        self.error.set_text(&self.model.form.error);
        self.error.set_visible(!self.model.form.error.is_empty());
        self.review
            .set_visible(self.model.form.error_page.is_some());
        self.review_page
            .set(self.model.form.error_page.unwrap_or(-1));
        for field in &self.model.form.fields {
            for (id, widget) in &self.widgets {
                if id == &field.id {
                    widget.update(&field.value);
                } else if field.kind == 11 && id == &field.choices[1] {
                    widget.update(&field.choices[0]);
                }
            }
        }
        for (id, _, enabled) in self
            .model
            .form
            .buttons
            .iter()
            .chain(self.model.form.rows.iter().flat_map(|r| r.actions.iter()))
        {
            if let Some(button) = self.buttons.get(id) {
                button.set_sensitive(*enabled);
            }
        }
        self.updating.set(false);
    }
    fn update(&mut self, model: Model) {
        if self.model.language != model.language
            || structure(&self.model.form) != structure(&model.form)
        {
            let selected = self
                .tabs
                .as_ref()
                .map(|(a, b, c)| {
                    [
                        a.current_page().unwrap_or(0),
                        b.current_page().unwrap_or(0),
                        c.current_page().unwrap_or(0),
                    ]
                })
                .unwrap_or([0; 3]);
            let focus = gtk::prelude::GtkWindowExt::focus(&self.window);
            let editing = self.widgets.iter().find_map(|(id, widget)| {
                if let Widget::Text(entry) = widget {
                    focus
                        .as_ref()
                        .filter(|focus| {
                            *focus == entry.upcast_ref::<gtk::Widget>() || focus.is_ancestor(entry)
                        })
                        .map(|_| (id.clone(), entry.selection_bounds(), entry.position()))
                } else {
                    None
                }
            });
            let mut rebuilt = Self::build(self.window.clone(), model, selected);
            rebuilt.system_dark = self.system_dark;
            *self = rebuilt;
            if let Some((id, range, position)) = editing
                && let Some((_, Widget::Text(entry))) = self
                    .widgets
                    .iter()
                    .find(|(key, widget)| key == &id && matches!(widget, Widget::Text(_)))
            {
                entry.grab_focus();
                if let Some((start, end)) = range {
                    entry.select_region(start, end);
                } else {
                    entry.set_position(position);
                }
            }
        } else {
            self.model = model;
            self.update_values();
        }
        if let Some(settings) = gtk::Settings::default() {
            match self.model.scheme {
                crate::appearance::ColorScheme::Dark => {
                    settings.set_gtk_application_prefer_dark_theme(true)
                }
                crate::appearance::ColorScheme::Light => {
                    settings.set_gtk_application_prefer_dark_theme(false)
                }
                crate::appearance::ColorScheme::System => {
                    settings.set_gtk_application_prefer_dark_theme(self.system_dark)
                }
            }
        }
    }
    #[cfg(feature = "screenshot")]
    fn capture(&self, path: &str) -> bool {
        let Some(surface) = self.window.surface() else {
            return false;
        };
        let snapshot = gtk::Snapshot::new();
        gtk::WidgetPaintable::new(Some(&self.window)).snapshot(
            &snapshot,
            self.window.width() as f64,
            self.window.height() as f64,
        );
        let Some(node) = snapshot.to_node() else {
            return false;
        };
        let Some(renderer) = gtk::gsk::Renderer::for_surface(&surface) else {
            return false;
        };
        let texture = renderer.render_texture(&node, None);
        let result = texture.save_to_png(path).is_ok();
        renderer.unrealize();
        result
    }
    #[cfg(feature = "screenshot")]
    fn smoke(&self, callback: &str, value: &str) -> bool {
        match callback {
            "native-field" => {
                let Ok((id, value)) = serde_json::from_str::<(String, String)>(value) else {
                    return false;
                };
                let mut found = false;
                for (key, widget) in &self.widgets {
                    if key == &id {
                        widget.update(&value);
                        found = true;
                    }
                }
                found
            }
            "native-action" if value == "close" => {
                self.window.close();
                true
            }
            "native-action" => {
                if value == "return"
                    && self.model.form.busy
                    && self.model.form.kind == "prompt"
                    && self.model.form.default_action
                    && self.model.form.error.is_empty()
                {
                    emit(&Event::Action("accept".into()));
                    return true;
                }
                let id = if value == "return" { "accept" } else { value };
                let Some(button) = self.buttons.get(id).filter(|b| b.is_sensitive()) else {
                    return false;
                };
                button.emit_clicked();
                true
            }
            "native-check" => {
                self.model.form.error.contains(value)
                    && self.window.is_visible()
                    && self.widgets.iter().all(|(id, widget)| {
                        let expected = self
                            .model
                            .form
                            .fields
                            .iter()
                            .find(|field| &field.id == id)
                            .map(|field| &field.value)
                            .or_else(|| {
                                self.model
                                    .form
                                    .fields
                                    .iter()
                                    .find(|field| {
                                        field.kind == 11 && field.choices.get(1) == Some(id)
                                    })
                                    .map(|field| &field.choices[0])
                            });
                        expected.is_some_and(|expected| match widget {
                            Widget::Choice(_, values) if !values.contains(expected) => {
                                widget.value().is_empty()
                            }
                            _ => &widget.value() == expected,
                        })
                    })
            }
            _ => false,
        }
    }
}
fn show_page(
    (tabs, folders, appearance): &(gtk::Notebook, gtk::Notebook, gtk::Notebook),
    page: i32,
) {
    match page {
        6 | 7 => {
            tabs.set_current_page(Some(2));
            appearance.set_current_page(Some((page - 5) as u32));
        }
        8 => {
            tabs.set_current_page(Some(1));
            folders.set_current_page(Some(1));
        }
        0..=5 => {
            tabs.set_current_page(Some(page as u32));
            if page == 1 {
                folders.set_current_page(Some(0));
            }
            if page == 2 {
                appearance.set_current_page(Some(0));
            }
        }
        _ => {}
    }
}
fn parent(window: &gtk::Window, owner: &Parent) -> bool {
    use gtk::glib::object::ObjectType;
    gtk::prelude::WidgetExt::realize(window);
    let Some(surface) = window.surface() else {
        return false;
    };
    unsafe {
        match owner {
            Parent::X11(owner) => {
                let symbol = libc::dlsym(libc::RTLD_DEFAULT, c"gdk_x11_surface_get_xid".as_ptr());
                if symbol.is_null() || !surface.type_().name().contains("X11") {
                    return false;
                }
                let xid: unsafe extern "C" fn(*mut gtk::gdk::ffi::GdkSurface) -> libc::c_ulong =
                    std::mem::transmute(symbol);
                use x11rb::protocol::xproto::{AtomEnum, PropMode};
                use x11rb::{connection::Connection, wrapper::ConnectionExt};
                let Ok((connection, _)) = x11rb::connect(None) else {
                    return false;
                };
                connection
                    .change_property32(
                        PropMode::REPLACE,
                        xid(surface.as_ptr()) as u32,
                        AtomEnum::WM_TRANSIENT_FOR,
                        AtomEnum::WINDOW,
                        &[*owner],
                    )
                    .is_ok()
                    && connection.flush().is_ok()
            }
            Parent::Wayland(owner) => {
                let symbol = libc::dlsym(
                    libc::RTLD_DEFAULT,
                    c"gdk_wayland_toplevel_set_transient_for_exported".as_ptr(),
                );
                let Ok(owner) = std::ffi::CString::new(owner.as_str()) else {
                    return false;
                };
                if symbol.is_null() || !surface.type_().name().contains("Wayland") {
                    return false;
                }
                let import: unsafe extern "C" fn(
                    *mut gtk::gdk::ffi::GdkSurface,
                    *const libc::c_char,
                ) -> libc::c_int = std::mem::transmute(symbol);
                import(surface.as_ptr(), owner.as_ptr()) != 0
            }
            _ => false,
        }
    }
}
pub(super) fn run(model: Model, incoming: Incoming) -> i32 {
    if gtk::init().is_err() {
        emit(&Event::Failed);
        return 2;
    }
    let main_loop = glib::MainLoop::new(None, false);
    let window = gtk::Window::builder()
        .title(&model.form.title)
        .default_width(if model.form.kind == "preferences" {
            840
        } else {
            640
        })
        .default_height(620)
        .modal(true)
        .build();
    let view = Rc::new(RefCell::new(View::build(
        window.clone(),
        model.clone(),
        [0; 3],
    )));
    view.borrow_mut().update(model.clone());
    if !parent(&window, &model.parent) {
        emit(&Event::Failed);
        return 2;
    }
    window.connect_close_request(|_| {
        emit(&Event::Action("cancel".into()));
        glib::Propagation::Stop
    });
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let key_view = view.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Return {
            let view = key_view.borrow();
            if view.model.form.busy
                && view.model.form.kind == "prompt"
                && view.model.form.default_action
                && view.model.form.error.is_empty()
            {
                emit(&Event::Action("accept".into()));
                return glib::Propagation::Stop;
            }
        }
        if key == gtk::gdk::Key::Escape {
            emit(&Event::Action("cancel".into()));
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(keys);
    let quit = main_loop.clone();
    glib::timeout_add_local(Duration::from_millis(15), move || {
        for request in incoming.try_iter() {
            match request {
                Request::Close => {
                    quit.quit();
                    return glib::ControlFlow::Break;
                }
                Request::Update(model) => view.borrow_mut().update(*model),
                #[cfg(feature = "screenshot")]
                Request::Page(page) => {
                    if let Some(tabs) = &view.borrow().tabs {
                        show_page(tabs, page);
                        emit(&Event::Checked(true));
                    } else {
                        emit(&Event::Checked(false));
                    }
                }
                #[cfg(feature = "screenshot")]
                Request::Capture(path) => emit(&Event::Captured(view.borrow().capture(&path))),
                #[cfg(feature = "screenshot")]
                Request::Smoke(callback, value) => {
                    emit(&Event::Checked(view.borrow().smoke(&callback, &value)))
                }
            }
        }
        glib::ControlFlow::Continue
    });
    window.present();
    emit(&Event::Ready);
    main_loop.run();
    0
}
