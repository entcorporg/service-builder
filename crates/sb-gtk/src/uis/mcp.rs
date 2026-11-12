//! The MCP page: activation in `.mcp.json`, tools, and the reports of the agents.

use std::path::Path;

use gtk::gio;
use gtk::glib::{self, clone};
use gtk::prelude::*;
use sb_app::feedback::{Feedback, ReportStatus};
use sb_app::mcp::{Activation, EntryState};

use super::icons::{self, icon};
use super::ui::Shared;
use super::widgets::{clear, fail, label, say};
use crate::updater;

const ISSUES: &str = "https://github.com/entcorporg/service-builder/issues/new";

/// What a client must run to start the MCP server: this program. An AppImage runs from a temporary mount, so its
/// own path is what `APPIMAGE` holds.
pub fn program() -> Result<String, String> {
    if let Some(appimage) = updater::appimage_path() {
        return Ok(appimage.display().to_string());
    }
    std::env::current_exe()
        .map(|p| p.display().to_string())
        .map_err(|e| format!("cannot locate the program: {e}"))
}

/// Quotes a word for a POSIX shell only when it needs it.
fn shell_quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

fn mono_label(text: &str) -> gtk::Label {
    let l = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .selectable(true)
        .build();
    l.add_css_class("font-mono");
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_max_width_chars(1);
    l
}

/// A titled block with a `Copier` button and the text in a monospaced body.
fn copy_block(ui: &Shared, title: &str, text: &str) -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.add_css_class("copy-block");
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    head.add_css_class("copy-head");
    let t = gtk::Label::new(Some(title));
    t.add_css_class("copy-title");
    t.set_hexpand(true);
    t.set_xalign(0.0);
    head.append(&t);
    let copy = gtk::Button::new();
    let c = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    c.append(&icon("copy", 14, icons::TEXT));
    c.append(&gtk::Label::new(Some("Copier")));
    copy.set_child(Some(&c));
    copy.add_css_class("ghost");
    let text_owned = text.to_string();
    copy.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            ui.window.clipboard().set_text(&text_owned);
            say(&ui, "Copié");
        }
    ));
    head.append(&copy);
    outer.append(&head);
    let body = mono_label(text);
    body.add_css_class("copy-body");
    outer.append(&body);
    outer
}

fn small_title(text: &str) -> gtk::Label {
    let l = gtk::Label::builder()
        .label(text.to_uppercase())
        .xalign(0.0)
        .build();
    l.add_css_class("mcp-h3");
    l
}

