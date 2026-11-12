//! The toolbar of the web version: logo, project name, the four views, the state of the project and the file actions.

use gtk::gdk;
use gtk::glib::{self, clone};
use gtk::prelude::*;

use super::edit::{edit, guard, save};
use super::files::{ask_new_project, export_code, pick_project, save_as};
use super::icons::{self, icon};
use super::project::load_sample;
use super::ui::Shared;
use super::updates::check_for_update;

type MenuEntry = (&'static str, fn(&Shared));

/// The views the tabs switch between: (page of the stack, tooltip, label, icon).
pub const TABS: [(&str, &str, &str); 4] = [
    ("architecture", "Architecture", "graph"),
    ("tables", "Tables", "table"),
    ("mcp", "MCP", "plug"),
    ("code", "Code généré", "code"),
];

/// A flat button holding an icon and, optionally, a text.
fn tool(icon_name: &str, text: &str, tip: &str, color: &str) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&icon(icon_name, 15, color));
    if !text.is_empty() {
        content.append(&gtk::Label::new(Some(text)));
    }
    let b = gtk::Button::new();
    b.set_child(Some(&content));
    b.set_tooltip_text(Some(tip));
    b.add_css_class("ghost");
    b.set_valign(gtk::Align::Center);
    b
}

/// The logo, locked at 26x26 whatever the size of the texture.
fn logo() -> gtk::Image {
    let bytes = glib::Bytes::from_static(include_bytes!("../../data/service-builder.png"));
    let image = match gdk::Texture::from_bytes(&bytes) {
        Ok(t) => gtk::Image::from_paintable(Some(&t)),
        Err(_) => gtk::Image::new(),
    };
    image.set_pixel_size(26);
    image.set_size_request(26, 26);
    image.set_halign(gtk::Align::Center);
    image.set_valign(gtk::Align::Center);
    image.set_hexpand(false);
    image.set_vexpand(false);
    image
}

/// The update button: an icon, `Mettre à jour` and the version on offer.
pub fn update_content(button: &gtk::Button, version: &str) {
    let c = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    c.append(&icon("update", 15, icons::ACCENT));
    c.append(&gtk::Label::new(Some("Mettre à jour")));
    if !version.is_empty() {
        let v = gtk::Label::new(Some(&format!("v{}", version.trim_start_matches('v'))));
        v.add_css_class("version");
        c.append(&v);
    }
    button.set_child(Some(&c));
}

