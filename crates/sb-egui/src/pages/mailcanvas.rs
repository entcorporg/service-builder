//! The Mail page, opened from a mail of a mail service in the inspector: the mails of the service on the left, the
//! blocks and a preview of the mail open in the middle, the properties of the selected block, the settings of the mail
//! and its sending rule (address, identity, sender) on the right. Nothing is typed as HTML: the HTML and the text are
//! derived from the blocks (`sb_model::maildesign`) and only shown. A mail written by hand shows its HTML and its text
//! instead, until it is composed with blocks.
//!
//! The editor works on a copy of the design, written to the project as it changes (once the mouse is released, so that
//! a colour being dragged is not analysed at every frame), and when the page is left. The changes made within two
//! seconds of each other are one entry of the history.
//!
//! The preview is drawn with egui, not by a mail client: it follows the structure, the colours, the alignment, the
//! relative sizes and the width of the mail, with the variables shown as `‹name›`. It does not load images (a grey box
//! with the alt text stands for them), has no serif font, and does not know what Outlook or Gmail would do to the HTML. The
//! "Ouvrir dans le navigateur" button writes the derived HTML to a file and opens it, which is the nearest to a mail client
//! the desktop has.

use eframe::egui::{
    self, Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, Sense, Stroke, Ui,
};
use sb_model::maildesign::{
    self, MailAlign, MailBlock, MailColumn, MailDesign, MailFont, MailIssue, parse_color,
};
use sb_model::{Project, ServiceKind};

use super::canvas::{Sel, select};
use super::common::{empty, icon_btn, link, section, separator};
use crate::app::{App, Page};
use crate::theme::*;
use crate::widgets::{faint, muted};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Preview,
    Html,
    Text,
}

/// The mail open on the Mail page. `path` addresses the selected block: `[i]`, or `[i, column, k]` inside columns.
pub struct MailEditor {
    si: usize,
    topic: String,
    /// The working copy of the design; `None` for a mail written by hand.
    design: Option<MailDesign>,
    /// The design as the project has it, when it was last read or written.
    synced: Option<MailDesign>,
    path: Option<Vec<usize>>,
    tab: Tab,
}

/// Opens the Mail page on the mail of `topic` in service `si` (the mail open before is written first).
pub fn open(app: &mut App, si: usize, topic: &str) {
    close(app);
    let design = template_design(&app.project(), si, topic);
    app.mail_editor = Some(MailEditor {
        si,
        topic: topic.into(),
        synced: design.clone(),
        design,
        path: None,
        tab: Tab::Preview,
    });
    app.page = Page::Mail;
}

/// Writes what the editor changed last, and closes it.
pub fn close(app: &mut App) {
    if let Some(mut ed) = app.mail_editor.take() {
        flush(app, &mut ed);
    }
}

/// A first design: a heading from the subject and an empty paragraph.
pub fn starter_design(subject: &str) -> MailDesign {
    MailDesign {
        settings: Default::default(),
        blocks: vec![
            MailBlock::heading(subject),
            MailBlock::paragraph("Bonjour,"),
        ],
    }
}

fn template_design(p: &Project, si: usize, topic: &str) -> Option<MailDesign> {
    p.services
        .get(si)?
        .templates
        .iter()
        .find(|t| t.event_type == topic)?
        .design
        .clone()
}

