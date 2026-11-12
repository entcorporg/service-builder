//! The schemas (events, proto, API) in the inspector of the Architecture page, as in the web version: three
//! blocks, one card per schema, and its fields edited in place.

use gtk::glib::clone;
use gtk::prelude::*;
use sb_app::rename;
use sb_model::names::{is_pascal_ident, is_snake_ident};
use sb_model::{Field, FieldType, Message, MessageKind, Project, Rule};

use super::canvas::{Sel, select};
use super::edit::edit;
use super::editors::{SCALARS, type_name};
use super::inspector::{
    add_message, card, choice, commit_entry, empty, field, icon_button, link, number, section,
};
use super::ui::Shared;
use super::widgets::say;

const KINDS: [MessageKind; 3] = [MessageKind::Event, MessageKind::Proto, MessageKind::Api];

fn title(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::Event => "Événements",
        MessageKind::Proto => "Proto (gRPC)",
        MessageKind::Api => "API",
        MessageKind::Row => "Lignes de table",
    }
}

fn one(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::Event => "Événement",
        MessageKind::Proto => "Proto",
        MessageKind::Api => "API",
        MessageKind::Row => "Ligne",
    }
}

fn hint(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::Event => "Charge utile d’un topic Kafka",
        MessageKind::Proto => "Requêtes, réponses gRPC et types partagés",
        MessageKind::Api => "Corps, paramètres et réponses des routes HTTP",
        MessageKind::Row => "Ligne d’une table",
    }
}

fn base_name(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::Event => "NewEvent",
        MessageKind::Proto => "NewMessage",
        MessageKind::Api => "NewApiSchema",
        MessageKind::Row => "NewRow",
    }
}

fn card_id(name: &str) -> String {
    format!("msg:{name}")
}

/// Selects a schema: the inspector shows all of them, this one open.
pub fn open_message(ui: &Shared, name: &str) {
    ui.canvas.state.borrow_mut().open.insert(card_id(name));
    select(ui, Some(Sel::Messages(name.to_string())));
}

/// Redraws the inspector with the same selection (after a refused value, to give the old one back).
fn redraw(ui: &Shared) {
    let sel = ui.canvas.state.borrow().selected.clone();
    select(ui, sel);
}

fn message_mut<'a>(p: &'a mut Project, name: &str) -> Option<&'a mut Message> {
    p.messages.iter_mut().find(|m| m.name == name)
}

/// The types a field of a schema of this kind can have: API schemas travel as JSON, which has no bytes or timestamp.
fn type_options(p: &Project, kind: MessageKind) -> Vec<String> {
    let api = kind == MessageKind::Api;
    let mut options: Vec<String> = SCALARS
        .iter()
        .map(|(n, _)| (*n).to_string())
        .filter(|n| !api || (n != "bytes" && n != "timestamp"))
        .collect();
    options.extend(
        p.messages
            .iter()
            .filter(|m| (m.kind == MessageKind::Api) == api)
            .map(|m| m.name.clone()),
    );
    options
}

fn parse_type(name: &str) -> FieldType {
    SCALARS.iter().find(|(n, _)| *n == name).map_or_else(
        || FieldType::Message {
            name: name.to_string(),
        },
        |(_, t)| t.clone(),
    )
}

fn next_tag(m: &Message) -> u32 {
    m.fields
        .iter()
        .enumerate()
        .map(|(i, f)| f.tag.unwrap_or(i as u32 + 1))
        .max()
        .unwrap_or(0)
        + 1
}

/// A change of the rules of a field.
type RulesChange = Box<dyn FnOnce(&mut Vec<Rule>)>;

fn is_text(ty: &FieldType) -> bool {
    matches!(ty, FieldType::String | FieldType::Uuid)
}

