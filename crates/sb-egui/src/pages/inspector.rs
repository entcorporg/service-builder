//! The right column of the Architecture page: the project, a service or a topic, edited in place.
//!
//! Every field applies when it is left (Enter or focus out), as in the web version. The logic of the handlers
//! (the steps) is edited on the Flows page, which the "Logique" buttons open.

use std::collections::HashSet;

use eframe::egui::{self, Frame, Margin, RichText, Sense, Stroke, Ui};
use sb_app::rename;
use sb_model::names::{is_pascal_ident, is_snake_ident, is_topic_name};
use sb_model::{
    Consumer, FieldType, HttpMethod, Job, MessageKind, Project, Route, Rpc, Service, ServiceKind,
};

use super::canvas::{Sel, select};
use super::common::{
    add_message, card, choice, commit_entry, danger_button, empty, field, icon_btn, link, number,
    section, separator, unique,
};
use super::editors::{delete_service, delete_topic, service_form};
use super::steps::{Flow, Loc};
use crate::app::{App, Page};
use crate::theme::*;

#[derive(Default)]
pub struct InspectorState {
    /// The cards that opened by themselves (the only one of its kind), so that they can be closed afterwards.
    auto_opened: HashSet<String>,
}

// ---- building blocks ----

/// The "+" button of a section; `true` when clicked.
pub(super) fn add_button(ui: &mut Ui, tip: &str, enabled: bool) -> bool {
    ui.add_enabled_ui(enabled, |ui| icon_btn(ui, "plus", tip, false).clicked())
        .inner
}

