//! The API page: the configurator of one HTTP route, in place of the canvas. The logic of the route on the left,
//! the JSON it receives and answers on the right.

use gtk::glib::{self, clone};
use gtk::prelude::*;
use sb_model::{Field, FieldType, HttpMethod, Message, MessageKind, Project, Route, Severity};

use std::rc::Rc;

use sb_app::scope::variables;
use sb_model::{Assignment, Expr, Step};

use super::assignments::{Update, assignments};
use super::edit::edit;
use super::editors::{SCALARS, type_name};
use super::icons::{self, icon};
use super::inspector::{choice, commit_entry, empty, icon_button, link, number};
use super::steps::{Flow, Loc};
use super::ui::Shared;
use super::widgets::{clear, say};

/// Opens the configurator on a route.
pub fn open(ui: &Shared, service: &str, route: &str) {
    *ui.endpoint.borrow_mut() = Some((service.to_string(), route.to_string()));
    ui.stack.set_visible_child_name("endpoint");
    draw(ui);
    // The structure on the left shows which route is open.
    super::palette::draw_tree(ui);
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

fn small(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .build();
    l.add_css_class(class);
    l
}

fn tag(method: HttpMethod) -> gtk::Label {
    let m = gtk::Label::new(Some(method.as_str()));
    m.add_css_class("tag-pill");
    m.add_css_class(&format!("m-{}", method.as_str()));
    m
}

fn labelled_field(label: &str, w: &impl IsA<gtk::Widget>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
    b.append(&small(&label.to_uppercase(), "ep-field-label"));
    b.append(w);
    b.set_valign(gtk::Align::Center);
    b
}

fn message_mut<'a>(p: &'a mut Project, name: &str) -> Option<&'a mut Message> {
    p.messages.iter_mut().find(|m| m.name == name)
}

/// The fields of a schema, edited in place. `origin` says where a request field comes from.
fn fields_editor(
    ui: &Shared,
    message: &str,
    origin: Option<&dyn Fn(&Field) -> String>,
) -> gtk::Box {
    let host = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let (fields, types) = {
        let s = ui.state.borrow();
        let fields = s
            .project
            .messages
            .iter()
            .find(|m| m.name == message)
            .map(|m| m.fields.clone())
            .unwrap_or_default();
        let mut types: Vec<String> = SCALARS.iter().map(|(n, _)| (*n).to_string()).collect();
        types.extend(
            s.project
                .messages
                .iter()
                .filter(|m| m.name != message)
                .map(|m| m.name.clone()),
        );
        (fields, types)
    };
    for (fi, f) in fields.iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let tagn = small(&format!("#{}", fi + 1), "field-tag");
        row.append(&tagn);
        let m = message.to_string();
        let name = commit_entry(
            &f.name,
            true,
            "",
            false,
            clone!(
                #[strong]
                ui,
                #[strong]
                m,
                move |v| edit(&ui, |p| {
                    if let Some(x) = message_mut(p, &m).and_then(|x| x.fields.get_mut(fi)) {
                        x.name = v;
                    }
                })
            ),
        );
        name.set_hexpand(true);
        row.append(&name);
        let m = message.to_string();
        let ty = choice(
            &types,
            &type_name(&f.ty),
            clone!(
                #[strong]
                ui,
                #[strong]
                m,
                move |v| edit(&ui, |p| {
                    let new = SCALARS
                        .iter()
                        .find(|(n, _)| *n == v)
                        .map_or(FieldType::Message { name: v }, |(_, t)| t.clone());
                    if let Some(x) = message_mut(p, &m).and_then(|x| x.fields.get_mut(fi)) {
                        x.ty = new;
                    }
                })
            ),
        );
        ty.set_hexpand(true);
        row.append(&ty);
        for (text, on, repeated) in [("[]", f.repeated, true), ("?", f.optional, false)] {
            let c = gtk::CheckButton::with_label(text);
            c.set_active(on);
            c.set_tooltip_text(Some(if repeated { "Répété" } else { "Optionnel" }));
            let m = message.to_string();
            c.connect_toggled(clone!(
                #[strong]
                ui,
                move |b| {
                    let on = b.is_active();
                    edit(&ui, |p| {
                        if let Some(x) = message_mut(p, &m).and_then(|x| x.fields.get_mut(fi)) {
                            if repeated {
                                x.repeated = on
                            } else {
                                x.optional = on
                            }
                        }
                    });
                }
            ));
            row.append(&c);
        }
        let m = message.to_string();
        row.append(&icon_button(
            "trash",
            &format!("Supprimer le champ {}", f.name),
            true,
            clone!(
                #[strong]
                ui,
                move || edit(&ui, |p| {
                    if let Some(x) = message_mut(p, &m)
                        && fi < x.fields.len()
                    {
                        x.fields.remove(fi);
                    }
                })
            ),
        ));
        host.append(&row);
        if let Some(o) = origin {
            let l = small(&o(f), "origin");
            l.set_halign(gtk::Align::Start);
            l.set_margin_start(26);
            host.append(&l);
        }
    }
    let m = message.to_string();
    host.append(&link(
        "+ Ajouter un champ",
        clone!(
            #[strong]
            ui,
            move || edit(&ui, |p| {
                if let Some(x) = message_mut(p, &m) {
                    let taken: Vec<&str> = x.fields.iter().map(|f| f.name.as_str()).collect();
                    let name = super::inspector::unique("field", &taken);
                    x.fields.push(Field::new(name, FieldType::String));
                }
            })
        ),
    ));
    host
}