/// The schemas of the project: a block per kind, a card per schema.
pub fn messages(ui: &Shared, host: &gtk::Box) {
    let project = ui.state.borrow().project.clone();
    for kind in KINDS {
        let add = icon_button(
            "plus",
            &format!("Ajouter un schéma {}", one(kind)),
            false,
            clone!(
                #[strong]
                ui,
                move || {
                    let mut added = String::new();
                    edit(&ui, |p| added = add_message(p, base_name(kind), kind));
                    open_message(&ui, &added);
                }
            ),
        );
        let (outer, body) = section(title(kind), "schema", Some(add.upcast()));
        let h = gtk::Label::new(Some(hint(kind)));
        h.set_xalign(0.0);
        h.set_wrap(true);
        h.add_css_class("kind-hint");
        body.append(&h);
        let entries: Vec<&Message> = project.messages.iter().filter(|m| m.kind == kind).collect();
        if entries.is_empty() {
            body.append(&empty("Aucun schéma de ce type."));
        }
        for m in entries {
            body.append(&message_card(ui, &project, m));
        }
        host.append(&outer);
    }
}

fn message_card(ui: &Shared, project: &Project, m: &Message) -> gtk::Box {
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);

    inner.append(&field(
        "Nom",
        &commit_entry(&m.name, true, "", false, {
            let (ui, from) = (ui.clone(), m.name.clone());
            move |v| {
                if !is_pascal_ident(&v) {
                    say(
                        &ui,
                        "Le nom doit être en PascalCase, par exemple OrderCreated.",
                    );
                    return redraw(&ui);
                }
                if ui.state.borrow().project.message(&v).is_some() {
                    say(&ui, &format!("Le schéma {v} existe déjà."));
                    return redraw(&ui);
                }
                {
                    let mut st = ui.canvas.state.borrow_mut();
                    if st.open.remove(&card_id(&from)) {
                        st.open.insert(card_id(&v));
                    }
                    if st.selected == Some(Sel::Messages(from.clone())) {
                        st.selected = Some(Sel::Messages(v.clone()));
                    }
                }
                edit(&ui, |p| rename::message(p, &from, &v));
            }
        }),
        "",
    ));

    let kinds: Vec<String> = KINDS.iter().map(|k| one(*k).to_string()).collect();
    inner.append(&field(
        "Type",
        &choice(&kinds, one(m.kind), {
            let (ui, name) = (ui.clone(), m.name.clone());
            move |v| {
                let Some(kind) = KINDS.into_iter().find(|k| one(*k) == v) else {
                    return;
                };
                edit(&ui, |p| {
                    if let Some(m) = message_mut(p, &name) {
                        m.kind = kind;
                    }
                });
            }
        }),
        hint(m.kind),
    ));

    let rows = gtk::Box::new(gtk::Orientation::Vertical, 6);
    rows.add_css_class("fields");
    for (i, f) in m.fields.iter().enumerate() {
        rows.append(&field_row(ui, project, m, i, f));
    }
    inner.append(&rows);
    inner.append(&link("+ Ajouter un champ", {
        let (ui, name) = (ui.clone(), m.name.clone());
        move || {
            edit(&ui, |p| {
                let Some(m) = message_mut(p, &name) else {
                    return;
                };
                let mut n = m.fields.len() + 1;
                while m.fields.iter().any(|f| f.name == format!("field_{n}")) {
                    n += 1;
                }
                let mut f = Field::new(format!("field_{n}"), FieldType::String);
                f.tag = Some(next_tag(m));
                m.fields.push(f);
            })
        }
    }));

    let count = m.fields.len();
    let delete = icon_button("trash", &format!("Supprimer {}", m.name), true, {
        let (ui, name) = (ui.clone(), m.name.clone());
        move || edit(&ui, |p| p.messages.retain(|m| m.name != name))
    });
    card(
        ui,
        card_id(&m.name),
        "schema",
        &m.name,
        &format!("{count} champ{}", if count > 1 { "s" } else { "" }),
        vec![delete],
        &inner,
    )
}

