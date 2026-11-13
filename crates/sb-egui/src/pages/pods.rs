//! The Pods page: runs the generated services in containers without an export. On the left, the form (resources,
//! admin interfaces, Traefik and its frontends), saved in the project; on the right, the stack: its containers, the
//! output of the build, and the files that were generated for it.
//!
//! Everything that talks to the container engine runs on a thread (`sb_app::pods` blocks) and comes back through
//! `App::spawn`.

use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};
use sb_app::pods::{Cancel, Container, Engine, GeneratedFile};
use sb_model::pods::RESTART_POLICIES;
use sb_model::{Frontend, PodsConfig, Project, ServiceKind, Severity};

use crate::app::App;
use crate::fonts::Weight;
use crate::pages::common::{self, commit_entry, icon_btn, section, separator};
use crate::theme::*;
use crate::widgets;

/// The containers are listed again after this delay while the page is shown.
const REFRESH: Duration = Duration::from_secs(3);
/// Lines of output kept: a first build prints thousands.
const MAX_LINES: usize = 6000;
const LABEL: f32 = 132.0;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum View {
    #[default]
    Containers,
    Output,
    Files,
}

struct Busy {
    what: &'static str,
    cancel: Cancel,
}

#[derive(Default)]
pub struct PodsState {
    /// The engine found on this machine, or why there is none. `None` until it was looked for.
    engine: Option<Result<Engine, String>>,
    detecting: bool,
    /// A start or a stop is running.
    busy: Option<Busy>,
    /// What the build and compose printed, shared with the thread that runs them.
    output: Arc<Mutex<Vec<String>>>,
    containers: Vec<Container>,
    listed: Option<Instant>,
    listing: bool,
    list_error: Option<String>,
    /// The service whose logs are shown.
    selected: Option<String>,
    logs: String,
    view: View,
    /// The generated file that is shown.
    file: Option<String>,
    /// The files of the stack, for the generation of the project they were made from.
    files: Option<(u64, Rc<Vec<GeneratedFile>>)>,
}

/// Shows the containers, the output (`journal`) or the generated files (`fichiers`): for the screenshots.
pub fn open_view(app: &mut App, name: &str) {
    app.pods.view = match name {
        "journal" => View::Output,
        "fichiers" => View::Files,
        _ => View::Containers,
    };
}

/// Tests and screenshots do not talk to an engine.
fn engine_off() -> bool {
    std::env::var_os("SB_NO_ENGINE").is_some()
}

fn push(output: &Arc<Mutex<Vec<String>>>, line: &str) {
    let mut lines = output.lock().unwrap_or_else(|e| e.into_inner());
    if lines.len() >= MAX_LINES {
        lines.drain(..MAX_LINES / 4);
    }
    lines.push(line.to_string());
}

// ---- the engine ------------------------------------------------------------------------

fn engine(app: &App) -> Option<Engine> {
    app.pods.engine.as_ref()?.as_ref().ok().cloned()
}

/// Looks for the engine once, then keeps the list of containers (and the logs that are open) fresh.
fn poll(app: &mut App) {
    if engine_off() {
        return;
    }
    if app.pods.engine.is_none() && !app.pods.detecting {
        app.pods.detecting = true;
        app.spawn(Engine::detect, |app, found| {
            app.pods.detecting = false;
            app.pods.engine = Some(found);
        });
    }
    let Some(engine) = engine(app) else {
        return;
    };
    let due = app.pods.listed.is_none_or(|at| at.elapsed() >= REFRESH);
    if due && !app.pods.listing {
        app.pods.listing = true;
        let stack = sb_app::pods::stack_name(&app.state.project);
        // The logs of the selected service come with the list: one thread, one repaint.
        let selected = app.pods.selected.clone();
        app.spawn(
            move || {
                let list = sb_app::pods::ps(&engine, &stack);
                let logs = list.as_ref().ok().and_then(|list| {
                    let c = list
                        .iter()
                        .find(|c| Some(&c.service) == selected.as_ref())?;
                    Some(sb_app::pods::logs(&engine, &c.id, 400).unwrap_or_else(|e| e))
                });
                (list, logs)
            },
            |app, (list, logs)| {
                app.pods.listing = false;
                app.pods.listed = Some(Instant::now());
                match list {
                    Ok(list) => {
                        app.pods.containers = list;
                        app.pods.list_error = None;
                    }
                    Err(e) => app.pods.list_error = Some(e),
                }
                if let Some(logs) = logs {
                    app.pods.logs = logs;
                }
            },
        );
    }
    app.ctx.request_repaint_after(REFRESH);
}

