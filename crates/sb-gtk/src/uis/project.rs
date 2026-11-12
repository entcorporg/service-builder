//! The project in the window: loading, redrawing, background analysis.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use gtk::gio;
use gtk::glib::{self, clone};
use gtk::prelude::*;
use sb_model::{Project, Severity};

use super::editors::draw_project;
use super::icons;
use super::mcp::draw_mcp;
use super::ui::{Shared, TITLE};
use super::widgets::{clear, row, say};

pub fn stamp_of(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64)
}

pub fn set_project(ui: &Shared, project: Project, path: Option<PathBuf>) {
    {
        let mut s = ui.state.borrow_mut();
        s.stamp = path.as_deref().map_or(0, stamp_of);
        s.project = project;
        s.path = path;
        s.dirty = false;
        s.undo.clear();
        s.redo.clear();
    }
    changed(ui);
}

/// The name of the project in the toolbar, marked with a dot while there are unsaved changes.
pub fn refresh_title(ui: &Shared) {
    let s = ui.state.borrow();
    let mark = if s.dirty { "• " } else { "" };
    if ui.title.text() != s.project.name {
        ui.title.set_text(&s.project.name);
    }
    ui.dirty_mark.set_visible(s.dirty);
    ui.file_label.set_text(
        &s.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or("non enregistré".to_string(), |n| {
                n.to_string_lossy().into_owned()
            }),
    );
    ui.window
        .set_title(Some(&format!("{mark}{} — {TITLE}", s.project.name)));
}

/// The project changed: redraw what is known now, then validate and generate in the background.
pub fn changed(ui: &Shared) {
    let (project, generation) = {
        let mut s = ui.state.borrow_mut();
        s.generation += 1;
        (s.project.clone(), s.generation)
    };
    refresh_title(ui);
    draw_project(ui);
    draw_mcp(ui);
    super::endpoint::draw(ui);

    glib::spawn_future_local(clone!(
        #[strong]
        ui,
        async move {
            let Ok(analysis) = gio::spawn_blocking(move || sb_app::analyze(&project)).await else {
                return;
            };
            if ui.state.borrow().generation != generation {
                return;
            }
            ui.state.borrow_mut().analysis = analysis;
            draw_analysis(&ui);
            super::endpoint::draw(&ui);
            super::canvas::refresh(&ui);
        }
    ));
}

pub fn watch_file(ui: &Shared) {
    let Some(path) = ui.state.borrow().path.clone() else {
        return;
    };
    let now = stamp_of(&path);
    // Unsaved edits are not thrown away for another program's: saving overwrites, as the user chose.
    if now == 0 || now == ui.state.borrow().stamp || ui.state.borrow().dirty {
        return;
    }
    // A file half-written by the other program fails to load: the next tick tries again, without a complaint.
    if let Ok(project) = sb_app::load_project(&path) {
        set_project(ui, project, Some(path));
        say(
            ui,
            "Le projet a été modifié par un autre programme : rechargé.",
        );
    }
}

pub fn load_sample(ui: &Shared, name: &str) {
    let project = match name {
        "showcase" => sb_model::sample::showcase(),
        _ => sb_model::sample::orders(),
    };
    set_project(ui, project, None);
}

// ---- drawing ----

pub fn draw_analysis(ui: &Shared) {
    let s = ui.state.borrow();
    let a = &s.analysis;

    clear(&ui.problems);
    for d in &a.diagnostics {
        let (mark, class) = match d.severity {
            Severity::Error => ("Erreur", "error"),
            Severity::Warning => ("Avertissement", "warning"),
        };
        let r = row(
            &format!("{mark} · {}", d.code),
            &format!("{}\n{}", d.message, d.path),
        );
        r.add_css_class(class);
        ui.problems.append(&r);
    }
    if a.diagnostics.is_empty() {
        ui.problems.append(&row("Aucun problème", ""));
    }
    let errors = a
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    let warnings = a.diagnostics.len() - errors;
    let (text, class, icon_name, color) = match (errors, warnings) {
        (0, 0) => ("Valide".to_string(), "ok", "ok", icons::OK),
        (0, w) => (
            format!("{w} avertissement{}", if w > 1 { "s" } else { "" }),
            "warning",
            "warn",
            icons::WARN,
        ),
        (e, _) => (
            format!("{e} erreur{}", if e > 1 { "s" } else { "" }),
            "error",
            "error",
            icons::ERR,
        ),
    };
    ui.pill_text.set_text(&text);
    if let Some(old) = ui.pill.first_child() {
        ui.pill.remove(&old);
    }
    ui.pill.prepend(&icons::icon(icon_name, 14, color));
    for c in ["ok", "warning", "error"] {
        ui.pill.remove_css_class(c);
    }
    ui.pill.add_css_class(class);
    ui.export_button.set_sensitive(errors == 0);
    ui.export_button.set_tooltip_text(Some(if errors == 0 {
        "Générer le workspace Cargo"
    } else {
        "Corrigez les erreurs avant d'exporter"
    }));
    // The panel at the bottom: a chevron, the title and the counts; it opens on the list.
    clear(&ui.problems_head);
    let chevron = icons::icon(
        if ui.problems_panel.reveals_child() {
            "down"
        } else {
            "right"
        },
        13,
        icons::FAINT,
    );
    ui.problems_head.append(&chevron);
    let title = gtk::Label::new(Some("Problèmes"));
    title.add_css_class("problems-title");
    ui.problems_head.append(&title);
    let count = |n: usize, icon_name: &str, color: &str, class: &str| {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        b.add_css_class("count");
        b.add_css_class(class);
        b.append(&icons::icon(icon_name, 12, color));
        b.append(&gtk::Label::new(Some(&n.to_string())));
        b
    };
    if errors > 0 {
        ui.problems_head
            .append(&count(errors, "error", icons::ERR, "error"));
    }
    if warnings > 0 {
        ui.problems_head
            .append(&count(warnings, "warn", icons::WARN, "warning"));
    }
    if errors == 0 && warnings == 0 {
        let ok = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        ok.add_css_class("count");
        ok.add_css_class("ok");
        ok.append(&icons::icon("ok", 12, icons::OK));
        ok.append(&gtk::Label::new(Some("Aucun problème")));
        ui.problems_head.append(&ok);
    }

    drop(s);
    super::codeview::draw(ui);
}
