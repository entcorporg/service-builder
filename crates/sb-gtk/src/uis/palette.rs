//! The left column of the Architecture page: what can be added, then the structure of the project.

use gtk::glib::clone;
use gtk::prelude::*;
use sb_model::{
    Field, FieldType, Message, MessageKind, Position, Project, Service, ServiceKind, Topic,
};

use super::canvas::{Sel, select};
use super::edit::edit;
use super::icons::{self, icon};
use super::ui::Shared;
use super::widgets::clear;

fn unique(base: &str, taken: &[&str]) -> String {
    if !taken.contains(&base) {
        return base.to_string();
    }
    (2..)
        .map(|i| format!("{base}{i}"))
        .find(|n| !taken.contains(&n.as_str()))
        .unwrap_or_default()
}

fn spot(p: &Project) -> Position {
    Position {
        x: 80.0 + p.services.len() as f64 * 40.0,
        y: 60.0 + p.topics.len() as f64 * 40.0,
    }
}

/// Adds a service of this kind with the next free ports, and selects it.
pub fn add_service(ui: &Shared, kind: ServiceKind) {
    let mut name = String::new();
    edit(ui, |p| {
        let port = p
            .services
            .iter()
            .map(|s| s.port)
            .max()
            .unwrap_or(0)
            .max(50050)
            + 1;
        let base = if kind == ServiceKind::Mta {
            "Mailer"
        } else if kind == ServiceKind::Realtime {
            "Live"
        } else {
            "NewService"
        };
        let taken: Vec<&str> = p.services.iter().map(|s| s.name.as_str()).collect();
        name = unique(base, &taken);
        let http_port = (kind != ServiceKind::Standard).then(|| {
            p.services
                .iter()
                .filter_map(|s| s.http_port)
                .max()
                .unwrap_or(8080)
                .max(8080)
                + 1
        });
        let at = spot(p);
        let mut svc = Service {
            name: name.clone(),
            port,
            rpcs: vec![],
            http_port,
            routes: vec![],
            tables: vec![],
            consumers: vec![],
            jobs: vec![],
            position: at,
            kind,
            templates: vec![],
            realtime: None,
        };
        if kind == ServiceKind::Realtime {
            svc.realtime = Some(sb_model::RealtimeConfig::default());
        }
        p.services.push(svc);
    });
    select(ui, Some(Sel::Service(name)));
}

/// Adds a topic that carries the first event schema (one is made if there is none), and selects it.
pub fn add_topic(ui: &Shared) {
    let mut name = String::new();
    edit(ui, |p| {
        let message = match p.messages.iter().find(|m| m.kind == MessageKind::Event) {
            Some(m) => m.name.clone(),
            None => {
                let taken: Vec<&str> = p.messages.iter().map(|m| m.name.as_str()).collect();
                let m = unique("Event", &taken);
                p.messages.push(Message {
                    name: m.clone(),
                    kind: MessageKind::Event,
                    fields: vec![Field::new("id", FieldType::String)],
                });
                m
            }
        };
        let taken: Vec<&str> = p.topics.iter().map(|t| t.name.as_str()).collect();
        name = unique("new.topic", &taken);
        let at = spot(p);
        p.topics.push(Topic {
            name: name.clone(),
            message,
            partitions: 3,
            replication_factor: 1,
            key_field: None,
            producers: vec![],
            position: at,
        });
    });
    select(ui, Some(Sel::Topic(name)));
}

fn tile(
    ui: &Shared,
    kind: &str,
    icon_name: &str,
    title: &str,
    hint: &str,
    add: impl Fn(&Shared) + 'static,
) -> gtk::Button {
    let accent = if kind == "topic" {
        icons::TOPIC
    } else {
        icons::SERVICE
    };
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&icon("grip", 14, icons::FAINT));
    let badge = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    badge.add_css_class("tile-icon");
    badge.add_css_class(kind);
    badge.set_valign(gtk::Align::Center);
    badge.set_halign(gtk::Align::Center);
    badge.append(&super::icons::centered(icon_name, 18, accent, 34));
    row.append(&badge);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    // Both wrap inside the width the tile is given, as in the web version (a 228 px column), instead of asking for it.
    let t = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .max_width_chars(1)
        .build();
    t.add_css_class("tile-title");
    let h = gtk::Label::builder()
        .label(hint)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .max_width_chars(1)
        .build();
    h.add_css_class("tile-hint");
    text.append(&t);
    text.append(&h);
    row.append(&text);
    row.append(&icon("plus", 16, icons::MUTED));
    let b = gtk::Button::new();
    b.add_css_class("tile");
    b.set_child(Some(&row));
    b.set_tooltip_text(Some(&format!("Ajouter : {title}")));
    b.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| add(&ui)
    ));
    b
}

fn heading(text: &str) -> gtk::Label {
    let l = gtk::Label::builder()
        .label(text.to_uppercase())
        .xalign(0.0)
        .build();
    l.add_css_class("palette-title");
    l
}

/// A title of the tree: written as it is.
fn subheading(text: &str) -> gtk::Label {
    let l = gtk::Label::builder().label(text).xalign(0.0).build();
    l.add_css_class("tree-title");
    l
}

fn tree_button(
    ui: &Shared,
    content: &impl IsA<gtk::Widget>,
    active: bool,
    sub: bool,
    go: impl Fn(&Shared) + 'static,
) -> gtk::Button {
    let b = gtk::Button::new();
    b.set_child(Some(content));
    b.add_css_class("tree-item");
    if sub {
        b.add_css_class("sub");
    }
    if active {
        b.add_css_class("active");
    }
    b.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| go(&ui)
    ));
    b
}