fn json_preview(title: &gtk::Box, body: Option<&str>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.add_css_class("json-preview");
    title.add_css_class("json-title");
    b.append(title);
    if let Some(text) = body {
        let l = gtk::Label::builder()
            .label(text)
            .xalign(0.0)
            .selectable(true)
            .build();
        l.add_css_class("json-body");
        b.append(&l);
    }
    b
}

fn card(
    ui: &Shared,
    title: &str,
    current: &str,
    apply_new: impl Fn() + 'static,
    on_pick: impl Fn(String) + 'static,
    status: Option<u16>,
) -> (gtk::Box, gtk::Box) {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 8);
    outer.add_css_class("ep-card");
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    head.append(&small(&title.to_uppercase(), "ep-card-title"));
    let apis: Vec<String> = ui
        .state
        .borrow()
        .project
        .messages
        .iter()
        .filter(|m| m.kind == MessageKind::Api)
        .map(|m| m.name.clone())
        .collect();
    let pick = choice(&apis, current, on_pick);
    pick.set_hexpand(true);
    head.append(&pick);
    head.append(&icon_button(
        "plus",
        &format!("Nouveau schéma de {}", title.to_lowercase()),
        false,
        apply_new,
    ));
    if let Some(s) = status {
        head.append(&small(&s.to_string(), "status-code"));
    }
    outer.append(&head);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    outer.append(&body);
    (outer, body)
}

fn route_mut<'a>(p: &'a mut Project, service: &str, route: &str) -> Option<&'a mut Route> {
    p.services
        .iter_mut()
        .find(|s| s.name == service)?
        .routes
        .iter_mut()
        .find(|r| r.name == route)
}