fn start(app: &mut App) {
    let Some(engine) = engine(app) else {
        return;
    };
    let project = app.state.project.clone();
    let dir = sb_app::pods::stack_dir(&project);
    let cancel = Cancel::default();
    app.pods.busy = Some(Busy {
        what: "Démarrage",
        cancel: cancel.clone(),
    });
    app.pods.view = View::Output;
    let output = app.pods.output.clone();
    output.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let (_, ctx) = app.sender();
    app.spawn(
        move || {
            let log = |line: &str| {
                push(&output, line);
                ctx.request_repaint();
            };
            sb_app::pods::up(&engine, &project, &dir, &log, &cancel)
        },
        |app, done| finished(app, done, "La stack est démarrée."),
    );
}

fn stop(app: &mut App, volumes: bool) {
    let Some(engine) = engine(app) else {
        return;
    };
    let project = app.state.project.clone();
    let dir = sb_app::pods::stack_dir(&project);
    let cancel = Cancel::default();
    app.pods.busy = Some(Busy {
        what: "Arrêt",
        cancel: cancel.clone(),
    });
    app.pods.view = View::Output;
    let output = app.pods.output.clone();
    let (_, ctx) = app.sender();
    app.spawn(
        move || {
            let log = |line: &str| {
                push(&output, line);
                ctx.request_repaint();
            };
            sb_app::pods::down(&engine, &project, &dir, volumes, &log, &cancel)
        },
        |app, done| finished(app, done, "La stack est arrêtée."),
    );
}

fn finished(app: &mut App, done: Result<(), String>, ok: &str) {
    app.pods.busy = None;
    // The list is read again right away.
    app.pods.listed = None;
    match done {
        Ok(()) => {
            app.pods.view = View::Containers;
            app.say(ok);
        }
        Err(e) => {
            push(&app.pods.output, &format!("✗ {e}"));
            app.say(&format!("Pods : {e}"));
        }
    }
}

fn act(app: &mut App, verb: &'static str, id: String) {
    let Some(engine) = engine(app) else {
        return;
    };
    app.spawn(
        move || sb_app::pods::container(&engine, verb, &id),
        |app, done| {
            app.pods.listed = None;
            if let Err(e) = done {
                app.say(&format!("Pods : {e}"));
            }
        },
    );
}

// ---- the form --------------------------------------------------------------------------

/// A label, its control, and a faint hint under it.
fn row<R>(ui: &mut Ui, label: &str, hint: &str, control: impl FnOnce(&mut Ui) -> R) -> R {
    let r = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.allocate_ui_with_layout(
                Vec2::new(LABEL, 30.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_size(Vec2::new(LABEL, 30.0));
                    ui.add(egui::Label::new(RichText::new(label).color(MUTED)).truncate());
                },
            );
            control(ui)
        })
        .inner;
    if !hint.is_empty() {
        note(ui, hint, FAINT);
    }
    r
}

fn note(ui: &mut Ui, text: &str, color: Color32) {
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = 0.0;
        ui.horizontal(|ui| {
            ui.add_space(LABEL + 10.0);
            ui.add(
                egui::Label::new(
                    RichText::new(text)
                        .size(11.0)
                        .line_height(Some(lh(11.0)))
                        .color(color),
                )
                .wrap(),
            );
        });
    });
}

/// The diagnostics of the project that point at `pointer`, under the field they are about.
fn problems(app: &App, ui: &mut Ui, pointer: &str) {
    for d in app
        .analysis()
        .diagnostics
        .iter()
        .filter(|d| d.path == pointer)
    {
        let color = match d.severity {
            Severity::Error => ERR,
            Severity::Warning => WARN,
        };
        note(ui, &d.message, color);
    }
}