pub(super) fn names_of(p: &Project, kind: MessageKind) -> Vec<String> {
    p.messages
        .iter()
        .filter(|m| m.kind == kind)
        .map(|m| m.name.clone())
        .collect()
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

fn open_logic(app: &mut App, ui: &mut Ui) {
    if link(ui, "Modifier la logique…").clicked() {
        app.page = Page::Flows;
    }
}

/// The small title over the steps of a handler.
fn steps_title(ui: &mut Ui) {
    ui.add_space(6.0);
    ui.label(RichText::new("LOGIQUE").size(11.0).strong().color(FAINT));
}

/// A card is open by itself when it is the only one of its list (once: the user can close it).
fn open_alone(app: &mut App, id: &str, count: usize) {
    if count == 1 && app.inspector.auto_opened.insert(id.to_string()) {
        app.open.insert(id.to_string());
    }
}

// ---- the inspector ----

/// The inspector of the current selection (`app.canvas.selected`), drawn in the column it is given (410 px).
pub fn show(app: &mut App, ui: &mut Ui) {
    match app.canvas.selected.clone() {
        Some(Sel::Service(name)) => service(app, ui, &name),
        Some(Sel::Topic(name)) => topic(app, ui, &name),
        Some(Sel::Messages(_)) => super::schemas::messages(app, ui),
        None => project(app, ui),
    }
}

fn project(app: &mut App, ui: &mut Ui) {
    Frame::new()
        .inner_margin(Margin {
            left: 0,
            right: 0,
            top: 6,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 12.0;
            ui.label(RichText::new("Rien de sélectionné").size(LG).strong());
            for text in [
                "Cliquez sur un service ou un topic dans le canvas pour configurer ses RPC, ses consumers et leur logique.",
                "Glissez un Service ou un Topic depuis la colonne de gauche pour construire votre architecture.",
            ] {
                ui.add(egui::Label::new(RichText::new(text).color(MUTED)).wrap());
            }
        });

    let attempts = app.project().outbox.max_attempts;
    let set = section(
        ui,
        "Publication des événements",
        "publish",
        |_| {},
        |ui| {
            field(
                ui,
                "Tentatives",
                "Chaque événement publié passe par l’outbox du service (même transaction que le flow). Si Kafka le refuse après ce nombre de tentatives, il est déplacé sur <topic>.dlq.",
                |ui| number(ui, "outbox-attempts", i64::from(attempts), 1, 100),
            )
        },
    );
    if let Some(v) = set {
        app.edit(|p| p.outbox.max_attempts = v.clamp(1, 100) as u32);
    }
}

fn service(app: &mut App, ui: &mut Ui, name: &str) {
    let project = app.project();
    let Some(si) = project.services.iter().position(|s| s.name == name) else {
        empty(ui, "Service introuvable.");
        return;
    };
    let svc = &project.services[si];
    let sname = svc.name.clone();

    section(
        ui,
        "Service",
        "service",
        |_| {},
        |ui| {
            if let Some(v) = field(ui, "Nom", "", |ui| {
                commit_entry(ui, ("svc-name", &svc.name), &svc.name, true, "", false)
            }) {
                if !is_pascal_ident(&v) {
                    app.say("Le nom doit être en PascalCase, par exemple Orders.");
                } else if project.service(&v).is_some() {
                    app.say(&format!("Le service {v} existe déjà."));
                } else {
                    let old = svc.name.clone();
                    app.edit(|p| rename::service(p, &old, &v));
                    select(app, Some(Sel::Service(v)));
                }
            }
            if let Some(v) = field(ui, "Port gRPC", "", |ui| {
                number(ui, ("svc-port", &sname), i64::from(svc.port), 1, 65535)
            }) {
                app.edit(|p| {
                    if let Some(s) = svc_mut(p, &sname) {
                        s.port = v as u16;
                    }
                });
            }
            if (!svc.routes.is_empty() || svc.kind != ServiceKind::Standard)
                && let Some(v) = field(
                    ui,
                    "Port HTTP",
                    "Port de l’API HTTP (les routes ci-dessous)",
                    |ui| {
                        number(
                            ui,
                            ("svc-http", &sname),
                            i64::from(svc.http_port.unwrap_or(0)),
                            1,
                            65535,
                        )
                    },
                )
            {
                app.edit(|p| {
                    if let Some(s) = svc_mut(p, &sname) {
                        s.http_port = Some(v as u16);
                    }
                });
            }
        },
    );

    // A ready-made service has no handlers to draw: its settings are in the form and on the Flows page.
    if svc.kind != ServiceKind::Standard {
        separator(ui);
        let mta = svc.kind == ServiceKind::Mta;
        section(
            ui,
            if mta {
                "Service mail"
            } else {
                "Service temps réel"
            },
            "publish",
            |_| {},
            |ui| {
                empty(
                    ui,
                    if mta {
                        "Service prêt à l’emploi : file d’attente, envoi SMTP, DKIM. Les événements à mailer et leurs modèles se règlent sur la page Flows."
                    } else {
                        "Service prêt à l’emploi : diffuse les événements aux navigateurs en SSE. Son réglage se fait sur la page Flows."
                    },
                );
                if link(ui, "Modifier le service…").clicked() {
                    service_form(app, Some(si));
                }
                open_logic(app, ui);
            },
        );
        delete_service_button(app, ui, si);
        return;
    }

    rpcs(app, ui, &project, si);
    routes(app, ui, svc, &sname);
    consumers(app, ui, &project, si);
    jobs(app, ui, svc, si);
    delete_service_button(app, ui, si);
}

fn delete_service_button(app: &mut App, ui: &mut Ui, si: usize) {
    if danger_button(ui, "Supprimer le service").clicked() {
        delete_service(app, si);
        select(app, None);
    }
}

fn rpcs(app: &mut App, ui: &mut Ui, project: &Project, si: usize) {
    separator(ui);
    let svc = &project.services[si];
    let sname = svc.name.clone();
    let mut add = false;
    section(
        ui,
        "RPC (gRPC)",
        "callRpc",
        |ui| add = add_button(ui, "Ajouter un RPC", true),
        |ui| {
            if svc.rpcs.is_empty() {
                empty(ui, "Aucun RPC : ce service n’expose pas d’API gRPC.");
            }
            let protos = names_of(project, MessageKind::Proto);
            for (ri, rpc) in svc.rpcs.iter().enumerate() {
                let id = format!("{}/rpc/{}", svc.name, rpc.name);
                open_alone(app, &id, svc.rpcs.len());
                card(
                    app,
                    ui,
                    &id,
                    Some(SERVICE),
                    &rpc.name,
                    &format!("{} → {}", rpc.request, rpc.response),
                    |app, ui| {
                        if icon_btn(ui, "trash", &format!("Supprimer {}", rpc.name), true).clicked()
                        {
                            app.edit(|p| {
                                if let Some(s) = svc_mut(p, &sname)
                                    && ri < s.rpcs.len()
                                {
                                    s.rpcs.remove(ri);
                                }
                            });
                        }
                    },
                    |app, ui| {
                        if let Some(v) = field(ui, "Nom", "", |ui| {
                            commit_entry(ui, ("rpc-name", &sname, ri), &rpc.name, true, "", false)
                        }) {
                            if !is_pascal_ident(&v) {
                                app.say("Le nom du RPC doit être en PascalCase.");
                            } else {
                                app.edit(|p| rename::rpc(p, &sname, &rpc.name, &v));
                            }
                        }
                        for (label, current, is_request) in [
                            ("Requête", &rpc.request, true),
                            ("Réponse", &rpc.response, false),
                        ] {
                            if let Some(v) = field(ui, label, "", |ui| {
                                choice(ui, ("rpc-msg", &sname, ri, is_request), &protos, current)
                            }) {
                                app.edit(|p| {
                                    if let Some(r) =
                                        svc_mut(p, &sname).and_then(|s| s.rpcs.get_mut(ri))
                                    {
                                        if is_request {
                                            r.request = v;
                                        } else {
                                            r.response = v;
                                        }
                                    }
                                });
                            }
                        }
                        // The steps of the handler, in place, as in the web version.
                        steps_title(ui);
                        super::stepcards::steps_box(
                            app,
                            ui,
                            &rpc.steps,
                            &Loc {
                                service: si,
                                flow: Flow::Rpc(ri),
                                nest: vec![],
                            },
                        );
                    },
                );
            }
        },
    );
    if add {
        app.edit(|p| {
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
        });
    }
}

fn method_color(m: HttpMethod) -> egui::Color32 {
    match m {
        HttpMethod::Get => OK,
        HttpMethod::Delete => ERR,
        HttpMethod::Put | HttpMethod::Patch => SERVICE,
        _ => TOPIC,
    }
}

fn routes(app: &mut App, ui: &mut Ui, svc: &Service, sname: &str) {
    separator(ui);
    let mut add = false;
    section(
        ui,
        "API HTTP",
        "publish",
        |ui| add = add_button(ui, "Ajouter une route HTTP", true),
        |ui| {
            if svc.routes.is_empty() {
                empty(
                    ui,
                    "Aucune route : ce service ne reçoit aucune requête HTTP du front.",
                );
            }
            for (ri, r) in svc.routes.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let w = ui.available_width() - 30.0;
                    let frame = Frame::new()
                        .fill(PANEL2)
                        .stroke(Stroke::new(1.0, BORDER))
                        .corner_radius(8)
                        .inner_margin(Margin::symmetric(10, 7))
                        .show(ui, |ui| {
                            ui.set_width(w - 22.0);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                let color = method_color(r.method);
                                Frame::new()
                                    .fill(alpha(color, 41))
                                    .corner_radius(4)
                                    .inner_margin(Margin::symmetric(5, 1))
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new(r.method.as_str())
                                                .size(9.0)
                                                .strong()
                                                .color(color),
                                        );
                                    });
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let n = r.steps.len();
                                        ui.label(
                                            RichText::new(format!(
                                                "{n} étape{}",
                                                if n > 1 { "s" } else { "" }
                                            ))
                                            .size(SM)
                                            .color(MUTED),
                                        );
                                        ui.add(
                                            egui::Label::new(RichText::new(&r.path).font(mono(SM)))
                                                .truncate(),
                                        );
                                    },
                                );
                            });
                        });
                    let hit = ui
                        .interact(
                            frame.response.rect,
                            ui.make_persistent_id(("route-open", ri)),
                            Sense::click(),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text("Ouvrir le configurateur d’endpoint");
                    if hit.hovered() {
                        ui.painter().rect_stroke(
                            frame.response.rect,
                            8,
                            Stroke::new(1.0, ACCENT),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if hit.clicked() {
                        super::endpoint::open(app, sname, &r.name);
                    }
                    if icon_btn(ui, "trash", &format!("Supprimer la route {}", r.name), true)
                        .clicked()
                    {
                        app.edit(|p| {
                            if let Some(s) = svc_mut(p, sname)
                                && ri < s.routes.len()
                            {
                                s.routes.remove(ri);
                            }
                        });
                    }
                });
            }
        },
    );
    if add {
        app.edit(|p| {
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
            if let Some(s) = svc_mut(p, sname) {
                if s.http_port.is_none() {
                    s.http_port = Some(next_http);
                }
                s.routes.push(Route {
                    path: format!("/{}", name.replace('_', "-")),
                    name,
                    method: HttpMethod::Get,
                    request,
                    response,
                    status: 200,
                    steps: vec![],
                });
            }
        });
    }
}

