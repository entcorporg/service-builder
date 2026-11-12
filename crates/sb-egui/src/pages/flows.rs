//! The Flows page: for each service, its RPCs, routes, consumers and jobs, and their steps.
//!
//! The names of flows are fixed once created (other steps call them); everything else can be edited.

use eframe::egui::{self, Frame, Margin, Ui};
use sb_app::rename;
use sb_model::names::{is_pascal_ident, is_snake_ident};
use sb_model::{Consumer, HttpMethod, Job, Route, Rpc, ServiceKind};

use super::common::{empty, list_card};
use super::mail::{draw_mail, draw_realtime};
use super::steps::{Flow, Loc, add_step, draw_steps};
use crate::app::App;
use crate::form::{Input, Kind, Value, ask};
use crate::widgets;

#[derive(Default)]
pub struct FlowsState {}

pub fn page(app: &mut App, ui: &mut Ui) {
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            let p = app.project();
            for (si, svc) in p.services.iter().enumerate() {
                // The title of the service, with a line under it.
                Frame::new()
                    .inner_margin(Margin {
                        left: 18,
                        right: 18,
                        top: 16,
                        bottom: 0,
                    })
                    .show(ui, |ui| widgets::section_title(ui, &svc.name));
                super::common::separator(ui);
                match svc.kind {
                    ServiceKind::Mta => {
                        Frame::new()
                            .inner_margin(Margin::symmetric(12, 0))
                            .show(ui, |ui| draw_mail(app, ui, &p, si));
                        continue;
                    }
                    ServiceKind::Realtime => {
                        Frame::new()
                            .inner_margin(Margin::symmetric(12, 0))
                            .show(ui, |ui| draw_realtime(app, ui, &p, si));
                        continue;
                    }
                    ServiceKind::Standard => {}
                }
                add_buttons(app, ui, si);
                let loc = |flow| Loc {
                    service: si,
                    flow,
                    nest: vec![],
                };
                for (i, r) in svc.rpcs.iter().enumerate() {
                    let l = loc(Flow::Rpc(i));
                    let title = format!("RPC {}", r.name);
                    let sub = format!("{} -> {}", r.request, r.response);
                    match flow_card(ui, &title, &sub) {
                        Some(0) => add_step(app, &l),
                        Some(1) => rpc_form(app, si, Some(i)),
                        Some(_) => app.edit(|p| {
                            p.services[si].rpcs.remove(i);
                        }),
                        None => {}
                    }
                    draw_steps(app, ui, &r.steps, &l);
                }
                for (i, r) in svc.routes.iter().enumerate() {
                    let l = loc(Flow::Route(i));
                    let title = format!("Route {}", r.name);
                    let sub = format!(
                        "{} {} · {} -> {} · {}",
                        r.method.as_str().to_uppercase(),
                        r.path,
                        r.request,
                        r.response,
                        r.status
                    );
                    match flow_card(ui, &title, &sub) {
                        Some(0) => add_step(app, &l),
                        Some(1) => route_form(app, si, Some(i)),
                        Some(_) => app.edit(|p| {
                            p.services[si].routes.remove(i);
                        }),
                        None => {}
                    }
                    draw_steps(app, ui, &r.steps, &l);
                }
                for (i, c) in svc.consumers.iter().enumerate() {
                    let l = loc(Flow::Consumer(i));
                    let title = format!("Consommateur {}", c.name);
                    let sub = format!("topic {}", c.topic);
                    match flow_card(ui, &title, &sub) {
                        Some(0) => add_step(app, &l),
                        Some(1) => consumer_form(app, si, Some(i)),
                        Some(_) => app.edit(|p| {
                            p.services[si].consumers.remove(i);
                        }),
                        None => {}
                    }
                    draw_steps(app, ui, &c.steps, &l);
                }
                for (i, j) in svc.jobs.iter().enumerate() {
                    let l = loc(Flow::Job(i));
                    let title = format!("Job {}", j.name);
                    let sub = format!("toutes les {} s", j.every_secs);
                    match flow_card(ui, &title, &sub) {
                        Some(0) => add_step(app, &l),
                        Some(1) => job_form(app, si, Some(i)),
                        Some(_) => app.edit(|p| {
                            p.services[si].jobs.remove(i);
                        }),
                        None => {}
                    }
                    draw_steps(app, ui, &j.steps, &l);
                }
            }
            if p.services.is_empty() {
                Frame::new()
                    .inner_margin(Margin::symmetric(12, 12))
                    .show(ui, |ui| {
                        empty(
                            ui,
                            "Aucun service. Ajoutez un service dans l'onglet Services.",
                        )
                    });
            }
            ui.add_space(24.0);
        });
}

