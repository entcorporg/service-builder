//! The right column of the Architecture page: the project, a service or a topic, edited in place.
//!
//! Every field applies when it is left (Enter or focus out), as in the web version. The logic of the handlers
//! (the steps) is edited on the Flows page, which the "Logique" buttons open.

use gtk::glib::{self, clone};
use gtk::prelude::*;
use sb_app::rename;
use sb_model::names::{is_pascal_ident, is_snake_ident, is_topic_name};
use sb_model::{
    Consumer, Field, FieldType, Job, Message, MessageKind, Project, Route, Rpc, Service,
    ServiceKind,
};

use super::canvas::{Sel, select};
use super::edit::edit;
use super::editors::{delete_service, delete_topic, service_form};
use super::icons::{self, icon};
use super::ui::Shared;
use super::widgets::{clear, say};

// ---- building blocks ----

/// A titled block of the inspector: an icon, a small uppercase title, an optional action on the right.
pub(super) fn section(
    title: &str,
    icon_name: &str,
    action: Option<gtk::Widget>,
) -> (gtk::Box, gtk::Box) {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.add_css_class("section");
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    head.append(&icon(icon_name, 14, icons::MUTED));
    let t = gtk::Label::new(Some(&title.to_uppercase()));
    t.add_css_class("section-head");
    t.set_hexpand(true);
    t.set_xalign(0.0);
    head.append(&t);
    if let Some(a) = action {
        head.append(&a);
    }
    outer.append(&head);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.set_margin_top(10);
    outer.append(&body);
    (outer, body)
}

pub(super) fn icon_button(
    name: &str,
    tip: &str,
    danger: bool,
    f: impl Fn() + 'static,
) -> gtk::Button {
    let b = gtk::Button::new();
    b.set_child(Some(&icon(name, 15, icons::MUTED)));
    b.set_tooltip_text(Some(tip));
    b.add_css_class("icon-btn");
    if danger {
        b.add_css_class("danger");
    }
    b.set_valign(gtk::Align::Center);
    b.connect_clicked(move |_| f());
    b
}

/// A label on the left, the control on the right, a faint hint under it.
pub(super) fn field(label: &str, control: &impl IsA<gtk::Widget>, hint: &str) -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 3);
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let l = gtk::Label::new(Some(label));
    l.add_css_class("row-label");
    l.set_xalign(0.0);
    l.set_width_request(84);
    l.set_valign(gtk::Align::Center);
    line.append(&l);
    control.set_hexpand(true);
    line.append(control);
    outer.append(&line);
    if !hint.is_empty() {
        let h = gtk::Label::builder()
            .label(hint)
            .xalign(0.0)
            .wrap(true)
            .build();
        h.add_css_class("row-hint");
        h.set_margin_start(94);
        outer.append(&h);
    }
    outer
}

pub(super) fn empty(text: &str) -> gtk::Label {
    let l = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .build();
    l.add_css_class("empty");
    l
}

/// Text that applies on Enter or when the focus leaves; Escape gives the old value back.
pub(super) fn commit_entry(
    value: &str,
    mono: bool,
    placeholder: &str,
    allow_empty: bool,
    apply: impl Fn(String) + 'static,
) -> gtk::Entry {
    let e = gtk::Entry::new();
    e.set_text(value);
    e.set_placeholder_text(Some(placeholder).filter(|p| !p.is_empty()));
    if mono {
        e.add_css_class("font-mono");
    }
    let original = value.to_string();
    let apply = std::rc::Rc::new(apply);
    fn done(e: &gtk::Entry, original: &str, apply: &dyn Fn(String), allow_empty: bool) {
        let v = e.text().trim().to_string();
        if v == original {
            return;
        }
        if v.is_empty() && !allow_empty {
            e.set_text(original);
        } else {
            apply(v);
        }
    }
    e.connect_activate({
        let (original, apply) = (original.clone(), apply.clone());
        move |e| done(e, &original, &*apply, allow_empty)
    });
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave({
        let e = e.clone();
        move |_| done(&e, &original, &*apply, allow_empty)
    });
    e.add_controller(focus);
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed({
        let value = value.to_string();
        move |c, key, _, _| {
            if key == gtk::gdk::Key::Escape
                && let Some(e) = c.widget().and_then(|w| w.downcast::<gtk::Entry>().ok())
            {
                e.set_text(&value);
            }
            gtk::glib::Propagation::Proceed
        }
    });
    e.add_controller(keys);
    e
}

