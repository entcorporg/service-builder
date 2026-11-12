//! The API page: the configurator of one HTTP route, in place of the canvas. The logic of the route on the left,
//! the JSON it receives and answers on the right.

use eframe::egui::{
    self, Align, CornerRadius, Frame, Layout, Margin, RichText, ScrollArea, Stroke, Ui, Vec2,
};
use sb_app::scope::variables;
use sb_model::{
    Assignment, Expr, Field, FieldType, HttpMethod, Message, MessageKind, Project, Route, Severity,
    Step,
};

use super::common::{choice, commit_entry, empty, icon_btn, link, number, unique};
use super::editors::{SCALARS, type_name};
use super::steps::{Flow, Loc};
use crate::fonts::Weight;
use crate::app::{App, Page};
use crate::icons::icon;
use crate::theme::*;

#[derive(Default)]
pub struct EndpointState {
    /// The route open: (service, route).
    pub open: Option<(String, String)>,
}

/// Opens the configurator on a route.
pub fn open(app: &mut App, service: &str, route: &str) {
    app.endpoint.open = Some((service.to_string(), route.to_string()));
    app.page = Page::Endpoint;
}

fn default_status(m: HttpMethod) -> u16 {
    if m == HttpMethod::Post { 201 } else { 200 }
}

/// The names between braces in a route path: the fields filled from the URL.
fn path_params(path: &str) -> Vec<String> {
    let mut out = vec![];
    let mut rest = path;
    while let Some(a) = rest.find('{') {
        let Some(b) = rest[a..].find('}') else { break };
        let name = &rest[a + 1..a + b];
        if !name.is_empty() && !name.contains(['{', '/']) {
            out.push(name.to_string());
        }
        rest = &rest[a + b + 1..];
    }
    out
}

fn example(p: &Project, ty: &FieldType, depth: usize) -> serde_json::Value {
    use serde_json::json;
    match ty {
        FieldType::String | FieldType::Uuid => json!("string"),
        FieldType::Bool => json!(false),
        FieldType::Int32 | FieldType::Int64 => json!(0),
        FieldType::Float | FieldType::Double => json!(0.0),
        FieldType::Bytes => json!("base64"),
        FieldType::Timestamp => json!("2024-01-01T00:00:00Z"),
        FieldType::Message { name } => example_message(p, name, depth + 1, &|_| true),
    }
}

fn example_message(
    p: &Project,
    name: &str,
    depth: usize,
    keep: &dyn Fn(&Field) -> bool,
) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    if depth < 4
        && let Some(m) = p.messages.iter().find(|m| m.name == name)
    {
        for f in m.fields.iter().filter(|f| keep(f)) {
            let v = example(p, &f.ty, depth);
            map.insert(
                f.name.clone(),
                if f.repeated {
                    serde_json::json!([v])
                } else {
                    v
                },
            );
        }
    }
    serde_json::Value::Object(map)
}

fn example_json(p: &Project, message: &str, keep: &dyn Fn(&Field) -> bool) -> String {
    serde_json::to_string_pretty(&example_message(p, message, 0, keep)).unwrap_or_default()
}

fn example_query(p: &Project, message: &str, keep: &dyn Fn(&Field) -> bool) -> String {
    let Some(m) = p.messages.iter().find(|m| m.name == message) else {
        return String::new();
    };
    let parts: Vec<String> = m
        .fields
        .iter()
        .filter(|f| keep(f) && !f.ty.is_message())
        .map(|f| {
            format!(
                "{}={}",
                f.name,
                example(p, &f.ty, 0).to_string().trim_matches('"')
            )
        })
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

/// A wrapping label of the given size and colour.
fn small(ui: &mut Ui, text: &str, size: f32, color: egui::Color32) {
    ui.add(egui::Label::new(RichText::new(text).size(size).color(color)).wrap());
}

/// The method pill (`.tag-pill.m-*`).
fn tag(ui: &mut Ui, method: HttpMethod) {
    let color = match method {
        HttpMethod::Get => OK,
        HttpMethod::Delete => ERR,
        HttpMethod::Put | HttpMethod::Patch => SERVICE,
        _ => TOPIC,
    };
    Frame::new()
        .fill(alpha(color, 41))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.add(egui::Label::new(
                RichText::new(method.as_str())
                    .size(9.0)
                    .semibold()
                    .color(color),
            ));
        });
}

