//! The Tables page: the database of each service. A column grid edited in place, then the isolation and the constraints.

use eframe::egui::{
    self, Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, Sense, Stroke, Ui, Vec2,
};
use sb_app::rename;
use sb_model::names::is_snake_ident;
use sb_model::{
    Column, ColumnType, DELETED_AT, ForeignKey, Index, Project, ReferentialAction, Service, Table,
    UniqueConstraint,
};

use super::common::{choice, icon_btn, link, unique};
use crate::app::App;
use crate::fonts::Weight;
use crate::icons::{self, icon};
use crate::theme::*;

/// The page state: the table that is open, `(service, table)` (`ui.table_sel` of the GTK version).
#[derive(Default)]
pub struct TablesState {
    pub sel: Option<(String, String)>,
}

fn type_name(t: ColumnType) -> String {
    serde_json::to_value(t)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

fn type_label(t: ColumnType) -> String {
    format!("{} · {}", type_name(t), t.sql().to_lowercase())
}

const ACTIONS: [(ReferentialAction, &str); 5] = [
    (ReferentialAction::NoAction, "Refuser (fin de transaction)"),
    (ReferentialAction::Restrict, "Refuser tout de suite"),
    (ReferentialAction::Cascade, "Propager"),
    (ReferentialAction::SetNull, "Mettre à NULL"),
    (ReferentialAction::SetDefault, "Valeur par défaut"),
];

/// The SQL the service applies at start-up for the table, to read the design as SQL.
fn create_table_sql(t: &Table) -> String {
    let mut cols: Vec<String> = t
        .columns
        .iter()
        .map(|c| {
            let default = c
                .default
                .as_deref()
                .map(str::trim)
                .filter(|d| !d.is_empty());
            format!(
                "  {} {}{}{}",
                c.name,
                c.ty.sql(),
                if c.nullable { "" } else { " NOT NULL" },
                default.map_or(String::new(), |d| format!(" DEFAULT {d}"))
            )
        })
        .collect();
    let pk: Vec<&str> = t
        .columns
        .iter()
        .filter(|c| c.primary_key)
        .map(|c| c.name.as_str())
        .collect();
    if !pk.is_empty() {
        cols.push(format!("  PRIMARY KEY ({})", pk.join(", ")));
    }
    let mut out = vec![format!(
        "CREATE TABLE IF NOT EXISTS {} (\n{}\n);",
        t.name,
        cols.join(",\n")
    )];
    for u in &t.uniques {
        out.push(format!(
            "ALTER TABLE {} ADD CONSTRAINT {} UNIQUE ({});",
            t.name,
            u.name,
            u.columns.join(", ")
        ));
    }
    for i in &t.indexes {
        out.push(format!(
            "CREATE {}INDEX IF NOT EXISTS {} ON {} ({});",
            if i.unique { "UNIQUE " } else { "" },
            i.name,
            t.name,
            i.columns.join(", ")
        ));
    }
    for f in &t.foreign_keys {
        out.push(format!(
            "ALTER TABLE {} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}) ON DELETE {} ON UPDATE {};",
            t.name,
            f.name,
            f.columns.join(", "),
            f.ref_table,
            f.ref_columns.join(", "),
            f.on_delete.sql(),
            f.on_update.sql()
        ));
    }
    out.join("\n")
}

/// How many SQL steps of the service read or write the table.
fn usages(svc: &Service, table: &str) -> usize {
    fn walk(v: &serde_json::Value, table: &str) -> usize {
        match v {
            serde_json::Value::Object(o) => {
                let me = usize::from(
                    o.get("type").and_then(|t| t.as_str()) == Some("sql")
                        && o.get("table").and_then(|t| t.as_str()) == Some(table),
                );
                me + o.values().map(|x| walk(x, table)).sum::<usize>()
            }
            serde_json::Value::Array(a) => a.iter().map(|x| walk(x, table)).sum(),
            _ => 0,
        }
    }
    let rpcs = serde_json::to_value((&svc.rpcs, &svc.routes, &svc.consumers, &svc.jobs))
        .unwrap_or_default();
    walk(&rpcs, table)
}

fn table_mut<'a>(p: &'a mut Project, service: &str, table: &str) -> Option<&'a mut Table> {
    p.services
        .iter_mut()
        .find(|s| s.name == service)?
        .tables
        .iter_mut()
        .find(|t| t.name == table)
}