/// The card of a flow; the index of the button clicked (`+ Étape`, `Modifier`, `Supprimer`).
fn flow_card(ui: &mut Ui, title: &str, subtitle: &str) -> Option<usize> {
    ui.add_space(6.0);
    Frame::new()
        .inner_margin(Margin::symmetric(12, 0))
        .show(ui, |ui| {
            list_card(
                ui,
                title,
                subtitle,
                0.0,
                &["+ Étape", "Modifier", "Supprimer"],
            )
        })
        .inner
}

fn add_buttons(app: &mut App, ui: &mut Ui, service: usize) {
    type Open = fn(&mut App, usize, Option<usize>);
    let buttons: [(&str, Open); 4] = [
        ("Ajouter un RPC", rpc_form),
        ("Ajouter une route", route_form),
        ("Ajouter un consommateur", consumer_form),
        ("Ajouter un job", job_form),
    ];
    Frame::new()
        .inner_margin(Margin::symmetric(16, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for (text, open) in buttons {
                    if ui.button(text).clicked() {
                        open(app, service, None);
                    }
                }
            });
        });
}

// ---- forms ----

fn name_input(existing: Option<&str>, hint: &str) -> Input {
    Input::new(
        format!("Nom ({hint})"),
        Kind::Text(existing.unwrap_or_default().to_string()),
    )
}

fn message_names(app: &App) -> Vec<String> {
    app.project()
        .messages
        .iter()
        .map(|m| m.name.clone())
        .collect()
}

fn choice(options: Vec<String>, selected: Option<&str>) -> Kind {
    Kind::Choice {
        options,
        selected: selected.map(String::from),
    }
}

fn text(v: &mut std::vec::IntoIter<Value>) -> String {
    v.next().map(Value::text).unwrap_or_default()
}

fn rpc_form(app: &mut App, si: usize, index: Option<usize>) {
    let messages = message_names(app);
    if messages.is_empty() {
        return app.say("Créez d'abord les messages de la requête et de la réponse.");
    }
    let cur = index.and_then(|i| app.project().services[si].rpcs.get(i).cloned());
    let inputs = vec![
        name_input(cur.as_ref().map(|r| r.name.as_str()), "PascalCase"),
        Input::new(
            "Requête",
            choice(messages.clone(), cur.as_ref().map(|r| r.request.as_str())),
        ),
        Input::new(
            "Réponse",
            choice(messages, cur.as_ref().map(|r| r.response.as_str())),
        ),
    ];
    ask(app, "RPC", inputs, move |app, values| {
        let mut v = values.into_iter();
        let (name, request, response) = (text(&mut v), text(&mut v), text(&mut v));
        let old = cur.as_ref().map(|r| r.name.clone());
        if old.as_deref() != Some(name.as_str()) {
            if !is_pascal_ident(&name) {
                return Err("Le nom doit être en PascalCase, par exemple CreateOrder.".into());
            }
            if app.project().services[si]
                .rpcs
                .iter()
                .any(|r| r.name == name)
            {
                return Err(format!("Le RPC {name} existe déjà."));
            }
        }
        app.edit(|p| {
            if let Some(o) = old.as_deref().filter(|o| *o != name) {
                let service = p.services[si].name.clone();
                rename::rpc(p, &service, o, &name);
            }
            let rpcs = &mut p.services[si].rpcs;
            match index.and_then(|i| rpcs.get_mut(i)) {
                Some(r) => {
                    r.request = request;
                    r.response = response;
                }
                None => rpcs.push(Rpc {
                    name,
                    request,
                    response,
                    steps: vec![],
                }),
            }
        });
        Ok(())
    });
}

fn route_form(app: &mut App, si: usize, index: Option<usize>) {
    let messages = message_names(app);
    if messages.is_empty() {
        return app.say("Créez d'abord les messages de la requête et de la réponse.");
    }
    let cur = index.and_then(|i| app.project().services[si].routes.get(i).cloned());
    let methods: Vec<String> = HttpMethod::ALL
        .iter()
        .map(|m| m.as_str().to_uppercase())
        .collect();
    let inputs = vec![
        name_input(cur.as_ref().map(|r| r.name.as_str()), "snake_case"),
        Input::new(
            "Méthode",
            choice(
                methods,
                cur.as_ref()
                    .map(|r| r.method.as_str().to_uppercase())
                    .as_deref(),
            ),
        ),
        Input::new(
            "Chemin",
            Kind::Text(cur.as_ref().map_or("/".into(), |r| r.path.clone())),
        )
        .help("Par exemple /orders/{id}"),
        Input::new(
            "Requête",
            choice(messages.clone(), cur.as_ref().map(|r| r.request.as_str())),
        ),
        Input::new(
            "Réponse",
            choice(messages, cur.as_ref().map(|r| r.response.as_str())),
        ),
        Input::new(
            "Statut de succès",
            choice(
                vec!["200".into(), "201".into(), "202".into()],
                Some(&cur.as_ref().map_or(200, |r| r.status).to_string()),
            ),
        ),
    ];
    ask(app, "Route HTTP", inputs, move |app, values| {
        let mut v = values.into_iter();
        let name = text(&mut v);
        let method = HttpMethod::ALL
            .into_iter()
            .find(|m| m.as_str().eq_ignore_ascii_case(&text(&mut v)))
            .unwrap_or(HttpMethod::Get);
        let (path, request, response) = (text(&mut v), text(&mut v), text(&mut v));
        let status = text(&mut v).parse().unwrap_or(200);
        if !path.starts_with('/') {
            return Err("Le chemin commence par /.".into());
        }
        let old = cur.as_ref().map(|c| c.name.clone());
        if old.as_deref() != Some(name.as_str()) {
            if !is_snake_ident(&name) {
                return Err("Le nom doit être en snake_case, par exemple create_order.".into());
            }
            if app.project().services[si]
                .routes
                .iter()
                .any(|r| r.name == name)
            {
                return Err(format!("La route {name} existe déjà."));
            }
        }
        app.edit(|p| {
            let routes = &mut p.services[si].routes;
            match index.and_then(|i| routes.get_mut(i)) {
                Some(r) => {
                    (r.name, r.method, r.path, r.request, r.response, r.status) =
                        (name, method, path, request, response, status)
                }
                None => routes.push(Route {
                    name,
                    method,
                    path,
                    request,
                    response,
                    status,
                    steps: vec![],
                }),
            }
        });
        Ok(())
    });
}