pub(super) fn number(
    value: i64,
    min: i64,
    max: i64,
    apply: impl Fn(i64) + 'static,
) -> gtk::SpinButton {
    let s = gtk::SpinButton::with_range(min as f64, max as f64, 1.0);
    s.set_value(value as f64);
    s.set_numeric(true);
    s.connect_value_changed(move |s| apply(s.value() as i64));
    s
}

/// A list of choices; `selected` may be absent from `options` (it is then added at the end, as is).
pub(super) fn choice(
    options: &[String],
    selected: &str,
    apply: impl Fn(String) + 'static,
) -> gtk::DropDown {
    let mut list: Vec<String> = options.to_vec();
    if !list.iter().any(|o| o == selected) {
        list.push(selected.to_string());
    }
    let refs: Vec<&str> = list.iter().map(String::as_str).collect();
    let d = gtk::DropDown::from_strings(&refs);
    d.set_selected(list.iter().position(|o| o == selected).unwrap_or(0) as u32);
    d.connect_selected_notify(move |d| {
        if let Some(item) = list.get(d.selected() as usize) {
            apply(item.clone());
        }
    });
    d
}

/// A card with a chevron, a title, a summary and actions; it opens and closes without redrawing the inspector.
pub(super) fn card(
    ui: &Shared,
    id: String,
    tone: &str,
    title: &str,
    summary: &str,
    actions: Vec<gtk::Button>,
    body: &gtk::Box,
) -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.add_css_class("card");
    outer.add_css_class(tone);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    // `SB_OPEN_ALL`, with `SB_SCREENSHOT`: every card open.
    let open = ui.canvas.state.borrow().open.contains(&id) || std::env::var_os("SB_OPEN_ALL").is_some();
    let chevron = icon(if open { "down" } else { "right" }, 14, icons::FAINT);
    let toggle = gtk::Button::new();
    toggle.add_css_class("card-toggle");
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let holder = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    holder.append(&chevron);
    line.append(&holder);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("font-mono");
    t.add_css_class("card-name");
    line.append(&t);
    let s = gtk::Label::builder()
        .label(summary)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    s.add_css_class("step-summary");
    line.append(&s);
    toggle.set_child(Some(&line));
    toggle.set_hexpand(true);
    head.append(&toggle);
    for a in actions {
        head.append(&a);
    }
    outer.append(&head);
    body.add_css_class("card-body");
    body.set_visible(open);
    outer.append(body);
    toggle.connect_clicked(clone!(
        #[strong]
        ui,
        #[weak]
        body,
        #[weak]
        holder,
        move |_| {
            let now = !body.is_visible();
            body.set_visible(now);
            if let Some(old) = holder.first_child() {
                holder.remove(&old);
            }
            holder.append(&icon(if now { "down" } else { "right" }, 14, icons::FAINT));
            let mut st = ui.canvas.state.borrow_mut();
            if now {
                st.open.insert(id.clone());
            } else {
                st.open.remove(&id);
            }
        }
    ));
    outer
}

pub(super) fn link(text: &str, f: impl Fn() + 'static) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    b.add_css_class("link-btn");
    b.set_halign(gtk::Align::Start);
    b.connect_clicked(move |_| f());
    b
}

fn danger_button(text: &str, f: impl Fn() + 'static) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    b.add_css_class("danger-btn");
    b.set_margin_top(22);
    b.connect_clicked(move |_| f());
    b
}

fn add_button(tip: &str, enabled: bool, f: impl Fn() + 'static) -> gtk::Widget {
    let b = icon_button("plus", tip, false, f);
    b.set_sensitive(enabled);
    b.upcast()
}

pub(super) fn unique(base: &str, taken: &[&str]) -> String {
    if !taken.contains(&base) {
        return base.to_string();
    }
    (2..)
        .map(|i| format!("{base}{i}"))
        .find(|n| !taken.contains(&n.as_str()))
        .unwrap_or_default()
}

fn names_of(p: &Project, kind: MessageKind) -> Vec<String> {
    p.messages
        .iter()
        .filter(|m| m.kind == kind)
        .map(|m| m.name.clone())
        .collect()
}

pub(super) fn add_message(p: &mut Project, base: &str, kind: MessageKind) -> String {
    let taken: Vec<&str> = p.messages.iter().map(|m| m.name.as_str()).collect();
    let name = unique(base, &taken);
    p.messages.push(Message {
        name: name.clone(),
        kind,
        fields: vec![Field::new("id", FieldType::String)],
    });
    name
}

fn svc_mut<'a>(p: &'a mut Project, name: &str) -> Option<&'a mut Service> {
    p.services.iter_mut().find(|s| s.name == name)
}