/// Renames a column and everything in the table that names it. SQL steps are left to the validator.
fn rename_column(p: &mut Project, service: &str, table: &str, from: &str, to: &str) {
    let Some(svc) = p.services.iter_mut().find(|s| s.name == service) else {
        return;
    };
    for t in &mut svc.tables {
        if t.name == table {
            for c in &mut t.columns {
                if c.name == from {
                    c.name = to.into();
                }
            }
            let fix = |list: &mut Vec<String>| {
                list.iter_mut()
                    .filter(|c| *c == from)
                    .for_each(|c| *c = to.into())
            };
            t.uniques.iter_mut().for_each(|u| fix(&mut u.columns));
            t.indexes.iter_mut().for_each(|i| fix(&mut i.columns));
            t.foreign_keys.iter_mut().for_each(|f| fix(&mut f.columns));
            if t.tenant_column.as_deref() == Some(from) {
                t.tenant_column = Some(to.into());
            }
        }
        for f in &mut t.foreign_keys {
            if f.ref_table == table {
                f.ref_columns
                    .iter_mut()
                    .filter(|c| *c == from)
                    .for_each(|c| *c = to.into());
            }
        }
    }
}

fn constraint_name(t: &Table, columns: &[String], prefix: &str) -> String {
    let base = format!("{prefix}_{}_{}", t.name, columns.join("_"));
    let taken: Vec<&str> = t
        .uniques
        .iter()
        .map(|u| u.name.as_str())
        .chain(t.indexes.iter().map(|i| i.name.as_str()))
        .chain(t.foreign_keys.iter().map(|f| f.name.as_str()))
        .collect();
    unique(&base, &taken)
}

// ---- drawing helpers ----

/// A small uppercase heading (`.table-h4`).
fn heading(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .size(11.0)
            .bold()
            .extra_letter_spacing(0.8)
            .color(MUTED),
    );
}

/// A bordered block (`.table-card`).
fn card_box<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            add(ui)
        })
        .inner
}

fn muted_small(ui: &mut Ui, text: &str) {
    ui.add(
        egui::Label::new(
            RichText::new(text)
                .size(11.5)
                .line_height(Some(lh(11.5)))
                .color(MUTED),
        )
        .wrap(),
    );
}

/// A tiny uppercase label followed by a control.
fn labelled<R>(ui: &mut Ui, text: &str, control: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(
            RichText::new(text.to_uppercase())
                .size(10.0)
                .extra_letter_spacing(0.7)
                .color(FAINT),
        );
        control(ui)
    })
    .inner
}

/// A control of a given width.
fn sized<R>(ui: &mut Ui, width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui(Vec2::new(width, 30.0), add).inner
}

/// A text input committed on Enter or when the focus leaves (as `common::commit_entry`), with a font size.
fn entry(
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    value: &str,
    size: f32,
    placeholder: &str,
    allow_empty: bool,
) -> Option<String> {
    let id = ui.make_persistent_id(id);
    let key = id.with("sb-buffer");
    let focused = ui.memory(|m| m.has_focus(id));
    let mut text: String = if focused {
        ui.data(|d| d.get_temp::<String>(key))
            .unwrap_or_else(|| value.to_string())
    } else {
        value.to_string()
    };
    let r = ui.add(
        egui::TextEdit::singleline(&mut text)
            .id(id)
            .desired_width(ui.available_width())
            .hint_text(placeholder)
            .font(if size > 14.0 {
                crate::theme::mono_bold(size)
            } else {
                mono(size)
            })
            .margin(Margin::symmetric(8, 7)),
    );
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(key, text.clone()));
    }
    if r.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(key));
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            return None;
        }
        let v = text.trim().to_string();
        if v != value && (allow_empty || !v.is_empty()) {
            return Some(v);
        }
    }
    None
}

/// The columns of a constraint, ticked in order: the order is the one of an index.
fn picker(ui: &mut Ui, table: &Table, selected: &[String]) -> Option<Vec<String>> {
    let known: Vec<&str> = table.columns.iter().map(|c| c.name.as_str()).collect();
    let mut names: Vec<String> = known.iter().map(|s| (*s).to_string()).collect();
    names.extend(
        selected
            .iter()
            .filter(|c| !known.contains(&c.as_str()))
            .cloned(),
    );
    let mut result = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        for name in names {
            let at = selected.iter().position(|c| *c == name);
            let text = match at {
                Some(i) if selected.len() > 1 => format!("{name} {}", i + 1),
                _ => name.clone(),
            };
            let mut on = at.is_some();
            let changed = Frame::new()
                .fill(PANEL2)
                .stroke(Stroke::new(1.0, if on { ACCENT } else { BORDER }))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::symmetric(8, 1))
                .show(ui, |ui| {
                    ui.checkbox(&mut on, RichText::new(text).font(mono(SM)))
                        .changed()
                })
                .inner;
            if changed {
                let mut next = selected.to_vec();
                if on {
                    next.push(name.clone());
                } else {
                    next.retain(|x| *x != name);
                }
                result = Some(next);
            }
        }
    });
    result
}