/// The variables a mail can use: the fields of its event that have a text form.
pub fn event_fields(p: &Project, topic: &str) -> Vec<String> {
    p.topic(topic)
        .and_then(|t| p.message(&t.message))
        .map(|m| {
            m.fields
                .iter()
                .filter(|f| !f.repeated && !matches!(f.ty, sb_model::FieldType::Message { .. }))
                .map(|f| f.name.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// The project changed under the editor (undo, a reload, a conversion): the copy follows it.
fn sync(app: &App, ed: &mut MailEditor) {
    let now = template_design(&app.project(), ed.si, &ed.topic);
    if now != ed.synced {
        ed.design = now.clone();
        ed.synced = now;
        ed.path = None;
    }
}

/// Writes the copy to the project when it differs.
fn flush(app: &mut App, ed: &mut MailEditor) {
    if ed.design == ed.synced || ed.design.is_none() {
        return;
    }
    let (si, topic, design) = (ed.si, ed.topic.clone(), ed.design.clone());
    app.edit_grouped(&format!("mail/{si}/{topic}"), |p| {
        if let Some(t) = p
            .services
            .get_mut(si)
            .and_then(|s| s.templates.iter_mut().find(|t| t.event_type == topic))
        {
            t.design = design;
        }
    });
    ed.synced = ed.design.clone();
}

/// What the page asks once drawn (it needs the editor back in the app).
enum Action {
    Open(String),
    Add(String),
    Delete,
    Handwritten,
    Compose,
    Back,
}

/// The Mail page.
pub fn page(app: &mut App, ui: &mut Ui) {
    let Some(mut ed) = app.mail_editor.take() else {
        return missing(app, ui, "Aucun mail ouvert.");
    };
    sync(app, &mut ed);
    let p = app.project();
    let si = ed.si;
    let found = p
        .services
        .get(si)
        .filter(|s| s.kind == ServiceKind::Mta)
        .and_then(|s| s.consumers.iter().position(|c| c.topic == ed.topic))
        .filter(|_| {
            p.services[si]
                .templates
                .iter()
                .any(|t| t.event_type == ed.topic)
        });
    let Some(ci) = found else {
        // Deleted (or undone): back to the service, or to the canvas.
        app.mail_editor = None;
        return missing(app, ui, "Ce mail n’existe plus.");
    };
    let mut action = None;

    egui::Panel::left("mail-list")
        .exact_size(250.0)
        .resizable(false)
        .frame(
            Frame::new()
                .fill(PANEL)
                .inner_margin(Margin::symmetric(12, 14)),
        )
        .show(ui, |ui| mails_list(ui, &p, &ed, &mut action));

    let fields = event_fields(&p, &ed.topic);
    let issues = ed.design.as_ref().map_or_else(Vec::new, |d| {
        maildesign::check(
            d,
            &fields.iter().map(String::as_str).collect::<Vec<_>>(),
            "",
        )
    });
    egui::Panel::top("mail-head")
        .frame(
            Frame::new()
                .fill(PANEL)
                .inner_margin(Margin::symmetric(14, 10)),
        )
        .show_separator_line(true)
        .show(ui, |ui| header(ui, &p, &ed, &issues, &mut action));

    egui::Panel::right("mail-props")
        .exact_size(360.0)
        .resizable(false)
        .frame(
            Frame::new()
                .fill(PANEL)
                .inner_margin(Margin::symmetric(16, 8)),
        )
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("mail-props")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_max_width(ui.available_width() - 12.0);
                    if let Some(design) = ed.design.as_mut() {
                        properties(ui, design, &mut ed.path, &fields);
                        problems(ui, &issues);
                        separator(ui);
                    }
                    section(
                        ui,
                        "Envoi",
                        "publish",
                        |_| {},
                        |ui| super::mail::rule_settings(app, ui, &p, si, ci),
                    );
                });
        });

    egui::CentralPanel::no_frame()
        .frame(Frame::new().inner_margin(Margin::same(12)))
        .show(ui, |ui| match ed.design.as_mut() {
            Some(design) => composed(ui, design, &mut ed.path, &mut ed.tab),
            None => handwritten(app, ui, &p, si, &ed.topic),
        });

    if !ui.input(|i| i.pointer.any_down()) {
        flush(app, &mut ed);
    }
    let (topic, si) = (ed.topic.clone(), ed.si);
    app.mail_editor = Some(ed);
    match action {
        Some(Action::Open(t)) => open(app, si, &t),
        Some(Action::Add(t)) => super::mail::add_mail(app, si, &t),
        Some(Action::Delete) => {
            close(app);
            super::mail::delete_mail(app, si, &topic);
            let next = app.project().services[si]
                .consumers
                .first()
                .map(|c| c.topic.clone());
            match next {
                Some(t) => open(app, si, &t),
                None => back_to_service(app, si),
            }
        }
        Some(Action::Handwritten) => {
            close(app);
            to_handwritten(app, si, &topic);
            open(app, si, &topic);
        }
        Some(Action::Compose) => {
            close(app);
            compose(app, si, &topic);
            open(app, si, &topic);
        }
        Some(Action::Back) => back_to_service(app, si),
        None => {}
    }
}

/// Back on the canvas, the mail service selected (its inspector lists the mails).
fn back_to_service(app: &mut App, si: usize) {
    close(app);
    let name = app.project().services.get(si).map(|s| s.name.clone());
    select(app, name.map(Sel::Service));
    app.page = Page::Architecture;
}