/// A control of a given width.
fn fixed<R>(ui: &mut Ui, w: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    fixed_h(ui, w, 26.0, add)
}

/// A control of a given size, centred in it.
fn fixed_h<R>(ui: &mut Ui, w: f32, h: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(
        Vec2::new(w, h),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_width(w);
            add(ui)
        },
    )
    .inner
}

fn labelled_field(ui: &mut Ui, label: &str, w: f32, add: impl FnOnce(&mut Ui)) {
    fixed_h(ui, w, 44.0, |ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            small(ui, &label.to_uppercase(), 10.0, FAINT);
            add(ui);
        });
    });
}

fn message_mut<'a>(p: &'a mut Project, name: &str) -> Option<&'a mut Message> {
    p.messages.iter_mut().find(|m| m.name == name)
}

fn route_mut<'a>(p: &'a mut Project, service: &str, route: &str) -> Option<&'a mut Route> {
    p.services
        .iter_mut()
        .find(|s| s.name == service)?
        .routes
        .iter_mut()
        .find(|r| r.name == route)
}

/// The fields of a schema, edited in place. `origin` says where a request field comes from.
fn fields_editor(
    app: &mut App,
    ui: &mut Ui,
    message: &str,
    origin: Option<&dyn Fn(&Field) -> String>,
) {
    let p = app.project();
    let fields = p
        .messages
        .iter()
        .find(|m| m.name == message)
        .map(|m| m.fields.clone())
        .unwrap_or_default();
    let mut types: Vec<String> = SCALARS.iter().map(|(n, _)| (*n).to_string()).collect();
    types.extend(
        p.messages
            .iter()
            .filter(|m| m.name != message)
            .map(|m| m.name.clone()),
    );
    ui.spacing_mut().item_spacing.y = 6.0;
    for (fi, f) in fields.iter().enumerate() {
        let m = message.to_string();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let total = ui.available_width();
            // tag, two checks, trash, and the gaps between the six items.
            let w = ((total - 22.0 - 2.0 * 40.0 - 26.0 - 5.0 * 6.0) / 2.0).max(60.0);
            fixed(ui, 22.0, |ui| {
                ui.label(
                    RichText::new(format!("#{}", fi + 1))
                        .font(mono(10.5))
                        .color(FAINT),
                );
            });
            if let Some(v) = fixed(ui, w, |ui| {
                commit_entry(ui, ("ep-fname", &m, fi), &f.name, true, "", false)
            }) {
                let m = m.clone();
                app.edit(move |p| {
                    if let Some(x) = message_mut(p, &m).and_then(|x| x.fields.get_mut(fi)) {
                        x.name = v;
                    }
                });
            }
            if let Some(v) = fixed(ui, w, |ui| {
                choice(ui, ("ep-ftype", &m, fi), &types, &type_name(&f.ty))
            }) {
                let m = m.clone();
                app.edit(move |p| {
                    let new = SCALARS
                        .iter()
                        .find(|(n, _)| *n == v)
                        .map_or(FieldType::Message { name: v }, |(_, t)| t.clone());
                    if let Some(x) = message_mut(p, &m).and_then(|x| x.fields.get_mut(fi)) {
                        x.ty = new;
                    }
                });
            }
            for (text, on, repeated) in [("[]", f.repeated, true), ("?", f.optional, false)] {
                let mut v = on;
                let r = fixed(ui, 40.0, |ui| ui.checkbox(&mut v, text))
                    .on_hover_text(if repeated { "Répété" } else { "Optionnel" });
                if r.changed() {
                    let m = m.clone();
                    app.edit(move |p| {
                        if let Some(x) = message_mut(p, &m).and_then(|x| x.fields.get_mut(fi)) {
                            if repeated {
                                x.repeated = v
                            } else {
                                x.optional = v
                            }
                        }
                    });
                }
            }
            if icon_btn(ui, "trash", &format!("Supprimer le champ {}", f.name), true).clicked() {
                let m = m.clone();
                app.edit(move |p| {
                    if let Some(x) = message_mut(p, &m)
                        && fi < x.fields.len()
                    {
                        x.fields.remove(fi);
                    }
                });
            }
        });
        if let Some(o) = origin {
            ui.horizontal(|ui| {
                ui.add_space(26.0);
                Frame::new()
                    .fill(alpha(SERVICE, 36))
                    .corner_radius(CornerRadius::same(4))
                    .inner_margin(Margin::symmetric(6, 0))
                    .show(ui, |ui| {
                        ui.label(RichText::new(o(f)).size(XS).color(SERVICE));
                    });
            });
        }
    }
    if link(ui, "+ Ajouter un champ").clicked() {
        let m = message.to_string();
        app.edit(move |p| {
            if let Some(x) = message_mut(p, &m) {
                let taken: Vec<&str> = x.fields.iter().map(|f| f.name.as_str()).collect();
                let name = unique("field", &taken);
                x.fields.push(Field::new(name, FieldType::String));
            }
        });
    }
}