fn consumers(app: &mut App, ui: &mut Ui, project: &Project, si: usize) {
    separator(ui);
    let svc = &project.services[si];
    let sname = svc.name.clone();
    let topics: Vec<String> = project.topics.iter().map(|t| t.name.clone()).collect();
    let mut add = false;
    section(
        ui,
        "Consumers (Kafka)",
        "topic",
        |ui| {
            add = add_button(
                ui,
                if topics.is_empty() {
                    "Créez d’abord un topic"
                } else {
                    "Ajouter un consumer"
                },
                !topics.is_empty(),
            )
        },
        |ui| {
            if svc.consumers.is_empty() {
                empty(
                    ui,
                    "Aucun consumer : ce service ne réagit à aucun événement.",
                );
            }
            for (ci, c) in svc.consumers.iter().enumerate() {
                let id = format!("{}/consumer/{}", svc.name, c.name);
                open_alone(app, &id, svc.consumers.len());
                card(
                    app,
                    ui,
                    &id,
                    Some(OK),
                    &c.name,
                    &format!("⇠ {}", c.topic),
                    |app, ui| {
                        if icon_btn(ui, "trash", &format!("Supprimer {}", c.name), true).clicked() {
                            app.edit(|p| {
                                if let Some(s) = svc_mut(p, &sname)
                                    && ci < s.consumers.len()
                                {
                                    s.consumers.remove(ci);
                                }
                            });
                        }
                    },
                    |app, ui| consumer_body(app, ui, project, si, ci, &topics),
                );
            }
        },
    );
    if add {
        app.edit(|p| {
            let Some(topic) = p.topics.first().map(|t| t.name.clone()) else {
                return;
            };
            if let Some(s) = svc_mut(p, &sname) {
                let taken: Vec<String> = s.consumers.iter().map(|c| c.name.clone()).collect();
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
        });
    }
}

fn consumer_body(
    app: &mut App,
    ui: &mut Ui,
    project: &Project,
    si: usize,
    ci: usize,
    topics: &[String],
) {
    let svc = &project.services[si];
    let c = &svc.consumers[ci];
    let sname = &svc.name;
    if let Some(v) = field(ui, "Nom", "", |ui| {
        commit_entry(ui, ("cons-name", sname, ci), &c.name, true, "", false)
    }) {
        if !is_snake_ident(&v) {
            app.say("Le nom du consumer doit être en snake_case.");
        } else {
            app.edit(|p| {
                if let Some(x) = svc_mut(p, sname).and_then(|s| s.consumers.get_mut(ci)) {
                    x.name = v;
                }
            });
        }
    }
    if let Some(v) = field(ui, "Topic", "", |ui| {
        choice(ui, ("cons-topic", sname, ci), topics, &c.topic)
    }) {
        app.edit(|p| {
            if let Some(x) = svc_mut(p, sname).and_then(|s| s.consumers.get_mut(ci)) {
                x.topic = v;
            }
        });
    }
    if let Some(v) = field(ui, "Groupe", "Par défaut : service-consumer", |ui| {
        commit_entry(
            ui,
            ("cons-group", sname, ci),
            c.group_id.as_deref().unwrap_or(""),
            true,
            &format!("{}-{}", svc.name.to_lowercase(), c.name),
            true,
        )
    }) {
        app.edit(|p| {
            if let Some(x) = svc_mut(p, sname).and_then(|s| s.consumers.get_mut(ci)) {
                x.group_id = Some(v).filter(|v| !v.is_empty());
            }
        });
    }
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
    let current = c.idempotent_on.clone().unwrap_or_else(|| none.clone());
    if let Some(v) = field(
        ui,
        "Dédoublonner sur",
        "Kafka livre au moins une fois : un événement déjà traité est ignoré, la vérification étant dans la transaction du handler. Mémorisé 7 jours.",
        |ui| choice(ui, ("cons-dedup", sname, ci), &options, &current),
    ) {
        app.edit(|p| {
            if let Some(x) = svc_mut(p, sname).and_then(|s| s.consumers.get_mut(ci)) {
                x.idempotent_on = Some(v).filter(|v| *v != none);
            }
        });
    }
    // The steps of the handler, in place, as in the web version.
    steps_title(ui);
    super::stepcards::steps_box(
        app,
        ui,
        &c.steps,
        &Loc {
            service: si,
            flow: Flow::Consumer(ci),
            nest: vec![],
        },
    );
}

fn jobs(app: &mut App, ui: &mut Ui, svc: &Service, si: usize) {
    separator(ui);
    let sname = svc.name.clone();
    let mut add = false;
    section(
        ui,
        "Tâches planifiées",
        "update",
        |ui| add = add_button(ui, "Ajouter une tâche planifiée", true),
        |ui| {
            if svc.jobs.is_empty() {
                empty(
                    ui,
                    "Aucune tâche : rien ne s’exécute ici sans requête ni événement.",
                );
            }
            for (ji, j) in svc.jobs.iter().enumerate() {
                card(
                    app,
                    ui,
                    &format!("{}/job/{}", svc.name, j.name),
                    Some(OK),
                    &j.name,
                    &format!("toutes les {}", every_label(j.every_secs)),
                    |app, ui| {
                        if icon_btn(ui, "trash", &format!("Supprimer la tâche {}", j.name), true)
                            .clicked()
                        {
                            app.edit(|p| {
                                if let Some(s) = svc_mut(p, &sname)
                                    && ji < s.jobs.len()
                                {
                                    s.jobs.remove(ji);
                                }
                            });
                        }
                    },
                    |app, ui| {
                        if let Some(v) = field(ui, "Nom", "", |ui| {
                            commit_entry(ui, ("job-name", &sname, ji), &j.name, true, "", false)
                        }) {
                            app.edit(|p| {
                                if let Some(x) = svc_mut(p, &sname).and_then(|s| s.jobs.get_mut(ji))
                                {
                                    x.name = v;
                                }
                            });
                        }
                        if let Some(v) = field(
                            ui,
                            "Toutes les",
                            "En secondes, de 1 à 2 592 000 (30 jours). Un premier passage a lieu au démarrage.",
                            |ui| {
                                number(
                                    ui,
                                    ("job-every", &sname, ji),
                                    i64::from(j.every_secs),
                                    1,
                                    2_592_000,
                                )
                            },
                        ) {
                            app.edit(|p| {
                                if let Some(x) = svc_mut(p, &sname).and_then(|s| s.jobs.get_mut(ji))
                                {
                                    x.every_secs = v as u32;
                                }
                            });
                        }
                        // The steps of the handler, in place, as in the web version.
                        steps_title(ui);
                        super::stepcards::steps_box(
                            app,
                            ui,
                            &j.steps,
                            &Loc {
                                service: si,
                                flow: Flow::Job(ji),
                                nest: vec![],
                            },
                        );
                    },
                );
            }
        },
    );
    if add {
        app.edit(|p| {
            if let Some(s) = svc_mut(p, &sname) {
                let taken: Vec<String> = s.jobs.iter().map(|j| j.name.clone()).collect();
                let refs: Vec<&str> = taken.iter().map(String::as_str).collect();
                s.jobs.push(Job {
                    name: unique("job", &refs),
                    every_secs: 3600,
                    steps: vec![],
                });
            }
        });
    }
}

fn topic(app: &mut App, ui: &mut Ui, name: &str) {
    let project = app.project();
    let Some((index, t)) = project
        .topics
        .iter()
        .enumerate()
        .find(|(_, t)| t.name == name)
    else {
        empty(ui, "Topic introuvable.");
        return;
    };
    section(
        ui,
        "Topic Kafka",
        "topic",
        |_| {},
        |ui| {
            if let Some(v) = field(ui, "Nom", "Lettres, chiffres, . _ -", |ui| {
                commit_entry(ui, ("topic-name", &t.name), &t.name, true, "", false)
            }) {
                if !is_topic_name(&v) {
                    app.say("Nom de topic invalide (lettres, chiffres, . _ -).");
                } else if project.topic(&v).is_some() {
                    app.say(&format!("Le topic {v} existe déjà."));
                } else {
                    let old = t.name.clone();
                    app.edit(|p| rename::topic(p, &old, &v));
                    select(app, Some(Sel::Topic(v)));
                }
            }
            if let Some(v) = field(ui, "Événement", "Message protobuf transporté", |ui| {
                choice(
                    ui,
                    ("topic-msg", &t.name),
                    &names_of(&project, MessageKind::Event),
                    &t.message,
                )
            }) {
                app.edit(|p| {
                    if let Some(t) = p.topics.get_mut(index) {
                        t.message = v;
                        t.key_field = None;
                    }
                });
            }
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
            let current = t.key_field.clone().unwrap_or_else(|| none.clone());
            if let Some(v) = field(
                ui,
                "Clé",
                "Les événements de même clé restent ordonnés",
                |ui| choice(ui, ("topic-key", &t.name), &options, &current),
            ) {
                app.edit(|p| {
                    if let Some(t) = p.topics.get_mut(index) {
                        t.key_field = Some(v).filter(|v| *v != none);
                    }
                });
            }
            if let Some(v) = field(ui, "Partitions", "", |ui| {
                number(
                    ui,
                    ("topic-parts", &t.name),
                    i64::from(t.partitions),
                    1,
                    10_000,
                )
            }) {
                app.edit(|p| {
                    if let Some(t) = p.topics.get_mut(index) {
                        t.partitions = v as i32;
                    }
                });
            }
            if let Some(v) = field(ui, "Réplication", "", |ui| {
                number(
                    ui,
                    ("topic-repl", &t.name),
                    i64::from(t.replication_factor),
                    1,
                    100,
                )
            }) {
                app.edit(|p| {
                    if let Some(t) = p.topics.get_mut(index) {
                        t.replication_factor = v as i16;
                    }
                });
            }
        },
    );

    separator(ui);
    section(
        ui,
        "Producteurs",
        "publish",
        |_| {},
        |ui| {
            if t.producers.is_empty() {
                empty(
                    ui,
                    "Aucun service ne produit ce topic : tirez une flèche d’un service vers lui, ou ajoutez-en un ci-dessous.",
                );
            }
            for producer in &t.producers {
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_btn(
                            ui,
                            "x",
                            &format!("Retirer {producer} des producteurs"),
                            true,
                        )
                        .clicked()
                        {
                            app.edit(|p| {
                                if let Some(t) = p.topics.get_mut(index) {
                                    t.producers.retain(|x| x != producer);
                                }
                            });
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            if link(ui, producer).clicked() {
                                select(app, Some(Sel::Service(producer.clone())));
                            }
                        });
                    });
                });
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
                if let Some(v) = choice(ui, ("topic-add-producer", &t.name), &options, &prompt)
                    && v != prompt
                {
                    app.edit(|p| {
                        if let Some(t) = p.topics.get_mut(index)
                            && !t.producers.contains(&v)
                        {
                            t.producers.push(v);
                        }
                    });
                }
            }
        },
    );

    separator(ui);
    section(
        ui,
        "Consommé par",
        "service",
        |_| {},
        |ui| {
            let consumers: Vec<&Service> = project
                .services
                .iter()
                .filter(|s| s.consumers.iter().any(|c| c.topic == t.name))
                .collect();
            if consumers.is_empty() {
                empty(
                    ui,
                    "Aucun service ne consomme ce topic : tirez une flèche du topic vers un service.",
                );
            }
            for s in consumers {
                if link(ui, &s.name).clicked() {
                    select(app, Some(Sel::Service(s.name.clone())));
                }
            }
        },
    );

    if danger_button(ui, "Supprimer le topic").clicked() {
        delete_topic(app, index);
        select(app, None);
    }
}
