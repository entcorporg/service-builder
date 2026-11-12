//! The Flows page: for each service, its RPCs, routes, consumers and jobs, and their steps.
//!
//! The names of flows are fixed once created (other steps call them); everything else can be edited.

use gtk::prelude::*;
use sb_model::names::{is_pascal_ident, is_snake_ident};
use sb_model::{Consumer, HttpMethod, Job, Project, Route, Rpc, ServiceKind};

use sb_app::rename;

use super::edit::edit;
use super::editors::{Action, act, card};
use super::form::{Input, Kind, Value, ask};
use super::mail::{draw_mail, draw_realtime};
use super::steps::{Flow, Loc, add_step, draw_steps};
use super::ui::Shared;
use super::widgets::{clear, row, section};

pub fn draw_flows(ui: &Shared, p: &Project) {
    clear(&ui.flows);
    for (si, svc) in p.services.iter().enumerate() {
        section(&ui.flows, &svc.name);
        match svc.kind {
            ServiceKind::Mta => {
                draw_mail(ui, p, si, &ui.flows);
                continue;
            }
            ServiceKind::Realtime => {
                draw_realtime(ui, p, si, &ui.flows);
                continue;
            }
            ServiceKind::Standard => {}
        }
        ui.flows.append(&add_buttons(ui, si));
        let loc = |flow| Loc {
            service: si,
            flow,
            nest: vec![],
        };

        for (i, r) in svc.rpcs.iter().enumerate() {
            let l = loc(Flow::Rpc(i));
            ui.flows.append(&flow_card(
                ui,
                &format!("RPC {}", r.name),
                &format!("{} → {}", r.request, r.response),
                &l,
                move |ui| rpc_form(ui, si, Some(i)),
                move |p| {
                    p.services[si].rpcs.remove(i);
                },
            ));
            draw_steps(ui, &r.steps, &l, &ui.flows);
        }
        for (i, r) in svc.routes.iter().enumerate() {
            let l = loc(Flow::Route(i));
            ui.flows.append(&flow_card(
                ui,
                &format!("Route {}", r.name),
                &format!(
                    "{} {} · {} → {} · {}",
                    r.method.as_str().to_uppercase(),
                    r.path,
                    r.request,
                    r.response,
                    r.status
                ),
                &l,
                move |ui| route_form(ui, si, Some(i)),
                move |p| {
                    p.services[si].routes.remove(i);
                },
            ));
            draw_steps(ui, &r.steps, &l, &ui.flows);
        }
        for (i, c) in svc.consumers.iter().enumerate() {
            let l = loc(Flow::Consumer(i));
            ui.flows.append(&flow_card(
                ui,
                &format!("Consommateur {}", c.name),
                &format!("topic {}", c.topic),
                &l,
                move |ui| consumer_form(ui, si, Some(i)),
                move |p| {
                    p.services[si].consumers.remove(i);
                },
            ));
            draw_steps(ui, &c.steps, &l, &ui.flows);
        }
        for (i, j) in svc.jobs.iter().enumerate() {
            let l = loc(Flow::Job(i));
            ui.flows.append(&flow_card(
                ui,
                &format!("Job {}", j.name),
                &format!("toutes les {} s", j.every_secs),
                &l,
                move |ui| job_form(ui, si, Some(i)),
                move |p| {
                    p.services[si].jobs.remove(i);
                },
            ));
            draw_steps(ui, &j.steps, &l, &ui.flows);
        }
    }
    if p.services.is_empty() {
        ui.flows.append(&row(
            "Aucun service",
            "Ajoutez un service dans l'onglet Services.",
        ));
    }
}

fn flow_card(
    ui: &Shared,
    title: &str,
    subtitle: &str,
    loc: &Loc,
    modify: impl Fn(&Shared) + 'static,
    remove: impl Fn(&mut Project) + Clone + 'static,
) -> gtk::ListBoxRow {
    let (u1, u2, u3) = (ui.clone(), ui.clone(), ui.clone());
    let l = loc.clone();
    let actions: Vec<Action> = vec![
        act("+ Étape", move || add_step(&u1, &l)),
        act("Modifier", move || modify(&u2)),
        act("Supprimer", move || edit(&u3, remove.clone())),
    ];
    let c = card(title, subtitle, 0, actions);
    c.add_css_class("flow");
    c
}

fn add_buttons(ui: &Shared, service: usize) -> gtk::ListBoxRow {
    let r = gtk::ListBoxRow::new();
    r.set_selectable(false);
    r.set_activatable(false);
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    line.set_margin_top(8);
    line.set_margin_bottom(8);
    line.set_margin_start(16);
    type Open = fn(&Shared, usize, Option<usize>);
    let buttons: [(&str, Open); 4] = [
        ("Ajouter un RPC", rpc_form),
        ("Ajouter une route", route_form),
        ("Ajouter un consommateur", consumer_form),
        ("Ajouter un job", job_form),
    ];
    for (text, open) in buttons {
        let b = gtk::Button::with_label(text);
        let ui = ui.clone();
        b.connect_clicked(move |_| open(&ui, service, None));
        line.append(&b);
    }
    r.set_child(Some(&line));
    r
}