pub fn toolbar(ui: &Shared) -> gtk::WindowHandle {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    bar.add_css_class("toolbar");

    let brand = gtk::Box::new(gtk::Orientation::Horizontal, 9);
    brand.append(&logo());
    let name = gtk::Label::new(Some("Service Builder"));
    name.add_css_class("brand");
    brand.append(&name);
    bar.append(&brand);

    // The name of the project: edited in place, committed with Enter or when the focus leaves.
    let title = ui.title.clone();
    title.add_css_class("project-name");
    title.set_width_chars(16);
    title.set_valign(gtk::Align::Center);
    let commit = clone!(
        #[strong]
        ui,
        move || {
            let name = ui.title.text().trim().to_string();
            if name.is_empty() {
                super::project::refresh_title(&ui);
            } else {
                edit(&ui, |p| p.name = name);
            }
        }
    );
    title.connect_activate({
        let commit = commit.clone();
        move |_| commit()
    });
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave(move |_| commit());
    title.add_controller(focus);
    let name_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    name_box.append(&title);
    name_box.append(&ui.dirty_mark);
    name_box.append(&ui.file_label);
    bar.append(&name_box);

    // The views, in a dark track with the current one raised.
    let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    tabs.add_css_class("tabs");
    tabs.set_valign(gtk::Align::Center);
    let mut buttons: Vec<(&str, gtk::ToggleButton, gtk::Box)> = Vec::new();
    let mut group: Option<gtk::ToggleButton> = None;
    for (page, label, icon_name) in TABS {
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        content.append(&icon(icon_name, 15, icons::MUTED));
        content.append(&gtk::Label::new(Some(label)));
        let b = gtk::ToggleButton::new();
        b.set_child(Some(&content));
        b.set_tooltip_text(Some(label));
        match &group {
            Some(g) => b.set_group(Some(g)),
            None => group = Some(b.clone()),
        }
        b.connect_clicked(clone!(
            #[strong]
            ui,
            move |_| {
                ui.stack.set_visible_child_name(page);
            }
        ));
        tabs.append(&b);
        buttons.push((page, b, content));
    }
    // A page that has no tab (the lists behind the inspector) leaves every tab off.
    let sync = clone!(
        #[strong]
        ui,
        move |buttons: &Vec<(&str, gtk::ToggleButton, gtk::Box)>| {
            let current = ui.stack.visible_child_name();
            // The API page is the architecture, zoomed on a route.
            let current = current.map(|c| {
                if c == "endpoint" {
                    "architecture".into()
                } else {
                    c
                }
            });
            for (page, b, content) in buttons {
                let on = current.as_deref() == Some(*page);
                b.set_active(on);
                if let Some(old) = content.first_child() {
                    let name = TABS.iter().find(|t| t.0 == *page).map_or("graph", |t| t.2);
                    content.remove(&old);
                    content.prepend(&icon(name, 15, if on { icons::TEXT } else { icons::MUTED }));
                }
            }
        }
    );
    sync(&buttons);
    ui.stack
        .connect_visible_child_name_notify(move |_| sync(&buttons));
    bar.append(&tabs);

    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    bar.append(&spacer);

    // `Valide`, or the number of errors and warnings.
    bar.append(&ui.pill);

    let group = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    let button = |icon_name: &str, text: &str, tip: &str, f: fn(&Shared)| {
        let b = tool(icon_name, text, tip, icons::TEXT);
        b.connect_clicked(clone!(
            #[strong]
            ui,
            move |_| f(&ui)
        ));
        group.append(&b);
    };
    button("new", "", "Nouveau projet vide", |ui| {
        guard(ui, ask_new_project)
    });
    button("sparkles", "", "Charger le projet d’exemple", |ui| {
        guard(ui, |ui| load_sample(ui, "orders"))
    });
    button(
        "schema",
        "",
        "Charger l’exemple complet : tous les types de blocs",
        |ui| guard(ui, |ui| load_sample(ui, "showcase")),
    );
    button("open", "", "Ouvrir (Ctrl+O)", |ui| guard(ui, pick_project));
    button("save", "", "Enregistrer (Ctrl+S)", save);
    button("copy", "", "Enregistrer sous… (Ctrl+Maj+S)", save_as);
    bar.append(&group);

    let update = ui.update_button.clone();
    update_content(&update, "");
    update.remove_css_class("flat");
    update.add_css_class("update");
    update.set_valign(gtk::Align::Center);
    update.set_tooltip_text(Some("Rechercher une nouvelle version"));
    bar.append(&update);

    let export = ui.export_button.clone();
    export.set_child(Some(&{
        let c = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        c.append(&icon("export", 15, "#1a0d0a"));
        c.append(&gtk::Label::new(Some("Exporter le code")));
        c
    }));
    export.add_css_class("primary");
    export.set_valign(gtk::Align::Center);
    export.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| export_code(&ui)
    ));
    bar.append(&export);

    // The less used actions: a menu at the end.
    let menu = gtk::MenuButton::new();
    menu.set_child(Some(&icon("grip", 15, icons::MUTED)));
    menu.add_css_class("ghost");
    menu.set_valign(gtk::Align::Center);
    let popover = gtk::Popover::new();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let entries: [MenuEntry; 2] = [
        ("Renommer le projet…", super::files::rename_project),
        ("Rechercher une mise à jour", |ui| {
            check_for_update(ui, true)
        }),
    ];
    for (label, action) in entries {
        let b = gtk::Button::with_label(label);
        b.add_css_class("flat");
        b.connect_clicked(clone!(
            #[strong]
            ui,
            #[weak]
            popover,
            move |_| {
                popover.popdown();
                action(&ui);
            }
        ));
        list.append(&b);
    }
    popover.set_child(Some(&list));
    menu.set_popover(Some(&popover));
    bar.append(&menu);

    let handle = gtk::WindowHandle::new();
    handle.set_child(Some(&bar));
    handle
}
