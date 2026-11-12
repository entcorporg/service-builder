//! A small modal form: labelled inputs, an error line, OK / Cancel.

use std::rc::Rc;

use gtk::prelude::*;

use super::ui::Shared;

pub enum Kind {
    Text(String),
    /// Several lines of text, in a monospace font.
    Multiline(String),
    /// Read-only: shown so that the user knows what is being edited.
    Fixed(String),
    Number {
        value: i64,
        min: i64,
        max: i64,
    },
    Choice {
        options: Vec<String>,
        selected: Option<String>,
    },
    Check(bool),
    Checks(Vec<(String, bool)>),
}

pub struct Input {
    pub label: String,
    pub kind: Kind,
    /// Shown as a tooltip on the label.
    pub help: String,
}

impl Input {
    pub fn new(label: impl Into<String>, kind: Kind) -> Self {
        Self {
            label: label.into(),
            kind,
            help: String::new(),
        }
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = help.into();
        self
    }
}

pub enum Value {
    Text(String),
    Number(i64),
    Choice(String),
    Check(bool),
    Checks(Vec<String>),
}

impl Value {
    pub fn text(self) -> String {
        match self {
            Value::Text(t) | Value::Choice(t) => t,
            _ => String::new(),
        }
    }
    pub fn number(self) -> i64 {
        match self {
            Value::Number(n) => n,
            _ => 0,
        }
    }
    /// Any value as the text it stands for (a checkbox is `true` or `false`).
    pub fn into_string(self) -> String {
        match self {
            Value::Text(t) | Value::Choice(t) => t,
            Value::Number(n) => n.to_string(),
            Value::Check(b) => b.to_string(),
            Value::Checks(c) => c.join(", "),
        }
    }
    pub fn check(self) -> bool {
        matches!(self, Value::Check(true))
    }
    pub fn checks(self) -> Vec<String> {
        match self {
            Value::Checks(c) => c,
            _ => vec![],
        }
    }
}

type Reader = Box<dyn Fn() -> Value>;

fn input_widget(kind: Kind) -> (gtk::Widget, Reader) {
    match kind {
        Kind::Text(initial) => {
            let e = gtk::Entry::builder()
                .text(initial)
                .width_chars(34)
                .activates_default(true)
                .build();
            (
                e.clone().upcast(),
                Box::new(move || Value::Text(e.text().trim().to_string())),
            )
        }
        Kind::Multiline(initial) => {
            let view = gtk::TextView::builder()
                .monospace(true)
                .wrap_mode(gtk::WrapMode::WordChar)
                .top_margin(4)
                .bottom_margin(4)
                .left_margin(6)
                .right_margin(6)
                .build();
            view.buffer().set_text(&initial);
            let frame = gtk::ScrolledWindow::builder()
                .child(&view)
                .min_content_height(110)
                .min_content_width(420)
                .has_frame(true)
                .build();
            (
                frame.upcast(),
                Box::new(move || {
                    let b = view.buffer();
                    Value::Text(b.text(&b.start_iter(), &b.end_iter(), false).to_string())
                }),
            )
        }
        Kind::Fixed(text) => {
            let l = gtk::Label::builder().label(&text).xalign(0.0).build();
            l.add_css_class("dim-label");
            (l.upcast(), Box::new(move || Value::Text(text.clone())))
        }
        Kind::Number { value, min, max } => {
            let s = gtk::SpinButton::with_range(min as f64, max as f64, 1.0);
            s.set_value(value as f64);
            (
                s.clone().upcast(),
                Box::new(move || Value::Number(s.value_as_int() as i64)),
            )
        }
        Kind::Choice { options, selected } => {
            let strs: Vec<&str> = options.iter().map(String::as_str).collect();
            let d = gtk::DropDown::from_strings(&strs);
            if let Some(i) = selected.and_then(|s| options.iter().position(|o| *o == s)) {
                d.set_selected(i as u32);
            }
            (
                d.clone().upcast(),
                Box::new(move || {
                    Value::Choice(
                        options
                            .get(d.selected() as usize)
                            .cloned()
                            .unwrap_or_default(),
                    )
                }),
            )
        }
        Kind::Check(on) => {
            let c = gtk::CheckButton::new();
            c.set_active(on);
            (
                c.clone().upcast(),
                Box::new(move || Value::Check(c.is_active())),
            )
        }
        Kind::Checks(items) => {
            let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
            let boxes: Vec<(String, gtk::CheckButton)> = items
                .into_iter()
                .map(|(name, on)| {
                    let c = gtk::CheckButton::with_label(&name);
                    c.set_active(on);
                    column.append(&c);
                    (name, c)
                })
                .collect();
            (
                column.upcast(),
                Box::new(move || {
                    Value::Checks(
                        boxes
                            .iter()
                            .filter(|(_, c)| c.is_active())
                            .map(|(n, _)| n.clone())
                            .collect(),
                    )
                }),
            )
        }
    }
}

/// Shows the form. `on_ok` gets the values in the order of `inputs`; an `Err` is shown in the form, which stays open.
pub fn ask(
    ui: &Shared,
    title: &str,
    inputs: Vec<Input>,
    on_ok: impl Fn(&Shared, Vec<Value>) -> Result<(), String> + 'static,
) {
    let dialog = gtk::Window::builder()
        .title(title)
        .transient_for(&ui.window)
        .modal(true)
        .resizable(false)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(16);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);
    let grid = gtk::Grid::builder()
        .row_spacing(10)
        .column_spacing(12)
        .build();
    let mut readers: Vec<Reader> = vec![];
    for (row, input) in inputs.into_iter().enumerate() {
        let l = gtk::Label::builder()
            .label(&input.label)
            .tooltip_text(&input.help)
            .xalign(0.0)
            .yalign(0.0)
            .build();
        let (widget, reader) = input_widget(input.kind);
        grid.attach(&l, 0, row as i32, 1, 1);
        grid.attach(&widget, 1, row as i32, 1, 1);
        readers.push(reader);
    }
    let error = gtk::Label::builder().xalign(0.0).wrap(true).build();
    error.add_css_class("error");
    let ok = gtk::Button::with_label("Valider");
    ok.add_css_class("suggested-action");
    let cancel = gtk::Button::with_label("Annuler");
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.append(&cancel);
    buttons.append(&ok);
    content.append(&grid);
    content.append(&error);
    content.append(&buttons);
    dialog.set_child(Some(&content));
    dialog.set_default_widget(Some(&ok));

    let on_ok = Rc::new(on_ok);
    let ui = ui.clone();
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| dialog.close());
    }
    {
        let dialog = dialog.clone();
        ok.connect_clicked(move |_| {
            let values = readers.iter().map(|r| r()).collect();
            match on_ok(&ui, values) {
                Ok(()) => dialog.close(),
                Err(e) => error.set_text(&e),
            }
        });
    }
    dialog.present();
}
