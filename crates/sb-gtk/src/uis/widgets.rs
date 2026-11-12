//! Small widget builders and the status line.

use gtk::prelude::*;

use super::ui::Shared;

pub fn scrolled(child: &impl IsA<gtk::Widget>, min_width: i32) -> gtk::ScrolledWindow {
    let s = gtk::ScrolledWindow::new();
    s.set_child(Some(child));
    s.set_hexpand(true);
    s.set_vexpand(true);
    if min_width > 0 {
        s.set_min_content_width(min_width);
    }
    s
}

pub fn list_page(stack: &gtk::Stack, name: &str, title: &str) -> gtk::ListBox {
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    stack.add_titled(&scrolled(&list, 0), Some(name), title);
    list
}

pub fn clear(list: &impl IsA<gtk::Widget>) {
    while let Some(child) = list.first_child() {
        if let Some(l) = list.dynamic_cast_ref::<gtk::ListBox>() {
            l.remove(&child);
        } else if let Some(b) = list.dynamic_cast_ref::<gtk::Box>() {
            b.remove(&child);
        } else {
            return;
        }
    }
}

pub fn label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .selectable(true)
        .build()
}

pub fn heading(text: &str) -> gtk::Label {
    let l = label(text);
    l.add_css_class("title-4");
    l.set_selectable(false);
    l.set_margin_top(12);
    l
}

pub fn row(title: &str, subtitle: &str) -> gtk::ListBoxRow {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    content.set_margin_top(8);
    content.set_margin_bottom(8);
    content.set_margin_start(16);
    content.set_margin_end(16);
    let t = label(title);
    t.add_css_class("heading");
    content.append(&t);
    if !subtitle.is_empty() {
        let s = label(subtitle);
        s.add_css_class("dim-label");
        content.append(&s);
    }
    let row = gtk::ListBoxRow::new();
    row.set_child(Some(&content));
    row
}

pub fn section(list: &gtk::ListBox, text: &str) {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    let l = heading(text);
    l.remove_css_class("title-4");
    l.add_css_class("section-title");
    l.set_label(&text.to_uppercase());
    l.set_margin_start(16);
    row.set_child(Some(&l));
    list.append(&row);
}

/// A message in the corner, for a few seconds.
pub fn say(ui: &Shared, text: &str) {
    ui.toast.set_text(text);
    ui.toast.set_visible(!text.is_empty());
    let (toast, shown) = (ui.toast.clone(), text.to_string());
    gtk::glib::timeout_add_seconds_local_once(4, move || {
        if toast.text() == shown {
            toast.set_visible(false);
        }
    });
}

pub fn fail(ui: &Shared, error: &str) {
    say(ui, error);
    gtk::AlertDialog::builder()
        .message("Une erreur est survenue")
        .detail(error)
        .build()
        .show(Some(&ui.window));
}

pub fn names<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    let all: Vec<&str> = items.into_iter().collect();
    if all.is_empty() {
        "—".into()
    } else {
        all.join(", ")
    }
}