fn missing(app: &mut App, ui: &mut Ui, text: &str) {
    ui.add_space(24.0);
    ui.horizontal(|ui| {
        ui.add_space(24.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            empty(ui, text);
            if link(ui, "← Retour à l’architecture").clicked() {
                app.page = Page::Architecture;
            }
        });
    });
}

/// The left column: the way back to the service, and its mails (the open one raised).
fn mails_list(ui: &mut Ui, p: &Project, ed: &MailEditor, action: &mut Option<Action>) {
    let svc = &p.services[ed.si];
    ui.spacing_mut().item_spacing.y = 8.0;
    if link(ui, &format!("← {}", svc.name)).clicked() {
        *action = Some(Action::Back);
    }
    ui.add_space(4.0);
    crate::widgets::section_title(ui, "Mails");
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for c in &svc.consumers {
                let on = c.topic == ed.topic;
                let r = Frame::new()
                    .fill(if on { RAISED } else { PANEL2 })
                    .stroke(Stroke::new(1.0, if on { ACCENT } else { BORDER }))
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(10, 7))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.add(egui::Label::new(RichText::new(&c.topic).font(mono(SM))).truncate());
                        ui.add(
                            egui::Label::new(
                                RichText::new(super::mail::mail_summary(p, ed.si, &c.topic))
                                    .size(SM)
                                    .color(MUTED),
                            )
                            .truncate(),
                        );
                    });
                let hit = ui
                    .interact(
                        r.response.rect,
                        ui.make_persistent_id(("mail-list", &c.topic)),
                        Sense::click(),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if hit.hovered() && !on {
                    ui.painter().rect_stroke(
                        r.response.rect,
                        8,
                        Stroke::new(1.0, alpha(ACCENT, 140)),
                        egui::StrokeKind::Inside,
                    );
                }
                if hit.clicked() && !on {
                    *action = Some(Action::Open(c.topic.clone()));
                }
            }
            let free = super::mail::free_topics(p, ed.si);
            if !free.is_empty() {
                let prompt = "+ Ajouter un mail…".to_string();
                let mut options = vec![prompt.clone()];
                options.extend(free);
                if let Some(t) =
                    super::common::choice(ui, ("mail-page-add", ed.si), &options, &prompt)
                    && t != prompt
                {
                    *action = Some(Action::Add(t));
                }
            }
        });
}

/// The bar over the mail: its event and service, its state, and what can be done with it.
fn header(
    ui: &mut Ui,
    p: &Project,
    ed: &MailEditor,
    issues: &[MailIssue],
    action: &mut Option<Action>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.label(RichText::new(&ed.topic).font(mono(BASE)).strong());
        ui.label(muted(format!("mail de {}", p.services[ed.si].name)));
        let errors = issues.iter().filter(|i| i.error).count();
        if errors > 0 {
            ui.label(RichText::new(format!("{errors} erreur(s)")).color(ERR));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icon_btn(ui, "trash", &format!("Supprimer le mail de {}", ed.topic), true).clicked() {
                *action = Some(Action::Delete);
            }
            if ed.design.is_some() {
                if ui
                    .button("Passer à l’écriture à la main")
                    .on_hover_text("Reprend le HTML et le texte dérivés des blocs comme gabarit écrit à la main.")
                    .clicked()
                {
                    *action = Some(Action::Handwritten);
                }
            } else if ui
                .button("Composer avec des blocs")
                .on_hover_text("Le HTML et le texte écrits à la main sont remplacés par ceux que dérivent les blocs. Ctrl+Z les rend.")
                .clicked()
            {
                *action = Some(Action::Compose);
            }
        });
    });
}

/// Gives the mail back to the hand: the derived HTML and text become the ones of the template, which has no design any more.
fn to_handwritten(app: &mut App, si: usize, topic: &str) {
    app.edit(|p| {
        if let Some(t) = p.services[si]
            .templates
            .iter_mut()
            .find(|t| t.event_type == topic)
            && let Some(design) = t.design.take()
        {
            t.html = maildesign::render_html(&design);
            t.text = maildesign::render_text(&design);
        }
    });
    app.say("Le mail est maintenant écrit à la main (HTML et texte repris des blocs)");
}