pub fn draw_mcp(ui: &Shared) {
    ui.syncing.set(true);
    clear(&ui.mcp);
    let path = ui.state.borrow().path.clone();

    let title = gtk::Label::builder()
        .label("Serveur MCP")
        .xalign(0.0)
        .build();
    title.add_css_class("mcp-title");
    // The title and the paragraph under it are one block in the web version (4 px apart, 16 px from the rest).
    let heading = gtk::Box::new(gtk::Orientation::Vertical, 4);
    heading.append(&title);
    let intro = label(
        "Un client IA (Claude Code, Claude Desktop…) lance Service Builder en mode mcp et lit, modifie, valide et \
         exporte ce projet par le protocole MCP, sur stdio. Le serveur travaille sur le fichier du projet.",
    );
    intro.set_selectable(false);
    intro.set_max_width_chars(1);
    intro.add_css_class("mcp-intro");
    heading.append(&intro);
    ui.mcp.append(&heading);

    let command = program().unwrap_or_else(|e| e);
    let activation: Option<Result<Activation, String>> = path
        .as_ref()
        .map(|p| program().and_then(|command| sb_app::mcp::activation(p, &command)));

    // ---- activation ----
    let card = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    card.add_css_class("mcp-card");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
    text.set_hexpand(true);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let strong = gtk::Label::new(Some("Activation"));
    strong.add_css_class("font-bold");
    head.append(&strong);
    let state = activation
        .as_ref()
        .and_then(|a| a.as_ref().ok())
        .map(|a| a.state);
    if let Some(state) = state {
        let (label_text, class) = match state {
            EntryState::On => ("Actif", "on"),
            EntryState::Stale => ("À mettre à jour", "stale"),
            _ => ("Inactif", "off"),
        };
        let pill = gtk::Label::new(Some(label_text));
        pill.add_css_class("state-pill");
        pill.add_css_class(class);
        head.append(&pill);
    }
    text.append(&head);
    let detail = match &activation {
        Some(Ok(a)) => format!(
            "Ajoute l’entrée {} au fichier {}, que Claude Code lit depuis ce dossier. Les autres entrées sont conservées.",
            a.server_name, a.config_path
        ),
        Some(Err(e)) => e.clone(),
        None => "Enregistrez le projet pour pouvoir activer le MCP : le serveur travaille sur son fichier.".to_string(),
    };
    let small = label(&detail);
    small.set_selectable(false);
    small.set_max_width_chars(1);
    small.add_css_class("mcp-small");
    small.add_css_class("mcp-xs");
    text.append(&small);
    card.append(&text);
    if let (Some(state), Some(path)) = (state, path.clone()) {
        let on = state == EntryState::On;
        let button = gtk::Button::with_label(match (on, state == EntryState::Stale) {
            (true, _) => "Désactiver",
            (_, true) => "Mettre à jour",
            _ => "Activer",
        });
        button.add_css_class(if on { "ghost" } else { "primary" });
        button.set_valign(gtk::Align::Center);
        button.connect_clicked(clone!(
            #[strong]
            ui,
            move |_| {
                let result = program().and_then(|command| {
                    if on {
                        sb_app::mcp::disable(&path, &command)
                    } else {
                        sb_app::mcp::enable(&path, &command)
                    }
                });
                match result {
                    Ok(_) => {
                        say(&ui, if on { "MCP désactivé" } else { "MCP activé" });
                        draw_mcp(&ui);
                    }
                    Err(e) => fail(&ui, &e),
                }
            }
        ));
        card.append(&button);
    }
    ui.mcp.append(&card);

    // ---- manual configuration ----
    let name = activation
        .as_ref()
        .and_then(|a| a.as_ref().ok())
        .map_or("service-builder".to_string(), |a| a.server_name.clone());
    let project_arg = path
        .as_ref()
        .map_or("<projet.sb.json>".to_string(), |p| p.display().to_string());
    let line = [command.as_str(), "mcp", project_arg.as_str()]
        .map(shell_quote)
        .join(" ");
    let json = serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": { name.clone(): { "command": command, "args": ["mcp", project_arg] } }
    }))
    .unwrap_or_default();
    // As in the web version, each titled part is one block: 16 px between the blocks, less inside them.
    let config = gtk::Box::new(gtk::Orientation::Vertical, 0);
    config.append(&small_title("Configuration manuelle"));
    config.append(&copy_block(ui, "Commande du serveur", &line));
    config.append(&copy_block(
        ui,
        "Claude Code",
        &format!("claude mcp add {name} -- {line}"),
    ));
    config.append(&copy_block(ui, "Autres clients (Claude Desktop…)", &json));
    ui.mcp.append(&config);

    // ---- tools ----
    let tools_part = gtk::Box::new(gtk::Orientation::Vertical, 0);
    tools_part.append(&small_title("Outils exposés"));
    let tools = gtk::Box::new(gtk::Orientation::Vertical, 0);
    tools.add_css_class("tool-list");
    for (i, tool) in sb_app::mcp::tool_summaries().iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("tool-row");
        if i == 0 {
            row.add_css_class("first");
        }
        let n = mono_label(tool.name);
        n.set_width_request(190);
        n.set_valign(gtk::Align::Start);
        n.add_css_class("tool-name");
        row.append(&n);
        let d = label(tool.description);
        d.set_selectable(false);
        d.set_max_width_chars(1);
        d.set_hexpand(true);
        d.add_css_class("tool-desc");
        row.append(&d);
        tools.append(&row);
    }
    tools_part.append(&tools);
    let note = label(
        "Les modifications d’un client sont écrites dans le fichier du projet. L’application le recharge quand la \
         fenêtre reprend le focus, sauf si des modifications non enregistrées sont en cours.",
    );
    note.set_selectable(false);
    note.set_max_width_chars(1);
    note.add_css_class("mcp-small");
    tools_part.append(&note);
    ui.mcp.append(&tools_part);

    if let Some(path) = path {
        match sb_app::feedback::load(&path) {
            Ok(feedback) => draw_feedback(ui, &path, &feedback),
            Err(e) => ui.mcp.append(&label(&e)),
        }
    }
    ui.syncing.set(false);
}