/// `+ …` link next to a heading; greyed out when `enabled` is false.
fn add_row(ui: &mut Ui, title: &str, add_text: &str, enabled: bool) -> bool {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        heading(ui, title);
        ui.add_enabled_ui(enabled, |ui| link(ui, add_text).clicked())
            .inner
    })
    .inner
}

/// A table of the sidebar: icon and name, highlighted when open.
fn table_item(ui: &mut Ui, name: &str, active: bool) -> bool {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
    if active {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(8), alpha(SERVICE, 46));
    } else if r.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(8), RAISED);
    }
    icons::paint(
        ui,
        "table",
        egui::Rect::from_center_size(rect.left_center() + Vec2::new(26.0, 0.0), Vec2::splat(13.0)),
        MUTED,
    );
    ui.painter().text(
        rect.left_center() + Vec2::new(40.0, 0.0),
        egui::Align2::LEFT_CENTER,
        name,
        mono(BASE),
        if active { Color32::WHITE } else { TEXT },
    );
    r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

// ---- the page ----

pub fn page(app: &mut App, ui: &mut Ui) {
    let p = app.project();

    // The table that is open: the one chosen, else the first there is.
    let first = p
        .services
        .iter()
        .find(|s| !s.tables.is_empty())
        .map(|s| (s.name.clone(), s.tables[0].name.clone()));
    let current = app
        .tables
        .sel
        .clone()
        .filter(|(s, t)| {
            p.service(s)
                .is_some_and(|s| s.tables.iter().any(|x| x.name == *t))
        })
        .or(first);

    // ---- sidebar ----
    egui::Panel::left("tables-side")
        .exact_size(250.0)
        .resizable(false)
        .frame(
            Frame::new()
                .fill(PANEL)
                .stroke(Stroke::new(1.0, BORDER))
                .inner_margin(Margin {
                    left: 8,
                    right: 8,
                    top: 14,
                    bottom: 12,
                }),
        )
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 2.0;
                if p.services.is_empty() {
                    super::common::empty(ui, "Ajoutez d’abord un service dans l’architecture.");
                }
                for svc in &p.services {
                    ui.add_space(3.0);
                    ui.horizontal(|ui| {
                        ui.add_space(6.0);
                        heading(ui, &svc.name);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if icon_btn(
                                ui,
                                "plus",
                                &format!("Ajouter une table à {}", svc.name),
                                false,
                            )
                            .clicked()
                            {
                                add_table(app, &svc.name);
                            }
                        });
                    });
                    ui.add_space(4.0);
                    if svc.tables.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(18.0);
                            super::common::empty(ui, "Aucune table");
                        });
                    }
                    for t in &svc.tables {
                        let active = current.as_ref() == Some(&(svc.name.clone(), t.name.clone()));
                        if table_item(ui, &t.name, active) {
                            app.tables.sel = Some((svc.name.clone(), t.name.clone()));
                        }
                    }
                }
            });
        });

    // ---- main ----
    egui::CentralPanel::no_frame().show(ui, |ui| {
        let Some((sname, tname)) = current else {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() / 2.0 - 70.0).max(20.0));
                ui.add(icon("table", 32.0, FAINT));
                ui.add_space(10.0);
                ui.add_sized(
                    [ui.available_width().min(420.0), 20.0],
                    egui::Label::new(RichText::new("Chaque service a sa propre base. Créez une table pour concevoir ses colonnes ; les blocs SQL des routes pourront la lire.").color(FAINT)).wrap(),
                );
                if let Some(first) = p.services.first()
                    && link(ui, &format!("+ Ajouter une table à {}", first.name)).clicked()
                {
                    add_table(app, &first.name);
                }
            });
            return;
        };
        let (Some(svc), Some(table)) = (
            p.service(&sname),
            p.service(&sname)
                .and_then(|s| s.tables.iter().find(|t| t.name == tname)),
        ) else {
            return;
        };
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                Frame::new()
                    .inner_margin(Margin {
                        left: 22,
                        right: 22,
                        top: 18,
                        bottom: 40,
                    })
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 14.0;
                        main(app, ui, svc, table);
                    });
            });
    });
}

