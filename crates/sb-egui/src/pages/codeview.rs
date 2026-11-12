//! The generated code: a tree of the files, the path of the one that is open, and its text with colours.

use std::collections::{BTreeMap, HashSet};
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontId, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2,
    text::{LayoutJob, TextFormat},
};
use sb_app::Analysis;

use crate::fonts::Weight;
use crate::app::App;
use crate::theme::{self, BORDER, CODE_BG, FAINT, MUTED, PANEL, RAISED, SERVICE, TEXT, TOPIC};
use crate::{icons, widgets};

/// The highlighted text of the open file, kept between frames.
struct Shown {
    path: String,
    /// Hash of the content, to notice that a new analysis did not change the file.
    hash: u64,
    ppp: f32,
    analysis: Rc<Analysis>,
    galley: Arc<egui::Galley>,
}

#[derive(Default)]
pub struct CodeState {
    /// Path of the open file; the default one (`flows/rpc.rs`, else the first) while `None` or gone.
    current: Option<String>,
    /// The folders that are closed, by path.
    closed: HashSet<String>,
    shown: Option<Shown>,
}

#[derive(Default)]
struct Dir {
    dirs: BTreeMap<String, Dir>,
    /// (name, index in the analysis)
    files: Vec<(String, usize)>,
}

fn insert(dir: &mut Dir, parts: &[&str], index: usize) {
    match parts {
        [name] => dir.files.push(((*name).to_string(), index)),
        [first, rest @ ..] => insert(
            dir.dirs.entry((*first).to_string()).or_default(),
            rest,
            index,
        ),
        [] => {}
    }
}

fn file_color(name: &str) -> Color32 {
    match name.rsplit('.').next() {
        Some("rs") => theme::hex(0xff9a6b),
        Some("proto") => theme::hex(0x8fb0ff),
        Some("toml" | "yml" | "yaml") => theme::hex(0x9be3b4),
        _ => MUTED,
    }
}