/// `420` → `7 min`, `3600` → `1 h`: the period of a job, as a person says it.
fn every_label(secs: u32) -> String {
    if secs.is_multiple_of(86400) {
        format!("{} j", secs / 86400)
    } else if secs.is_multiple_of(3600) {
        format!("{} h", secs / 3600)
    } else if secs.is_multiple_of(60) {
        format!("{} min", secs / 60)
    } else {
        format!("{secs} s")
    }
}

fn open_logic(ui: &Shared) -> gtk::Button {
    let b = gtk::Button::with_label("Modifier la logique…");
    b.add_css_class("link-btn");
    b.set_halign(gtk::Align::Start);
    b.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| ui.stack.set_visible_child_name("flows")
    ));
    b
}

// ---- the inspector ----

pub fn inspect(ui: &Shared) {
    let host = &ui.canvas.inspector;
    clear(host);
    let selected = ui.canvas.state.borrow().selected.clone();
    match selected {
        Some(Sel::Service(name)) => service(ui, host, &name),
        Some(Sel::Topic(name)) => topic(ui, host, &name),
        Some(Sel::Messages(_)) => super::schemas::messages(ui, host),
        None => project(ui, host),
    }
}

fn project(ui: &Shared, host: &gtk::Box) {
    let intro = gtk::Box::new(gtk::Orientation::Vertical, 12);
    intro.add_css_class("inspector-empty");
    let h = gtk::Label::new(Some("Rien de sélectionné"));
    h.add_css_class("inspector-title");
    h.set_xalign(0.0);
    intro.append(&h);
    for text in [
        "Cliquez sur un service ou un topic dans le canvas pour configurer ses RPC, ses consumers et leur logique.",
        "Glissez un Service ou un Topic depuis la colonne de gauche pour construire votre architecture.",
    ] {
        intro.append(
            &gtk::Label::builder()
                .label(text)
                .xalign(0.0)
                .wrap(true)
                .build(),
        );
    }
    host.append(&intro);

    let attempts = ui.state.borrow().project.outbox.max_attempts;
    let (outer, body) = section("Publication des événements", "publish", None);
    body.append(&field(
        "Tentatives",
        &number(i64::from(attempts), 1, 100, clone!(
            #[strong]
            ui,
            move |v| edit(&ui, |p| p.outbox.max_attempts = v.clamp(1, 100) as u32)
        )),
        "Chaque événement publié passe par l’outbox du service (même transaction que le flow). Si Kafka le refuse après ce nombre de tentatives, il est déplacé sur <topic>.dlq.",
    ));
    host.append(&outer);
}