fn json_preview(ui: &mut Ui, title: impl FnOnce(&mut Ui), body: Option<&str>) {
    Frame::new()
        .fill(CODE_BG)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            Frame::new()
                .inner_margin(Margin::symmetric(10, 5))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        title(ui)
                    });
                });
            if let Some(text) = body {
                let rect = ui.min_rect();
                ui.painter()
                    .hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, BORDER));
                Frame::new()
                    .inner_margin(Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(
                            egui::Label::new(
                                RichText::new(text).font(mono(SM)).color(hex(0x9be3b4)),
                            )
                            .selectable(true),
                        );
                    });
            }
        });
}

/// A card of the contract: the title, the schema picker, a button for a new schema, the status; then the body.
#[allow(clippy::too_many_arguments)]
fn card(
    app: &mut App,
    ui: &mut Ui,
    title: &str,
    current: &str,
    new_schema: impl FnOnce(&mut App),
    on_pick: impl FnOnce(&mut App, String),
    status: Option<u16>,
    body: impl FnOnce(&mut App, &mut Ui),
) {
    Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin {
            left: 14,
            right: 14,
            top: 12,
            bottom: 14,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            let apis: Vec<String> = app
                .project()
                .messages
                .iter()
                .filter(|m| m.kind == MessageKind::Api)
                .map(|m| m.name.clone())
                .collect();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let title_w = 80.0;
                fixed(ui, title_w, |ui| {
                    ui.label(
                        RichText::new(title.to_uppercase())
                            .size(11.0)
                            .semibold()
                            .color(MUTED),
                    );
                });
                let rest = ui.available_width() - 26.0 - 8.0;
                let rest = if status.is_some() { rest - 38.0 } else { rest };
                if let Some(v) = fixed(ui, rest.max(80.0), |ui| {
                    choice(ui, ("ep-pick", title), &apis, current)
                }) {
                    on_pick(app, v);
                }
                if icon_btn(
                    ui,
                    "plus",
                    &format!("Nouveau schéma de {}", title.to_lowercase()),
                    false,
                )
                .clicked()
                {
                    new_schema(app);
                }
                if let Some(s) = status {
                    ui.label(
                        RichText::new(s.to_string())
                            .font(mono(BASE))
                            .semibold()
                            .color(OK),
                    );
                }
            });
            body(app, ui);
        });
}

fn source_name(s: &sb_model::RequestSource) -> String {
    match s {
        sb_model::RequestSource::Header { name } | sb_model::RequestSource::Cookie { name } => {
            name.clone()
        }
    }
}

pub fn page(app: &mut App, ui: &mut Ui) {
    egui::Panel::left("endpoint-tree")
        .exact_size(228.0)
        .resizable(false)
        .frame(Frame::new().fill(PANEL).inner_margin(Margin::same(12)))
        .show(ui, |ui| {
            crate::widgets::section_title(ui, "Structure");
            ScrollArea::vertical().show(ui, |ui| super::palette::tree(app, ui));
        });
    egui::CentralPanel::no_frame().show(ui, |ui| draw(app, ui));
}