pub fn draw(ui: &Shared) {
    let host = &ui.endpoint_page;
    clear(host);
    let project = ui.state.borrow().project.clone();
    // The route that was opened, else the first there is (so that the page is never blank).
    let chosen = ui.endpoint.borrow().clone();
    let found = chosen
        .as_ref()
        .and_then(|(s, r)| {
            project.service(s).and_then(|svc| {
                svc.routes
                    .iter()
                    .find(|x| x.name == *r)
                    .map(|_| (s.clone(), r.clone()))
            })
        })
        .or_else(|| {
            project
                .services
                .iter()
                .find_map(|s| s.routes.first().map(|r| (s.name.clone(), r.name.clone())))
        });
    let back = link(
        "← Retour à l’architecture",
        clone!(
            #[strong]
            ui,
            move || ui.stack.set_visible_child_name("architecture")
        ),
    );
    let Some((sname, rname)) = found else {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 10);
        b.set_margin_start(24);
        b.set_margin_top(24);
        b.append(&empty("Route introuvable."));
        b.append(&back);
        host.append(&b);
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
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    head.add_css_class("ep-head");
    let back_btn = gtk::Button::with_label("← Architecture");
    back_btn.add_css_class("ep-back");
    back_btn.set_valign(gtk::Align::Center);
    back_btn.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| ui.stack.set_visible_child_name("architecture")
    ));
    head.append(&back_btn);
    let owner = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    owner.set_valign(gtk::Align::Center);
    owner.append(&icon("service", 14, icons::MUTED));
    owner.append(&small(&svc.name, "text-muted"));
    head.append(&owner);
    let methods: Vec<String> = HttpMethod::ALL
        .iter()
        .map(|m| m.as_str().to_string())
        .collect();
    let method = choice(
        &methods,
        route.method.as_str(),
        clone!(
            #[strong]
            ui,
            #[strong]
            sname,
            #[strong]
            rname,
            move |v| {
                if let Some(m) = HttpMethod::ALL.iter().find(|m| m.as_str() == v) {
                    let m = *m;
                    edit(&ui, |p| {
                        if let Some(r) = route_mut(p, &sname, &rname) {
                            let before = default_status(r.method);
                            r.method = m;
                            if r.status == before {
                                r.status = default_status(m);
                            }
                        }
                    });
                }
            }
        ),
    );
    method.set_valign(gtk::Align::Center);
    method.add_css_class("ep-method");
    head.append(&method);
    let path = commit_entry(
        &route.path,
        true,
        "",
        false,
        clone!(
            #[strong]
            ui,
            #[strong]
            sname,
            #[strong]
            rname,
            move |v| edit(&ui, |p| {
                if let Some(r) = route_mut(p, &sname, &rname) {
                    r.path = v;
                }
            })
        ),
    );
    path.set_hexpand(true);
    path.set_valign(gtk::Align::Center);
    path.add_css_class("ep-path");
    head.append(&path);
    let name = commit_entry(
        &route.name,
        true,
        "",
        false,
        clone!(
            #[strong]
            ui,
            #[strong]
            sname,
            #[strong]
            rname,
            move |v| {
                if !sb_model::names::is_snake_ident(&v) {
                    say(&ui, "Le nom de la route doit être en snake_case.");
                } else {
                    edit(&ui, |p| {
                        if let Some(r) = route_mut(p, &sname, &rname) {
                            r.name = v.clone();
                        }
                    });
                    *ui.endpoint.borrow_mut() = Some((sname.clone(), v));
                }
            }
        ),
    );
    name.set_width_request(150);
    head.append(&labelled_field("Nom", &name));
    let statuses: Vec<String> = ["200 OK", "201 Created", "202 Accepted"]
        .map(String::from)
        .to_vec();
    let status_now = statuses
        .iter()
        .find(|s| s.starts_with(&route.status.to_string()))
        .cloned()
        .unwrap_or_else(|| route.status.to_string());
    let status = choice(
        &statuses,
        &status_now,
        clone!(
            #[strong]
            ui,
            #[strong]
            sname,
            #[strong]
            rname,
            move |v| {
                let code: u16 = v
                    .split(' ')
                    .next()
                    .and_then(|c| c.parse().ok())
                    .unwrap_or(200);
                edit(&ui, |p| {
                    if let Some(r) = route_mut(p, &sname, &rname) {
                        r.status = code;
                    }
                });
            }
        ),
    );
    head.append(&labelled_field("Statut", &status));
    let port = number(
        i64::from(svc.http_port.unwrap_or(0)),
        1,
        65535,
        clone!(
            #[strong]
            ui,
            #[strong]
            sname,
            move |v| edit(&ui, |p| {
                if let Some(s) = p.services.iter_mut().find(|s| s.name == sname) {
                    s.http_port = Some(v as u16);
                }
            })
        ),
    );
    head.append(&labelled_field("Port HTTP", &port));
    head.append(&icon_button(
        "trash",
        &format!("Supprimer la route {}", route.name),
        true,
        clone!(
            #[strong]
            ui,
            #[strong]
            sname,
            #[strong]
            rname,
            move || {
                edit(&ui, |p| {
                    if let Some(s) = p.services.iter_mut().find(|s| s.name == sname) {
                        s.routes.retain(|r| r.name != rname);
                    }
                });
                ui.stack.set_visible_child_name("architecture");
            }
        ),
    ));
    host.append(&head);

    // ---- problems of the route itself ----
    let mine: Vec<_> = ui
        .state
        .borrow()
        .analysis
        .diagnostics
        .iter()
        .filter(|d| {
            (d.path == base || d.path.starts_with(&format!("{base}/")))
                && !d.path.starts_with(&format!("{base}/steps"))
        })
        .cloned()
        .collect();
    if !mine.is_empty() {
        let list = gtk::Box::new(gtk::Orientation::Vertical, 3);
        list.add_css_class("ep-problems");
        for d in mine {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let bad = d.severity == Severity::Error;
            line.append(&icon(
                if bad { "error" } else { "warn" },
                13,
                if bad { icons::ERR } else { icons::WARN },
            ));
            line.append(&small(&d.code, "font-mono"));
            line.append(&small(&d.message, if bad { "ep-err" } else { "ep-warn" }));
            list.append(&line);
        }
        host.append(&list);
    }

    // ---- body ----
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.set_vexpand(true);
    body.set_homogeneous(true);

    // The logic.
    let logic = gtk::Box::new(gtk::Orientation::Vertical, 8);
    logic.add_css_class("ep-col");
    logic.append(&small("LOGIQUE", "ep-col-title"));
    logic.append(&small(
        "Les étapes s’exécutent dans une transaction. input est la requête ; la réponse est construite ici, puis renvoyée par l’étape « Répondre ».",
        "ep-hint",
    ));
    let loc = Loc {
        service: si,
        flow: Flow::Route(ri),
        nest: vec![],
    };
    logic.append(&super::stepcards::steps_box(ui, &route.steps, &loc));
    let logic_scroll = gtk::ScrolledWindow::new();
    logic_scroll.set_child(Some(&logic));
    logic_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    logic_scroll.add_css_class("ep-logic");
    body.append(&logic_scroll);

    // The contract.
    let contract = gtk::Box::new(gtk::Orientation::Vertical, 14);
    contract.add_css_class("ep-col");
    let params = path_params(&route.path);
    let has_body = route.method.has_body();
    let new_schema = |which_request: bool| {
        let (sname, rname) = (sname.clone(), rname.clone());
        clone!(
            #[strong]
            ui,
            move || edit(&ui, |p| {
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
                let name = super::inspector::unique(
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
        )
    };
    let pick = |which_request: bool| {
        let (sname, rname) = (sname.clone(), rname.clone());
        clone!(
            #[strong]
            ui,
            move |v: String| edit(&ui, |p| {
                if let Some(r) = route_mut(p, &sname, &rname) {
                    if which_request {
                        r.request = v
                    } else {
                        r.response = v
                    }
                }
            })
        )
    };

    let (req_card, req_body) = card(
        ui,
        "Requête",
        &route.request,
        new_schema(true),
        pick(true),
        None,
    );
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
    req_body.append(&small(&hint, "ep-hint"));
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
        req_body.append(&fields_editor(ui, &route.request, Some(&origin)));
        let not_path = |f: &Field| !params.contains(&f.name) && f.source.is_none();
        let title = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        title.append(&tag(route.method));
        let p = small(&route.path, "font-mono");
        title.append(&p);
        if !has_body {
            title.append(&small(
                &example_query(&project, &route.request, &not_path),
                "font-mono",
            ));
        }
        let text = has_body.then(|| example_json(&project, &route.request, &not_path));
        req_body.append(&json_preview(&title, text.as_deref()));
    } else {
        req_body.append(&empty("Choisissez un schéma API pour la requête."));
    }
    contract.append(&req_card);

    let (res_card, res_body) = card(
        ui,
        "Réponse",
        &route.response,
        new_schema(false),
        pick(false),
        Some(route.status),
    );
    if project.messages.iter().any(|m| m.name == route.response) {
        res_body.append(&fields_editor(ui, &route.response, None));
        let title = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        title.append(&small("JSON renvoyé", "text-muted"));
        res_body.append(&json_preview(
            &title,
            Some(&example_json(&project, &route.response, &|_| true)),
        ));
        res_body.append(&small("VALEURS, PRODUITES PAR LA LOGIQUE", "ep-sub"));
        let respond_at = route
            .steps
            .iter()
            .rposition(|s| matches!(s, Step::Respond { .. }));
        let scope_end = respond_at.unwrap_or(route.steps.len());
        let vars = variables(&project, Some(&route.request), &route.steps[..scope_end]);
        let nested = route.steps.iter().any(|s| matches!(s, Step::Branch { .. }));
        if let Some(at) = respond_at {
            let Step::Respond { fields, .. } = &route.steps[at] else {
                unreachable!()
            };
            let (s, r) = (sname.clone(), rname.clone());
            let update: Update = Rc::new(clone!(
                #[strong]
                ui,
                move |f| edit(&ui, |p| {
                    ensure_respond(p, &s, &r);
                    if let Some(list) = respond_fields(p, &s, &r) {
                        f(list);
                    }
                })
            ));
            res_body.append(&assignments(ui, &route.response, fields, &vars, update));
        } else if nested {
            res_body.append(&empty(
                "La réponse est produite dans une condition : éditez-la dans la logique.",
            ));
        } else {
            res_body.append(&empty(
                "Aucune étape « Répondre » : la route renverra un objet vide.",
            ));
            let (s, r) = (sname.clone(), rname.clone());
            res_body.append(&link(
                "+ Ajouter l’étape « Répondre »",
                clone!(
                    #[strong]
                    ui,
                    move || edit(&ui, |p| ensure_respond(p, &s, &r))
                ),
            ));
        }
        if respond_at.is_some() || !nested {
            for v in vars
                .iter()
                .filter(|v| v.path != "input" && !v.path.contains('.') && v.ty == route.response)
            {
                let (s, r, name) = (sname.clone(), rname.clone(), v.path.clone());
                let b = link(
                    &format!("✦ Tout lire depuis {}", v.path),
                    clone!(
                        #[strong]
                        ui,
                        move || edit(&ui, |p| read_from(p, &s, &r, &name))
                    ),
                );
                res_body.append(&b);
            }
            let (s, r) = (sname.clone(), rname.clone());
            let taken: Vec<String> = vars
                .iter()
                .map(|v| v.path.split('.').next().unwrap_or("").to_string())
                .collect();
            let build = link(
                "‹› Construire la réponse par la logique (Rust)",
                clone!(
                    #[strong]
                    ui,
                    move || edit(&ui, |p| build_by_logic(p, &s, &r, &taken))
                ),
            );
            build.set_tooltip_text(Some("Ajoute une variable du type de la réponse, remplie en Rust (requêtes sur plusieurs tables par exemple)"));
            res_body.append(&build);
        }
    } else {
        res_body.append(&empty("Choisissez un schéma API pour la réponse."));
    }
    contract.append(&res_card);
    let contract_scroll = gtk::ScrolledWindow::new();
    contract_scroll.set_child(Some(&contract));
    contract_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    body.append(&contract_scroll);
    host.append(&body);
    let _ = glib::ControlFlow::Continue;
}

fn source_name(s: &sb_model::RequestSource) -> String {
    match s {
        sb_model::RequestSource::Header { name } | sb_model::RequestSource::Cookie { name } => {
            name.clone()
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
    let name = super::inspector::unique("result", &refs);
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