fn main(app: &mut App, ui: &mut Ui, svc: &Service, table: &Table) {
    let (sname, tname) = (svc.name.clone(), table.name.clone());

    // header
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        ui.add(icon("table", 18.0, TEXT));
        ui.label(RichText::new(format!("{} ·", svc.name)).color(MUTED));
        let renamed = sized(ui, 260.0, |ui| {
            entry(ui, ("table-name", &sname, &tname), &table.name, 18.0, "", false)
        });
        if let Some(v) = renamed {
            let taken = svc.tables.iter().any(|t| t.name == v);
            if !is_snake_ident(&v) || v.len() > 63 {
                app.say(
                    "Le nom doit être en snake_case (63 caractères au plus), par exemple order_lines.",
                );
            } else if taken {
                app.say(&format!("La table {v} existe déjà."));
            } else {
                app.edit(|p| rename::table(p, &sname, &tname, &v));
                app.tables.sel = Some((sname.clone(), v));
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icon_btn(ui, "trash", &format!("Supprimer la table {}", table.name), true)
                .clicked()
            {
                app.edit(|p| {
                    if let Some(s) = p.services.iter_mut().find(|s| s.name == sname) {
                        s.tables.retain(|t| t.name != tname);
                        for t in &mut s.tables {
                            t.foreign_keys.retain(|f| f.ref_table != tname);
                        }
                    }
                });
                app.tables.sel = None;
            }
            ui.label(
                RichText::new(format!(
                    "{} étape(s) SQL l’utilisent",
                    usages(svc, &table.name)
                ))
                .color(MUTED),
            );
        });
    });

    columns_grid(app, ui, &sname, &tname, table);

    if link(ui, "+ Ajouter une colonne").clicked() {
        app.edit(|p| {
            if let Some(tb) = table_mut(p, &sname, &tname) {
                let taken: Vec<&str> = tb.columns.iter().map(|c| c.name.as_str()).collect();
                let name = unique("column", &taken);
                let mut c = Column::new(name, ColumnType::String);
                c.nullable = true;
                tb.columns.push(c);
            }
        });
    }
    muted_small(
        ui,
        "Les tables sont créées au démarrage du service. Ajouter une colonne met à jour une base existante ; renommer ou supprimer une colonne ne change pas les tables déjà créées. Une colonne ajoutée est nulle par défaut, pour ne pas échouer sur des lignes existantes.",
    );

    constraints(app, ui, svc, table);

    Frame::new()
        .fill(CODE_BG)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::Label::new(
                    RichText::new(create_table_sql(table))
                        .font(mono(11.5))
                        .color(hex(0x9be3b4)),
                )
                .selectable(true),
            );
        });
}

/// Mutates one column of the open table.
fn column_edit(app: &mut App, s: &str, t: &str, ci: usize, f: impl FnOnce(&mut Column)) {
    app.edit(|p| {
        if let Some(col) = table_mut(p, s, t).and_then(|tb| tb.columns.get_mut(ci)) {
            f(col);
        }
    });
}