fn draw(app: &mut App, ui: &mut Ui) {
    let project = app.project();
    // The route that was opened, else the first there is (so that the page is never blank).
    let found = app
        .endpoint
        .open
        .clone()
        .and_then(|(s, r)| {
            project.service(&s).and_then(|svc| {
                svc.routes
                    .iter()
                    .find(|x| x.name == r)
                    .map(|_| (s.clone(), r.clone()))
            })
        })
        .or_else(|| {
            project
                .services
                .iter()
                .find_map(|s| s.routes.first().map(|r| (s.name.clone(), r.name.clone())))
        });
    let Some((sname, rname)) = found else {
        ui.add_space(24.0);
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                empty(ui, "Route introuvable.");
                if link(ui, "← Retour à l’architecture").clicked() {
                    app.page = Page::Architecture;
                }
            });
        });
        return;
    };
    let si = project
        .services
        .iter()
        .position(|s| s.name == sname)
        .unwrap_or(0);
    let svc = project.services[si].clone();
    let ri = svc.routes.iter().position(|r| r.name == rname).unwrap_or(0);
    let route = svc.routes[ri].clone();
    let base = format!("/services/{si}/routes/{ri}");

    // ---- header ----
    egui::Panel::top("endpoint-head")
        .frame(
            Frame::new()
                .fill(PANEL)
                .inner_margin(Margin::symmetric(14, 10)),
        )
        .show_separator_line(true)
        .show(ui, |ui| {
            header(app, ui, &sname, &rname, si, ri, &svc, &route)
        });

    // ---- problems of the route itself ----
    let analysis = app.analysis();
    let mine: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter(|d| {
            (d.path == base || d.path.starts_with(&format!("{base}/")))
                && !d.path.starts_with(&format!("{base}/steps"))
        })
        .collect();
    if !mine.is_empty() {
        egui::Panel::top("endpoint-problems")
            .frame(
                Frame::new()
                    .fill(alpha(ERR, 15))
                    .inner_margin(Margin::symmetric(14, 8)),
            )
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 3.0;
                for d in mine {
                    let bad = d.severity == Severity::Error;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.add(icon(
                            if bad { "error" } else { "warn" },
                            13.0,
                            if bad { ERR } else { WARN },
                        ));
                        ui.label(RichText::new(&d.code).font(mono(SM)).color(MUTED));
                        ui.label(RichText::new(&d.message).size(SM).color(if bad {
                            hex(0xffc9d0)
                        } else {
                            WARN
                        }));
                    });
                }
            });
    }

    // ---- body: the logic, then the contract ----
    let half = (ui.available_width() / 2.0).floor();
    egui::Panel::left("endpoint-logic")
        .exact_size(half)
        .resizable(false)
        .frame(Frame::new())
        .show_separator_line(true)
        .show(ui, |ui| {
            ScrollArea::vertical()
                .id_salt("endpoint-logic-scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    Frame::new()
                        .inner_margin(Margin {
                            left: 18,
                            right: 18,
                            top: 16,
                            bottom: 40,
                        })
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 8.0;
                            ui.label(
                                RichText::new("LOGIQUE").size(11.0).semibold().color(FAINT),
                            );
                            small(
                                ui,
                                "Les étapes s’exécutent dans une transaction. input est la requête ; la réponse est construite ici, puis renvoyée par l’étape « Répondre ».",
                                SM,
                                MUTED,
                            );
                            let loc = Loc {
                                service: si,
                                flow: Flow::Route(ri),
                                nest: vec![],
                            };
                            super::stepcards::steps_box(app, ui, &route.steps, &loc);
                        });
                });
        });
    egui::CentralPanel::no_frame().show(ui, |ui| {
        ScrollArea::vertical()
            .id_salt("endpoint-contract-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                Frame::new()
                    .inner_margin(Margin {
                        left: 18,
                        right: 18,
                        top: 16,
                        bottom: 40,
                    })
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 14.0;
                        contract(app, ui, &project, &sname, &rname, &route);
                    });
            });
    });
}