/// Composes a mail written by hand with blocks: its HTML and its text are replaced (Ctrl+Z gives them back).
fn compose(app: &mut App, si: usize, topic: &str) {
    app.edit(|p| {
        if let Some(t) = p.services[si]
            .templates
            .iter_mut()
            .find(|t| t.event_type == topic)
        {
            t.design = Some(starter_design(&t.subject));
            t.html.clear();
            t.text.clear();
        }
    });
    app.say("Le mail est composé avec des blocs : Ctrl+Z rend le HTML écrit à la main");
}

/// The middle of a mail composed with blocks: the blocks, and the preview or what they derive.
fn composed(ui: &mut Ui, design: &mut MailDesign, path: &mut Option<Vec<usize>>, tab: &mut Tab) {
    egui_extras::StripBuilder::new(ui)
        .size(egui_extras::Size::exact(260.0))
        .size(egui_extras::Size::remainder().at_least(320.0))
        .horizontal(|mut strip| {
            strip.cell(|ui| {
                ui.label(RichText::new("Blocs").strong());
                egui::ScrollArea::vertical()
                    .id_salt("mail-blocks")
                    .show(ui, |ui| blocks_list(ui, &mut design.blocks, &[], path, false));
            });
            strip.cell(|ui| {
                ui.horizontal(|ui| {
                    for (t, label) in [
                        (Tab::Preview, "Aperçu"),
                        (Tab::Html, "HTML dérivé"),
                        (Tab::Text, "Texte dérivé"),
                    ] {
                        ui.selectable_value(tab, t, label);
                    }
                    if ui
                        .button("Ouvrir dans le navigateur")
                        .on_hover_text("Écrit le HTML dérivé dans un fichier (variables remplacées par [nom]) et l'ouvre.")
                        .clicked()
                    {
                        open_in_browser(ui, design);
                    }
                });
                egui::ScrollArea::both().id_salt("mail-center").show(ui, |ui| match tab {
                    Tab::Preview => preview(ui, design),
                    Tab::Html => {
                        ui.add(
                            egui::TextEdit::multiline(&mut maildesign::render_html(design).as_str())
                                .font(mono(SM))
                                .desired_width(f32::INFINITY),
                        );
                    }
                    Tab::Text => {
                        ui.add(
                            egui::TextEdit::multiline(&mut maildesign::render_text(design).as_str())
                                .font(mono(SM))
                                .desired_width(f32::INFINITY),
                        );
                    }
                });
            });
        });
}

/// The middle of a mail written by hand: its HTML and its text, applied when left.
fn handwritten(app: &mut App, ui: &mut Ui, p: &Project, si: usize, topic: &str) {
    let Some(t) = p.services[si]
        .templates
        .iter()
        .find(|t| t.event_type == topic)
    else {
        return;
    };
    egui::ScrollArea::vertical()
        .id_salt("mail-hand")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            empty(
                ui,
                "Ce mail est écrit à la main. « Composer avec des blocs » le remplace par un mail en blocs, sans HTML à écrire.",
            );
            for (label, value, html) in [("HTML", &t.html, true), ("Texte", &t.text, false)] {
                ui.label(RichText::new(label).strong());
                if let Some(v) = crate::widgets::text_area(ui, ("mail-hand", si, topic, html), value, 0.0) {
                    let topic = topic.to_string();
                    app.edit(move |p| {
                        if let Some(t) = p.services[si]
                            .templates
                            .iter_mut()
                            .find(|t| t.event_type == topic)
                        {
                            if html {
                                t.html = v;
                            } else {
                                t.text = v;
                            }
                        }
                    });
                }
            }
        });
}

fn mono(size: f32) -> egui::FontId {
    egui::FontId::monospace(size)
}

fn problems(ui: &mut Ui, issues: &[MailIssue]) {
    if issues.is_empty() {
        return;
    }
    ui.add_space(8.0);
    ui.label(RichText::new("Problèmes").strong());
    for i in issues {
        let color = if i.error { ERR } else { WARN };
        ui.label(
            RichText::new(format!("{} — {}", i.code, i.message))
                .size(SM)
                .color(color),
        );
    }
}

// ---- the blocks ---------------------------------------------------------------------------

const KINDS: [&str; 7] = [
    "Titre",
    "Paragraphe",
    "Bouton",
    "Image",
    "Séparateur",
    "Espace",
    "Colonnes",
];

fn new_block(kind: usize) -> MailBlock {
    match kind {
        0 => MailBlock::heading("Titre"),
        1 => MailBlock::paragraph("Texte"),
        2 => MailBlock::button("Ouvrir", "https://"),
        3 => MailBlock::image("https://", ""),
        4 => MailBlock::divider(),
        5 => MailBlock::spacer(),
        _ => MailBlock::columns(2),
    }
}

