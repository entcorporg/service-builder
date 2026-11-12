//! A small modal form: labelled inputs, an error line, OK / Cancel. Same API as the GTK version's `form::ask`:
//! `ask(app, title, inputs, |app, values| …)`; the values come in the order of the inputs, and an `Err` is shown in
//! the form, which stays open. Also draws the confirmations queued by `App::confirm`.

use eframe::egui::{self, RichText};

use crate::app::App;
use crate::theme::*;

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

type OnOk = Box<dyn Fn(&mut App, Vec<Value>) -> Result<(), String>>;

pub struct Form {
    id: egui::Id,
    title: String,
    inputs: Vec<Input>,
    /// The text of the number inputs while they are typed.
    numbers: Vec<String>,
    error: String,
    on_ok: OnOk,
    first_frame: bool,
}

/// Shows the form. `on_ok` gets the values in the order of `inputs`; an `Err` is shown in the form, which stays open.
pub fn ask(
    app: &mut App,
    title: &str,
    inputs: Vec<Input>,
    on_ok: impl Fn(&mut App, Vec<Value>) -> Result<(), String> + 'static,
) {
    let numbers = inputs
        .iter()
        .map(|i| match &i.kind {
            Kind::Number { value, .. } => value.to_string(),
            _ => String::new(),
        })
        .collect();
    let id = egui::Id::unique(("form", app.forms.len(), title));
    app.forms.push(Form {
        id,
        title: title.into(),
        inputs,
        numbers,
        error: String::new(),
        on_ok: Box::new(on_ok),
        first_frame: true,
    });
}

impl Form {
    fn values(&self) -> Vec<Value> {
        self.inputs
            .iter()
            .zip(&self.numbers)
            .map(|(i, n)| match &i.kind {
                Kind::Text(t) => Value::Text(t.trim().to_string()),
                Kind::Multiline(t) | Kind::Fixed(t) => Value::Text(t.clone()),
                Kind::Number { value, min, max } => {
                    Value::Number(n.trim().parse::<i64>().unwrap_or(*value).clamp(*min, *max))
                }
                Kind::Choice { selected, options } => Value::Choice(
                    selected
                        .clone()
                        .or_else(|| options.first().cloned())
                        .unwrap_or_default(),
                ),
                Kind::Check(b) => Value::Check(*b),
                Kind::Checks(items) => Value::Checks(
                    items
                        .iter()
                        .filter(|(_, on)| *on)
                        .map(|(n, _)| n.clone())
                        .collect(),
                ),
            })
            .collect()
    }

    /// Draws the inputs; true when Enter was pressed in a one-line input.
    fn body(&mut self, ui: &mut egui::Ui) -> bool {
        let mut enter = false;
        let first = std::mem::take(&mut self.first_frame);
        egui::Grid::new(self.id.with("grid"))
            .num_columns(2)
            .spacing([12.0, 10.0])
            .show(ui, |ui| {
                for (row, (input, number)) in
                    self.inputs.iter_mut().zip(&mut self.numbers).enumerate()
                {
                    let l = ui.label(RichText::new(&input.label).color(MUTED));
                    if !input.help.is_empty() {
                        l.on_hover_text(&input.help);
                    }
                    let r = match &mut input.kind {
                        Kind::Text(t) => {
                            Some(ui.add(egui::TextEdit::singleline(t).desired_width(300.0)))
                        }
                        Kind::Multiline(t) => Some(
                            ui.add(
                                egui::TextEdit::multiline(t)
                                    .font(mono(SM))
                                    .desired_width(420.0)
                                    .desired_rows(6),
                            ),
                        ),
                        Kind::Fixed(t) => {
                            ui.label(RichText::new(t.as_str()).color(MUTED));
                            None
                        }
                        Kind::Number { .. } => {
                            Some(ui.add(egui::TextEdit::singleline(number).desired_width(100.0)))
                        }
                        Kind::Choice { options, selected } => {
                            let current = selected
                                .clone()
                                .or_else(|| options.first().cloned())
                                .unwrap_or_default();
                            egui::ComboBox::from_id_salt(self.id.with(row))
                                .selected_text(current.as_str())
                                .width(300.0)
                                .show_ui(ui, |ui| {
                                    for o in options.iter() {
                                        if ui.selectable_label(*o == current, o).clicked() {
                                            *selected = Some(o.clone());
                                        }
                                    }
                                });
                            None
                        }
                        Kind::Check(b) => {
                            ui.checkbox(b, "");
                            None
                        }
                        Kind::Checks(items) => {
                            ui.vertical(|ui| {
                                for (name, on) in items.iter_mut() {
                                    ui.checkbox(on, name.as_str());
                                }
                            });
                            None
                        }
                    };
                    if let Some(r) = r {
                        if first && row == 0 {
                            r.request_focus();
                        }
                        let single = !matches!(input.kind, Kind::Multiline(_));
                        if single && r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
                        {
                            enter = true;
                        }
                    }
                    ui.end_row();
                }
            });
        enter
    }
}

/// Draws the topmost form or confirmation, if any; called by the frame loop after the page.
pub fn show_modals(app: &mut App, ctx: &egui::Context) {
    if let Some(mut form) = app.forms.pop() {
        let mut close = false;
        let mut submit = false;
        let modal = egui::Modal::new(form.id).show(ctx, |ui| {
            ui.set_min_width(380.0);
            ui.label(RichText::new(&form.title).size(LG).strong());
            ui.add_space(8.0);
            submit |= form.body(ui);
            if !form.error.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new(&form.error).color(ERR));
            }
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::widgets::primary(ui, "", "Valider", true).clicked() {
                    submit = true;
                }
                if ui.button("Annuler").clicked() {
                    close = true;
                }
            });
        });
        if modal.should_close() && !submit {
            close = true;
        }
        if submit {
            let values = form.values();
            let on_ok = std::mem::replace(&mut form.on_ok, Box::new(|_, _| Ok(())));
            // The form is off the stack while `on_ok` runs: it may open another one.
            let depth = app.forms.len();
            match on_ok(app, values) {
                Ok(()) => close = true,
                Err(e) => form.error = e,
            }
            form.on_ok = on_ok;
            if !close {
                app.forms.insert(depth, form);
            }
        } else if !close {
            app.forms.push(form);
        }
        return;
    }

    if let Some(mut c) = app.confirms.pop() {
        let mut chosen: Option<usize> = None;
        let modal =
            egui::Modal::new(egui::Id::unique(("confirm", app.confirms.len()))).show(ctx, |ui| {
                ui.set_min_width(360.0);
                ui.set_max_width(520.0);
                ui.label(RichText::new(&c.title).size(LG).strong());
                if !c.detail.is_empty() {
                    ui.add_space(6.0);
                    ui.label(RichText::new(&c.detail).color(MUTED));
                }
                ui.add_space(12.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for (i, (label, primary, _)) in c.buttons.iter().enumerate().rev() {
                        let clicked = if *primary {
                            crate::widgets::primary(ui, "", label, true).clicked()
                        } else {
                            ui.button(label.as_str()).clicked()
                        };
                        if clicked {
                            chosen = Some(i);
                        }
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    chosen = c.buttons.iter().position(|b| b.1).or(chosen);
                }
            });
        if chosen.is_none() && modal.should_close() {
            chosen = Some(0);
        }
        match chosen {
            Some(i) => {
                if let Some(action) = c.buttons.get_mut(i).and_then(|b| b.2.take()) {
                    action(app);
                }
            }
            None => app.confirms.push(c),
        }
    }
}