/// A text of the configuration: `pointer` is where it is in the project, which also names the input.
fn text(
    app: &mut App,
    ui: &mut Ui,
    (label, hint, placeholder): (&str, &str, &str),
    pointer: &str,
    value: &str,
    set: impl FnOnce(&mut PodsConfig, String),
) {
    let new = row(ui, label, hint, |ui| {
        commit_entry(ui, pointer, value, true, placeholder, true)
    });
    problems(app, ui, pointer);
    if let Some(v) = new {
        app.edit(|p| set(&mut p.pods, v));
    }
}

fn number(
    app: &mut App,
    ui: &mut Ui,
    (label, hint): (&str, &str),
    pointer: &str,
    value: i64,
    max: i64,
    set: impl FnOnce(&mut PodsConfig, i64),
) {
    let new = row(ui, label, hint, |ui| {
        common::number(ui, pointer, value, 1, max)
    });
    problems(app, ui, pointer);
    if let Some(v) = new {
        app.edit(|p| set(&mut p.pods, v));
    }
}

fn toggle(
    app: &mut App,
    ui: &mut Ui,
    (label, hint): (&str, &str),
    on: bool,
    set: impl FnOnce(&mut PodsConfig, bool),
) {
    let clicked = row(ui, label, hint, |ui| widgets::switch(ui, on).clicked());
    if clicked {
        app.edit(|p| set(&mut p.pods, !on));
    }
}

fn form(app: &mut App, ui: &mut Ui) {
    let p = app.project();
    let pods = &p.pods;

    section(
        ui,
        "Ressources",
        "service",
        |_| {},
        |ui| {
            text(
                app,
                ui,
                (
                    "Processeurs",
                    "Les CPU où tournent les conteneurs (cpuset). Vide : tous.",
                    "0-5",
                ),
                "/pods/cpuset",
                &pods.cpuset,
                |c, v| c.cpuset = v,
            );
            let options: Vec<String> = RESTART_POLICIES.iter().map(|s| s.to_string()).collect();
            let picked = row(ui, "Redémarrage", "", |ui| {
                common::choice(ui, "/pods/restart", &options, &pods.restart)
            });
            problems(app, ui, "/pods/restart");
            if let Some(v) = picked {
                app.edit(|p| p.pods.restart = v);
            }
            text(
                app,
                ui,
                (
                    "Mémoire service",
                    "mem_limit de chaque service généré ; le swap est plafonné à la même valeur.",
                    "160m",
                ),
                "/pods/serviceMemory",
                &pods.service_memory,
                |c, v| c.service_memory = v,
            );
            text(
                app,
                ui,
                ("Mémoire Kafka", "", "768m"),
                "/pods/kafkaMemory",
                &pods.kafka_memory,
                |c, v| c.kafka_memory = v,
            );
            text(
                app,
                ui,
                ("Mémoire Postgres", "", "256m"),
                "/pods/postgresMemory",
                &pods.postgres_memory,
                |c, v| c.postgres_memory = v,
            );
            toggle(
                app,
                ui,
                (
                    "Publier les ports",
                    "Les ports des services (et des interfaces sans Traefik) sur 127.0.0.1.",
                ),
                pods.publish_ports,
                |c, v| c.publish_ports = v,
            );
            let mut browse = false;
            let new = row(
                ui,
                "Fichier .env",
                "Des lignes NOM=valeur données à chaque service (clés, secrets) : elles restent hors du projet.",
                |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    browse = icon_btn(ui, "folder", "Choisir un fichier…", false).clicked();
                    commit_entry(ui, "/pods/envFile", &pods.env_file, true, "aucun", true)
                },
            );
            problems(app, ui, "/pods/envFile");
            let picked = browse
                .then(|| {
                    rfd::FileDialog::new()
                        .set_title("Fichier d'environnement")
                        .pick_file()
                })
                .flatten()
                .map(|path| path.display().to_string());
            if let Some(v) = new.or(picked) {
                app.edit(|p| p.pods.env_file = v);
            }
        },
    );
    separator(ui);

    section(
        ui,
        "Interfaces d'administration",
        "table",
        |_| {},
        |ui| {
            toggle(
                app,
                ui,
                ("Adminer", "Une interface web pour PostgreSQL."),
                pods.adminer,
                |c, v| c.adminer = v,
            );
            toggle(
                app,
                ui,
                (
                    "Kafka UI",
                    "Une interface web pour le broker : topics, messages, groupes.",
                ),
                pods.kafka_ui,
                |c, v| c.kafka_ui = v,
            );
        },
    );
    separator(ui);

    let t = &pods.traefik;
    let mut flip = false;
    section(
        ui,
        "Traefik",
        "graph",
        |ui| flip = widgets::switch(ui, t.enabled).clicked(),
        |ui| {
            if !t.enabled {
                ui.label(
                    widgets::faint(
                        "Un reverse proxy devant la stack : un hôte par interface et par frontend, et l'API des \
                         services sur le sien. Sa configuration est générée.",
                    )
                    .size(SM),
                );
                return;
            }
            problems(app, ui, "/pods/traefik");
            traefik(app, ui, &p);
        },
    );
    if flip {
        let on = !t.enabled;
        app.edit(|p| p.pods.traefik.enabled = on);
    }
    ui.add_space(12.0);
}