fn kind_name(b: &MailBlock) -> &'static str {
    match b {
        MailBlock::Heading { .. } => KINDS[0],
        MailBlock::Paragraph { .. } => KINDS[1],
        MailBlock::Button { .. } => KINDS[2],
        MailBlock::Image { .. } => KINDS[3],
        MailBlock::Divider { .. } => KINDS[4],
        MailBlock::Spacer { .. } => KINDS[5],
        MailBlock::Columns { .. } => KINDS[6],
    }
}

fn summary(b: &MailBlock) -> String {
    let first = |t: &str| {
        t.lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(28)
            .collect::<String>()
    };
    match b {
        MailBlock::Heading { text, .. } | MailBlock::Paragraph { text, .. } => first(text),
        MailBlock::Button { label, .. } => first(label),
        MailBlock::Image { alt, url, .. } => first(if alt.is_empty() { url } else { alt }),
        MailBlock::Columns { columns } => format!("{} colonnes", columns.len()),
        MailBlock::Divider { .. } | MailBlock::Spacer { .. } => String::new(),
    }
}

/// The list of the blocks of one level (the mail, or a column): select, move, remove, add. `prefix` is where the list is.
/// Returns whether the structure changed (the selection is then dropped, its path may no longer mean the same block).
fn blocks_list(
    ui: &mut Ui,
    blocks: &mut Vec<MailBlock>,
    prefix: &[usize],
    selected: &mut Option<Vec<usize>>,
    nested: bool,
) {
    let mut mv: Option<(usize, isize)> = None;
    let mut remove = None;
    let count = blocks.len();
    for (i, b) in blocks.iter_mut().enumerate() {
        let mut here = prefix.to_vec();
        here.push(i);
        ui.horizontal(|ui| {
            // The buttons first, on the right: a long summary is cut before them.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("✕").on_hover_text("Supprimer").clicked() {
                    remove = Some(i);
                }
                if ui
                    .add_enabled(i + 1 < count, egui::Button::new("↓").small())
                    .on_hover_text("Descendre")
                    .clicked()
                {
                    mv = Some((i, 1));
                }
                if ui
                    .add_enabled(i > 0, egui::Button::new("↑").small())
                    .on_hover_text("Monter")
                    .clicked()
                {
                    mv = Some((i, -1));
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let text = format!("{} {}", kind_name(b), summary(b));
                    let is = selected.as_ref() == Some(&here);
                    if ui
                        .add(egui::Button::selectable(is, text).truncate())
                        .clicked()
                    {
                        *selected = Some(here.clone());
                    }
                });
            });
        });
        if let MailBlock::Columns { columns } = b {
            for (c, column) in columns.iter_mut().enumerate() {
                ui.indent(("col", prefix, i, c), |ui| {
                    ui.label(faint(format!("Colonne {}", c + 1)));
                    let mut inner = here.clone();
                    inner.push(c);
                    blocks_list(ui, &mut column.blocks, &inner, selected, true);
                });
            }
        }
    }
    if let Some((i, d)) = mv {
        let j = (i as isize + d) as usize;
        blocks.swap(i, j);
        *selected = None;
    }
    if let Some(i) = remove {
        blocks.remove(i);
        *selected = None;
    }
    ui.menu_button("+ Ajouter un bloc", |ui| {
        for (k, name) in KINDS.iter().enumerate() {
            // Columns do not nest: the model refuses it, so the editor does not offer it.
            if nested && k == 6 {
                continue;
            }
            if ui.button(*name).clicked() {
                blocks.push(new_block(k));
                *selected = Some({
                    let mut p = prefix.to_vec();
                    p.push(blocks.len() - 1);
                    p
                });
                ui.close();
            }
        }
    });
}

fn block_at<'a>(blocks: &'a mut [MailBlock], path: &[usize]) -> Option<&'a mut MailBlock> {
    let (first, rest) = path.split_first()?;
    let block = blocks.get_mut(*first)?;
    if rest.is_empty() {
        return Some(block);
    }
    let MailBlock::Columns { columns } = block else {
        return None;
    };
    let (c, rest) = rest.split_first()?;
    block_at(&mut columns.get_mut(*c)?.blocks, rest)
}

// ---- the properties -----------------------------------------------------------------------

