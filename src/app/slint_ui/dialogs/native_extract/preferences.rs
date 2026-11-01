//! Real AppKit preference tabs and controls; the controller still owns the draft.
use super::*;
use objc2::AnyThread;
use objc2_app_kit::{NSTabView, NSTabViewItem};

struct Page {
    scroll: Retained<NSScrollView>,
    fields: Vec<Field>,
    rows: Vec<super::super::Row>,
    widgets: Vec<(String, Widget)>,
    action_slots: Vec<usize>,
}
pub(super) struct View {
    root: Retained<NSTabView>,
    folders: Retained<NSTabView>,
    appearance: Retained<NSTabView>,
    pages: Vec<Page>,
}

fn tabs(mtm: MainThreadMarker, frame: NSRect) -> Retained<NSTabView> {
    let tabs = NSTabView::initWithFrame(NSTabView::alloc(mtm), frame);
    tabs.setFont(&NSFont::systemFontOfSize(13.0));
    tabs
}
fn tab(_mtm: MainThreadMarker, tabs: &NSTabView, name: &str, view: &NSView) {
    // SAFETY: the identifier is an ordinary retained NSString.
    let item = unsafe {
        NSTabViewItem::initWithIdentifier(NSTabViewItem::alloc(), Some(&*translated(name)))
    };
    item.setLabel(&translated(name));
    item.setView(Some(view));
    tabs.addTabViewItem(&item);
}
fn action(target: &Target, id: &str) -> usize {
    let mut actions = target.ivars().actions.borrow_mut();
    if let Some(index) = actions.iter().position(String::is_empty) {
        actions[index] = id.into();
        index
    } else {
        actions.push(id.into());
        actions.len() - 1
    }
}
fn text(mtm: MainThreadMarker, target: &Target, field: &Field, frame: NSRect) -> Widget {
    let text = NSTextField::initWithFrame(NSTextField::alloc(mtm), frame);
    text.setBezeled(true);
    text.setBezelStyle(NSTextFieldBezelStyle::RoundedBezel);
    text.setFont(Some(&NSFont::systemFontOfSize(13.0)));
    text.setMaximumNumberOfLines(1);
    text.setAccessibilityLabel(Some(&translated(&field.title)));
    if let Some(cell) = text.cell() {
        cell.setSendsActionOnEndEditing(false);
        cell.setWraps(false);
        cell.setScrollable(true);
        cell.setUsesSingleLineMode(true);
    }
    unsafe {
        text.setDelegate(Some(ProtocolObject::from_ref(target)));
        if field.id != "shortcut-filter" {
            text.setTarget(Some(target));
            text.setAction(Some(sel!(perform:)));
            text.setTag(0);
        }
    }
    let widget = Widget::Text(text);
    widget.update(&field.value);
    widget
}
fn schema(fields: &[Field]) -> Vec<Field> {
    fields
        .iter()
        .cloned()
        .map(|mut field| {
            if !matches!(field.kind, 6 | 9) {
                field.value.clear();
            }
            if field.kind == 11 {
                field.choices[0].clear();
            }
            field
        })
        .collect()
}
impl View {
    pub fn new(mtm: MainThreadMarker, _target: &Target) -> Self {
        let root = tabs(mtm, rect(16.0, 96.0, 788.0, 526.0));
        let folders = tabs(mtm, rect(0.0, 0.0, 760.0, 480.0));
        let appearance = tabs(mtm, rect(0.0, 0.0, 760.0, 480.0));
        let pages = (0..9)
            .map(|page| {
                let scroll = NSScrollView::initWithFrame(
                    NSScrollView::alloc(mtm),
                    rect(
                        0.0,
                        0.0,
                        744.0,
                        if matches!(page, 1 | 2 | 6 | 7 | 8) {
                            440.0
                        } else {
                            480.0
                        },
                    ),
                );
                scroll.setHasVerticalScroller(true);
                scroll.setAutohidesScrollers(true);
                scroll.setDrawsBackground(false);
                Page {
                    scroll,
                    fields: Vec::new(),
                    rows: Vec::new(),
                    widgets: Vec::new(),
                    action_slots: Vec::new(),
                }
            })
            .collect::<Vec<_>>();
        tab(mtm, &folders, "Browsing", &pages[1].scroll);
        tab(mtm, &folders, "Columns", &pages[8].scroll);
        tab(mtm, &appearance, "Theme and icons", &pages[2].scroll);
        tab(mtm, &appearance, "Toolbar", &pages[6].scroll);
        tab(mtm, &appearance, "Command bar", &pages[7].scroll);
        for (name, view) in [
            ("General", &*pages[0].scroll as &NSView),
            ("Folders", &*folders),
            ("Appearance", &*appearance),
            ("Shortcuts", &*pages[3].scroll),
            ("Mail", &*pages[4].scroll),
            ("Advanced", &*pages[5].scroll),
        ] {
            tab(mtm, &root, name, view);
        }
        Self {
            root,
            folders,
            appearance,
            pages,
        }
    }
    pub fn view(&self) -> &NSView {
        &self.root
    }
    pub fn show_page(&self, page: i32) {
        match page {
            6 | 7 => {
                self.root.selectTabViewItemAtIndex(2);
                self.appearance
                    .selectTabViewItemAtIndex((page - 5) as isize);
            }
            8 => {
                self.root.selectTabViewItemAtIndex(1);
                self.folders.selectTabViewItemAtIndex(1);
            }
            0..=5 => {
                self.root.selectTabViewItemAtIndex(page as isize);
                if page == 1 {
                    self.folders.selectTabViewItemAtIndex(0);
                }
                if page == 2 {
                    self.appearance.selectTabViewItemAtIndex(0);
                }
            }
            _ => {}
        }
    }
    pub fn update(&mut self, spec: &Dialog, target: &Target, window: &NSWindow) {
        let editing = target
            .ivars()
            .fields
            .borrow()
            .iter()
            .find_map(|(id, widget)| {
                widget
                    .control()
                    .and_then(|control| control.currentEditor())
                    .map(|editor| (id.clone(), editor.selectedRange()))
            });
        let mut changed = false;
        for (index, page) in self.pages.iter_mut().enumerate() {
            let fields = spec
                .fields
                .iter()
                .filter(|field| field.page == index as i32)
                .cloned()
                .collect::<Vec<_>>();
            let rows = spec
                .rows
                .iter()
                .filter(|row| row.page == index as i32)
                .cloned()
                .collect::<Vec<_>>();
            if schema(&page.fields) != schema(&fields) || page.rows != rows {
                page.build(window.mtm(), target, &fields, &rows, index);
                changed = true;
            }
            page.fields = fields;
            page.rows = rows;
        }
        if changed {
            *target.ivars().fields.borrow_mut() =
                self.pages.iter().flat_map(|p| p.widgets.clone()).collect();
            window.recalculateKeyViewLoop();
            if let Some((id, range)) = editing
                && let Some((_, widget)) = target
                    .ivars()
                    .fields
                    .borrow()
                    .iter()
                    .find(|(key, _)| key == &id)
            {
                window.makeFirstResponder(Some(widget.view()));
                if let Some(editor) = widget.control().and_then(|c| c.currentEditor()) {
                    editor.setSelectedRange(range);
                }
            }
        }
    }
}
impl Page {
    fn build(
        &mut self,
        mtm: MainThreadMarker,
        target: &Target,
        fields: &[Field],
        rows: &[super::super::Row],
        index: usize,
    ) {
        for slot in self.action_slots.drain(..) {
            target.ivars().actions.borrow_mut()[slot].clear();
        }
        for (_, widget) in &self.widgets {
            if let Widget::Text(text) = widget {
                unsafe {
                    text.setDelegate(None);
                }
            }
        }
        let old_y = self.scroll.contentView().bounds().origin.y;
        let total = fields
            .iter()
            .map(|f| match f.kind {
                6 => 64.0,
                10 => 38.0,
                _ => 38.0,
            })
            .sum::<f64>()
            + rows.len() as f64 * 36.0
            + 24.0;
        let total = total.max(self.scroll.contentSize().height);
        let doc = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, 720.0, total));
        let mut y = total - 12.0;
        self.widgets.clear();
        for field in fields {
            let height = if field.kind == 6 { 64.0 } else { 38.0 };
            y -= height;
            let title = translated(&field.title);
            if field.kind == 10 {
                let heading = label(mtm, &title.to_string(), rect(12.0, y + 4.0, 690.0, 24.0));
                heading.setFont(Some(&NSFont::boldSystemFontOfSize(13.0)));
                doc.addSubview(&heading);
                continue;
            }
            if field.kind == 6 {
                doc.addSubview(&label(mtm, &field.value, rect(12.0, y + 4.0, 690.0, 56.0)));
                continue;
            }
            if field.kind == 12 {
                for (value, x, width) in [
                    (&field.title, 12.0, 200.0),
                    (&field.value, 220.0, 236.0),
                    (&field.choices[0], 466.0, 236.0),
                ] {
                    let heading = label(mtm, value, rect(x, y + 8.0, width, 24.0));
                    heading.setTextColor(Some(&NSColor::secondaryLabelColor()));
                    doc.addSubview(&heading);
                }
                continue;
            }
            if field.kind != 2 {
                doc.addSubview(&label(
                    mtm,
                    &title.to_string(),
                    rect(12.0, y + 8.0, 200.0, 24.0),
                ));
            }
            let widget = match field.kind {
                0 | 11 => text(
                    mtm,
                    target,
                    field,
                    rect(
                        220.0,
                        y + 6.0,
                        if field.kind == 11 { 236.0 } else { 482.0 },
                        26.0,
                    ),
                ),
                2 => {
                    let check = unsafe {
                        NSButton::checkboxWithTitle_target_action(
                            &title,
                            Some(target),
                            Some(sel!(changed:)),
                            mtm,
                        )
                    };
                    check.setFrame(rect(12.0, y + 6.0, 690.0, 26.0));
                    check.setEnabled(field.id != "column-Name");
                    Widget::Check(check)
                }
                3 => {
                    let choice = NSPopUpButton::initWithFrame_pullsDown(
                        NSPopUpButton::alloc(mtm),
                        rect(218.0, y + 4.0, 486.0, 28.0),
                        false,
                    );
                    for value in &field.choices {
                        choice.addItemWithTitle(&translated(value));
                    }
                    unsafe {
                        choice.setTarget(Some(target));
                        choice.setAction(Some(sel!(changed:)));
                    }
                    Widget::Choice(choice, field.choices.clone())
                }
                _ => continue,
            };
            widget.view().setAccessibilityLabel(Some(&title));
            widget.update(&field.value);
            doc.addSubview(widget.view());
            self.widgets.push((field.id.clone(), widget));
            if field.kind == 11 {
                let alternate = Field::text(
                    &field.choices[1],
                    format!("{} — {}", field.title, crate::app::tr("Alternate shortcut")),
                    &field.choices[0],
                );
                let widget = text(mtm, target, &alternate, rect(466.0, y + 6.0, 236.0, 26.0));
                doc.addSubview(widget.view());
                self.widgets.push((alternate.id, widget));
            }
        }
        for row in rows {
            y -= 36.0;
            doc.addSubview(&label(mtm, &row.title, rect(12.0, y + 6.0, 480.0, 24.0)));
            for (i, (id, title, enabled)) in row.actions.iter().enumerate() {
                let button = unsafe {
                    NSButton::buttonWithTitle_target_action(
                        &translated(title),
                        Some(target),
                        Some(sel!(perform:)),
                        mtm,
                    )
                };
                button.setBezelStyle(NSBezelStyle::Push);
                button.setFrame(rect(510.0 + i as f64 * 96.0, y + 2.0, 92.0, 30.0));
                button.setEnabled(*enabled);
                let slot = action(target, id);
                button.setTag(slot as isize);
                self.action_slots.push(slot);
                doc.addSubview(&button);
            }
        }
        self.scroll.setDocumentView(Some(&doc));
        let origin = if self.fields.is_empty() {
            (total - self.scroll.contentSize().height).max(0.0)
        } else {
            old_y.min((total - self.scroll.contentSize().height).max(0.0))
        };
        self.scroll
            .contentView()
            .scrollToPoint(NSPoint::new(0.0, origin));
        let _ = index;
    }
}