fn traefik(app: &mut App, ui: &mut Ui, p: &Project) {
    let pods = &p.pods;
    let t = &pods.traefik;
    text(
        app,
        ui,
        (
            "Domaine",
            "Avec un domaine : HTTPS et certificats Let's Encrypt. Vide : HTTP sur *.localhost.",
            "localhost",
        ),
        "/pods/traefik/domain",
        &t.domain,
        |c, v| c.traefik.domain = v.to_lowercase(),
    );
    if t.tls() {
        text(
            app,
            ui,
            (
                "E-mail ACME",
                "Le compte Let's Encrypt qui obtient les certificats.",
                "ops@example.com",
            ),
            "/pods/traefik/acmeEmail",
            &t.acme_email,
            |c, v| c.traefik.acme_email = v,
        );
    }
    number(
        app,
        ui,
        ("Port HTTP", ""),
        "/pods/traefik/httpPort",
        i64::from(t.http_port),
        65535,
        |c, v| c.traefik.http_port = v as u16,
    );
    if t.tls() {
        number(
            app,
            ui,
            ("Port HTTPS", ""),
            "/pods/traefik/httpsPort",
            i64::from(t.https_port),
            65535,
            |c, v| c.traefik.https_port = v as u16,
        );
    }
    number(
        app,
        ui,
        (
            "Débit de l'API",
            "Requêtes par seconde et par client, en moyenne.",
        ),
        "/pods/traefik/rateAverage",
        i64::from(t.rate_average),
        1_000_000,
        |c, v| c.traefik.rate_average = v as u32,
    );
    number(
        app,
        ui,
        (
            "Rafale",
            "Requêtes acceptées d'un coup au-dessus de la moyenne.",
        ),
        "/pods/traefik/rateBurst",
        i64::from(t.rate_burst),
        1_000_000,
        |c, v| c.traefik.rate_burst = v as u32,
    );
    toggle(
        app,
        ui,
        (
            "Tableau de bord",
            "Le dashboard de Traefik, sur son propre hôte.",
        ),
        t.dashboard,
        |c, v| c.traefik.dashboard = v,
    );
    text(
        app,
        ui,
        (
            "Authentification",
            "Protège le tableau de bord et les interfaces : une ligne utilisateur:hash de « htpasswd -nB ». Vide : aucune.",
            "admin:$2y$…",
        ),
        "/pods/traefik/adminAuth",
        &t.admin_auth,
        |c, v| c.traefik.admin_auth = v,
    );

    // ---- the hosts ----
    ui.add_space(4.0);
    widgets::section_title(ui, "Hôtes");
    let mta = p.services.iter().any(|s| s.kind == ServiceKind::Mta);
    type Set = fn(&mut PodsConfig, String);
    let hosts: [(bool, &str, &str, &str, Set); 5] = [
        (true, "API", "api", &t.hosts.api, |c, v| {
            c.traefik.hosts.api = v
        }),
        (
            t.dashboard,
            "Tableau de bord",
            "dashboard",
            &t.hosts.dashboard,
            |c, v| c.traefik.hosts.dashboard = v,
        ),
        (
            pods.adminer,
            "Adminer",
            "adminer",
            &t.hosts.adminer,
            |c, v| c.traefik.hosts.adminer = v,
        ),
        (
            pods.kafka_ui,
            "Kafka UI",
            "kafkaUi",
            &t.hosts.kafka_ui,
            |c, v| c.traefik.hosts.kafka_ui = v,
        ),
        (mta, "Mailpit", "mailpit", &t.hosts.mailpit, |c, v| {
            c.traefik.hosts.mailpit = v
        }),
    ];
    for (shown, label, key, value, set) in hosts {
        if shown {
            let pointer = format!("/pods/traefik/hosts/{key}");
            let hint = t.url(value);
            text(app, ui, (label, &hint, ""), &pointer, value, move |c, v| {
                set(c, v.to_lowercase())
            });
        }
    }

    // ---- the frontends ----
    ui.add_space(4.0);
    widgets::section_title(ui, "Frontends");
    ui.label(
        widgets::faint(
            "Des serveurs de développement sur l'hôte (Vite…), qui doivent écouter sur toutes les interfaces. \
             Chacun appelle l'API sous son préfixe, que Traefik retire avant de joindre le service.",
        )
        .size(11.0),
    );
    for (i, f) in t.frontends.iter().enumerate() {
        let at = format!("/pods/traefik/frontends/{i}");
        let summary = format!("{}  →  :{}", t.url(&f.host), f.port);
        common::card(
            app,
            ui,
            &at,
            Some(SERVICE),
            if f.name.is_empty() {
                "frontend"
            } else {
                &f.name
            },
            &summary,
            |app, ui| {
                if icon_btn(ui, "trash", "Supprimer ce frontend", true).clicked() {
                    app.later(move |app| {
                        app.edit(|p| {
                            p.pods.traefik.frontends.remove(i);
                        })
                    });
                }
            },
            |app, ui| frontend(app, ui, i, f, &at),
        );
    }
    if widgets::add_row(ui, "Ajouter un frontend").clicked() {
        let taken: Vec<&str> = t.frontends.iter().map(|f| f.name.as_str()).collect();
        let name = common::unique("front", &taken);
        let port = t
            .frontends
            .iter()
            .map(|f| f.port)
            .max()
            .map_or(5173, |p| p.saturating_add(1));
        app.open
            .insert(format!("/pods/traefik/frontends/{}", t.frontends.len()));
        app.edit(|p| {
            p.pods.traefik.frontends.push(Frontend {
                host: name.clone(),
                name,
                port,
            })
        });
    }
}

