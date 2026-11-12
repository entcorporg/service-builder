//! The left column: tiles to add a service or a topic, then the structure tree of the project.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, Galley, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2,
    text::LayoutJob,
};
use sb_model::{HttpMethod, MessageKind, Position, Project, Service, ServiceKind, Topic};

use super::canvas::{Sel, select};
use super::common::{add_message, unique};
use crate::app::{App, Page};
use crate::icons;
use crate::theme::{self, *};

fn spot(p: &Project) -> Position {
    Position {
        x: 80.0 + p.services.len() as f64 * 40.0,
        y: 60.0 + p.topics.len() as f64 * 40.0,
    }
}

/// Adds a service of this kind with the next free ports, and selects it.
pub fn add_service(app: &mut App, kind: ServiceKind) {
    let mut name = String::new();
    app.edit(|p| {
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
    select(app, Some(Sel::Service(name)));
}

/// Adds a topic that carries the first event schema (one is made if there is none), and selects it.
pub fn add_topic(app: &mut App) {
    let mut name = String::new();
    app.edit(|p| {
        let message = match p.messages.iter().find(|m| m.kind == MessageKind::Event) {
            Some(m) => m.name.clone(),
            None => add_message(p, "Event", MessageKind::Event),
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
    select(app, Some(Sel::Topic(name)));
}

// ---- painting helpers ----

/// The text laid out on one line, cut with an ellipsis at `max` wide.
fn line(ui: &Ui, text: &str, font: egui::FontId, color: Color32, max: f32) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap.max_width = max.max(1.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    ui.painter().layout_job(job)
}

fn wrapped(ui: &Ui, text: &str, font: egui::FontId, color: Color32, width: f32) -> Arc<Galley> {
    let job = LayoutJob::simple(text.to_string(), font, color, width.max(1.0));
    ui.painter().layout_job(job)
}

/// A title of the panel (`.palette-title`).
fn heading(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(RichText::new(text.to_uppercase()).size(11.0).color(FAINT));
    });
}

/// A title of the tree (`.tree-title`): written as it is.
fn subheading(ui: &mut Ui, text: &str) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(RichText::new(text).size(11.0).color(MUTED));
    });
}

fn empty(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(RichText::new(text).size(12.0).color(FAINT));
    });
}

/// One tile; true when it was clicked.
fn tile(ui: &mut Ui, topic: bool, icon_name: &str, title: &str, hint: &str) -> bool {
    let accent = if topic { TOPIC } else { SERVICE };
    let width = ui.available_width();
    // 1 px border, 8 px of padding; grip, badge, then the text, then the plus.
    let text_w = width - 2.0 - 16.0 - (14.0 + 8.0 + 34.0 + 8.0) - (8.0 + 16.0);
    let t = wrapped(ui, title, theme::prop(BASE), TEXT, text_w);
    let h = wrapped(ui, hint, theme::prop(11.0), MUTED, text_w);
    let text_h = t.size().y + h.size().y;
    let height = (text_h.max(34.0) + 18.0 + 2.0).ceil();
    let (rect, r) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let r = r
        .on_hover_text(format!("Ajouter : {title}"))
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let radius = CornerRadius::same(10);
    let hov = r.hovered();
    ui.painter()
        .rect_filled(rect, radius, if hov { RAISED } else { PANEL2 });
    ui.painter().rect_stroke(
        rect.shrink(0.5),
        radius,
        Stroke::new(1.0, if hov { BORDER_STRONG } else { BORDER }),
        StrokeKind::Inside,
    );
    let left = rect.left() + 9.0;
    let cy = rect.center().y;
    icons::paint(
        ui,
        "grip",
        Rect::from_center_size(Pos2::new(left + 7.0, cy), Vec2::splat(14.0)),
        FAINT,
    );
    let badge = Rect::from_center_size(Pos2::new(left + 14.0 + 8.0 + 17.0, cy), Vec2::splat(34.0));
    ui.painter()
        .rect_filled(badge, CornerRadius::same(9), alpha(accent, 41));
    icons::paint(
        ui,
        icon_name,
        Rect::from_center_size(badge.center(), Vec2::splat(18.0)),
        accent,
    );
    let tx = badge.right() + 8.0;
    let ty = cy - text_h / 2.0;
    let th = t.size().y;
    ui.painter().galley(Pos2::new(tx, ty), t, TEXT);
    ui.painter().galley(Pos2::new(tx, ty + th), h, MUTED);
    icons::paint(
        ui,
        "plus",
        Rect::from_center_size(Pos2::new(rect.right() - 9.0 - 8.0, cy), Vec2::splat(16.0)),
        MUTED,
    );
    r.clicked()
}

/// The tiles that add a service or a topic.
pub fn tiles(app: &mut App, ui: &mut Ui) {
    ui.spacing_mut().item_spacing.y = 8.0;
    heading(ui, "Éléments");
    if tile(
        ui,
        false,
        "service",
        "Service",
        "gRPC, HTTP + consumers Kafka",
    ) {
        add_service(app, ServiceKind::Standard);
    }
    if tile(
        ui,
        false,
        "publish",
        "Service mail",
        "MTA prêt à l’emploi : événements → mails",
    ) {
        add_service(app, ServiceKind::Mta);
    }
    if tile(
        ui,
        false,
        "publish",
        "Service temps réel",
        "SSE prêt à l’emploi : événements → navigateurs (DragonflyDB)",
    ) {
        add_service(app, ServiceKind::Realtime);
    }
    if tile(ui, true, "topic", "Topic", "Flux d’événements Kafka") {
        add_topic(app);
    }
    heading(ui, "Structure");
}