fn consumer_form(app: &mut App, si: usize, index: Option<usize>) {
    let topics: Vec<String> = app
        .project()
        .topics
        .iter()
        .map(|t| t.name.clone())
        .collect();
    if topics.is_empty() {
        return app.say("Créez d'abord un topic à consommer.");
    }
    let cur = index.and_then(|i| app.project().services[si].consumers.get(i).cloned());
    let inputs = vec![
        name_input(cur.as_ref().map(|c| c.name.as_str()), "snake_case"),
        Input::new("Topic", choice(topics, cur.as_ref().map(|c| c.topic.as_str()))),
        Input::new(
            "Dédoublonner sur le champ (facultatif)",
            Kind::Text(cur.as_ref().and_then(|c| c.idempotent_on.clone()).unwrap_or_default()),
        )
        .help("Kafka livre au moins une fois : ce champ de l'événement l'identifie, un doublon est ignoré."),
    ];
    ask(app, "Consommateur", inputs, move |app, values| {
        let mut v = values.into_iter();
        let (name, topic) = (text(&mut v), text(&mut v));
        let idempotent_on = Some(text(&mut v)).filter(|f| !f.is_empty());
        let old = cur.as_ref().map(|c| c.name.clone());
        if old.as_deref() != Some(name.as_str()) {
            if !is_snake_ident(&name) {
                return Err("Le nom doit être en snake_case, par exemple on_order_created.".into());
            }
            if app.project().services[si]
                .consumers
                .iter()
                .any(|c| c.name == name)
            {
                return Err(format!("Le consommateur {name} existe déjà."));
            }
        }
        app.edit(|p| {
            let consumers = &mut p.services[si].consumers;
            match index.and_then(|i| consumers.get_mut(i)) {
                Some(c) => {
                    c.name = name;
                    c.topic = topic;
                    c.idempotent_on = idempotent_on;
                }
                None => consumers.push(Consumer {
                    name,
                    topic,
                    group_id: None,
                    idempotent_on,
                    mail: None,
                    steps: vec![],
                }),
            }
        });
        Ok(())
    });
}

fn job_form(app: &mut App, si: usize, index: Option<usize>) {
    let cur = index.and_then(|i| app.project().services[si].jobs.get(i).cloned());
    let inputs = vec![
        name_input(cur.as_ref().map(|j| j.name.as_str()), "snake_case"),
        Input::new(
            "Toutes les (secondes)",
            Kind::Number {
                value: cur.as_ref().map_or(60, |j| j.every_secs as i64),
                min: 1,
                max: 31_536_000,
            },
        ),
    ];
    ask(app, "Job planifié", inputs, move |app, values| {
        let mut v = values.into_iter();
        let name = text(&mut v);
        let every_secs = v.next().map_or(60, Value::number) as u32;
        let old = cur.as_ref().map(|c| c.name.clone());
        if old.as_deref() != Some(name.as_str()) {
            if !is_snake_ident(&name) {
                return Err("Le nom doit être en snake_case, par exemple purge_expired.".into());
            }
            if app.project().services[si]
                .jobs
                .iter()
                .any(|j| j.name == name)
            {
                return Err(format!("Le job {name} existe déjà."));
            }
        }
        app.edit(|p| {
            let jobs = &mut p.services[si].jobs;
            match index.and_then(|i| jobs.get_mut(i)) {
                Some(j) => {
                    j.name = name;
                    j.every_secs = every_secs;
                }
                None => jobs.push(Job {
                    name,
                    every_secs,
                    steps: vec![],
                }),
            }
        });
        Ok(())
    });
}