fn service(ui: &Shared, host: &gtk::Box, name: &str) {
    let Some(svc) = ui.state.borrow().project.service(name).cloned() else {
        host.append(&empty("Service introuvable."));
        return;
    };
    let project = ui.state.borrow().project.clone();
    let si = project
        .services
        .iter()
        .position(|s| s.name == svc.name)
        .unwrap_or(0);

    let (outer, body) = section("Service", "service", None);
    body.append(&field(
        "Nom",
        &commit_entry(
            &svc.name,
            true,
            "",
            false,
            clone!(
                #[strong]
                ui,
                #[strong(rename_to = old)]
                svc.name,
                move |v| {
                    if !is_pascal_ident(&v) {
                        say(&ui, "Le nom doit être en PascalCase, par exemple Orders.");
                    } else if ui.state.borrow().project.service(&v).is_some() {
                        say(&ui, &format!("Le service {v} existe déjà."));
                    } else {
                        edit(&ui, |p| rename::service(p, &old, &v));
                        select(&ui, Some(Sel::Service(v)));
                    }
                }
            ),
        ),
        "",
    ));
    body.append(&field(
        "Port gRPC",
        &number(
            i64::from(svc.port),
            1,
            65535,
            clone!(
                #[strong]
                ui,
                #[strong(rename_to = n)]
                svc.name,
                move |v| edit(&ui, |p| {
                    if let Some(s) = svc_mut(p, &n) {
                        s.port = v as u16;
                    }
                })
            ),
        ),
        "",
    ));
    if !svc.routes.is_empty() || svc.kind != ServiceKind::Standard {
        body.append(&field(
            "Port HTTP",
            &number(
                i64::from(svc.http_port.unwrap_or(0)),
                1,
                65535,
                clone!(
                    #[strong]
                    ui,
                    #[strong(rename_to = n)]
                    svc.name,
                    move |v| edit(&ui, |p| {
                        if let Some(s) = svc_mut(p, &n) {
                            s.http_port = Some(v as u16);
                        }
                    })
                ),
            ),
            "Port de l’API HTTP (les routes ci-dessous)",
        ));
    }
    host.append(&outer);

    // A ready-made service has no handlers to draw: its settings are in the form and on the Flows page.
    if svc.kind != ServiceKind::Standard {
        let (outer, body) = section(
            if svc.kind == ServiceKind::Mta {
                "Service mail"
            } else {
                "Service temps réel"
            },
            "publish",
            None,
        );
        body.append(&empty(if svc.kind == ServiceKind::Mta {
            "Service prêt à l’emploi : file d’attente, envoi SMTP, DKIM. Les événements à mailer et leurs modèles se règlent sur la page Flows."
        } else {
            "Service prêt à l’emploi : diffuse les événements aux navigateurs en SSE. Son réglage se fait sur la page Flows."
        }));
        let i = project
            .services
            .iter()
            .position(|s| s.name == svc.name)
            .unwrap_or(0);
        body.append(&link(
            "Modifier le service…",
            clone!(
                #[strong]
                ui,
                move || service_form(&ui, Some(i))
            ),
        ));
        body.append(&open_logic(ui));
        host.append(&outer);
        host.append(&danger_button(
            "Supprimer le service",
            clone!(
                #[strong]
                ui,
                move || {
                    delete_service(&ui, i);
                    select(&ui, None);
                }
            ),
        ));
        return;
    }

    // ---- RPC ----
    let sname = svc.name.clone();
    let (outer, body) = section(
        "RPC (gRPC)",
        "callRpc",
        Some(add_button(
            "Ajouter un RPC",
            true,
            clone!(
                #[strong]
                ui,
                #[strong]
                sname,
                move || edit(&ui, |p| {
                    let taken: Vec<&str> = p
                        .services
                        .iter()
                        .find(|s| s.name == sname)
                        .map(|s| s.rpcs.iter().map(|r| r.name.as_str()).collect())
                        .unwrap_or_default();
                    let name = unique("NewRpc", &taken);
                    let request = add_message(p, &format!("{name}Request"), MessageKind::Proto);
                    let response = add_message(p, &format!("{name}Response"), MessageKind::Proto);
                    if let Some(s) = svc_mut(p, &sname) {
                        s.rpcs.push(Rpc {
                            name,
                            request,
                            response,
                            steps: vec![],
                        });
                    }
                })
            ),
        )),
    );
    if svc.rpcs.is_empty() {
        body.append(&empty("Aucun RPC : ce service n’expose pas d’API gRPC."));
    }
    let protos = names_of(&project, MessageKind::Proto);
    for (ri, rpc) in svc.rpcs.iter().enumerate() {
        let id = format!("{}/rpc/{}", svc.name, rpc.name);
        if svc.rpcs.len() == 1 {
            ui.canvas.state.borrow_mut().open.insert(id.clone());
        }
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);
        inner.append(&field(
            "Nom",
            &commit_entry(
                &rpc.name,
                true,
                "",
                false,
                clone!(
                    #[strong]
                    ui,
                    #[strong]
                    sname,
                    #[strong(rename_to = old)]
                    rpc.name,
                    move |v| {
                        if !is_pascal_ident(&v) {
                            say(&ui, "Le nom du RPC doit être en PascalCase.");
                        } else {
                            edit(&ui, |p| rename::rpc(p, &sname, &old, &v));
                        }
                    }
                ),
            ),
            "",
        ));
        for (label, current, is_request) in [
            ("Requête", &rpc.request, true),
            ("Réponse", &rpc.response, false),
        ] {
            inner.append(&field(
                label,
                &choice(
                    &protos,
                    current,
                    clone!(
                        #[strong]
                        ui,
                        #[strong]
                        sname,
                        move |v| edit(&ui, |p| {
                            if let Some(r) = svc_mut(p, &sname).and_then(|s| s.rpcs.get_mut(ri)) {
                                if is_request {
                                    r.request = v
                                } else {
                                    r.response = v
                                }
                            }
                        })
                    ),
                ),
                "",
            ));
        }
        // The steps of the handler, in place, as in the web version.
        let steps = gtk::Label::new(Some("LOGIQUE"));
        steps.add_css_class("steps-title");
        steps.set_xalign(0.0);
        inner.append(&steps);
        inner.append(&super::stepcards::steps_box(
            ui,
            &rpc.steps,
            &super::steps::Loc {
                service: si,
                flow: super::steps::Flow::Rpc(ri),
                nest: vec![],
            },
        ));
        let trash = icon_button(
            "trash",
            &format!("Supprimer {}", rpc.name),
            true,
            clone!(
                #[strong]
                ui,
                #[strong]
                sname,
                move || edit(&ui, |p| {
                    if let Some(s) = svc_mut(p, &sname)
                        && ri < s.rpcs.len()
                    {
                        s.rpcs.remove(ri);
                    }
                })
            ),
        );
        body.append(&card(
            ui,
            id,
            "rpc",
            &rpc.name,
            &format!("{} → {}", rpc.request, rpc.response),
            vec![trash],
            &inner,
        ));
    }
    host.append(&outer);

    // ---- HTTP ----
    let (outer, body) = section(
        "API HTTP",
        "publish",
        Some(add_button(
            "Ajouter une route HTTP",
            true,
            clone!(
                #[strong]
                ui,
                #[strong]
                sname,
                move || edit(&ui, |p| {
                    let taken: Vec<&str> = p
                        .services
                        .iter()
                        .find(|s| s.name == sname)
                        .map(|s| s.routes.iter().map(|r| r.name.as_str()).collect())
                        .unwrap_or_default();
                    let name = unique("new_route", &taken);
                    let pascal: String = name
                        .split('_')
                        .map(|w| {
                            let mut c = w.chars();
                            c.next()
                                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                                .unwrap_or_default()
                        })
                        .collect();
                    let request = add_message(p, &format!("{pascal}Request"), MessageKind::Api);
                    let response = add_message(p, &format!("{pascal}Response"), MessageKind::Api);
                    let next_http = p
                        .services
                        .iter()
                        .filter_map(|s| s.http_port)
                        .max()
                        .unwrap_or(8080)
                        .max(8080)
                        + 1;
                    if let Some(s) = svc_mut(p, &sname) {
                        if s.http_port.is_none() {
                            s.http_port = Some(next_http);
                        }
                        s.routes.push(Route {
                            path: format!("/{}", name.replace('_', "-")),
                            name,
                            method: sb_model::HttpMethod::Get,
                            request,
                            response,
                            status: 200,
                            steps: vec![],
                        });
                    }
                })
            ),
        )),
    );
    if svc.routes.is_empty() {
        body.append(&empty(
            "Aucune route : ce service ne reçoit aucune requête HTTP du front.",
        ));
    }
    for (ri, r) in svc.routes.iter().enumerate() {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let open = gtk::Button::new();
        open.add_css_class("route-open");
        let c = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let m = gtk::Label::new(Some(r.method.as_str()));
        m.add_css_class("tag-pill");
        m.add_css_class(&format!("m-{}", r.method.as_str()));
        c.append(&m);
        let path = gtk::Label::builder()
            .label(&r.path)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .hexpand(true)
            .build();
        path.add_css_class("font-mono");
        c.append(&path);
        let n = r.steps.len();
        let s = gtk::Label::new(Some(&format!("{n} étape{}", if n > 1 { "s" } else { "" })));
        s.add_css_class("step-summary");
        c.append(&s);
        open.set_child(Some(&c));
        open.set_hexpand(true);
        open.set_tooltip_text(Some("Ouvrir le configurateur d’endpoint"));
        let route_name = r.name.clone();
        open.connect_clicked(clone!(
            #[strong]
            ui,
            #[strong]
            sname,
            move |_| super::endpoint::open(&ui, &sname, &route_name)
        ));
        line.append(&open);
        line.append(&icon_button(
            "trash",
            &format!("Supprimer la route {}", r.name),
            true,
            clone!(
                #[strong]
                ui,
                #[strong]
                sname,
                move || edit(&ui, |p| {
                    if let Some(s) = svc_mut(p, &sname)
                        && ri < s.routes.len()
                    {
                        s.routes.remove(ri);
                    }
                })
            ),
        ));
        body.append(&line);
    }
    host.append(&outer);

    // ---- consumers ----
    let topics: Vec<String> = project.topics.iter().map(|t| t.name.clone()).collect();
    let (outer, body) = section(
        "Consumers (Kafka)",
        "topic",
        Some(add_button(
            if topics.is_empty() {
                "Créez d’abord un topic"
            } else {
                "Ajouter un consumer"
            },
            !topics.is_empty(),
            clone!(
                #[strong]
                ui,
                #[strong]
                sname,
                move || edit(&ui, |p| {
                    let Some(topic) = p.topics.first().map(|t| t.name.clone()) else {
                        return;
                    };
                    if let Some(s) = svc_mut(p, &sname) {
                        let taken: Vec<String> =
                            s.consumers.iter().map(|c| c.name.clone()).collect();
                        let refs: Vec<&str> = taken.iter().map(String::as_str).collect();
                        s.consumers.push(Consumer {
                            name: unique("on_event", &refs),
                            topic,
                            group_id: None,
                            idempotent_on: None,
                            mail: None,
                            steps: vec![],
                        });
                    }
                })
            ),
        )),
    );
    if svc.consumers.is_empty() {
        body.append(&empty(
            "Aucun consumer : ce service ne réagit à aucun événement.",
        ));
    }
    for (ci, c) in svc.consumers.iter().enumerate() {
        let id = format!("{}/consumer/{}", svc.name, c.name);
        if svc.consumers.len() == 1 {
            ui.canvas.state.borrow_mut().open.insert(id.clone());
        }
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);
        inner.append(&field(
            "Nom",
            &commit_entry(
                &c.name,
                true,
                "",
                false,
                clone!(
                    #[strong]
                    ui,
                    #[strong]
                    sname,
                    move |v| {
                        if !is_snake_ident(&v) {
                            say(&ui, "Le nom du consumer doit être en snake_case.");
                        } else {
                            edit(&ui, |p| {
                                if let Some(x) =
                                    svc_mut(p, &sname).and_then(|s| s.consumers.get_mut(ci))
                                {
                                    x.name = v;
                                }
                            })
                        }
                    }
                ),
            ),
            "",
        ));
        inner.append(&field(
            "Topic",
            &choice(
                &topics,
                &c.topic,
                clone!(
                    #[strong]
                    ui,
                    #[strong]
                    sname,
                    move |v| edit(&ui, |p| {
                        if let Some(x) = svc_mut(p, &sname).and_then(|s| s.consumers.get_mut(ci)) {
                            x.topic = v;
                        }
                    })
                ),
            ),
            "",
        ));
        inner.append(&field(
            "Groupe",
            &commit_entry(
                c.group_id.as_deref().unwrap_or(""),
                true,
                &format!("{}-{}", svc.name.to_lowercase(), c.name),
                true,
                clone!(
                    #[strong]
                    ui,
                    #[strong]
                    sname,
                    move |v| edit(&ui, |p| {
                        if let Some(x) = svc_mut(p, &sname).and_then(|s| s.consumers.get_mut(ci)) {
                            x.group_id = Some(v).filter(|v| !v.is_empty());
                        }
                    })
                ),
            ),
            "Par défaut : service-consumer",
        ));
        let keys: Vec<String> = project
            .topic(&c.topic)
            .and_then(|t| project.messages.iter().find(|m| m.name == t.message))
            .map(|m| {
                m.fields
                    .iter()
                    .filter(|f| {
                        !f.repeated
                            && !f.optional
                            && matches!(
                                f.ty,
                                FieldType::String
                                    | FieldType::Uuid
                                    | FieldType::Int32
                                    | FieldType::Int64
                            )
                    })
                    .map(|f| f.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        let none = "aucun : chaque livraison est traitée".to_string();
        let mut options = vec![none.clone()];
        options.extend(keys);
        inner.append(&field(
            "Dédoublonner sur",
            &choice(&options, &c.idempotent_on.clone().unwrap_or_else(|| none.clone()), clone!(
                #[strong]
                ui,
                #[strong]
                sname,
                move |v| edit(&ui, |p| {
                    if let Some(x) = svc_mut(p, &sname).and_then(|s| s.consumers.get_mut(ci)) {
                        x.idempotent_on = Some(v).filter(|v| *v != none);
                    }
                })
            )),
            "Kafka livre au moins une fois : un événement déjà traité est ignoré, la vérification étant dans la transaction du handler. Mémorisé 7 jours.",
        ));
        // The steps of the handler, in place, as in the web version.
        let steps = gtk::Label::new(Some("LOGIQUE"));
        steps.add_css_class("steps-title");
        steps.set_xalign(0.0);
        inner.append(&steps);
        inner.append(&super::stepcards::steps_box(
            ui,
            &c.steps,
            &super::steps::Loc {
                service: si,
                flow: super::steps::Flow::Consumer(ci),
                nest: vec![],
            },
        ));
        let trash = icon_button(
            "trash",
            &format!("Supprimer {}", c.name),
            true,
            clone!(
                #[strong]
                ui,
                #[strong]
                sname,
                move || edit(&ui, |p| {
                    if let Some(s) = svc_mut(p, &sname)
                        && ci < s.consumers.len()
                    {
                        s.consumers.remove(ci);
                    }
                })
            ),
        );
        body.append(&card(
            ui,
            id,
            "consumer",
            &c.name,
            &format!("⇠ {}", c.topic),
            vec![trash],
            &inner,
        ));
    }
    host.append(&outer);

    // ---- jobs ----
    let (outer, body) = section(
        "Tâches planifiées",
        "update",
        Some(add_button(
            "Ajouter une tâche planifiée",
            true,
            clone!(
                #[strong]
                ui,
                #[strong]
                sname,
                move || edit(&ui, |p| {
                    if let Some(s) = svc_mut(p, &sname) {
                        let taken: Vec<String> = s.jobs.iter().map(|j| j.name.clone()).collect();
                        let refs: Vec<&str> = taken.iter().map(String::as_str).collect();
                        s.jobs.push(Job {
                            name: unique("job", &refs),
                            every_secs: 3600,
                            steps: vec![],
                        });
                    }
                })
            ),
        )),
    );
    if svc.jobs.is_empty() {
        body.append(&empty(
            "Aucune tâche : rien ne s’exécute ici sans requête ni événement.",
        ));
    }
    for (ji, j) in svc.jobs.iter().enumerate() {
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);
        inner.append(&field(
            "Nom",
            &commit_entry(
                &j.name,
                true,
                "",
                false,
                clone!(
                    #[strong]
                    ui,
                    #[strong]
                    sname,
                    move |v| edit(&ui, |p| {
                        if let Some(x) = svc_mut(p, &sname).and_then(|s| s.jobs.get_mut(ji)) {
                            x.name = v;
                        }
                    })
                ),
            ),
            "",
        ));
        inner.append(&field(
            "Toutes les",
            &number(
                i64::from(j.every_secs),
                1,
                2_592_000,
                clone!(
                    #[strong]
                    ui,
                    #[strong]
                    sname,
                    move |v| edit(&ui, |p| {
                        if let Some(x) = svc_mut(p, &sname).and_then(|s| s.jobs.get_mut(ji)) {
                            x.every_secs = v as u32;
                        }
                    })
                ),
            ),
            "En secondes, de 1 à 2 592 000 (30 jours). Un premier passage a lieu au démarrage.",
        ));
        // The steps of the handler, in place, as in the web version.
        let steps = gtk::Label::new(Some("LOGIQUE"));
        steps.add_css_class("steps-title");
        steps.set_xalign(0.0);
        inner.append(&steps);
        inner.append(&super::stepcards::steps_box(
            ui,
            &j.steps,
            &super::steps::Loc {
                service: si,
                flow: super::steps::Flow::Job(ji),
                nest: vec![],
            },
        ));
        let trash = icon_button(
            "trash",
            &format!("Supprimer la tâche {}", j.name),
            true,
            clone!(
                #[strong]
                ui,
                #[strong]
                sname,
                move || edit(&ui, |p| {
                    if let Some(s) = svc_mut(p, &sname)
                        && ji < s.jobs.len()
                    {
                        s.jobs.remove(ji);
                    }
                })
            ),
        );
        body.append(&card(
            ui,
            format!("{}/job/{}", svc.name, j.name),
            "consumer",
            &j.name,
            &format!("toutes les {}", every_label(j.every_secs)),
            vec![trash],
            &inner,
        ));
    }
    host.append(&outer);

    let i = project
        .services
        .iter()
        .position(|s| s.name == svc.name)
        .unwrap_or(0);
    host.append(&danger_button(
        "Supprimer le service",
        clone!(
            #[strong]
            ui,
            move || {
                delete_service(&ui, i);
                select(&ui, None);
            }
        ),
    ));
}