/// A row of the tree; `paint` draws the content from the left-centre point, within `max` wide.
fn tree_row(
    ui: &mut Ui,
    sub: bool,
    active: bool,
    paint: impl FnOnce(&Ui, Pos2, f32),
) -> egui::Response {
    let width = ui.available_width();
    let height = if sub { 26.0 } else { 28.0 };
    let (rect, r) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
    if active {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(7), alpha(SERVICE, 46));
    } else if r.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(7), RAISED);
    }
    let left = rect.left() + if sub { 26.0 } else { 8.0 };
    paint(
        ui,
        Pos2::new(left, rect.center().y),
        rect.right() - 8.0 - left,
    );
    r
}

/// A row with an icon and a name.
fn named_row(ui: &mut Ui, icon_name: &str, text: &str, mono: bool, active: bool) -> egui::Response {
    tree_row(ui, false, active, |ui, at, max| {
        icons::paint(
            ui,
            icon_name,
            Rect::from_center_size(Pos2::new(at.x + 6.5, at.y), Vec2::splat(13.0)),
            MUTED,
        );
        let font = if mono {
            theme::mono(SM)
        } else {
            theme::prop(BASE)
        };
        let color = if active { Color32::WHITE } else { TEXT };
        let g = line(ui, text, font, color, max - 21.0);
        let pos = Pos2::new(at.x + 21.0, at.y - g.size().y / 2.0);
        ui.painter().galley(pos, g, color);
    })
}

/// A route: the method as a pill, then the path.
fn route_row(ui: &mut Ui, method: HttpMethod, path: &str, active: bool) -> egui::Response {
    tree_row(ui, true, active, |ui, at, max| {
        let color = match method {
            HttpMethod::Get => OK,
            HttpMethod::Delete => ERR,
            HttpMethod::Put | HttpMethod::Patch => SERVICE,
            HttpMethod::Post => TOPIC,
        };
        let pill = line(ui, method.as_str(), theme::mono(9.0), color, 200.0);
        let pw = pill.size().x + 10.0;
        let pr = Rect::from_center_size(Pos2::new(at.x + pw / 2.0, at.y), Vec2::new(pw, 15.0));
        ui.painter()
            .rect_filled(pr, CornerRadius::same(4), alpha(color, 41));
        ui.painter().galley(
            Pos2::new(pr.left() + 5.0, at.y - pill.size().y / 2.0),
            pill,
            color,
        );
        let g = line(ui, path, theme::mono(11.0), TEXT, max - pw - 6.0);
        let pos = Pos2::new(at.x + pw + 6.0, at.y - g.size().y / 2.0);
        ui.painter().galley(pos, g, TEXT);
    })
}

/// The structure of the project (services, their routes…); also the left column of the API page, where only the
/// services, their routes and the API schemas matter.
pub fn tree(app: &mut App, ui: &mut Ui) {
    let api_only = app.page == Page::Endpoint;
    let selected = app.canvas.selected.clone();
    let p = app.project();
    ui.spacing_mut().item_spacing.y = 2.0;

    subheading(ui, "Services");
    if p.services.is_empty() {
        empty(ui, "Aucun service");
    }
    for svc in &p.services {
        let active = !api_only && selected == Some(Sel::Service(svc.name.clone()));
        if named_row(ui, "service", &svc.name, false, active).clicked() {
            let name = svc.name.clone();
            app.later(move |app| {
                // On the API page, a service is the architecture, with that service selected.
                if api_only {
                    app.page = Page::Architecture;
                }
                select(app, Some(Sel::Service(name)));
            });
        }
        for r in &svc.routes {
            let open = api_only
                && app
                    .endpoint
                    .open
                    .as_ref()
                    .is_some_and(|(s, n)| *s == svc.name && *n == r.name);
            if route_row(ui, r.method, &r.path, open).clicked() {
                let (service, route) = (svc.name.clone(), r.name.clone());
                app.later(move |app| super::endpoint::open(app, &service, &route));
            }
        }
    }

    if !api_only {
        subheading(ui, "Topics");
        if p.topics.is_empty() {
            empty(ui, "Aucun topic");
        }
        for t in &p.topics {
            let active = selected == Some(Sel::Topic(t.name.clone()));
            if named_row(ui, "topic", &t.name, true, active).clicked() {
                let name = t.name.clone();
                app.later(move |app| select(app, Some(Sel::Topic(name))));
            }
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
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label(RichText::new(title).size(11.0).color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (rect, r) = ui.allocate_exact_size(Vec2::splat(19.0), Sense::click());
                if r.hovered() {
                    ui.painter()
                        .rect_filled(rect, CornerRadius::same(7), RAISED);
                }
                let plus = Rect::from_center_size(rect.center(), Vec2::splat(13.0));
                icons::paint(ui, "plus", plus, MUTED);
                let tip = format!("Ajouter un schéma {title}");
                if r.on_hover_text(tip).clicked() {
                    app.later(move |app| {
                        let mut added = String::new();
                        app.edit(|p| {
                            let base = match kind {
                                MessageKind::Event => "NewEvent",
                                MessageKind::Api => "NewApiSchema",
                                _ => "NewMessage",
                            };
                            added = add_message(p, base, kind);
                        });
                        super::schemas::open_message(app, &added);
                    });
                }
            });
        });
        let list: Vec<_> = p.messages.iter().filter(|m| m.kind == kind).collect();
        if list.is_empty() {
            empty(ui, "Aucun schéma");
        }
        for m in list {
            let active = selected == Some(Sel::Messages(m.name.clone()));
            if named_row(ui, "schema", &m.name, true, active).clicked() {
                let name = m.name.clone();
                app.later(move |app| super::schemas::open_message(app, &name));
            }
        }
    }
}