/// One row of the tree; true when clicked.
#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut Ui,
    level: usize,
    chevron: Option<&str>,
    icon_name: &str,
    icon_color: Color32,
    name: &str,
    text_color: Color32,
    active: bool,
) -> bool {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::click());
    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, CornerRadius::same(6), theme::alpha(SERVICE, 46));
    } else if r.hovered() {
        painter.rect_filled(rect, CornerRadius::same(6), RAISED);
    }
    let mut x = rect.left() + 4.0 + level as f32 * 14.0;
    if let Some(c) = chevron {
        icons::paint(
            ui,
            c,
            egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 6.0), Vec2::splat(12.0)),
            FAINT,
        );
    }
    x += 18.0;
    icons::paint(
        ui,
        icon_name,
        egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 7.0), Vec2::splat(14.0)),
        icon_color,
    );
    x += 20.0;
    let color = if active {
        Color32::WHITE
    } else if r.hovered() {
        TEXT
    } else {
        text_color
    };
    ui.painter().text(
        egui::pos2(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        theme::mono(theme::SM),
        color,
    );
    r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

#[allow(clippy::too_many_arguments)]
fn draw_dir(
    ui: &mut Ui,
    state: &mut CodeState,
    paths: &[&str],
    dir: &Dir,
    prefix: &str,
    level: usize,
    current: Option<&str>,
    picked: &mut Option<String>,
) {
    for (name, d) in &dir.dirs {
        let path = format!("{prefix}{name}/");
        let open = !state.closed.contains(&path);
        if row(
            ui,
            level,
            Some(if open { "down" } else { "right" }),
            if open { "folderOpen" } else { "folder" },
            TOPIC,
            name,
            TEXT,
            false,
        ) && !state.closed.remove(&path)
        {
            state.closed.insert(path.clone());
        }
        if open {
            draw_dir(ui, state, paths, d, &path, level + 1, current, picked);
        }
    }
    // Alphabetical, as the web version (`localeCompare`): the generator's own order is not meant for reading.
    let mut sorted: Vec<&(String, usize)> = dir.files.iter().collect();
    sorted.sort_by_key(|(name, _)| name.to_lowercase());
    for (name, index) in sorted {
        let path = paths[*index];
        let active = current == Some(path);
        if row(
            ui,
            level,
            None,
            "file",
            file_color(name),
            name,
            MUTED,
            active,
        ) {
            *picked = Some(path.to_string());
        }
    }
}

pub fn page(app: &mut App, ui: &mut Ui) {
    let analysis = app.analysis();
    let files = &analysis.files;

    if files.is_empty() {
        let errors = analysis
            .diagnostics
            .iter()
            .any(|d| d.severity == sb_model::Severity::Error);
        Frame::new().fill(CODE_BG).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            if errors {
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.label(widgets::muted(
                        "Corrigez les erreurs du projet pour générer le code.",
                    ));
                });
            }
        });
        return;
    }

    // The open file: the one that was, when it still exists.
    let index = app
        .code
        .current
        .as_ref()
        .and_then(|c| files.iter().position(|f| &f.path == c))
        .or_else(|| files.iter().position(|f| f.path.ends_with("flows/rpc.rs")))
        .unwrap_or(0);
    let file = &files[index];
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let mut picked: Option<String> = None;
    let mut copied = false;

    let mut root = Dir::default();
    for (i, f) in files.iter().enumerate() {
        let parts: Vec<&str> = f.path.split('/').collect();
        insert(&mut root, &parts, i);
    }

    egui::Panel::left("code-tree")
        .exact_size(236.0)
        .resizable(false)
        .frame(
            Frame::new()
                .fill(PANEL)
                .stroke(Stroke::NONE)
                .inner_margin(Margin::symmetric(6, 10)),
        )
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.horizontal(|ui| {
                ui.add_space(2.0);
                ui.label(
                    RichText::new("FICHIERS")
                        .size(theme::XS)
                        .color(FAINT)
                        .semibold(),
                );
            });
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw_dir(
                        ui,
                        &mut app.code,
                        &paths,
                        &root,
                        "",
                        0,
                        Some(&file.path),
                        &mut picked,
                    );
                });
        });

    egui::CentralPanel::no_frame().show(ui, |ui| {
        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, CODE_BG);
        ui.painter()
            .vline(full.left(), full.y_range(), Stroke::new(1.0, BORDER));
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        Frame::new()
            .inner_margin(Margin::symmetric(14, 7))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&file.path)
                            .font(theme::mono(theme::SM))
                            .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::ghost(ui, "copy", "Copier", "").clicked() {
                            copied = true;
                        }
                    });
                });
            });
        ui.painter()
            .hline(full.x_range(), ui.cursor().top(), Stroke::new(1.0, BORDER));

        let galley = galley_of(ui, &mut app.code, &analysis, index);
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let size = galley.size() + Vec2::new(28.0, 20.0);
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                if ui.is_rect_visible(rect) {
                    ui.painter()
                        .galley(rect.min + Vec2::new(14.0, 10.0), galley, CODE_TEXT);
                }
            });
    });

    if copied {
        ui.ctx().copy_text(file.content.clone());
        app.say("Copié");
    }
    if let Some(p) = picked {
        app.code.current = Some(p);
    } else if app.code.current.as_deref() != Some(file.path.as_str()) {
        app.code.current = Some(file.path.clone());
    }
}

/// The laid out text of the file; made again only when the file, its content or the pixel ratio changed.
fn galley_of(
    ui: &Ui,
    state: &mut CodeState,
    analysis: &Rc<Analysis>,
    index: usize,
) -> Arc<egui::Galley> {
    let file = &analysis.files[index];
    let ppp = ui.ctx().pixels_per_point();
    if let Some(s) = &mut state.shown
        && s.path == file.path
        && s.ppp == ppp
    {
        if Rc::ptr_eq(&s.analysis, analysis) {
            return s.galley.clone();
        }
        let hash = hash_of(&file.content);
        if s.hash == hash {
            s.analysis = analysis.clone();
            return s.galley.clone();
        }
    }
    let galley = ui.fonts_mut(|f| f.layout_job(highlight(&file.content, &file.path)));
    state.shown = Some(Shown {
        path: file.path.clone(),
        hash: hash_of(&file.content),
        ppp,
        analysis: analysis.clone(),
        galley: galley.clone(),
    });
    galley
}