fn columns_grid(app: &mut App, ui: &mut Ui, s: &str, t: &str, table: &Table) {
    Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin {
            left: 12,
            right: 12,
            top: 6,
            bottom: 10,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (w_type, w_check, w_trash, gap) = (190.0, 40.0, 26.0, 8.0);
            // 14: what the grid takes beyond its columns (measured).
            let rest = ui.available_width() - 14.0 - w_type - 2.0 * w_check - w_trash - 5.0 * gap;
            let w_name = (rest / 2.0).max(60.0);
            let types: Vec<String> = ColumnType::ALL.iter().map(|t| type_label(*t)).collect();
            egui::Grid::new(("columns-grid", s, t))
                .num_columns(6)
                .spacing([gap, 8.0])
                .show(ui, |ui| {
                    for (title, w) in [
                        ("Nom", w_name),
                        ("Type", w_type),
                        ("Nul", w_check),
                        ("Clé", w_check),
                        ("Défaut (SQL)", w_name),
                        ("", w_trash),
                    ] {
                        ui.allocate_ui(Vec2::new(w, 18.0), |ui| {
                            ui.label(
                                RichText::new(title.to_uppercase())
                                    .size(10.0)
                                    .extra_letter_spacing(0.7)
                                    .color(FAINT),
                            );
                        });
                    }
                    ui.end_row();
                    for (ci, c) in table.columns.iter().enumerate() {
                        let renamed = sized(ui, w_name, |ui| {
                            entry(ui, ("col-name", s, t, ci), &c.name, SM, "", false)
                        });
                        if let Some(v) = renamed {
                            let taken = table.columns.iter().any(|c| c.name == v);
                            if !is_snake_ident(&v) || v.len() > 63 {
                                app.say("Le nom doit être en snake_case (63 caractères au plus).");
                            } else if taken {
                                app.say(&format!("La colonne {v} existe déjà."));
                            } else {
                                app.edit(|p| rename_column(p, s, t, &c.name, &v));
                            }
                        }

                        let picked = sized(ui, w_type, |ui| {
                            choice(ui, ("col-type", s, t, ci), &types, &type_label(c.ty))
                        });
                        if let Some(v) = picked
                            && let Some(ty) = ColumnType::ALL.iter().find(|x| type_label(**x) == v)
                        {
                            let ty = *ty;
                            column_edit(app, s, t, ci, |col| col.ty = ty);
                        }

                        let mut null = c.nullable;
                        let r = ui
                            .allocate_ui_with_layout(
                                Vec2::new(w_check, 24.0),
                                Layout::top_down(Align::Center),
                                |ui| {
                                    ui.add_enabled(
                                        !c.primary_key,
                                        egui::Checkbox::without_text(&mut null),
                                    )
                                    .on_hover_text("La colonne accepte NULL")
                                },
                            )
                            .inner;
                        if r.changed() {
                            column_edit(app, s, t, ci, |col| col.nullable = null);
                        }

                        let mut key = c.primary_key;
                        let r = ui
                            .allocate_ui_with_layout(
                                Vec2::new(w_check, 24.0),
                                Layout::top_down(Align::Center),
                                |ui| {
                                    ui.add(egui::Checkbox::without_text(&mut key))
                                        .on_hover_text("Clé primaire")
                                },
                            )
                            .inner;
                        if r.changed() {
                            column_edit(app, s, t, ci, |col| {
                                col.primary_key = key;
                                if key {
                                    col.nullable = false;
                                }
                            });
                        }

                        let default = sized(ui, w_name, |ui| {
                            entry(
                                ui,
                                ("col-default", s, t, ci),
                                c.default.as_deref().unwrap_or(""),
                                SM,
                                "now(), 0, 'x'…",
                                true,
                            )
                        });
                        if let Some(v) = default {
                            column_edit(app, s, t, ci, |col| {
                                col.default = Some(v).filter(|v| !v.is_empty())
                            });
                        }

                        if icon_btn(
                            ui,
                            "trash",
                            &format!("Supprimer la colonne {}", c.name),
                            true,
                        )
                        .clicked()
                        {
                            let gone = c.name.clone();
                            app.edit(|p| {
                                if let Some(t) = table_mut(p, s, t) {
                                    t.columns.retain(|c| c.name != gone);
                                    t.uniques.retain(|u| !u.columns.contains(&gone));
                                    t.indexes.retain(|i| !i.columns.contains(&gone));
                                    t.foreign_keys.retain(|f| !f.columns.contains(&gone));
                                    if t.tenant_column.as_deref() == Some(&gone) {
                                        t.tenant_column = None;
                                    }
                                }
                            });
                        }
                        ui.end_row();
                    }
                });
        });
}

fn add_table(app: &mut App, service: &str) {
    let mut name = String::new();
    app.edit(|p| {
        if let Some(s) = p.services.iter_mut().find(|s| s.name == service) {
            let taken: Vec<&str> = s.tables.iter().map(|t| t.name.as_str()).collect();
            name = unique("new_table", &taken);
            let mut id = Column::new("id", ColumnType::Uuid);
            id.primary_key = true;
            id.default = Some("gen_random_uuid()".into());
            s.tables.push(Table::new(name.clone(), vec![id]));
        }
    });
    app.tables.sel = Some((service.to_string(), name));
}

/// The header of a constraint card: a name field, `extra`, then the trash button. Returns (new name, delete?).
fn card_head(
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    name: &str,
    trash_tip: &str,
    extra: impl FnOnce(&mut Ui),
) -> (Option<String>, bool) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let renamed = sized(ui, 260.0, |ui| entry(ui, id, name, SM, "", false));
        extra(ui);
        let del = ui
            .with_layout(Layout::right_to_left(Align::Center), |ui| {
                icon_btn(ui, "trash", trash_tip, true).clicked()
            })
            .inner;
        (renamed, del)
    })
    .inner
}

/// Mutates one foreign key of the open table.
fn fk_edit(app: &mut App, s: &str, t: &str, i: usize, f: impl FnOnce(&mut ForeignKey)) {
    app.edit(|p| {
        if let Some(fk) = table_mut(p, s, t).and_then(|tb| tb.foreign_keys.get_mut(i)) {
            f(fk);
        }
    });
}

