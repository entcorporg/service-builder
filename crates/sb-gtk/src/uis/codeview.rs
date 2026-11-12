//! The generated code: a tree of the files, the path of the one that is open, and its text with colours.

use std::collections::BTreeMap;

use gtk::glib::clone;
use gtk::prelude::*;

use super::icons::{self, icon};
use super::ui::Shared;
use super::widgets::{clear, say};

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

fn row_button(level: i32) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("tree-row");
    b.set_margin_start(level * 14);
    b
}

fn file_color(name: &str) -> &'static str {
    match name.rsplit('.').next() {
        Some("rs") => "#ff9a6b",
        Some("proto") => "#8fb0ff",
        Some("toml" | "yml" | "yaml") => "#9be3b4",
        _ => icons::MUTED,
    }
}

fn draw_dir(ui: &Shared, host: &gtk::Box, dir: &Dir, level: i32, current: Option<&str>) {
    let files = &ui.state.borrow().analysis.files;
    for (name, d) in &dir.dirs {
        let b = row_button(level);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let chevron = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        chevron.append(&icon("down", 12, icons::FAINT));
        line.append(&chevron);
        line.append(&icon("folderOpen", 14, icons::TOPIC));
        let l = gtk::Label::new(Some(name));
        l.add_css_class("font-mono");
        line.append(&l);
        b.set_child(Some(&line));
        b.add_css_class("dir");
        host.append(&b);
        let children = gtk::Box::new(gtk::Orientation::Vertical, 0);
        draw_dir(ui, &children, d, level + 1, current);
        let revealer = gtk::Revealer::new();
        revealer.set_child(Some(&children));
        revealer.set_reveal_child(true);
        host.append(&revealer);
        b.connect_clicked(move |_| {
            let open = !revealer.reveals_child();
            revealer.set_reveal_child(open);
            if let Some(old) = chevron.first_child() {
                chevron.remove(&old);
            }
            chevron.append(&icon(if open { "down" } else { "right" }, 12, icons::FAINT));
        });
    }
    // Alphabetical, as the web version (`localeCompare`): the generator's own order is not meant for reading.
    let mut sorted: Vec<&(String, usize)> = dir.files.iter().collect();
    sorted.sort_by_key(|(name, _)| name.to_lowercase());
    for (name, index) in sorted {
        let b = row_button(level);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_width_request(12);
        line.append(&spacer);
        line.append(&icon("file", 14, file_color(name)));
        let l = gtk::Label::new(Some(name));
        l.add_css_class("font-mono");
        line.append(&l);
        b.set_child(Some(&line));
        b.add_css_class("leaf");
        if current == files.get(*index).map(|f| f.path.as_str()) {
            b.add_css_class("active");
        }
        let index = *index;
        b.connect_clicked(clone!(
            #[strong]
            ui,
            move |_| show_file(&ui, Some(index))
        ));
        host.append(&b);
    }
}

/// Rebuilds the tree from the last analysis, keeping the file that was open when it still exists.
pub fn draw(ui: &Shared) {
    clear(&ui.files);
    let (paths, errors, current): (Vec<String>, bool, Option<String>) = {
        let s = ui.state.borrow();
        (
            s.analysis.files.iter().map(|f| f.path.clone()).collect(),
            s.analysis
                .diagnostics
                .iter()
                .any(|d| d.severity == sb_model::Severity::Error),
            ui.current_file.borrow().clone(),
        )
    };
    if paths.is_empty() {
        ui.code_path.set_text("");
        ui.code.buffer().set_text(if errors {
            "Corrigez les erreurs du projet pour générer le code."
        } else {
            ""
        });
        return;
    }
    let mut root = Dir::default();
    for (i, p) in paths.iter().enumerate() {
        let parts: Vec<&str> = p.split('/').collect();
        insert(&mut root, &parts, i);
    }
    let index = current
        .as_ref()
        .and_then(|c| paths.iter().position(|p| p == c))
        .or_else(|| paths.iter().position(|p| p.ends_with("flows/rpc.rs")))
        .unwrap_or(0);
    let open = paths.get(index).cloned();
    let title = gtk::Label::builder().label("FICHIERS").xalign(0.0).build();
    title.add_css_class("palette-title");
    title.set_margin_start(8);
    ui.files.append(&title);
    draw_dir(ui, &ui.files, &root, 0, open.as_deref());
    show_file(ui, Some(index));
}