#[allow(clippy::too_many_arguments)]
fn header(
    app: &mut App,
    ui: &mut Ui,
    sname: &str,
    rname: &str,
    si: usize,
    ri: usize,
    svc: &sb_model::Service,
    route: &Route,
) {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        if icon_btn(
            ui,
            "trash",
            &format!("Supprimer la route {}", route.name),
            true,
        )
        .clicked()
        {
            let (s, r) = (sname.to_string(), rname.to_string());
            app.edit(move |p| {
                if let Some(s) = p.services.iter_mut().find(|x| x.name == s) {
                    s.routes.retain(|x| x.name != r);
                }
            });
            app.page = Page::Architecture;
        }
        labelled_field(ui, "Port HTTP", 90.0, |ui| {
            if let Some(v) = number(
                ui,
                ("ep-port", si),
                i64::from(svc.http_port.unwrap_or(0)),
                1,
                65535,
            ) {
                let s = sname.to_string();
                app.edit(move |p| {
                    if let Some(s) = p.services.iter_mut().find(|x| x.name == s) {
                        s.http_port = Some(v as u16);
                    }
                });
            }
        });
        let statuses: Vec<String> = ["200 OK", "201 Created", "202 Accepted"]
            .map(String::from)
            .to_vec();
        let now = statuses
            .iter()
            .find(|s| s.starts_with(&route.status.to_string()))
            .cloned()
            .unwrap_or_else(|| route.status.to_string());
        labelled_field(ui, "Statut", 130.0, |ui| {
            if let Some(v) = choice(ui, ("ep-status", si, ri), &statuses, &now) {
                let code: u16 = v
                    .split(' ')
                    .next()
                    .and_then(|c| c.parse().ok())
                    .unwrap_or(200);
                let (s, r) = (sname.to_string(), rname.to_string());
                app.edit(move |p| {
                    if let Some(r) = route_mut(p, &s, &r) {
                        r.status = code;
                    }
                });
            }
        });
        labelled_field(ui, "Nom", 150.0, |ui| {
            if let Some(v) = commit_entry(ui, ("ep-name", si, ri), &route.name, true, "", false) {
                if !sb_model::names::is_snake_ident(&v) {
                    app.say("Le nom de la route doit être en snake_case.");
                } else {
                    let (s, r) = (sname.to_string(), rname.to_string());
                    let new = v.clone();
                    app.edit(move |p| {
                        if let Some(r) = route_mut(p, &s, &r) {
                            r.name = new;
                        }
                    });
                    app.endpoint.open = Some((sname.to_string(), v));
                }
            }
        });
        let rest = ui.available_width();
        fixed_h(ui, rest, 44.0, |ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let back = ui.add(
                egui::Button::image_and_text(
                    icon("right", 14.0, TEXT).rotate(std::f32::consts::PI, Vec2::splat(0.5)),
                    "Architecture",
                )
                .fill(PANEL2)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(CornerRadius::same(8)),
            );
            if back.clicked() {
                app.page = Page::Architecture;
            }
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.add(icon("service", 14.0, MUTED));
                ui.label(RichText::new(&svc.name).color(MUTED));
            });
            let methods: Vec<String> = HttpMethod::ALL
                .iter()
                .map(|m| m.as_str().to_string())
                .collect();
            if let Some(v) = fixed(ui, 90.0, |ui| {
                choice(ui, ("ep-method", si, ri), &methods, route.method.as_str())
            }) && let Some(m) = HttpMethod::ALL.iter().find(|m| m.as_str() == v).copied()
            {
                let (s, r) = (sname.to_string(), rname.to_string());
                app.edit(move |p| {
                    if let Some(r) = route_mut(p, &s, &r) {
                        let before = default_status(r.method);
                        r.method = m;
                        if r.status == before {
                            r.status = default_status(m);
                        }
                    }
                });
            }
            if let Some(v) = commit_entry(ui, ("ep-path", si, ri), &route.path, true, "", false) {
                let (s, r) = (sname.to_string(), rname.to_string());
                app.edit(move |p| {
                    if let Some(r) = route_mut(p, &s, &r) {
                        r.path = v;
                    }
                });
            }
        });
    });
}