fn frontend(app: &mut App, ui: &mut Ui, i: usize, f: &Frontend, at: &str) {
    type Set = fn(&mut Frontend, String);
    let fields: [(&str, &str, &str, &str, &str, Set); 2] = [
        ("Nom", "", "admin", "name", &f.name, |f, v| {
            f.name = v.to_lowercase()
        }),
        (
            "Sous-domaine",
            "Vide : le domaine lui-même.",
            "",
            "host",
            &f.host,
            |f, v| f.host = v.to_lowercase(),
        ),
    ];
    for (label, hint, placeholder, key, value, set) in fields {
        let pointer = format!("{at}/{key}");
        let new = row(ui, label, hint, |ui| {
            commit_entry(ui, &pointer, value, true, placeholder, true)
        });
        problems(app, ui, &pointer);
        if let Some(v) = new {
            app.edit(|p| {
                if let Some(f) = p.pods.traefik.frontends.get_mut(i) {
                    set(f, v)
                }
            });
        }
    }
    let pointer = format!("{at}/port");
    let new = row(
        ui,
        "Port sur l'hôte",
        "Celui de son serveur de développement.",
        |ui| common::number(ui, &pointer, i64::from(f.port), 1, 65535),
    );
    problems(app, ui, &pointer);
    if let Some(v) = new {
        app.edit(|p| {
            if let Some(f) = p.pods.traefik.frontends.get_mut(i) {
                f.port = v as u16
            }
        });
    }
}