pub fn show_file(ui: &Shared, index: Option<usize>) {
    let file = {
        let s = ui.state.borrow();
        index.and_then(|i| s.analysis.files.get(i)).cloned()
    };
    let Some(file) = file else {
        return;
    };
    let changed = ui.current_file.borrow().as_deref() != Some(file.path.as_str());
    *ui.current_file.borrow_mut() = Some(file.path.clone());
    ui.code_path.set_text(&file.path);
    let buffer = ui.code.buffer();
    buffer.set_text(&file.content);
    highlight(&buffer, &file.path);
    if changed {
        // The highlighted row follows the click: the tree is drawn again with the new current file.
        let (files, host) = (ui.state.borrow().analysis.files.len(), ui.files.clone());
        if files > 0 && host.first_child().is_some() {
            clear(&host);
            let mut root = Dir::default();
            let s = ui.state.borrow();
            for (i, f) in s.analysis.files.iter().enumerate() {
                let parts: Vec<&str> = f.path.split('/').collect();
                insert(&mut root, &parts, i);
            }
            drop(s);
            let title = gtk::Label::builder().label("FICHIERS").xalign(0.0).build();
            title.add_css_class("palette-title");
            title.set_margin_start(8);
            host.append(&title);
            draw_dir(ui, &host, &root, 0, Some(&file.path));
        }
    }
}

pub fn copy(ui: &Shared) {
    let buffer = ui.code.buffer();
    let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
    ui.window.clipboard().set_text(&text);
    say(ui, "Copié");
}

// ---- colours ----

fn tags(buffer: &gtk::TextBuffer) {
    let table = buffer.tag_table();
    for (name, color, italic) in [
        ("kw", "#ff8fb8", false),
        ("str", "#9be3b4", false),
        ("num", "#ffb04a", false),
        ("com", "#667099", true),
        ("fun", "#8fb0ff", false),
        ("ty", "#e6c07b", false),
        ("meta", "#c7a0ff", false),
    ] {
        if table.lookup(name).is_none() {
            let t = gtk::TextTag::builder().name(name).foreground(color).build();
            if italic {
                t.set_style(gtk::pango::Style::Italic);
            }
            table.add(&t);
        }
    }
}

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "message", "service", "rpc", "returns", "syntax", "package",
    "import", "option", "repeated", "optional", "oneof", "map",
];

fn highlight(buffer: &gtk::TextBuffer, path: &str) {
    tags(buffer);
    let text = buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string();
    let rust = path.ends_with(".rs") || path.ends_with(".proto");
    let hash_comments = path.ends_with(".toml")
        || path.ends_with(".yml")
        || path.ends_with(".yaml")
        || path.ends_with(".gitignore")
        || path.ends_with(".md") && false;
    // Offsets in characters, as GTK wants them.
    let chars: Vec<char> = text.chars().collect();
    let mut spans: Vec<(&str, usize, usize)> = vec![];
    let mut i = 0;
    let mut previous_word = String::new();
    while i < chars.len() {
        let c = chars[i];
        let slashes = rust && c == '/' && chars.get(i + 1) == Some(&'/');
        if slashes || (hash_comments && c == '#') {
            let start = i;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            spans.push(("com", start, i));
        } else if c == '"' {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != '"' && chars[i] != '\n' {
                if chars[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(chars.len());
            spans.push(("str", start, i));
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '.' || chars[i] == '_')
            {
                i += 1;
            }
            spans.push(("num", start, i));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if rust && KEYWORDS.contains(&word.as_str()) {
                spans.push(("kw", start, i));
            } else if rust && previous_word == "fn" {
                spans.push(("fun", start, i));
            } else if rust && chars.get(i) == Some(&'!') {
                spans.push(("meta", start, i + 1));
            } else if rust && word.chars().next().is_some_and(char::is_uppercase) {
                spans.push(("ty", start, i));
            }
            previous_word = word;
        } else {
            i += 1;
        }
    }
    for (tag, a, b) in spans {
        buffer.apply_tag_by_name(
            tag,
            &buffer.iter_at_offset(a as i32),
            &buffer.iter_at_offset(b as i32),
        );
    }
}