fn constraints(app: &mut App, ui: &mut Ui, svc: &Service, table: &Table) {
    let (s, t) = (svc.name.as_str(), table.name.as_str());

    // ---- isolation and soft delete ----
    ui.add_space(8.0);
    heading(ui, "Isolation et suppression");
    card_box(ui, |ui| {
        let candidates: Vec<String> = table
            .columns
            .iter()
            .filter(|c| !c.nullable && matches!(c.ty, ColumnType::String | ColumnType::Uuid))
            .map(|c| c.name.clone())
            .collect();
        let none = "aucune".to_string();
        let mut options = vec![none.clone()];
        options.extend(candidates);
        let current = table.tenant_column.clone().unwrap_or_else(|| none.clone());
        let picked = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.label(RichText::new("Colonne tenant").color(MUTED));
                choice(ui, ("tenant", s, t), &options, &current)
            })
            .inner;
        if let Some(v) = picked {
            app.edit(|p| {
                if let Some(tb) = table_mut(p, s, t) {
                    tb.tenant_column = Some(v).filter(|v| *v != none);
                }
            });
        }
        muted_small(
            ui,
            "Chaque bloc SQL sur cette table est filtré par cette colonne et chaque insertion la renseigne, à partir de la variable du même nom que le flux a liée (par exemple depuis le jeton vérifié) : on ne peut pas l’oublier, ni lire ou écrire les lignes d’un autre tenant avec ces blocs. Le code Rust libre n’est pas filtré.",
        );
        let mut soft = table.soft_delete;
        if ui.checkbox(&mut soft, "Suppression logique").changed() {
            app.edit(|p| {
                if let Some(tb) = table_mut(p, s, t) {
                    tb.soft_delete = soft;
                    if soft && tb.column(DELETED_AT).is_none() {
                        let mut c = Column::new(DELETED_AT, ColumnType::Timestamp);
                        c.nullable = true;
                        tb.columns.push(c);
                    }
                }
            });
        }
        muted_small(
            ui,
            "Une suppression ne retire pas la ligne : elle date deleted_at (colonne ajoutée au besoin), et les lectures, mises à jour et suppressions ignorent les lignes datées. Les contraintes UNIQUE continuent de compter les lignes supprimées.",
        );
    });

    // ---- unique ----
    if add_row(ui, "Contraintes unique", "+ Ajouter une contrainte", true) {
        app.edit(|p| {
            if let Some(tb) = table_mut(p, s, t) {
                let columns: Vec<String> =
                    tb.columns.iter().take(1).map(|c| c.name.clone()).collect();
                let name = constraint_name(tb, &columns, "uq");
                tb.uniques.push(UniqueConstraint { name, columns });
            }
        });
    }
    for (i, u) in table.uniques.iter().enumerate() {
        card_box(ui, |ui| {
            let (renamed, del) = card_head(
                ui,
                ("uq-name", s, t, i),
                &u.name,
                &format!("Supprimer la contrainte {}", u.name),
                |_| {},
            );
            if let Some(v) = renamed {
                app.edit(|p| {
                    if let Some(x) = table_mut(p, s, t).and_then(|tb| tb.uniques.get_mut(i)) {
                        x.name = v;
                    }
                });
            }
            if del {
                app.edit(|p| {
                    if let Some(tb) = table_mut(p, s, t)
                        && i < tb.uniques.len()
                    {
                        tb.uniques.remove(i);
                    }
                });
            }
            if let Some(cols) = picker(ui, table, &u.columns) {
                app.edit(|p| {
                    if let Some(x) = table_mut(p, s, t).and_then(|tb| tb.uniques.get_mut(i)) {
                        x.columns = cols;
                    }
                });
            }
        });
    }

    // ---- indexes ----
    if add_row(ui, "Index", "+ Ajouter un index", true) {
        app.edit(|p| {
            if let Some(tb) = table_mut(p, s, t) {
                let columns: Vec<String> =
                    tb.columns.iter().take(1).map(|c| c.name.clone()).collect();
                let name = constraint_name(tb, &columns, "idx");
                tb.indexes.push(Index {
                    name,
                    columns,
                    unique: false,
                });
            }
        });
    }
    for (i, x) in table.indexes.iter().enumerate() {
        card_box(ui, |ui| {
            let mut uniq = x.unique;
            let mut toggled = false;
            let (renamed, del) = card_head(
                ui,
                ("idx-name", s, t, i),
                &x.name,
                &format!("Supprimer l’index {}", x.name),
                |ui| toggled = ui.checkbox(&mut uniq, "unique").changed(),
            );
            if let Some(v) = renamed {
                app.edit(|p| {
                    if let Some(ix) = table_mut(p, s, t).and_then(|tb| tb.indexes.get_mut(i)) {
                        ix.name = v;
                    }
                });
            }
            if toggled {
                app.edit(|p| {
                    if let Some(ix) = table_mut(p, s, t).and_then(|tb| tb.indexes.get_mut(i)) {
                        ix.unique = uniq;
                    }
                });
            }
            if del {
                app.edit(|p| {
                    if let Some(tb) = table_mut(p, s, t)
                        && i < tb.indexes.len()
                    {
                        tb.indexes.remove(i);
                    }
                });
            }
            if let Some(cols) = picker(ui, table, &x.columns) {
                app.edit(|p| {
                    if let Some(ix) = table_mut(p, s, t).and_then(|tb| tb.indexes.get_mut(i)) {
                        ix.columns = cols;
                    }
                });
            }
        });
    }

    // ---- foreign keys ----
    let can_reference = svc
        .tables
        .iter()
        .any(|tb| tb.columns.iter().any(|c| c.primary_key));
    if add_row(
        ui,
        "Clés étrangères",
        "+ Ajouter une clé étrangère",
        can_reference,
    ) {
        app.edit(|p| add_foreign_key(p, s, t));
    }
    if !can_reference {
        ui.add(
            egui::Label::new(
                RichText::new("Une clé étrangère référence la clé primaire d’une table du service : définissez-en une d’abord.")
                    .size(SM)
                    .color(MUTED),
            )
            .wrap(),
        );
    }
    for (i, fk) in table.foreign_keys.iter().enumerate() {
        fk_card(app, ui, svc, table, i, fk);
    }
}