// ---- the stack -------------------------------------------------------------------------

fn state_color(c: &Container) -> Color32 {
    match c.state.as_str() {
        "running" if c.status.contains("unhealthy") => ERR,
        "running" if c.status.contains("starting") => WARN,
        "running" => OK,
        "created" | "paused" | "restarting" | "configured" => WARN,
        "exited" | "stopped" | "dead" => ERR,
        _ => FAINT,
    }
}

fn pill(ui: &mut Ui, text: &str, color: Color32) {
    Frame::new()
        .fill(alpha(color, 28))
        .corner_radius(CornerRadius::same(99))
        .inner_margin(Margin::symmetric(9, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).font(semibold(SM)).color(color));
        });
}

/// The title, the engine, and the actions on the whole stack.
fn head(app: &mut App, ui: &mut Ui) {
    let errors = app
        .analysis()
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error);
    let busy = app.pods.busy.as_ref().map(|b| b.what);
    let ready = engine(app).is_some() && busy.is_none();
    let running = app
        .pods
        .containers
        .iter()
        .filter(|c| c.state == "running")
        .count();
    let total = app.pods.containers.len();

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.label(RichText::new("Pods").size(XL).bold());
        match &app.pods.engine {
            Some(Ok(engine)) => pill(ui, engine.label(), SERVICE),
            Some(Err(_)) => pill(ui, "Aucun moteur", ERR),
            None => pill(ui, "Recherche du moteur…", FAINT),
        }
        if total > 0 {
            let color = if running == total { OK } else { WARN };
            pill(ui, &format!("{running}/{total} en marche"), color);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if let Some(what) = busy {
                if widgets::button(ui, "x", "Interrompre").clicked()
                    && let Some(b) = &app.pods.busy
                {
                    b.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                ui.label(widgets::muted(format!("{what}…")));
                ui.spinner();
                return;
            }
            let label = if total > 0 { "Relancer" } else { "Lancer" };
            let start_it = widgets::primary(ui, "play", label, ready && !errors).on_hover_text(
                if errors {
                    "Corrigez les erreurs du projet avant de le lancer"
                } else {
                    "Compile les services et démarre la stack ; un service déjà lancé n'est redémarré que si son binaire a changé"
                },
            );
            if start_it.clicked() {
                start(app);
            }
            if total > 0 && ready {
                if widgets::button(ui, "stop", "Arrêter").clicked() {
                    stop(app, false);
                }
                if icon_btn(ui, "trash", "Arrêter et supprimer les données (bases, certificats)", true)
                    .clicked()
                {
                    app.confirm(
                        "Supprimer les données de la stack ?",
                        "Les conteneurs sont arrêtés et leurs volumes supprimés : les bases de données des services \
                         repartent de zéro au prochain lancement.",
                        vec![
                            ("Annuler".into(), false, None),
                            (
                                "Supprimer".into(),
                                true,
                                Some(Box::new(|app: &mut App| stop(app, true))),
                            ),
                        ],
                    );
                }
            }
        });
    });
    ui.add_space(4.0);
    ui.label(
        widgets::muted(
            "Lance les services générés dans des conteneurs, avec Kafka et PostgreSQL, sans exporter le code. \
             Le formulaire est enregistré dans le projet.",
        )
        .size(SM),
    );
    if let Some(Err(e)) = &app.pods.engine {
        ui.add_space(4.0);
        ui.label(RichText::new(e).color(ERR).size(SM));
    }
    if let Some(e) = &app.pods.list_error {
        ui.add_space(4.0);
        ui.label(RichText::new(e).color(ERR).size(SM));
    }

    // What a browser can open, once something runs.
    if running > 0 {
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            for link in sb_app::pods::links(&app.project()) {
                let r = widgets::chip(ui, &format!("{}  ↗", link.label), SERVICE)
                    .interact(Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(&link.url);
                if r.clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(&link.url));
                }
            }
        });
    }
}