fn contract(
    app: &mut App,
    ui: &mut Ui,
    project: &Project,
    sname: &str,
    rname: &str,
    route: &Route,
) {
    let params = path_params(&route.path);
    let has_body = route.method.has_body();
    let new_schema = |which_request: bool| {
        let (sname, rname) = (sname.to_string(), rname.to_string());
        move |app: &mut App| {
            app.edit(move |p| {
                let pascal: String = rname
                    .split('_')
                    .map(|w| {
                        let mut c = w.chars();
                        c.next()
                            .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                            .unwrap_or_default()
                    })
                    .collect();
                let taken: Vec<&str> = p.messages.iter().map(|m| m.name.as_str()).collect();
                let name = unique(
                    &format!(
                        "{pascal}{}",
                        if which_request { "Request" } else { "Response" }
                    ),
                    &taken,
                );
                p.messages.push(Message {
                    name: name.clone(),
                    kind: MessageKind::Api,
                    fields: vec![Field::new("id", FieldType::String)],
                });
                if let Some(r) = route_mut(p, &sname, &rname) {
                    if which_request {
                        r.request = name
                    } else {
                        r.response = name
                    }
                }
            })
        }
    };
    let pick = |which_request: bool| {
        let (sname, rname) = (sname.to_string(), rname.to_string());
        move |app: &mut App, v: String| {
            app.edit(move |p| {
                if let Some(r) = route_mut(p, &sname, &rname) {
                    if which_request {
                        r.request = v
                    } else {
                        r.response = v
                    }
                }
            })
        }
    };

    // ---- request ----
    card(
        app,
        ui,
        "Requête",
        &route.request,
        new_schema(true),
        pick(true),
        None,
        |app, ui| {
            let hint = format!(
                "{}{}",
                if params.is_empty() {
                    String::new()
                } else {
                    format!(
                        "Dans le chemin : {}. ",
                        params
                            .iter()
                            .map(|n| format!("{{{n}}}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    )
                },
                if has_body {
                    "Le reste vient du corps JSON."
                } else {
                    "Le reste vient de la query string (champs simples)."
                }
            );
            small(ui, &hint, SM, MUTED);
            if project.messages.iter().any(|m| m.name == route.request) {
                let (params2, method) = (params.clone(), route.method);
                let origin = move |f: &Field| -> String {
                    if params2.contains(&f.name) {
                        "chemin".into()
                    } else if let Some(s) = &f.source {
                        format!(
                            "{} {}",
                            if matches!(s, sb_model::RequestSource::Header { .. }) {
                                "en-tête"
                            } else {
                                "cookie"
                            },
                            source_name(s)
                        )
                    } else if method.has_body() {
                        "corps".into()
                    } else {
                        "query".into()
                    }
                };
                fields_editor(app, ui, &route.request, Some(&origin));
                let not_path = |f: &Field| !params.contains(&f.name) && f.source.is_none();
                let text = has_body.then(|| example_json(project, &route.request, &not_path));
                json_preview(
                    ui,
                    |ui| {
                        tag(ui, route.method);
                        ui.label(RichText::new(&route.path).font(mono(SM)).color(MUTED));
                        if !has_body {
                            ui.label(
                                RichText::new(example_query(project, &route.request, &not_path))
                                    .font(mono(SM))
                                    .color(MUTED),
                            );
                        }
                    },
                    text.as_deref(),
                );
            } else {
                empty(ui, "Choisissez un schéma API pour la requête.");
            }
        },
    );

    // ---- response ----
    card(
        app,
        ui,
        "Réponse",
        &route.response,
        new_schema(false),
        pick(false),
        Some(route.status),
        |app, ui| response_body(app, ui, project, sname, rname, route),
    );
}

fn response_body(
    app: &mut App,
    ui: &mut Ui,
    project: &Project,
    sname: &str,
    rname: &str,
    route: &Route,
) {
    if !project.messages.iter().any(|m| m.name == route.response) {
        empty(ui, "Choisissez un schéma API pour la réponse.");
        return;
    }
    fields_editor(app, ui, &route.response, None);
    json_preview(
        ui,
        |ui| {
            ui.label(RichText::new("JSON renvoyé").size(SM).color(MUTED));
        },
        Some(&example_json(project, &route.response, &|_| true)),
    );
    ui.add_space(8.0);
    ui.label(
        RichText::new("VALEURS, PRODUITES PAR LA LOGIQUE")
            .size(11.0)
            .semibold()
            .color(FAINT),
    );
    let respond_at = route
        .steps
        .iter()
        .rposition(|s| matches!(s, Step::Respond { .. }));
    let scope_end = respond_at.unwrap_or(route.steps.len());
    let vars = variables(project, Some(&route.request), &route.steps[..scope_end]);
    let nested = route.steps.iter().any(|s| matches!(s, Step::Branch { .. }));
    if let Some(at) = respond_at {
        let Step::Respond { fields, .. } = &route.steps[at] else {
            unreachable!()
        };
        if let Some(change) = super::assignments::assignments(
            app,
            ui,
            ("ep-respond", sname, rname),
            &route.response,
            fields,
            &vars,
        ) {
            let (s, r) = (sname.to_string(), rname.to_string());
            app.edit(move |p| {
                ensure_respond(p, &s, &r);
                if let Some(list) = respond_fields(p, &s, &r) {
                    change(list);
                }
            });
        }
    } else if nested {
        empty(
            ui,
            "La réponse est produite dans une condition : éditez-la dans la logique.",
        );
    } else {
        empty(
            ui,
            "Aucune étape « Répondre » : la route renverra un objet vide.",
        );
        if link(ui, "+ Ajouter l’étape « Répondre »").clicked() {
            let (s, r) = (sname.to_string(), rname.to_string());
            app.edit(move |p| ensure_respond(p, &s, &r));
        }
    }
    if respond_at.is_some() || !nested {
        for v in vars
            .iter()
            .filter(|v| v.path != "input" && !v.path.contains('.') && v.ty == route.response)
        {
            if link(ui, &format!("✦ Tout lire depuis {}", v.path)).clicked() {
                let (s, r, name) = (sname.to_string(), rname.to_string(), v.path.clone());
                app.edit(move |p| read_from(p, &s, &r, &name));
            }
        }
        let taken: Vec<String> = vars
            .iter()
            .map(|v| v.path.split('.').next().unwrap_or("").to_string())
            .collect();
        if link(ui, "‹› Construire la réponse par la logique (Rust)")
            .on_hover_text("Ajoute une variable du type de la réponse, remplie en Rust (requêtes sur plusieurs tables par exemple)")
            .clicked()
        {
            let (s, r) = (sname.to_string(), rname.to_string());
            app.edit(move |p| build_by_logic(p, &s, &r, &taken));
        }
    }
}

fn respond_fields<'a>(
    p: &'a mut Project,
    service: &str,
    route: &str,
) -> Option<&'a mut Vec<Assignment>> {
    route_mut(p, service, route)?
        .steps
        .iter_mut()
        .rev()
        .find_map(|s| match s {
            Step::Respond { fields, .. } => Some(fields),
            _ => None,
        })
}

/// The route answers through its last top-level « Répondre » step: this makes sure there is one.
fn ensure_respond(p: &mut Project, service: &str, route: &str) {
    if let Some(r) = route_mut(p, service, route)
        && !r.steps.iter().any(|s| matches!(s, Step::Respond { .. }))
        && let Ok(step) = sb_app::stepform::blank("respond", &super::steps::new_id())
    {
        r.steps.push(step);
    }
}

/// Every field of the response reads the same field of `variable`.
fn read_from(p: &mut Project, service: &str, route: &str, variable: &str) {
    ensure_respond(p, service, route);
    let Some(response) = route_mut(p, service, route).map(|r| r.response.clone()) else {
        return;
    };
    let names: Vec<String> = p
        .messages
        .iter()
        .find(|m| m.name == response)
        .map(|m| m.fields.iter().map(|f| f.name.clone()).collect())
        .unwrap_or_default();
    if let Some(list) = respond_fields(p, service, route) {
        *list = names
            .into_iter()
            .map(|f| Assignment {
                value: Expr::Path {
                    path: format!("{variable}.{f}"),
                },
                field: f,
            })
            .collect();
    }
}

/// A variable of the response schema, to fill with Rust, then read field by field.
fn build_by_logic(p: &mut Project, service: &str, route: &str, taken: &[String]) {
    let Some(response) = route_mut(p, service, route).map(|r| r.response.clone()) else {
        return;
    };
    let refs: Vec<&str> = taken.iter().map(String::as_str).collect();
    let name = unique("result", &refs);
    let code = format!(
        "{{\n    // Interrogez vos tables avec `tx`, par exemple :\n    // let rows = sqlx::query(\"SELECT …\").fetch_all(&mut *tx).await?;\n    pb::{response} {{ ..Default::default() }}\n}}"
    );
    let step = Step::Let {
        id: super::steps::new_id(),
        name: name.clone(),
        value_type: FieldType::Message { name: response },
        optional: false,
        repeated: false,
        value: Expr::Raw { code },
    };
    if let Some(r) = route_mut(p, service, route) {
        let at = r
            .steps
            .iter()
            .rposition(|s| matches!(s, Step::Respond { .. }))
            .unwrap_or(r.steps.len());
        r.steps.insert(at, step);
    }
    read_from(p, service, route, &name);
}