fn add_foreign_key(p: &mut Project, s: &str, t: &str) {
    let Some(svc) = p.services.iter_mut().find(|x| x.name == s) else {
        return;
    };
    let Some(target) = svc
        .tables
        .iter()
        .find(|tb| tb.columns.iter().any(|c| c.primary_key))
        .cloned()
    else {
        return;
    };
    let Some(tb) = svc.tables.iter_mut().find(|x| x.name == t) else {
        return;
    };
    let ref_columns: Vec<String> = target
        .columns
        .iter()
        .filter(|c| c.primary_key)
        .map(|c| c.name.clone())
        .collect();
    let mut used: Vec<String> = vec![];
    let columns: Vec<String> = ref_columns
        .iter()
        .map(|r| {
            let wanted = target.column(r).map(|c| c.ty);
            let fits: Vec<&Column> = tb
                .columns
                .iter()
                .filter(|c| {
                    Some(c.ty) == wanted
                        && !used.contains(&c.name)
                        && !(tb.name == target.name && c.primary_key)
                })
                .collect();
            let name = fits
                .iter()
                .find(|c| c.name == *r)
                .or(fits.first())
                .map(|c| c.name.clone())
                .or_else(|| {
                    tb.columns
                        .iter()
                        .find(|c| !used.contains(&c.name))
                        .map(|c| c.name.clone())
                })
                .unwrap_or_default();
            used.push(name.clone());
            name
        })
        .collect();
    let name = constraint_name(tb, &columns, "fk");
    tb.foreign_keys.push(ForeignKey {
        name,
        columns,
        ref_table: target.name.clone(),
        ref_columns,
        on_delete: ReferentialAction::NoAction,
        on_update: ReferentialAction::NoAction,
    });
}