fn topic(ui: &Shared, host: &gtk::Box, name: &str) {
    let project = ui.state.borrow().project.clone();
    let Some((index, t)) = project
        .topics
        .iter()
        .enumerate()
        .find(|(_, t)| t.name == name)
    else {
        host.append(&empty("Topic introuvable."));
        return;
    };
    let (outer, body) = section("Topic Kafka", "topic", None);
    body.append(&field(
        "Nom",
        &commit_entry(
            &t.name,
            true,
            "",
            false,
            clone!(
                #[strong]
                ui,
                #[strong(rename_to = old)]
                t.name,
                move |v| {
                    if !is_topic_name(&v) {
                        say(&ui, "Nom de topic invalide (lettres, chiffres, . _ -).");
                    } else if ui.state.borrow().project.topic(&v).is_some() {
                        say(&ui, &format!("Le topic {v} existe déjà."));
                    } else {
                        edit(&ui, |p| rename::topic(p, &old, &v));
                        select(&ui, Some(Sel::Topic(v)));
                    }
                }
            ),
        ),
        "Lettres, chiffres, . _ -",
    ));
    body.append(&field(
        "Événement",
        &choice(
            &names_of(&project, MessageKind::Event),
            &t.message,
            clone!(
                #[strong]
                ui,
                move |v| edit(&ui, |p| {
                    if let Some(t) = p.topics.get_mut(index) {
                        t.message = v;
                        t.key_field = None;
                    }
                })
            ),
        ),
        "Message protobuf transporté",
    ));
    let keyable: Vec<String> = project
        .messages
        .iter()
        .find(|m| m.name == t.message)
        .map(|m| {
            m.fields
                .iter()
                .filter(|f| {
                    !f.repeated
                        && !f.optional
                        && matches!(
                            f.ty,
                            FieldType::String
                                | FieldType::Int32
                                | FieldType::Int64
                                | FieldType::Bool
                        )
                })
                .map(|f| f.name.clone())
                .collect()
        })
        .unwrap_or_default();
    let none = "aucune".to_string();
    let mut options = vec![none.clone()];
    options.extend(keyable);
    body.append(&field(
        "Clé",
        &choice(
            &options,
            &t.key_field.clone().unwrap_or_else(|| none.clone()),
            clone!(
                #[strong]
                ui,
                move |v| edit(&ui, |p| {
                    if let Some(t) = p.topics.get_mut(index) {
                        t.key_field = Some(v).filter(|v| *v != none);
                    }
                })
            ),
        ),
        "Les événements de même clé restent ordonnés",
    ));
    body.append(&field(
        "Partitions",
        &number(
            i64::from(t.partitions),
            1,
            10_000,
            clone!(
                #[strong]
                ui,
                move |v| edit(&ui, |p| {
                    if let Some(t) = p.topics.get_mut(index) {
                        t.partitions = v as i32;
                    }
                })
            ),
        ),
        "",
    ));
    body.append(&field(
        "Réplication",
        &number(
            i64::from(t.replication_factor),
            1,
            100,
            clone!(
                #[strong]
                ui,
                move |v| edit(&ui, |p| {
                    if let Some(t) = p.topics.get_mut(index) {
                        t.replication_factor = v as i16;
                    }
                })
            ),
        ),
        "",
    ));
    host.append(&outer);

    let (outer, body) = section("Producteurs", "publish", None);
    if t.producers.is_empty() {
        body.append(&empty("Aucun service ne produit ce topic : tirez une flèche d’un service vers lui, ou ajoutez-en un ci-dessous."));
    }
    for producer in &t.producers {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let target = producer.clone();
        let l = link(
            producer,
            clone!(
                #[strong]
                ui,
                move || select(&ui, Some(Sel::Service(target.clone())))
            ),
        );
        l.set_hexpand(true);
        line.append(&l);
        let gone = producer.clone();
        line.append(&icon_button(
            "x",
            &format!("Retirer {producer} des producteurs"),
            true,
            clone!(
                #[strong]
                ui,
                move || edit(&ui, |p| {
                    if let Some(t) = p.topics.get_mut(index) {
                        t.producers.retain(|x| *x != gone);
                    }
                })
            ),
        ));
        body.append(&line);
    }
    let candidates: Vec<String> = project
        .services
        .iter()
        .filter(|s| !t.producers.contains(&s.name))
        .map(|s| s.name.clone())
        .collect();
    if !candidates.is_empty() {
        let prompt = "+ Ajouter un producteur…".to_string();
        let mut options = vec![prompt.clone()];
        options.extend(candidates);
        let shown = prompt.clone();
        body.append(&choice(
            &options,
            &shown,
            clone!(
                #[strong]
                ui,
                move |v| {
                    if v != prompt {
                        edit(&ui, |p| {
                            if let Some(t) = p.topics.get_mut(index)
                                && !t.producers.contains(&v)
                            {
                                t.producers.push(v);
                            }
                        });
                    }
                }
            ),
        ));
    }
    host.append(&outer);

    let (outer, body) = section("Consommé par", "service", None);
    let consumers: Vec<&Service> = project
        .services
        .iter()
        .filter(|s| s.consumers.iter().any(|c| c.topic == t.name))
        .collect();
    if consumers.is_empty() {
        body.append(&empty(
            "Aucun service ne consomme ce topic : tirez une flèche du topic vers un service.",
        ));
    }
    for s in consumers {
        let target = s.name.clone();
        body.append(&link(
            &s.name,
            clone!(
                #[strong]
                ui,
                move || select(&ui, Some(Sel::Service(target.clone())))
            ),
        ));
    }
    host.append(&outer);

    host.append(&danger_button(
        "Supprimer le topic",
        clone!(
            #[strong]
            ui,
            move || {
                delete_topic(&ui, index);
                select(&ui, None);
            }
        ),
    ));
}