pub fn draw_feedback(ui: &Shared, path: &Path, feedback: &Feedback) {
    ui.mcp.append(&small_title("Suggestions des agents"));
    let switch = gtk::Switch::new();
    switch.set_active(feedback.enabled);
    switch.set_valign(gtk::Align::Center);
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    line.append(&switch);
    line.append(&label("Demander un rapport à l’agent tous les 50 appels"));
    ui.mcp.append(&line);
    let project_file = path.to_path_buf();
    switch.connect_state_set(clone!(
        #[strong]
        ui,
        #[strong]
        project_file,
        move |_, on| {
            if !ui.syncing.get()
                && let Err(e) = sb_app::feedback::set_enabled(&project_file, on)
            {
                fail(&ui, &e);
            }
            glib::Propagation::Proceed
        }
    ));

    for report in &feedback.reports {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
        card.add_css_class("card");
        card.set_margin_top(6);
        let opened = if report.status == ReportStatus::Opened {
            " (issue ouverte)"
        } else {
            ""
        };
        let summary = if report.summary.is_empty() {
            "Rapport"
        } else {
            &report.summary
        };
        card.append(&label(&format!("#{} · {summary}{opened}", report.id)));
        for s in &report.suggestions {
            card.append(&label(&format!("• {} {}", s.title, s.details)));
        }
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let issue = gtk::Button::with_label("Ouvrir l'issue GitHub");
        let dismiss = gtk::Button::with_label("Ignorer");
        actions.append(&issue);
        actions.append(&dismiss);
        card.append(&actions);
        ui.mcp.append(&card);

        let id = report.id;
        issue.connect_clicked(clone!(
            #[strong]
            ui,
            #[strong]
            project_file,
            move |_| match sb_app::feedback::issue_url_for(&project_file, id) {
                Ok(url) => {
                    open_url(&ui, &url);
                    draw_mcp(&ui);
                }
                Err(e) => fail(&ui, &e),
            }
        ));
        dismiss.connect_clicked(clone!(
            #[strong]
            ui,
            #[strong]
            project_file,
            move |_| match sb_app::feedback::dismiss(&project_file, id) {
                Ok(_) => draw_mcp(&ui),
                Err(e) => fail(&ui, &e),
            }
        ));
    }
}

/// Opens a page in the browser; only the project's "new issue" pages (or its releases) are ever passed here.
pub fn open_url(ui: &Shared, url: &str) {
    let allowed = url.starts_with(ISSUES)
        || url.starts_with("https://github.com/entcorporg/service-builder/");
    if !allowed {
        fail(ui, "adresse refusée");
        return;
    }
    gtk::UriLauncher::new(url).launch(
        Some(&ui.window),
        gio::Cancellable::NONE,
        clone!(
            #[strong]
            ui,
            move |result| {
                if let Err(e) = result {
                    say(&ui, &format!("Impossible d'ouvrir le navigateur : {e}"));
                }
            }
        ),
    );
}