fn named(icon_name: &str, text: &str, mono: bool) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&icon(icon_name, 13, icons::MUTED));
    let l = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    if mono {
        l.add_css_class("font-mono");
    }
    row.append(&l);
    row
}

/// Rebuilds the structure under the tiles.
pub fn draw_tree(ui: &Shared) {
    fill_tree(ui, &ui.canvas.tree.clone(), false);
    fill_tree(ui, &ui.endpoint_tree.clone(), true);
}

/// `api_only`: on the API page, only the services, their routes and the API schemas matter.
fn fill_tree(ui: &Shared, tree: &gtk::Box, api_only: bool) {
    clear(tree);
    let selected = ui.canvas.state.borrow().selected.clone();
    let s = ui.state.borrow();
    let p = &s.project;

    tree.append(&subheading("Services"));
    if p.services.is_empty() {
        tree.append(
            &gtk::Label::builder()
                .label("Aucun service")
                .xalign(0.0)
                .css_classes(["empty"])
                .build(),
        );
    }
    for svc in &p.services {
        let name = svc.name.clone();
        let active = selected == Some(Sel::Service(name.clone()));
        tree.append(&tree_button(
            ui,
            &named("service", &svc.name, false),
            active,
            false,
            move |ui| {
                // On the API page, a service is the architecture, with that service selected.
                if api_only {
                    ui.stack.set_visible_child_name("architecture");
                }
                select(ui, Some(Sel::Service(name.clone())))
            },
        ));
        for r in &svc.routes {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let m = gtk::Label::new(Some(&r.method.as_str().to_uppercase()));
            m.add_css_class("tag-pill");
            m.add_css_class(&format!("m-{}", r.method.as_str()));
            row.append(&m);
            let path = gtk::Label::builder()
                .label(&r.path)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            path.add_css_class("font-mono");
            row.append(&path);
            let (service, route) = (svc.name.clone(), r.name.clone());
            let active = ui.endpoint.borrow().as_ref() == Some(&(service.clone(), route.clone()))
                && ui.stack.visible_child_name().as_deref() == Some("endpoint");
            tree.append(&tree_button(ui, &row, active, true, move |ui| {
                super::endpoint::open(ui, &service, &route)
            }));
        }
    }

    if !api_only {
        tree.append(&subheading("Topics"));
        if p.topics.is_empty() {
            tree.append(
                &gtk::Label::builder()
                    .label("Aucun topic")
                    .xalign(0.0)
                    .css_classes(["empty"])
                    .build(),
            );
        }
        for t in &p.topics {
            let name = t.name.clone();
            let active = selected == Some(Sel::Topic(name.clone()));
            tree.append(&tree_button(
                ui,
                &named("topic", &t.name, true),
                active,
                false,
                move |ui| select(ui, Some(Sel::Topic(name.clone()))),
            ));
        }
    }
    for (kind, title) in [
        (MessageKind::Event, "Événements"),
        (MessageKind::Proto, "Proto"),
        (MessageKind::Api, "API"),
    ]
    .into_iter()
    .filter(|(k, _)| !api_only || *k == MessageKind::Api)
    {
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let h = subheading(title);
        h.set_hexpand(true);
        head.append(&h);
        let add = gtk::Button::new();
        add.set_child(Some(&icon("plus", 13, icons::MUTED)));
        add.add_css_class("icon-btn");
        add.set_tooltip_text(Some(&format!("Ajouter un schéma {title}")));
        add.connect_clicked(clone!(
            #[strong]
            ui,
            move |_| {
                let mut added = String::new();
                edit(&ui, |p| {
                    let base = match kind {
                        MessageKind::Event => "NewEvent",
                        MessageKind::Api => "NewApiSchema",
                        _ => "NewMessage",
                    };
                    let taken: Vec<&str> = p.messages.iter().map(|m| m.name.as_str()).collect();
                    let name = unique(base, &taken);
                    added.clone_from(&name);
                    p.messages.push(Message {
                        name,
                        kind,
                        fields: vec![Field::new("id", FieldType::String)],
                    });
                });
                super::schemas::open_message(&ui, &added);
            }
        ));
        head.append(&add);
        tree.append(&head);
        let list: Vec<&Message> = p.messages.iter().filter(|m| m.kind == kind).collect();
        if list.is_empty() {
            tree.append(
                &gtk::Label::builder()
                    .label("Aucun schéma")
                    .xalign(0.0)
                    .css_classes(["empty"])
                    .build(),
            );
        }
        for m in list {
            let name = m.name.clone();
            let active = selected == Some(Sel::Messages(name.clone()));
            tree.append(&tree_button(
                ui,
                &named("schema", &m.name, true),
                active,
                false,
                move |ui| super::schemas::open_message(ui, &name),
            ));
        }
    }
}

/// The tiles, built once.
pub fn fill_tiles(ui: &Shared, tiles: &gtk::Box) {
    tiles.append(&heading("Éléments"));
    tiles.append(&tile(
        ui,
        "service",
        "service",
        "Service",
        "gRPC, HTTP + consumers Kafka",
        |ui| add_service(ui, ServiceKind::Standard),
    ));
    tiles.append(&tile(
        ui,
        "service",
        "publish",
        "Service mail",
        "MTA prêt à l’emploi : événements → mails",
        |ui| add_service(ui, ServiceKind::Mta),
    ));
    tiles.append(&tile(
        ui,
        "service",
        "publish",
        "Service temps réel",
        "SSE prêt à l’emploi : événements → navigateurs (DragonflyDB)",
        |ui| add_service(ui, ServiceKind::Realtime),
    ));
    tiles.append(&tile(
        ui,
        "topic",
        "topic",
        "Topic",
        "Flux d’événements Kafka",
        add_topic,
    ));
    tiles.append(&heading("Structure"));
}