fn fk_card(app: &mut App, ui: &mut Ui, svc: &Service, table: &Table, i: usize, fk: &ForeignKey) {
    let (s, t) = (svc.name.as_str(), table.name.as_str());
    card_box(ui, |ui| {
        let (renamed, del) = card_head(
            ui,
            ("fk-name", s, t, i),
            &fk.name,
            &format!("Supprimer la clé étrangère {}", fk.name),
            |_| {},
        );
        if let Some(v) = renamed {
            fk_edit(app, s, t, i, |f| f.name = v);
        }
        if del {
            app.edit(|p| {
                if let Some(tb) = table_mut(p, s, t)
                    && i < tb.foreign_keys.len()
                {
                    tb.foreign_keys.remove(i);
                }
            });
        }

        let tables: Vec<String> = svc.tables.iter().map(|x| x.name.clone()).collect();
        let picked = labelled(ui, "Référence", |ui| {
            sized(ui, 240.0, |ui| {
                choice(ui, ("fk-ref", s, t, i), &tables, &fk.ref_table)
            })
        });
        if let Some(v) = picked {
            app.edit(|p| {
                let Some(svc) = p.services.iter_mut().find(|x| x.name == s) else {
                    return;
                };
                let target = svc.tables.iter().find(|x| x.name == v).cloned();
                let Some(tb) = svc.tables.iter_mut().find(|x| x.name == t) else {
                    return;
                };
                let pk: Vec<String> = target
                    .iter()
                    .flat_map(|x| {
                        x.columns
                            .iter()
                            .filter(|c| c.primary_key)
                            .map(|c| c.name.clone())
                    })
                    .collect();
                let refs = if pk.is_empty() {
                    target
                        .iter()
                        .flat_map(|x| x.columns.iter().take(1).map(|c| c.name.clone()))
                        .collect()
                } else {
                    pk
                };
                if let Some(f) = tb.foreign_keys.get_mut(i) {
                    let old = f.columns.clone();
                    f.ref_table = v;
                    f.columns = refs
                        .iter()
                        .enumerate()
                        .map(|(k, _)| old.get(k).cloned().unwrap_or_default())
                        .collect();
                    f.ref_columns = refs;
                }
            });
        }

        let target = svc.tables.iter().find(|x| x.name == fk.ref_table);
        let mine: Vec<String> = table.columns.iter().map(|c| c.name.clone()).collect();
        let theirs: Vec<String> = target
            .map(|x| x.columns.iter().map(|c| c.name.clone()).collect())
            .unwrap_or_default();
        for (k, c) in fk.columns.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let w = ((ui.available_width() - 26.0 - 16.0 - 24.0) / 2.0).max(80.0);
                if let Some(v) = sized(ui, w, |ui| choice(ui, ("fk-l", s, t, i, k), &mine, c)) {
                    fk_edit(app, s, t, i, |f| {
                        if let Some(x) = f.columns.get_mut(k) {
                            *x = v;
                        }
                    });
                }
                ui.label("→");
                let shown = fk.ref_columns.get(k).map_or("", String::as_str);
                if let Some(v) = sized(ui, w, |ui| choice(ui, ("fk-r", s, t, i, k), &theirs, shown))
                {
                    fk_edit(app, s, t, i, |f| {
                        if let Some(x) = f.ref_columns.get_mut(k) {
                            *x = v;
                        }
                    });
                }
                let removable = fk.columns.len() > 1;
                let r = ui
                    .add_enabled_ui(removable, |ui| {
                        icon_btn(ui, "trash", &format!("Retirer la paire {}", k + 1), true)
                    })
                    .inner;
                if removable && r.clicked() {
                    fk_edit(app, s, t, i, |f| {
                        if k < f.columns.len() {
                            f.columns.remove(k);
                            if k < f.ref_columns.len() {
                                f.ref_columns.remove(k);
                            }
                        }
                    });
                }
            });
        }
        if link(ui, "+ Ajouter une paire de colonnes").clicked() {
            app.edit(|p| {
                let Some(svc) = p.services.iter().find(|x| x.name == s) else {
                    return;
                };
                let first_of = |name: &str| {
                    svc.tables
                        .iter()
                        .find(|x| x.name == name)
                        .and_then(|x| x.columns.first())
                        .map(|c| c.name.clone())
                        .unwrap_or_default()
                };
                let Some(f) = svc
                    .tables
                    .iter()
                    .find(|x| x.name == t)
                    .and_then(|tb| tb.foreign_keys.get(i))
                else {
                    return;
                };
                let (mine, theirs) = (first_of(t), first_of(&f.ref_table));
                if let Some(f) = table_mut(p, s, t).and_then(|tb| tb.foreign_keys.get_mut(i)) {
                    f.columns.push(mine);
                    f.ref_columns.push(theirs);
                }
            });
        }

        let labels: Vec<String> = ACTIONS.iter().map(|(_, l)| (*l).to_string()).collect();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            for (title, on_delete) in [("À la suppression", true), ("À la mise à jour", false)] {
                let current = if on_delete {
                    fk.on_delete
                } else {
                    fk.on_update
                };
                let shown = ACTIONS
                    .iter()
                    .find(|(a, _)| *a == current)
                    .map_or("", |(_, l)| *l);
                let picked = labelled(ui, title, |ui| {
                    sized(ui, 230.0, |ui| {
                        choice(ui, ("fk-rule", s, t, i, on_delete), &labels, shown)
                    })
                });
                if let Some(v) = picked
                    && let Some((a, _)) = ACTIONS.iter().find(|(_, l)| *l == v)
                {
                    let a = *a;
                    fk_edit(app, s, t, i, |f| {
                        if on_delete {
                            f.on_delete = a;
                        } else {
                            f.on_update = a;
                        }
                    });
                }
            }
        });
    });
}