fn properties(
    ui: &mut Ui,
    design: &mut MailDesign,
    path: &mut Option<Vec<usize>>,
    fields: &[String],
) {
    ui.add_space(6.0);
    ui.label(RichText::new("Bloc sélectionné").strong());
    match path.clone().and_then(|p| block_at(&mut design.blocks, &p)) {
        None => {
            ui.label(muted("Choisissez un bloc dans la liste."));
        }
        Some(b) => block_props(ui, b, fields),
    }
    ui.add_space(12.0);
    egui::CollapsingHeader::new(RichText::new("Réglages du mail").strong())
        .default_open(true)
        .show(ui, |ui| settings(ui, design, fields));
}

fn block_props(ui: &mut Ui, b: &mut MailBlock, fields: &[String]) {
    match b {
        MailBlock::Heading {
            text,
            level,
            align,
            color,
        } => {
            text_var(ui, "Texte", text, false, fields);
            ui.horizontal(|ui| {
                ui.label("Niveau");
                for l in 1..=3u8 {
                    ui.selectable_value(level, l, l.to_string());
                }
            });
            align_row(ui, align);
            optional_color(ui, "Couleur", color);
        }
        MailBlock::Paragraph {
            text,
            align,
            size,
            color,
        } => {
            text_var(ui, "Texte", text, true, fields);
            align_row(ui, align);
            number(ui, "Taille (px)", size, 10..=32);
            optional_color(ui, "Couleur", color);
        }
        MailBlock::Button {
            label,
            url,
            align,
            background,
            color,
            radius,
        } => {
            text_var(ui, "Libellé", label, false, fields);
            text_var(ui, "Adresse du lien", url, false, fields);
            align_row(ui, align);
            color_row(ui, "Fond", background);
            color_row(ui, "Texte", color);
            number(ui, "Arrondi (px)", radius, 0..=40);
        }
        MailBlock::Image {
            url,
            alt,
            width,
            align,
            link,
        } => {
            text_var(ui, "Adresse de l'image", url, false, fields);
            text_var(ui, "Texte alternatif", alt, false, fields);
            let mut fixed = width.is_some();
            if ui.checkbox(&mut fixed, "Largeur fixe").changed() {
                *width = fixed.then_some(200);
            }
            if let Some(w) = width {
                number(ui, "Largeur (px)", w, 16..=800);
            }
            align_row(ui, align);
            let mut linked = link.is_some();
            if ui.checkbox(&mut linked, "L'image est un lien").changed() {
                *link = linked.then(|| "https://".to_string());
            }
            if let Some(l) = link {
                text_var(ui, "Adresse du lien", l, false, fields);
            }
        }
        MailBlock::Divider { color, thickness } => {
            color_row(ui, "Couleur", color);
            number(ui, "Épaisseur (px)", thickness, 1..=10);
        }
        MailBlock::Spacer { height } => number(ui, "Hauteur (px)", height, 1..=200),
        MailBlock::Columns { columns } => {
            ui.horizontal(|ui| {
                ui.label("Colonnes");
                for n in 2..=3usize {
                    if ui
                        .selectable_label(columns.len() == n, n.to_string())
                        .clicked()
                    {
                        while columns.len() < n {
                            columns.push(MailColumn {
                                blocks: vec![MailBlock::paragraph("")],
                            });
                        }
                        // A column that goes away takes its blocks: the author sees them go.
                        columns.truncate(n);
                    }
                }
            });
        }
    }
}

fn settings(ui: &mut Ui, d: &mut MailDesign, fields: &[String]) {
    let s = &mut d.settings;
    number(ui, "Largeur (px)", &mut s.width, 320..=800);
    color_row(ui, "Fond de page", &mut s.background);
    color_row(ui, "Fond du contenu", &mut s.content_background);
    color_row(ui, "Texte", &mut s.text_color);
    ui.horizontal(|ui| {
        ui.label("Police");
        for (f, name) in [
            (MailFont::Sans, "Sans-serif"),
            (MailFont::Serif, "Serif"),
            (MailFont::Mono, "Chasse fixe"),
        ] {
            ui.selectable_value(&mut s.font, f, name);
        }
    });
    text_var(ui, "Pré-en-tête", &mut s.preheader, false, fields);
    ui.label(faint(
        "La ligne que la boîte de réception montre à côté du sujet.",
    ));
    text_var(ui, "Pied de mail", &mut s.footer, true, fields);
}