fn views(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (view, label) in [
            (View::Containers, "Conteneurs"),
            (View::Output, "Journal"),
            (View::Files, "Fichiers générés"),
        ] {
            let on = app.pods.view == view;
            let text = RichText::new(label)
                .font(if on { semibold(BASE) } else { prop(BASE) })
                .color(if on { TEXT } else { MUTED });
            let b = egui::Button::new(text)
                .fill(if on { RAISED } else { Color32::TRANSPARENT })
                .stroke(Stroke::NONE)
                .corner_radius(CornerRadius::same(7));
            if ui.add(b).clicked() {
                app.pods.view = view;
            }
        }
    });
}

/// Monospaced lines in a dark box, scrolled both ways; `follow` keeps the end in view.
fn lines_box(ui: &mut Ui, id: &str, lines: &[&str], follow: bool) {
    Frame::new()
        .fill(CODE_BG)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            let height = ui.fonts_mut(|f| f.row_height(&mono(SM)));
            // The rows are placed by their height alone: the scroll area counts on it.
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::both()
                .id_salt(id)
                .auto_shrink([false, false])
                .stick_to_bottom(follow)
                .show_rows(ui, height, lines.len(), |ui, range| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for line in &lines[range] {
                        let color = if line.starts_with('▸') {
                            ACCENT
                        } else if line.starts_with('✗') || line.contains("error") {
                            ERR
                        } else if line.contains("warning") {
                            WARN
                        } else {
                            hex(0xc9cee8)
                        };
                        ui.add(
                            egui::Label::new(RichText::new(*line).font(mono(SM)).color(color))
                                .wrap_mode(egui::TextWrapMode::Extend)
                                .selectable(true),
                        );
                    }
                });
        });
}

fn containers(app: &mut App, ui: &mut Ui) {
    if app.pods.containers.is_empty() {
        common::empty(
            ui,
            if engine_off() || engine(app).is_none() {
                "Aucun conteneur."
            } else {
                "Aucun conteneur : la stack n'est pas lancée. « Lancer » compile les services et démarre tout."
            },
        );
        return;
    }
    let list = app.pods.containers.clone();
    let selected = app.pods.selected.clone();
    // The list takes what it needs, up to half of the height when logs are open under it.
    let max = if selected.is_some() {
        (ui.available_height() * 0.45).max(120.0)
    } else {
        ui.available_height()
    };
    egui::ScrollArea::vertical()
        .id_salt("pods-containers")
        .max_height(max)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            for c in &list {
                let on = selected.as_deref() == Some(c.service.as_str());
                let color = state_color(c);
                Frame::new()
                    .fill(PANEL2)
                    .stroke(Stroke::new(1.0, if on { BORDER_STRONG } else { BORDER }))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(12, 7))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            let (dot, _) =
                                ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                            ui.painter().circle_filled(dot.center(), 4.5, color);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 1.0;
                                ui.label(RichText::new(&c.service).font(mono_bold(BASE)));
                                ui.label(
                                    RichText::new(format!("{}  ·  {}", c.status, c.image))
                                        .size(SM)
                                        .color(MUTED),
                                );
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.spacing_mut().item_spacing.x = 2.0;
                                    if icon_btn(ui, "restart", "Redémarrer", false).clicked() {
                                        act(app, "restart", c.id.clone());
                                    }
                                    if c.state == "running" {
                                        if icon_btn(ui, "stop", "Arrêter", false).clicked() {
                                            act(app, "stop", c.id.clone());
                                        }
                                    } else if icon_btn(ui, "play", "Démarrer", false).clicked() {
                                        act(app, "start", c.id.clone());
                                    }
                                    if icon_btn(ui, "log", "Voir les logs", false).clicked() {
                                        if on {
                                            app.pods.selected = None;
                                        } else {
                                            app.pods.selected = Some(c.service.clone());
                                            app.pods.logs.clear();
                                            // Read with the next listing, now.
                                            app.pods.listed = None;
                                        }
                                    }
                                },
                            );
                        });
                    });
            }
        });

    if let Some(service) = selected {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("logs · {service}"))
                    .font(mono(SM))
                    .color(MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::ghost(ui, "x", "", "Fermer les logs").clicked() {
                    app.pods.selected = None;
                }
                if widgets::ghost(ui, "copy", "", "Copier").clicked() {
                    ui.ctx().copy_text(app.pods.logs.clone());
                    app.say("Copié");
                }
            });
        });
        let lines: Vec<&str> = app.pods.logs.lines().collect();
        lines_box(ui, "pods-logs", &lines, true);
    }
}

