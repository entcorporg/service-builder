//! The Infra page: what runs the services in containers. On the left, the form (resources, admin interfaces, Traefik and
//! its frontends), saved in the project; on the right, the files it produces, and the button that writes them out.
//!
//! Service Builder starts nothing: "Exporter l'infra" hands over `docker-compose.yml`, `docker-compose.override.yml`
//! and the configuration files (Traefik, Garage, the image), and `docker compose up -d` does the rest.

use std::rc::Rc;

use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};
use sb_app::GeneratedFile;
use sb_model::pods::RESTART_POLICIES;
use sb_model::{Frontend, PodsConfig, Project, ServiceKind, Severity};

use crate::app::App;
use crate::fonts::Weight;
use crate::pages::common::{self, commit_entry, icon_btn, section, separator};
use crate::theme::*;
use crate::widgets;

const LABEL: f32 = 132.0;

#[derive(Default)]
pub struct InfraState {
    /// The generated file that is shown.
    file: Option<String>,
    /// The files, for the generation of the project they were made from.
    files: Option<(u64, Rc<Vec<GeneratedFile>>)>,
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
                "Des lignes NOM=valeur données à chaque service (clés, secrets) : elles restent hors du projet. Un fichier désigné par un chemin absolu (une clé) est monté dans les conteneurs.",
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
    // Only a project that stores objects has something to choose.
    if p.services.iter().any(|s| !s.bucket_envs().is_empty()) {
        separator(ui);
        section(
            ui,
            "Stockage objet",
            "table",
            |_| {},
            |ui| {
                toggle(
                    app,
                    ui,
                    (
                        "Garage",
                        "Un stockage compatible S3 dans la stack : ses buckets et la clé d'accès des services sont créés au démarrage. Désactivé : le stockage est celui que désigne le fichier .env.",
                    ),
                    pods.garage,
                    |c, v| c.garage = v,
                );
            },
        );
    }
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
    let hosts: [(bool, &str, &str, &str, Set); 6] = [
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
        (p.uses_garage(), "Garage (S3)", "s3", &t.hosts.s3, |c, v| {
            c.traefik.hosts.s3 = v
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

/// The files of the stack, for the project as it is now.
fn stack_files(app: &mut App) -> Rc<Vec<GeneratedFile>> {
    let generation = app.state.generation;
    if let Some((at, files)) = &app.infra.files
        && *at == generation
    {
        return files.clone();
    }
    let files = Rc::new(sb_app::infra::stack_files(&app.state.project));
    app.infra.files = Some((generation, files.clone()));
    files
}

fn files(app: &mut App, ui: &mut Ui) {
    let files = stack_files(app);
    let current = app
        .infra
        .file
        .as_ref()
        .and_then(|path| files.iter().position(|f| &f.path == path))
        .or_else(|| files.iter().position(|f| f.path.ends_with("override.yml")))
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
                app.infra.file = Some(f.path.clone());
            }
        }
        if widgets::ghost(ui, "copy", "Copier", "").clicked() {
            ui.ctx().copy_text(file.content.clone());
            app.say("Copié");
        }
    });
    ui.add_space(4.0);
    let lines: Vec<&str> = file.content.lines().collect();
    lines_box(ui, "infra-file", &lines, false);
}

/// The title, the export button, and what the stack gives access to once it runs.
fn head(app: &mut App, ui: &mut Ui) {
    let errors = app
        .analysis()
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.label(RichText::new("Infra").size(XL).bold());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let export = widgets::primary(ui, "export", "Exporter l'infra", !errors).on_hover_text(
                if errors {
                    "Corrigez les erreurs du projet avant d'exporter"
                } else {
                    "Écrit ces fichiers dans le dossier de votre choix"
                },
            );
            if export.clicked() {
                crate::files::export_infra(app);
            }
        });
    });
    ui.add_space(4.0);
    ui.label(
        widgets::muted(
            "Les fichiers qui lancent les services : l'infrastructure (Kafka, PostgreSQL…) dans docker-compose.yml, \
             les limites, les interfaces d'administration et les services eux-mêmes dans docker-compose.override.yml, \
             et la configuration de Traefik et de Garage. Exportez-les dans le dossier du code exporté, puis \
             `docker compose up -d`. Le formulaire est enregistré dans le projet (le bouton est aussi dans le menu ⋮).",
        )
        .size(SM),
    );
    let links = sb_app::infra::links(&app.project());
    if !links.is_empty() {
        ui.add_space(8.0);
        ui.label(widgets::faint("Une fois la stack lancée :").size(SM));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            for link in links {
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

pub fn page(app: &mut App, ui: &mut Ui) {
    egui::Panel::left("infra-form")
        .exact_size(crate::theme::side(ui, 452.0, 330.0))
        .resizable(false)
        .frame(Frame::new().fill(PANEL).inner_margin(Margin::ZERO))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("infra-form-scroll")
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
                files(app, ui);
            });
    });
}