fn align_row(ui: &mut Ui, align: &mut MailAlign) {
    ui.horizontal(|ui| {
        ui.label("Alignement");
        for (a, name) in [
            (MailAlign::Left, "Gauche"),
            (MailAlign::Center, "Centre"),
            (MailAlign::Right, "Droite"),
        ] {
            ui.selectable_value(align, a, name);
        }
    });
}

fn number<T: egui::emath::Numeric>(
    ui: &mut Ui,
    label: &str,
    v: &mut T,
    range: std::ops::RangeInclusive<T>,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(v).range(range));
    });
}

fn to_color(s: &str) -> Color32 {
    parse_color(s).map_or(Color32::BLACK, |[r, g, b]| Color32::from_rgb(r, g, b))
}

fn color_row(ui: &mut Ui, label: &str, hex: &mut String) {
    ui.horizontal(|ui| {
        ui.label(label);
        let c = to_color(hex);
        let mut rgb = [c.r(), c.g(), c.b()];
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            *hex = format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
        }
        ui.label(faint(hex.clone()));
    });
}

/// A colour that may be left to the settings of the mail.
fn optional_color(ui: &mut Ui, label: &str, color: &mut Option<String>) {
    let mut own = color.is_some();
    if ui
        .checkbox(&mut own, format!("{label} propre au bloc"))
        .changed()
    {
        *color = own.then(|| "#222222".to_string());
    }
    if let Some(c) = color {
        color_row(ui, label, c);
    }
}

/// A text with the picker of the variables of the event: choosing one inserts `{{ name }}` at the end of the text.
fn text_var(ui: &mut Ui, label: &str, text: &mut String, multiline: bool, fields: &[String]) {
    ui.label(label);
    if multiline {
        ui.add(
            egui::TextEdit::multiline(text)
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
    } else {
        ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
    }
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(("var", label, multiline))
            .selected_text("Insérer une variable")
            .show_ui(ui, |ui| {
                if fields.is_empty() {
                    ui.label(muted("L'événement n'a aucun champ."));
                }
                for f in fields {
                    if ui.selectable_label(false, f).clicked() {
                        text.push_str(&format!("{{{{ {f} }}}}"));
                    }
                }
            });
        if let Err(why) = maildesign::segments(text) {
            ui.label(RichText::new(why).size(SM).color(ERR));
        }
    });
}

// ---- the preview ----------------------------------------------------------------------------

fn shown(text: &str) -> String {
    maildesign::fill(text, |v| format!("‹{v}›"))
}

fn halign(a: MailAlign) -> Align {
    match a {
        MailAlign::Left => Align::Min,
        MailAlign::Center => Align::Center,
        MailAlign::Right => Align::Max,
    }
}

fn preview(ui: &mut Ui, d: &MailDesign) {
    let s = &d.settings;
    let page = to_color(&s.background);
    let content = to_color(&s.content_background);
    let fg = to_color(&s.text_color);
    let family = match s.font {
        MailFont::Mono => egui::FontFamily::Monospace,
        _ => egui::FontFamily::Proportional,
    };
    egui::Frame::new()
        .fill(page)
        .inner_margin(12.0)
        .show(ui, |ui| {
            let avail = ui.available_width().max(200.0);
            let w = avail.min(s.width as f32);
            ui.set_width(avail);
            ui.vertical_centered(|ui| {
                egui::Frame::new().fill(content).show(ui, |ui| {
                    ui.set_width(w);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for b in &d.blocks {
                        draw_block(ui, b, fg, &family, w);
                    }
                    ui.add_space(8.0);
                });
                if !s.footer.trim().is_empty() {
                    ui.add_space(8.0);
                    ui.set_width(w);
                    ui.label(
                        RichText::new(shown(&s.footer))
                            .size(11.0)
                            .color(Color32::from_gray(120)),
                    );
                }
            });
        });
}