/// A change of one field of one schema.
fn change(ui: &Shared, message: &str, index: usize, f: impl FnOnce(&mut Field)) {
    edit(ui, |p| {
        if let Some(field) = message_mut(p, message).and_then(|m| m.fields.get_mut(index)) {
            f(field);
        }
    });
}

fn field_row(ui: &Shared, project: &Project, m: &Message, index: usize, f: &Field) -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let line = gtk::Grid::new();
    line.set_column_spacing(6);
    let tag = gtk::Label::new(Some(&format!("#{}", f.tag.unwrap_or(index as u32 + 1))));
    tag.add_css_class("field-tag");
    tag.set_width_request(26);
    tag.set_xalign(0.0);
    tag.set_tooltip_text(Some("Numéro de champ protobuf, stable"));
    line.attach(&tag, 0, 0, 1, 1);

    let name = commit_entry(&f.name, true, "", false, {
        let (ui, message, from) = (ui.clone(), m.name.clone(), f.name.clone());
        move |v| {
            let taken = ui
                .state
                .borrow()
                .project
                .message(&message)
                .is_some_and(|m| m.fields.iter().any(|x| x.name == v));
            if !is_snake_ident(&v) {
                say(&ui, "Le nom doit être en snake_case, par exemple order_id.");
                return redraw(&ui);
            }
            if taken {
                say(&ui, &format!("Le champ {v} existe déjà."));
                return redraw(&ui);
            }
            edit(&ui, |p| rename_field(p, &message, &from, &v));
        }
    });
    name.set_hexpand(true);
    name.set_width_chars(1);
    name.set_tooltip_text(Some("Nom du champ"));
    line.attach(&name, 1, 0, 1, 1);

    let ty = choice(&type_options(project, m.kind), &type_name(&f.ty), {
        let (ui, message) = (ui.clone(), m.name.clone());
        move |v| {
            change(&ui, &message, index, |x| {
                x.ty = parse_type(&v);
                if matches!(x.ty, FieldType::Message { .. }) {
                    x.optional = false;
                }
                if !is_text(&x.ty) {
                    x.rules.clear();
                    x.source = None;
                }
            })
        }
    });
    ty.set_hexpand(true);
    ty.set_tooltip_text(Some("Type du champ"));
    line.attach(&ty, 2, 0, 1, 1);

    let is_message = matches!(f.ty, FieldType::Message { .. });
    let repeated = gtk::CheckButton::with_label("[]");
    repeated.set_active(f.repeated);
    repeated.set_tooltip_text(Some("Liste"));
    repeated.add_css_class("check-mono");
    repeated.connect_toggled({
        let (ui, message) = (ui.clone(), m.name.clone());
        move |c| {
            let on = c.is_active();
            change(&ui, &message, index, |x| {
                x.repeated = on;
                if on {
                    x.optional = false;
                    x.source = None;
                }
            })
        }
    });
    line.attach(&repeated, 3, 0, 1, 1);
    let optional = gtk::CheckButton::with_label("?");
    optional.set_active(f.optional);
    optional.set_sensitive(!is_message && !f.repeated);
    optional.set_tooltip_text(Some("Optionnel (présence suivie)"));
    optional.add_css_class("check-mono");
    optional.connect_toggled({
        let (ui, message) = (ui.clone(), m.name.clone());
        move |c| {
            let on = c.is_active();
            change(&ui, &message, index, |x| {
                x.optional = on;
                if on {
                    x.source = None;
                }
            })
        }
    });
    line.attach(&optional, 4, 0, 1, 1);
    line.attach(
        &icon_button("trash", &format!("Supprimer {}", f.name), true, {
            let (ui, message) = (ui.clone(), m.name.clone());
            move || {
                edit(&ui, |p| {
                    if let Some(m) = message_mut(p, &message)
                        && index < m.fields.len()
                    {
                        m.fields.remove(index);
                    }
                })
            }
        }),
        5,
        0,
        1,
        1,
    );
    outer.append(&line);
    if m.kind == MessageKind::Api && is_text(&f.ty) {
        outer.append(&rules_row(ui, &m.name, index, f));
    }
    outer
}