fn output(app: &mut App, ui: &mut Ui) {
    let lines = app
        .pods
        .output
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    if lines.is_empty() {
        common::empty(
            ui,
            "Le journal du prochain lancement s'affichera ici : compilation, puis démarrage.",
        );
        return;
    }
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    lines_box(ui, "pods-output", &lines, true);
}

/// The files written for the stack, for the project as it is now.
fn stack_files(app: &mut App) -> Rc<Vec<GeneratedFile>> {
    let generation = app.state.generation;
    if let Some((at, files)) = &app.pods.files
        && *at == generation
    {
        return files.clone();
    }
    let files = Rc::new(sb_app::pods::stack_files(&app.state.project));
    app.pods.files = Some((generation, files.clone()));
    files
}

fn files(app: &mut App, ui: &mut Ui) {
    let files = stack_files(app);
    let current = app
        .pods
        .file
        .as_ref()
        .and_then(|path| files.iter().position(|f| &f.path == path))
        .or_else(|| files.iter().position(|f| f.path.ends_with(".yml")))
        .unwrap_or(0);
    let Some(file) = files.get(current) else {
        return;
    };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for (i, f) in files.iter().enumerate() {
            let on = i == current;
            let text = RichText::new(&f.path)
                .font(mono(SM))
                .color(if on { TEXT } else { MUTED });
            let b = egui::Button::new(text)
                .fill(if on { RAISED } else { PANEL2 })
                .stroke(Stroke::new(1.0, if on { BORDER_STRONG } else { BORDER }))
                .corner_radius(CornerRadius::same(7));
            if ui.add(b).clicked() {
                app.pods.file = Some(f.path.clone());
            }
        }
        if widgets::ghost(ui, "copy", "Copier", "").clicked() {
            ui.ctx().copy_text(file.content.clone());
            app.say("Copié");
        }
    });
    ui.add_space(6.0);
    let dir = sb_app::pods::stack_dir(&app.state.project);
    ui.label(
        RichText::new(format!("Écrits au lancement dans {}", dir.display()))
            .font(mono(XS + 0.5))
            .color(FAINT),
    );
    ui.add_space(4.0);
    let lines: Vec<&str> = file.content.lines().collect();
    lines_box(ui, "pods-file", &lines, false);
}

pub fn page(app: &mut App, ui: &mut Ui) {
    poll(app);

    egui::Panel::left("pods-form")
        .exact_size(452.0)
        .resizable(false)
        .frame(Frame::new().fill(PANEL).inner_margin(Margin::ZERO))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("pods-form-scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    Frame::new()
                        .inner_margin(Margin::symmetric(16, 10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            form(app, ui);
                        });
                });
        });

    egui::CentralPanel::no_frame().show(ui, |ui| {
        let full = ui.max_rect();
        ui.painter()
            .vline(full.left(), full.y_range(), Stroke::new(1.0, BORDER));
        Frame::new()
            .inner_margin(Margin::symmetric(24, 20))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                head(app, ui);
                ui.add_space(14.0);
                views(app, ui);
                ui.add_space(10.0);
                match app.pods.view {
                    View::Containers => containers(app, ui),
                    View::Output => output(app, ui),
                    View::Files => files(app, ui),
                }
            });
    });
}