fn draw_block(ui: &mut Ui, b: &MailBlock, fg: Color32, family: &egui::FontFamily, w: f32) {
    let pad = (w * 0.04).clamp(8.0, 24.0);
    let text = |ui: &mut Ui, t: &str, size: f32, color: Color32, bold: bool, a: MailAlign| {
        ui.allocate_ui_with_layout(
            egui::vec2(w - 2.0 * pad, 0.0),
            Layout::top_down(halign(a)),
            |ui| {
                ui.set_width(w - 2.0 * pad);
                let mut rt = RichText::new(shown(t))
                    .size(size)
                    .color(color)
                    .family(family.clone());
                if bold {
                    rt = rt.strong();
                }
                ui.add(egui::Label::new(rt).halign(halign(a)).wrap());
            },
        );
    };
    match b {
        MailBlock::Heading {
            text: t,
            level,
            align,
            color,
        } => {
            let size = match level {
                2 => 22.0,
                3 => 18.0,
                _ => 28.0,
            };
            ui.add_space(12.0);
            let c = color.as_deref().map_or(fg, to_color);
            ui.horizontal(|ui| {
                ui.add_space(pad);
                text(ui, t, size, c, true, *align);
            });
            ui.add_space(6.0);
        }
        MailBlock::Paragraph {
            text: t,
            align,
            size,
            color,
        } => {
            ui.add_space(6.0);
            let c = color.as_deref().map_or(fg, to_color);
            ui.horizontal(|ui| {
                ui.add_space(pad);
                text(ui, t, *size as f32, c, false, *align);
            });
            ui.add_space(6.0);
        }
        MailBlock::Button {
            label,
            align,
            background,
            color,
            radius,
            ..
        } => {
            ui.add_space(12.0);
            ui.allocate_ui_with_layout(
                egui::vec2(w, 0.0),
                Layout::top_down(halign(*align)),
                |ui| {
                    ui.set_width(w);
                    ui.add_space(0.0);
                    ui.horizontal(|ui| {
                        let label = shown(label);
                        let galley_w = label.chars().count() as f32 * 9.0 + 56.0;
                        let free = (w - galley_w).max(0.0);
                        ui.add_space(match align {
                            MailAlign::Left => pad,
                            MailAlign::Center => free / 2.0,
                            MailAlign::Right => free - pad,
                        });
                        egui::Frame::new()
                            .fill(to_color(background))
                            .corner_radius(CornerRadius::same((*radius).min(40) as u8))
                            .inner_margin(egui::Margin::symmetric(28, 11))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(label)
                                        .strong()
                                        .size(15.0)
                                        .color(to_color(color)),
                                );
                            });
                    });
                },
            );
            ui.add_space(12.0);
        }
        MailBlock::Image {
            alt, width, align, ..
        } => {
            ui.add_space(6.0);
            let iw = width.map_or(w - 2.0 * pad, |x| (x as f32).min(w - 2.0 * pad));
            ui.horizontal(|ui| {
                let free = (w - iw).max(0.0);
                ui.add_space(match align {
                    MailAlign::Left => pad,
                    MailAlign::Center => free / 2.0,
                    MailAlign::Right => free - pad,
                });
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(iw, iw.min(120.0) * 0.6 + 20.0),
                    egui::Sense::hover(),
                );
                ui.painter().rect_filled(rect, 2.0, Color32::from_gray(225));
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("image · {}", shown(alt)),
                    egui::FontId::proportional(12.0),
                    Color32::from_gray(110),
                );
            });
            ui.add_space(6.0);
        }
        MailBlock::Divider { color, thickness } => {
            ui.add_space(12.0);
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(w, *thickness as f32), egui::Sense::hover());
            ui.painter().rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(rect.left() + pad, rect.top()),
                    egui::pos2(rect.right() - pad, rect.bottom()),
                ),
                0.0,
                to_color(color),
            );
            ui.add_space(12.0);
        }
        MailBlock::Spacer { height } => ui.add_space(*height as f32),
        MailBlock::Columns { columns } => {
            let n = columns.len().max(1);
            let cw = w / n as f32;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for c in columns {
                    ui.allocate_ui_with_layout(
                        egui::vec2(cw, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(cw);
                            for inner in &c.blocks {
                                draw_block(ui, inner, fg, family, cw);
                            }
                        },
                    );
                }
            });
        }
    }
}

/// Writes the derived HTML, variables replaced by `[name]`, to a file and opens it in the browser.
fn open_in_browser(ui: &Ui, d: &MailDesign) {
    let mut html = maildesign::render_html(d);
    let mut names = maildesign::variables(&html);
    names.sort();
    names.dedup();
    for v in names {
        html = html.replace(&format!("{{{{ {v} }}}}"), &format!("[{v}]"));
    }
    let path = std::env::temp_dir().join("service-builder-mail-preview.html");
    if std::fs::write(&path, html).is_ok() {
        ui.ctx()
            .open_url(egui::OpenUrl::new_tab(format!("file://{}", path.display())));
    }
}
