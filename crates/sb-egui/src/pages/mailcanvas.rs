//! The visual editor of a mail ("canvas"): the blocks on the left, a preview in the middle, the properties of the selected
//! block and the settings of the mail on the right. Nothing is typed as HTML: the HTML and the text are derived from the
//! blocks (`sb_model::maildesign`) and only shown.
//!
//! The editor works on a copy of the design; "Enregistrer" writes it to the project in one undoable change.
//!
//! The preview is drawn with egui, not by a mail client: it follows the structure, the colours, the alignment, the
//! relative sizes and the width of the mail, with the variables shown as `‹name›`. It does not load images (a grey box
//! with the alt text stands for them), has no serif font, and does not know what Outlook or Gmail would do to the HTML. The
//! "Ouvrir dans le navigateur" button writes the derived HTML to a file and opens it, which is the nearest to a mail client
//! the desktop has.

use eframe::egui::{self, Align, Color32, CornerRadius, Layout, RichText, Ui};
use sb_model::Project;
use sb_model::maildesign::{
    self, MailAlign, MailBlock, MailColumn, MailDesign, MailFont, MailIssue, parse_color,
};

use crate::app::App;
use crate::theme::*;
use crate::widgets::{faint, muted};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Preview,
    Html,
    Text,
}

/// The editor open on one mail. `path` addresses the selected block: `[i]`, or `[i, column, k]` inside columns.
pub struct MailEditor {
    si: usize,
    topic: String,
    design: MailDesign,
    saved: MailDesign,
    fields: Vec<String>,
    path: Option<Vec<usize>>,
    tab: Tab,
}

/// Opens the editor on the mail of `topic` in service `si`. A mail written by hand becomes a design first: the author is
/// asked, since its HTML and text are replaced by what the blocks derive.
pub fn open(app: &mut App, si: usize, topic: &str) {
    let p = app.project();
    let Some(t) = p.services[si]
        .templates
        .iter()
        .find(|t| t.event_type == topic)
    else {
        return;
    };
    let topic = topic.to_string();
    if t.design.is_some() {
        return start(app, si, &topic);
    }
    app.confirm(
        "Composer ce mail avec des blocs ?",
        "Le HTML et le texte écrits à la main seront remplacés par ceux que dérivent les blocs. Annuler (Ctrl+Z) les rend.",
        vec![
            ("Annuler".into(), false, None),
            (
                "Composer avec des blocs".into(),
                true,
                Some(Box::new(move |app: &mut App| {
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
                    start(app, si, &topic);
                })),
            ),
        ],
    );
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

fn start(app: &mut App, si: usize, topic: &str) {
    let p = app.project();
    let Some(design) = p.services[si]
        .templates
        .iter()
        .find(|t| t.event_type == topic)
        .and_then(|t| t.design.clone())
    else {
        return;
    };
    app.mail_editor = Some(MailEditor {
        si,
        topic: topic.into(),
        saved: design.clone(),
        design,
        fields: event_fields(&p, topic),
        path: None,
        tab: Tab::Preview,
    });
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

/// Draws the editor when one is open (called once per frame).
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut ed) = app.mail_editor.take() else {
        return;
    };
    let mut open = true;
    let mut action = None;
    egui::Window::new(format!("Éditeur visuel du mail — {}", ed.topic))
        .open(&mut open)
        .collapsible(false)
        .default_size([1180.0, 720.0])
        .show(ctx, |ui| action = body(ui, &mut ed));
    match action {
        Some(Action::Save) => {
            save(app, &mut ed);
        }
        Some(Action::SaveAndClose) => {
            save(app, &mut ed);
            open = false;
        }
        Some(Action::Handwritten) => {
            to_handwritten(app, &ed);
            return;
        }
        None => {}
    }
    if !open && ed.design != ed.saved {
        // Closing with the cross keeps nothing: say so rather than lose the work silently.
        app.say("Modifications du mail abandonnées");
    }
    if open {
        app.mail_editor = Some(ed);
    }
}

enum Action {
    Save,
    SaveAndClose,
    Handwritten,
}

fn save(app: &mut App, ed: &mut MailEditor) {
    let (si, topic, design) = (ed.si, ed.topic.clone(), ed.design.clone());
    app.edit(|p| {
        if let Some(t) = p.services[si]
            .templates
            .iter_mut()
            .find(|t| t.event_type == topic)
        {
            t.design = Some(design);
        }
    });
    ed.saved = ed.design.clone();
    app.say("Mail enregistré");
}