/// What a request must satisfy, checked before any step runs (text fields of API schemas).
fn rules_row(ui: &Shared, message: &str, index: usize, f: &Field) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_margin_start(36);
    let t = gtk::Label::new(Some("RÈGLES"));
    t.add_css_class("tiny-label");
    row.append(&t);
    // One change of the rules of this field.
    let set = {
        let (ui, message) = (ui.clone(), message.to_string());
        std::rc::Rc::new(move |apply: RulesChange| {
            change(&ui, &message, index, |x| apply(&mut x.rules));
        })
    };
    for (i, r) in f.rules.iter().enumerate() {
        let chip = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        chip.add_css_class("rule-chip");
        match r {
            Rule::Length { min, max } => {
                chip.append(&gtk::Label::new(Some("longueur")));
                let (min, max) = (*min, *max);
                let lo = number(i64::from(min), 0, 1_000_000, {
                    let set = set.clone();
                    move |v| {
                        set(Box::new(move |rules| {
                            rules[i] = Rule::Length { min: v as u32, max };
                        }))
                    }
                });
                lo.set_width_chars(4);
                chip.append(&lo);
                chip.append(&gtk::Label::new(Some("à")));
                let hi = number(i64::from(max), 0, 1_000_000, {
                    let set = set.clone();
                    move |v| {
                        set(Box::new(move |rules| {
                            rules[i] = Rule::Length { min, max: v as u32 };
                        }))
                    }
                });
                hi.set_width_chars(4);
                chip.append(&hi);
                let unit = gtk::Label::new(Some("car."));
                unit.add_css_class("field-tag");
                chip.append(&unit);
            }
            Rule::Email => chip.append(&gtk::Label::new(Some("E-mail"))),
            Rule::Siren => chip.append(&gtk::Label::new(Some("SIREN"))),
        }
        chip.append(&icon_button(
            "x",
            &format!("Retirer la règle {} de {}", r.kind(), f.name),
            false,
            {
                let set = set.clone();
                move || {
                    set(Box::new(move |rules| {
                        rules.remove(i);
                    }))
                }
            },
        ));
        row.append(&chip);
    }
    let left: Vec<(&str, Rule)> = [
        ("Longueur", Rule::Length { min: 1, max: 128 }),
        ("E-mail", Rule::Email),
        ("SIREN", Rule::Siren),
    ]
    .into_iter()
    .filter(|(_, r)| !f.rules.iter().any(|x| x.kind() == r.kind()))
    .collect();
    if !left.is_empty() {
        let mut labels = vec!["+ règle…"];
        labels.extend(left.iter().map(|(l, _)| *l));
        let add = gtk::DropDown::from_strings(&labels);
        add.set_tooltip_text(Some(&format!("Ajouter une règle à {}", f.name)));
        add.connect_selected_notify(move |d| {
            let Some(i) = (d.selected() as usize).checked_sub(1) else {
                return;
            };
            let rule = left[i].1.clone();
            set(Box::new(move |rules| rules.push(rule)));
        });
        row.append(&add);
    }
    row
}

/// Renames a field and what points at it: the key of the topics that carry the schema, the `{param}` of the paths.
fn rename_field(p: &mut Project, message: &str, from: &str, to: &str) {
    let Some(field) =
        message_mut(p, message).and_then(|m| m.fields.iter_mut().find(|f| f.name == from))
    else {
        return;
    };
    field.name = to.to_string();
    for t in p.topics.iter_mut().filter(|t| t.message == message) {
        if t.key_field.as_deref() == Some(from) {
            t.key_field = Some(to.to_string());
        }
    }
    for s in &mut p.services {
        for r in s.routes.iter_mut().filter(|r| r.request == message) {
            r.path = r.path.replace(&format!("{{{from}}}"), &format!("{{{to}}}"));
        }
    }
}