// ---- forms ----

fn name_input(existing: Option<&str>, hint: &str) -> Input {
    Input::new(
        format!("Nom ({hint})"),
        Kind::Text(existing.unwrap_or_default().to_string()),
    )
}

fn message_names(ui: &Shared) -> Vec<String> {
    ui.state
        .borrow()
        .project
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

fn rpc_form(ui: &Shared, si: usize, index: Option<usize>) {
    let messages = message_names(ui);
    if messages.is_empty() {
        return super::widgets::say(
            ui,
            "Créez d'abord les messages de la requête et de la réponse.",
        );
    }
    let cur = index.and_then(|i| ui.state.borrow().project.services[si].rpcs.get(i).cloned());
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
    ask(ui, "RPC", inputs, move |ui, values| {
        let mut v = values.into_iter();
        let (name, request, response) = (text(&mut v), text(&mut v), text(&mut v));
        let old = cur.as_ref().map(|r| r.name.clone());
        if old.as_deref() != Some(name.as_str()) {
            if !is_pascal_ident(&name) {
                return Err("Le nom doit être en PascalCase, par exemple CreateOrder.".into());
            }
            if ui.state.borrow().project.services[si]
                .rpcs
                .iter()
                .any(|r| r.name == name)
            {
                return Err(format!("Le RPC {name} existe déjà."));
            }
        }
        edit(ui, |p| {
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

fn route_form(ui: &Shared, si: usize, index: Option<usize>) {
    let messages = message_names(ui);
    if messages.is_empty() {
        return super::widgets::say(
            ui,
            "Créez d'abord les messages de la requête et de la réponse.",
        );
    }
    let cur = index.and_then(|i| {
        ui.state.borrow().project.services[si]
            .routes
            .get(i)
            .cloned()
    });
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
    ask(ui, "Route HTTP", inputs, move |ui, values| {
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
            if ui.state.borrow().project.services[si]
                .routes
                .iter()
                .any(|r| r.name == name)
            {
                return Err(format!("La route {name} existe déjà."));
            }
        }
        edit(ui, |p| {
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

fn consumer_form(ui: &Shared, si: usize, index: Option<usize>) {
    let topics: Vec<String> = ui
        .state
        .borrow()
        .project
        .topics
        .iter()
        .map(|t| t.name.clone())
        .collect();
    if topics.is_empty() {
        return super::widgets::say(ui, "Créez d'abord un topic à consommer.");
    }
    let cur = index.and_then(|i| {
        ui.state.borrow().project.services[si]
            .consumers
            .get(i)
            .cloned()
    });
    let inputs = vec![
        name_input(cur.as_ref().map(|c| c.name.as_str()), "snake_case"),
        Input::new("Topic", choice(topics, cur.as_ref().map(|c| c.topic.as_str()))),
        Input::new(
            "Dédoublonner sur le champ (facultatif)",
            Kind::Text(cur.as_ref().and_then(|c| c.idempotent_on.clone()).unwrap_or_default()),
        )
        .help("Kafka livre au moins une fois : ce champ de l'événement l'identifie, un doublon est ignoré."),
    ];
    ask(ui, "Consommateur", inputs, move |ui, values| {
        let mut v = values.into_iter();
        let (name, topic) = (text(&mut v), text(&mut v));
        let idempotent_on = Some(text(&mut v)).filter(|f| !f.is_empty());
        let old = cur.as_ref().map(|c| c.name.clone());
        if old.as_deref() != Some(name.as_str()) {
            if !is_snake_ident(&name) {
                return Err("Le nom doit être en snake_case, par exemple on_order_created.".into());
            }
            if ui.state.borrow().project.services[si]
                .consumers
                .iter()
                .any(|c| c.name == name)
            {
                return Err(format!("Le consommateur {name} existe déjà."));
            }
        }
        edit(ui, |p| {
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

fn job_form(ui: &Shared, si: usize, index: Option<usize>) {
    let cur = index.and_then(|i| ui.state.borrow().project.services[si].jobs.get(i).cloned());
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
    ask(ui, "Job planifié", inputs, move |ui, values| {
        let mut v = values.into_iter();
        let name = text(&mut v);
        let every_secs = v.next().map_or(60, Value::number) as u32;
        let old = cur.as_ref().map(|c| c.name.clone());
        if old.as_deref() != Some(name.as_str()) {
            if !is_snake_ident(&name) {
                return Err("Le nom doit être en snake_case, par exemple purge_expired.".into());
            }
            if ui.state.borrow().project.services[si]
                .jobs
                .iter()
                .any(|j| j.name == name)
            {
                return Err(format!("Le job {name} existe déjà."));
            }
        }
        edit(ui, |p| {
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