/// Gives the mail back to the hand: the derived HTML and text become the ones of the template, which has no design any more.
fn to_handwritten(app: &mut App, ed: &MailEditor) {
    let (si, topic) = (ed.si, ed.topic.clone());
    let design = ed.design.clone();
    app.edit(|p| {
        if let Some(t) = p.services[si]
            .templates
            .iter_mut()
            .find(|t| t.event_type == topic)
        {
            t.html = maildesign::render_html(&design);
            t.text = maildesign::render_text(&design);
            t.design = None;
        }
    });
    app.say("Le mail est maintenant écrit à la main (HTML et texte repris des blocs)");
}

fn body(ui: &mut Ui, ed: &mut MailEditor) -> Option<Action> {
    let mut action = None;
    let issues = maildesign::check(
        &ed.design,
        &ed.fields.iter().map(String::as_str).collect::<Vec<_>>(),
        "",
    );
    ui.horizontal(|ui| {
        let dirty = ed.design != ed.saved;
        if ui
            .add_enabled(dirty, egui::Button::new("Enregistrer"))
            .clicked()
        {
            action = Some(Action::Save);
        }
        if ui.button("Enregistrer et fermer").clicked() {
            action = Some(Action::SaveAndClose);
        }
        if ui
            .button("Passer à l'écriture à la main")
            .on_hover_text(
                "Reprend le HTML et le texte dérivés des blocs comme gabarit écrit à la main.",
            )
            .clicked()
        {
            action = Some(Action::Handwritten);
        }
        if dirty {
            ui.label(RichText::new("modifié").color(WARN));
        }
        let errors = issues.iter().filter(|i| i.error).count();
        if errors > 0 {
            ui.label(RichText::new(format!("{errors} erreur(s)")).color(ERR));
        }
    });
    ui.separator();
    egui_extras::StripBuilder::new(ui)
        .size(egui_extras::Size::exact(270.0))
        .size(egui_extras::Size::remainder().at_least(320.0))
        .size(egui_extras::Size::exact(320.0))
        .horizontal(|mut strip| {
            strip.cell(|ui| {
                ui.label(RichText::new("Blocs").strong());
                egui::ScrollArea::vertical()
                    .id_salt("mail-blocks")
                    .show(ui, |ui| {
                        let mut path = ed.path.clone();
                        blocks_list(ui, &mut ed.design.blocks, &[], &mut path, false);
                        ed.path = path;
                    });
            });
            strip.cell(|ui| {
                ui.horizontal(|ui| {
                    for (tab, label) in [
                        (Tab::Preview, "Aperçu"),
                        (Tab::Html, "HTML dérivé"),
                        (Tab::Text, "Texte dérivé"),
                    ] {
                        ui.selectable_value(&mut ed.tab, tab, label);
                    }
                    if ui
                        .button("Ouvrir dans le navigateur")
                        .on_hover_text("Écrit le HTML dérivé dans un fichier (variables remplacées par [nom]) et l'ouvre.")
                        .clicked()
                    {
                        open_in_browser(ui, &ed.design);
                    }
                });
                egui::ScrollArea::both().id_salt("mail-center").show(ui, |ui| match ed.tab {
                    Tab::Preview => preview(ui, &ed.design),
                    Tab::Html => {
                        ui.add(
                            egui::TextEdit::multiline(&mut maildesign::render_html(&ed.design).as_str())
                                .font(mono(SM))
                                .desired_width(f32::INFINITY),
                        );
                    }
                    Tab::Text => {
                        ui.add(
                            egui::TextEdit::multiline(&mut maildesign::render_text(&ed.design).as_str())
                                .font(mono(SM))
                                .desired_width(f32::INFINITY),
                        );
                    }
                });
            });
            strip.cell(|ui| {
                egui::ScrollArea::vertical().id_salt("mail-props").show(ui, |ui| {
                    properties(ui, ed);
                    problems(ui, &issues);
                });
            });
        });
    action
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
            let text = format!("{} {}", kind_name(b), summary(b));
            let is = selected.as_ref() == Some(&here);
            if ui.selectable_label(is, text).clicked() {
                *selected = Some(here.clone());
            }
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
    ui.menu_button("＋ Ajouter un bloc", |ui| {
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

fn properties(ui: &mut Ui, ed: &mut MailEditor) {
    ui.label(RichText::new("Bloc sélectionné").strong());
    let fields = ed.fields.clone();
    match ed
        .path
        .clone()
        .and_then(|p| block_at(&mut ed.design.blocks, &p))
    {
        None => {
            ui.label(muted("Choisissez un bloc dans la liste."));
        }
        Some(b) => block_props(ui, b, &fields),
    }
    ui.add_space(12.0);
    egui::CollapsingHeader::new(RichText::new("Réglages du mail").strong())
        .default_open(true)
        .show(ui, |ui| settings(ui, &mut ed.design, &fields));
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