fn hash_of(text: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

// ---- colours ----

const CODE_TEXT: Color32 = theme::hex(0xd6d9ee);

#[derive(Clone, Copy)]
enum Tag {
    Kw,
    Str,
    Num,
    Com,
    Fun,
    Ty,
    Meta,
}

impl Tag {
    fn color(self) -> Color32 {
        theme::hex(match self {
            Tag::Kw => 0xff8fb8,
            Tag::Str => 0x9be3b4,
            Tag::Num => 0xffb04a,
            Tag::Com => 0x667099,
            Tag::Fun => 0x8fb0ff,
            Tag::Ty => 0xe6c07b,
            Tag::Meta => 0xc7a0ff,
        })
    }
}

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "message", "service", "rpc", "returns", "syntax", "package",
    "import", "option", "repeated", "optional", "oneof", "map",
];

fn format(font: &FontId, color: Color32, italic: bool) -> TextFormat {
    TextFormat {
        font_id: font.clone(),
        color,
        italics: italic,
        line_height: Some(20.0),
        ..Default::default()
    }
}

/// The text as one `LayoutJob`, with the colours of the GTK version's tags.
fn highlight(text: &str, path: &str) -> LayoutJob {
    let rust = path.ends_with(".rs") || path.ends_with(".proto");
    let hash_comments = path.ends_with(".toml")
        || path.ends_with(".yml")
        || path.ends_with(".yaml")
        || path.ends_with(".gitignore");
    // (byte offset, char): the spans are cut on char boundaries.
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let at = |i: usize| chars.get(i).map_or(text.len(), |c| c.0);
    let ch = |i: usize| chars.get(i).map(|c| c.1);
    let mut spans: Vec<(Tag, usize, usize)> = vec![];
    let mut i = 0;
    let mut previous_word = String::new();
    while i < chars.len() {
        let c = chars[i].1;
        let slashes = rust && c == '/' && ch(i + 1) == Some('/');
        if slashes || (hash_comments && c == '#') {
            let start = i;
            while i < chars.len() && chars[i].1 != '\n' {
                i += 1;
            }
            spans.push((Tag::Com, at(start), at(i)));
        } else if c == '"' {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i].1 != '"' && chars[i].1 != '\n' {
                if chars[i].1 == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(chars.len());
            spans.push((Tag::Str, at(start), at(i)));
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len()
                && (chars[i].1.is_ascii_alphanumeric() || chars[i].1 == '.' || chars[i].1 == '_')
            {
                i += 1;
            }
            spans.push((Tag::Num, at(start), at(i)));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].1.is_alphanumeric() || chars[i].1 == '_') {
                i += 1;
            }
            let word = &text[at(start)..at(i)];
            if rust && KEYWORDS.contains(&word) {
                spans.push((Tag::Kw, at(start), at(i)));
            } else if rust && previous_word == "fn" {
                spans.push((Tag::Fun, at(start), at(i)));
            } else if rust && ch(i) == Some('!') {
                spans.push((Tag::Meta, at(start), at(i + 1)));
            } else if rust && word.chars().next().is_some_and(char::is_uppercase) {
                spans.push((Tag::Ty, at(start), at(i)));
            }
            previous_word = word.to_string();
        } else {
            i += 1;
        }
    }

    let font = theme::mono(12.5);
    let mut job = LayoutJob::default();
    let mut pos = 0;
    for (tag, a, b) in spans {
        if a < pos || b <= a {
            continue;
        }
        if a > pos {
            job.append(&text[pos..a], 0.0, format(&font, CODE_TEXT, false));
        }
        job.append(
            &text[a..b],
            0.0,
            format(&font, tag.color(), matches!(tag, Tag::Com)),
        );
        pos = b;
    }
    if pos < text.len() {
        job.append(&text[pos..], 0.0, format(&font, CODE_TEXT, false));
    }
    job
}
