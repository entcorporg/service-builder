//! The Tables page: the database of each service. A column grid edited in place, then the isolation and the constraints.

use gtk::glib::{self, clone};
use gtk::prelude::*;
use sb_app::rename;
use sb_model::names::is_snake_ident;
use sb_model::{
    Column, ColumnType, DELETED_AT, ForeignKey, Index, Project, ReferentialAction, Service, Table,
    UniqueConstraint,
};

use super::edit::edit;
use super::icons::{self, icon};
use super::inspector::{choice, commit_entry, empty, icon_button, link, unique};
use super::ui::Shared;
use super::widgets::{clear, say};

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

fn heading(text: &str) -> gtk::Label {
    let l = gtk::Label::builder()
        .label(text.to_uppercase())
        .xalign(0.0)
        .build();
    l.add_css_class("table-h4");
    l
}

fn card_box() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.add_css_class("table-card");
    b
}

fn labelled(text: &str, w: &impl IsA<gtk::Widget>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let l = gtk::Label::new(Some(text));
    l.add_css_class("tiny-label");
    b.append(&l);
    b.append(w);
    b
}

/// The columns of a constraint, ticked in order: the order is the one of an index.
fn picker(
    table: &Table,
    selected: &[String],
    apply: impl Fn(Vec<String>) + 'static,
) -> gtk::FlowBox {
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_max_children_per_line(12);
    flow.set_column_spacing(6);
    flow.set_row_spacing(6);
    let apply = std::rc::Rc::new(apply);
    let known: Vec<&str> = table.columns.iter().map(|c| c.name.as_str()).collect();
    let mut names: Vec<String> = known.iter().map(|s| (*s).to_string()).collect();
    names.extend(
        selected
            .iter()
            .filter(|c| !known.contains(&c.as_str()))
            .cloned(),
    );
    for name in names {
        let at = selected.iter().position(|c| *c == name);
        let text = match at {
            Some(i) if selected.len() > 1 => format!("{name} {}", i + 1),
            _ => name.clone(),
        };
        let check = gtk::CheckButton::with_label(&text);
        check.set_active(at.is_some());
        check.add_css_class("chip");
        let (selected, apply, name) = (selected.to_vec(), apply.clone(), name.clone());
        check.connect_toggled(move |c| {
            let mut next = selected.clone();
            if c.is_active() {
                next.push(name.clone());
            } else {
                next.retain(|x| *x != name);
            }
            apply(next);
        });
        flow.append(&check);
    }
    flow
}

pub fn draw(ui: &Shared, p: &Project) {
    let host = &ui.tables;
    clear(host);

    // The table that is open: the one chosen, else the first there is.
    let chosen = ui.table_sel.borrow().clone();
    let first = p
        .services
        .iter()
        .find(|s| !s.tables.is_empty())
        .map(|s| (s.name.clone(), s.tables[0].name.clone()));
    let current = chosen
        .filter(|(s, t)| {
            p.service(s)
                .is_some_and(|s| s.tables.iter().any(|x| x.name == *t))
        })
        .or(first);

    // ---- sidebar ----
    let side = gtk::Box::new(gtk::Orientation::Vertical, 2);
    side.add_css_class("tables-side");
    if p.services.is_empty() {
        let none = empty("Ajoutez d’abord un service dans l’architecture.");
        none.set_margin_start(18);
        none.set_margin_end(18);
        side.append(&none);
    }
    for svc in &p.services {
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let t = gtk::Label::builder()
            .label(svc.name.to_uppercase())
            .xalign(0.0)
            .hexpand(true)
            .build();
        t.add_css_class("table-h4");
        head.append(&t);
        let name = svc.name.clone();
        head.append(&icon_button(
            "plus",
            &format!("Ajouter une table à {}", svc.name),
            false,
            clone!(
                #[strong]
                ui,
                move || add_table(&ui, &name)
            ),
        ));
        side.append(&head);
        if svc.tables.is_empty() {
            let none = empty("Aucune table");
            none.set_margin_start(18);
            side.append(&none);
        }
        for t in &svc.tables {
            let b = gtk::Button::new();
            b.add_css_class("tree-item");
            b.add_css_class("table-item");
            if current.as_ref() == Some(&(svc.name.clone(), t.name.clone())) {
                b.add_css_class("active");
            }
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.append(&icon("table", 13, icons::MUTED));
            let l = gtk::Label::new(Some(&t.name));
            l.add_css_class("font-mono");
            row.append(&l);
            b.set_child(Some(&row));
            let key = (svc.name.clone(), t.name.clone());
            b.connect_clicked(clone!(
                #[strong]
                ui,
                move |_| {
                    *ui.table_sel.borrow_mut() = Some(key.clone());
                    draw(&ui, &ui.state.borrow().project.clone());
                }
            ));
            side.append(&b);
        }
    }
    let side_scroll = gtk::ScrolledWindow::new();
    side_scroll.set_child(Some(&side));
    // `External`: no scrollbar, and the width of the column is not stretched by what is in it.
    side_scroll.set_policy(gtk::PolicyType::External, gtk::PolicyType::Automatic);
    side_scroll.set_hexpand(false);
    side_scroll.set_width_request(250);
    side_scroll.set_min_content_width(250);
    side_scroll.set_max_content_width(250);
    side_scroll.set_propagate_natural_width(false);
    side_scroll.add_css_class("tables-side-scroll");
    host.append(&side_scroll);

    // ---- main ----
    let main = gtk::Box::new(gtk::Orientation::Vertical, 14);
    main.add_css_class("tables-main");
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_hexpand(true);
    scroll.set_child(Some(&main));
    host.append(&scroll);

    let Some((sname, tname)) = current else {
        let none = gtk::Box::new(gtk::Orientation::Vertical, 10);
        none.set_valign(gtk::Align::Center);
        none.set_halign(gtk::Align::Center);
        none.append(&icon("table", 32, icons::FAINT));
        let t = gtk::Label::builder()
            .label("Chaque service a sa propre base. Créez une table pour concevoir ses colonnes ; les blocs SQL des routes pourront la lire.")
            .wrap(true)
            .max_width_chars(60)
            .justify(gtk::Justification::Center)
            .build();
        t.add_css_class("empty");
        none.append(&t);
        if let Some(first) = p.services.first() {
            let name = first.name.clone();
            none.append(&link(
                &format!("+ Ajouter une table à {}", first.name),
                clone!(
                    #[strong]
                    ui,
                    move || add_table(&ui, &name)
                ),
            ));
        }
        host.remove(&scroll);
        host.append(&none);
        none.set_hexpand(true);
        return;
    };
    let svc = p
        .service(&sname)
        .cloned()
        .expect("service of the open table");
    let table = svc
        .tables
        .iter()
        .find(|t| t.name == tname)
        .cloned()
        .expect("open table");

    // header
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    head.append(&icon("table", 18, icons::TEXT));
    let owner = gtk::Label::new(Some(&format!("{} ·", svc.name)));
    owner.add_css_class("text-muted");
    head.append(&owner);
    let name_entry = commit_entry(
        &table.name,
        true,
        "",
        false,
        clone!(
            #[strong]
            ui,
            #[strong]
            sname,
            #[strong]
            tname,
            move |v| {
                let taken = ui
                    .state
                    .borrow()
                    .project
                    .service(&sname)
                    .is_some_and(|s| s.tables.iter().any(|t| t.name == v));
                if !is_snake_ident(&v) || v.len() > 63 {
                    say(
                        &ui,
                        "Le nom doit être en snake_case (63 caractères au plus), par exemple order_lines.",
                    );
                } else if taken {
                    say(&ui, &format!("La table {v} existe déjà."));
                } else {
                    edit(&ui, |p| rename::table(p, &sname, &tname, &v));
                    *ui.table_sel.borrow_mut() = Some((sname.clone(), v));
                }
            }
        ),
    );
    name_entry.add_css_class("table-name");
    name_entry.set_width_request(260);
    name_entry.set_hexpand(false);
    head.append(&name_entry);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    head.append(&spacer);
    let used = gtk::Label::new(Some(&format!(
        "{} étape(s) SQL l’utilisent",
        usages(&svc, &table.name)
    )));
    used.add_css_class("text-muted");
    head.append(&used);
    head.append(&icon_button(
        "trash",
        &format!("Supprimer la table {}", table.name),
        true,
        clone!(
            #[strong]
            ui,
            #[strong]
            sname,
            #[strong]
            tname,
            move || {
                edit(&ui, |p| {
                    if let Some(s) = p.services.iter_mut().find(|s| s.name == sname) {
                        s.tables.retain(|t| t.name != tname);
                        for t in &mut s.tables {
                            t.foreign_keys.retain(|f| f.ref_table != tname);
                        }
                    }
                });
                *ui.table_sel.borrow_mut() = None;
            }
        ),
    ));
    main.append(&head);

    // columns grid
    let grid = gtk::Grid::new();
    grid.add_css_class("columns-grid");
    grid.set_column_spacing(8);
    grid.set_row_spacing(0);
    for (i, (title, w)) in [
        ("Nom", 1),
        ("Type", 1),
        ("Nul", 1),
        ("Clé", 1),
        ("Défaut (SQL)", 1),
        ("", 1),
    ]
    .iter()
    .enumerate()
    {
        let l = gtk::Label::builder()
            .label(title.to_uppercase())
            .xalign(0.0)
            .build();
        l.add_css_class("grid-head");
        grid.attach(&l, i as i32, 0, *w, 1);
    }
    let types: Vec<String> = ColumnType::ALL.iter().map(|t| type_label(*t)).collect();
    for (ci, c) in table.columns.iter().enumerate() {
        let r = ci as i32 + 1;
        let (s1, t1) = (sname.clone(), tname.clone());
        let name = commit_entry(
            &c.name,
            true,
            "",
            false,
            clone!(
                #[strong]
                ui,
                #[strong(rename_to = old)]
                c.name,
                move |v| {
                    let taken = ui
                        .state
                        .borrow()
                        .project
                        .service(&s1)
                        .and_then(|s| s.tables.iter().find(|t| t.name == t1))
                        .is_some_and(|t| t.columns.iter().any(|c| c.name == v));
                    if !is_snake_ident(&v) || v.len() > 63 {
                        say(
                            &ui,
                            "Le nom doit être en snake_case (63 caractères au plus).",
                        );
                    } else if taken {
                        say(&ui, &format!("La colonne {v} existe déjà."));
                    } else {
                        edit(&ui, |p| rename_column(p, &s1, &t1, &old, &v));
                    }
                }
            ),
        );
        name.set_hexpand(true);
        grid.attach(&name, 0, r, 1, 1);

        let (s2, t2) = (sname.clone(), tname.clone());
        let ty = choice(
            &types,
            &type_label(c.ty),
            clone!(
                #[strong]
                ui,
                move |v| {
                    if let Some(t) = ColumnType::ALL.iter().find(|t| type_label(**t) == v) {
                        let t = *t;
                        edit(&ui, |p| {
                            if let Some(col) =
                                table_mut(p, &s2, &t2).and_then(|tb| tb.columns.get_mut(ci))
                            {
                                col.ty = t;
                            }
                        });
                    }
                }
            ),
        );
        grid.attach(&ty, 1, r, 1, 1);

        let (s3, t3) = (sname.clone(), tname.clone());
        let nullable = gtk::CheckButton::new();
        nullable.set_active(c.nullable);
        nullable.set_sensitive(!c.primary_key);
        nullable.set_halign(gtk::Align::Center);
        nullable.set_tooltip_text(Some("La colonne accepte NULL"));
        nullable.connect_toggled(clone!(
            #[strong]
            ui,
            move |b| {
                let on = b.is_active();
                edit(&ui, |p| {
                    if let Some(col) = table_mut(p, &s3, &t3).and_then(|tb| tb.columns.get_mut(ci))
                    {
                        col.nullable = on;
                    }
                });
            }
        ));
        grid.attach(&nullable, 2, r, 1, 1);

        let (s4, t4) = (sname.clone(), tname.clone());
        let key = gtk::CheckButton::new();
        key.set_active(c.primary_key);
        key.set_halign(gtk::Align::Center);
        key.set_tooltip_text(Some("Clé primaire"));
        key.connect_toggled(clone!(
            #[strong]
            ui,
            move |b| {
                let on = b.is_active();
                edit(&ui, |p| {
                    if let Some(col) = table_mut(p, &s4, &t4).and_then(|tb| tb.columns.get_mut(ci))
                    {
                        col.primary_key = on;
                        if on {
                            col.nullable = false;
                        }
                    }
                });
            }
        ));
        grid.attach(&key, 3, r, 1, 1);

        let (s5, t5) = (sname.clone(), tname.clone());
        let default = commit_entry(
            c.default.as_deref().unwrap_or(""),
            true,
            "now(), 0, 'x'…",
            true,
            clone!(
                #[strong]
                ui,
                move |v| edit(&ui, |p| {
                    if let Some(col) = table_mut(p, &s5, &t5).and_then(|tb| tb.columns.get_mut(ci))
                    {
                        col.default = Some(v).filter(|v| !v.is_empty());
                    }
                })
            ),
        );
        default.set_hexpand(true);
        grid.attach(&default, 4, r, 1, 1);

        let (s6, t6) = (sname.clone(), tname.clone());
        let trash = icon_button(
            "trash",
            &format!("Supprimer la colonne {}", c.name),
            true,
            clone!(
                #[strong]
                ui,
                #[strong(rename_to = gone)]
                c.name,
                move || edit(&ui, |p| {
                    if let Some(t) = table_mut(p, &s6, &t6) {
                        t.columns.retain(|c| c.name != gone);
                        t.uniques.retain(|u| !u.columns.contains(&gone));
                        t.indexes.retain(|i| !i.columns.contains(&gone));
                        t.foreign_keys.retain(|f| !f.columns.contains(&gone));
                        if t.tenant_column.as_deref() == Some(&gone) {
                            t.tenant_column = None;
                        }
                    }
                })
            ),
        );
        grid.attach(&trash, 5, r, 1, 1);
    }
    main.append(&grid);

    let (s, t) = (sname.clone(), tname.clone());
    main.append(&link(
        "+ Ajouter une colonne",
        clone!(
            #[strong]
            ui,
            move || edit(&ui, |p| {
                if let Some(tb) = table_mut(p, &s, &t) {
                    let taken: Vec<&str> = tb.columns.iter().map(|c| c.name.as_str()).collect();
                    let name = unique("column", &taken);
                    let mut c = Column::new(name, ColumnType::String);
                    c.nullable = true;
                    tb.columns.push(c);
                }
            })
        ),
    ));
    let note = gtk::Label::builder()
        .label("Les tables sont créées au démarrage du service. Ajouter une colonne met à jour une base existante ; renommer ou supprimer une colonne ne change pas les tables déjà créées. Une colonne ajoutée est nulle par défaut, pour ne pas échouer sur des lignes existantes.")
        .xalign(0.0)
        .wrap(true)
        .build();
    note.add_css_class("muted-small");
    main.append(&note);

    constraints(ui, &main, &svc, &table);

    let sql = gtk::Label::builder()
        .label(create_table_sql(&table))
        .xalign(0.0)
        .wrap(true)
        .selectable(true)
        .build();
    sql.add_css_class("sql-preview");
    main.append(&sql);
}

fn add_table(ui: &Shared, service: &str) {
    let mut name = String::new();
    edit(ui, |p| {
        if let Some(s) = p.services.iter_mut().find(|s| s.name == service) {
            let taken: Vec<&str> = s.tables.iter().map(|t| t.name.as_str()).collect();
            name = unique("new_table", &taken);
            let mut id = Column::new("id", ColumnType::Uuid);
            id.primary_key = true;
            id.default = Some("gen_random_uuid()".into());
            s.tables.push(Table::new(name.clone(), vec![id]));
        }
    });
    *ui.table_sel.borrow_mut() = Some((service.to_string(), name));
    let p = ui.state.borrow().project.clone();
    draw(ui, &p);
}

fn constraints(ui: &Shared, main: &gtk::Box, svc: &Service, table: &Table) {
    let (s, t) = (svc.name.clone(), table.name.clone());

    // ---- isolation and soft delete ----
    main.append(&heading("Isolation et suppression"));
    let card = card_box();
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
    let (s1, t1, none1) = (s.clone(), t.clone(), none.clone());
    // A label of the form grid (84 px) and a select that takes the rest, as in the web version.
    card.append(&super::inspector::field(
        "Colonne tenant",
        &choice(
            &options,
            &current,
            clone!(
                #[strong]
                ui,
                move |v| edit(&ui, |p| {
                    if let Some(tb) = table_mut(p, &s1, &t1) {
                        tb.tenant_column = Some(v).filter(|v| *v != none1);
                    }
                })
            ),
        ),
        "",
    ));
    let help = |text: &str| {
        let l = gtk::Label::builder()
            .label(text)
            .xalign(0.0)
            .wrap(true)
            .build();
        l.add_css_class("muted-small");
        l
    };
    card.append(&help("Chaque bloc SQL sur cette table est filtré par cette colonne et chaque insertion la renseigne, à partir de la variable du même nom que le flux a liée (par exemple depuis le jeton vérifié) : on ne peut pas l’oublier, ni lire ou écrire les lignes d’un autre tenant avec ces blocs. Le code Rust libre n’est pas filtré."));
    let soft = gtk::CheckButton::with_label("Suppression logique");
    soft.set_active(table.soft_delete);
    let (s2, t2) = (s.clone(), t.clone());
    soft.connect_toggled(clone!(
        #[strong]
        ui,
        move |b| {
            let on = b.is_active();
            edit(&ui, |p| {
                if let Some(tb) = table_mut(p, &s2, &t2) {
                    tb.soft_delete = on;
                    if on && tb.column(DELETED_AT).is_none() {
                        let mut c = Column::new(DELETED_AT, ColumnType::Timestamp);
                        c.nullable = true;
                        tb.columns.push(c);
                    }
                }
            });
        }
    ));
    card.append(&soft);
    card.append(&help("Une suppression ne retire pas la ligne : elle date deleted_at (colonne ajoutée au besoin), et les lectures, mises à jour et suppressions ignorent les lignes datées. Les contraintes UNIQUE continuent de compter les lignes supprimées."));
    main.append(&card);

    // ---- unique ----
    let add_row = |title: &str, add_text: &str, enabled: bool, f: Box<dyn Fn()>| {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.append(&heading(title));
        let b = link(add_text, f);
        b.set_sensitive(enabled);
        row.append(&b);
        row
    };
    let (s3, t3) = (s.clone(), t.clone());
    main.append(&add_row(
        "Contraintes unique",
        "+ Ajouter une contrainte",
        true,
        Box::new(clone!(
            #[strong]
            ui,
            move || edit(&ui, |p| {
                if let Some(tb) = table_mut(p, &s3, &t3) {
                    let columns: Vec<String> =
                        tb.columns.iter().take(1).map(|c| c.name.clone()).collect();
                    let name = constraint_name(tb, &columns, "uq");
                    tb.uniques.push(UniqueConstraint { name, columns });
                }
            })
        )),
    ));
    for (i, u) in table.uniques.iter().enumerate() {
        let card = card_box();
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let (sa, ta) = (s.clone(), t.clone());
        let n = commit_entry(
            &u.name,
            true,
            "",
            false,
            clone!(
                #[strong]
                ui,
                move |v| edit(&ui, |p| {
                    if let Some(x) = table_mut(p, &sa, &ta).and_then(|tb| tb.uniques.get_mut(i)) {
                        x.name = v;
                    }
                })
            ),
        );
        n.set_width_request(260);
        n.set_hexpand(false);
        head.append(&n);
        let sp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        sp.set_hexpand(true);
        head.append(&sp);
        let (sb, tb_) = (s.clone(), t.clone());
        head.append(&icon_button(
            "trash",
            &format!("Supprimer la contrainte {}", u.name),
            true,
            clone!(
                #[strong]
                ui,
                move || edit(&ui, |p| {
                    if let Some(tb) = table_mut(p, &sb, &tb_)
                        && i < tb.uniques.len()
                    {
                        tb.uniques.remove(i);
                    }
                })
            ),
        ));
        card.append(&head);
        let (sc, tc) = (s.clone(), t.clone());
        card.append(&picker(
            table,
            &u.columns,
            clone!(
                #[strong]
                ui,
                move |cols| edit(&ui, |p| {
                    if let Some(x) = table_mut(p, &sc, &tc).and_then(|tb| tb.uniques.get_mut(i)) {
                        x.columns = cols;
                    }
                })
            ),
        ));
        main.append(&card);
    }

    // ---- indexes ----
    let (s4, t4) = (s.clone(), t.clone());
    main.append(&add_row(
        "Index",
        "+ Ajouter un index",
        true,
        Box::new(clone!(
            #[strong]
            ui,
            move || edit(&ui, |p| {
                if let Some(tb) = table_mut(p, &s4, &t4) {
                    let columns: Vec<String> =
                        tb.columns.iter().take(1).map(|c| c.name.clone()).collect();
                    let name = constraint_name(tb, &columns, "idx");
                    tb.indexes.push(Index {
                        name,
                        columns,
                        unique: false,
                    });
                }
            })
        )),
    ));
    for (i, x) in table.indexes.iter().enumerate() {
        let card = card_box();
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let (sa, ta) = (s.clone(), t.clone());
        let n = commit_entry(
            &x.name,
            true,
            "",
            false,
            clone!(
                #[strong]
                ui,
                move |v| edit(&ui, |p| {
                    if let Some(ix) = table_mut(p, &sa, &ta).and_then(|tb| tb.indexes.get_mut(i)) {
                        ix.name = v;
                    }
                })
            ),
        );
        n.set_width_request(260);
        n.set_hexpand(false);
        head.append(&n);
        let uniq = gtk::CheckButton::with_label("unique");
        uniq.set_active(x.unique);
        let (sb, tb_) = (s.clone(), t.clone());
        uniq.connect_toggled(clone!(
            #[strong]
            ui,
            move |b| {
                let on = b.is_active();
                edit(&ui, |p| {
                    if let Some(ix) = table_mut(p, &sb, &tb_).and_then(|tb| tb.indexes.get_mut(i)) {
                        ix.unique = on;
                    }
                });
            }
        ));
        head.append(&uniq);
        let sp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        sp.set_hexpand(true);
        head.append(&sp);
        let (sd, td) = (s.clone(), t.clone());
        head.append(&icon_button(
            "trash",
            &format!("Supprimer l’index {}", x.name),
            true,
            clone!(
                #[strong]
                ui,
                move || edit(&ui, |p| {
                    if let Some(tb) = table_mut(p, &sd, &td)
                        && i < tb.indexes.len()
                    {
                        tb.indexes.remove(i);
                    }
                })
            ),
        ));
        card.append(&head);
        let (sc, tc) = (s.clone(), t.clone());
        card.append(&picker(
            table,
            &x.columns,
            clone!(
                #[strong]
                ui,
                move |cols| edit(&ui, |p| {
                    if let Some(ix) = table_mut(p, &sc, &tc).and_then(|tb| tb.indexes.get_mut(i)) {
                        ix.columns = cols;
                    }
                })
            ),
        ));
        main.append(&card);
    }

    // ---- foreign keys ----
    let can_reference = svc
        .tables
        .iter()
        .any(|tb| tb.columns.iter().any(|c| c.primary_key));
    let (s5, t5) = (s.clone(), t.clone());
    main.append(&add_row(
        "Clés étrangères",
        "+ Ajouter une clé étrangère",
        can_reference,
        Box::new(clone!(
            #[strong]
            ui,
            move || edit(&ui, |p| {
                let Some(svc) = p.services.iter_mut().find(|x| x.name == s5) else {
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
                let Some(tb) = svc.tables.iter_mut().find(|x| x.name == t5) else {
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
            })
        )),
    ));
    if !can_reference {
        let l = gtk::Label::builder().label("Une clé étrangère référence la clé primaire d’une table du service : définissez-en une d’abord.").xalign(0.0).wrap(true).build();
        l.add_css_class("text-muted");
        l.add_css_class("text-sm");
        main.append(&l);
    }
    for (i, fk) in table.foreign_keys.iter().enumerate() {
        main.append(&fk_card(ui, svc, table, i, fk));
    }
}

fn fk_card(ui: &Shared, svc: &Service, table: &Table, i: usize, fk: &ForeignKey) -> gtk::Box {
    let (s, t) = (svc.name.clone(), table.name.clone());
    let card = card_box();
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let (sa, ta) = (s.clone(), t.clone());
    let n = commit_entry(
        &fk.name,
        true,
        "",
        false,
        clone!(
            #[strong]
            ui,
            move |v| edit(&ui, |p| {
                if let Some(f) = table_mut(p, &sa, &ta).and_then(|tb| tb.foreign_keys.get_mut(i)) {
                    f.name = v;
                }
            })
        ),
    );
    n.set_width_request(260);
    n.set_hexpand(false);
    head.append(&n);
    let sp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    sp.set_hexpand(true);
    head.append(&sp);
    let (sb, tb_) = (s.clone(), t.clone());
    head.append(&icon_button(
        "trash",
        &format!("Supprimer la clé étrangère {}", fk.name),
        true,
        clone!(
            #[strong]
            ui,
            move || edit(&ui, |p| {
                if let Some(tb) = table_mut(p, &sb, &tb_)
                    && i < tb.foreign_keys.len()
                {
                    tb.foreign_keys.remove(i);
                }
            })
        ),
    ));
    card.append(&head);

    let tables: Vec<String> = svc.tables.iter().map(|x| x.name.clone()).collect();
    let (sc, tc) = (s.clone(), t.clone());
    card.append(&labelled(
        "Référence",
        &choice(
            &tables,
            &fk.ref_table,
            clone!(
                #[strong]
                ui,
                move |v| edit(&ui, |p| {
                    let Some(svc) = p.services.iter_mut().find(|x| x.name == sc) else {
                        return;
                    };
                    let target = svc.tables.iter().find(|x| x.name == v).cloned();
                    let Some(tb) = svc.tables.iter_mut().find(|x| x.name == tc) else {
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
                })
            ),
        ),
    ));

    let target = svc.tables.iter().find(|x| x.name == fk.ref_table);
    let mine: Vec<String> = table.columns.iter().map(|c| c.name.clone()).collect();
    let theirs: Vec<String> = target
        .map(|x| x.columns.iter().map(|c| c.name.clone()).collect())
        .unwrap_or_default();
    for (k, c) in fk.columns.iter().enumerate() {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let (sd, td) = (s.clone(), t.clone());
        let left = choice(
            &mine,
            c,
            clone!(
                #[strong]
                ui,
                move |v| edit(&ui, |p| {
                    if let Some(f) =
                        table_mut(p, &sd, &td).and_then(|tb| tb.foreign_keys.get_mut(i))
                        && let Some(x) = f.columns.get_mut(k)
                    {
                        *x = v;
                    }
                })
            ),
        );
        left.set_hexpand(true);
        line.append(&left);
        line.append(&gtk::Label::new(Some("→")));
        let (se, te) = (s.clone(), t.clone());
        let right = choice(
            &theirs,
            fk.ref_columns.get(k).map_or("", String::as_str),
            clone!(
                #[strong]
                ui,
                move |v| edit(&ui, |p| {
                    if let Some(f) =
                        table_mut(p, &se, &te).and_then(|tb| tb.foreign_keys.get_mut(i))
                        && let Some(x) = f.ref_columns.get_mut(k)
                    {
                        *x = v;
                    }
                })
            ),
        );
        right.set_hexpand(true);
        line.append(&right);
        let (sf, tf) = (s.clone(), t.clone());
        let remove = icon_button(
            "trash",
            &format!("Retirer la paire {}", k + 1),
            true,
            clone!(
                #[strong]
                ui,
                move || edit(&ui, |p| {
                    if let Some(f) =
                        table_mut(p, &sf, &tf).and_then(|tb| tb.foreign_keys.get_mut(i))
                        && f.columns.len() > 1
                        && k < f.columns.len()
                    {
                        f.columns.remove(k);
                        if k < f.ref_columns.len() {
                            f.ref_columns.remove(k);
                        }
                    }
                })
            ),
        );
        remove.set_sensitive(fk.columns.len() > 1);
        line.append(&remove);
        card.append(&line);
    }
    let (sg, tg) = (s.clone(), t.clone());
    let b = link(
        "+ Ajouter une paire de colonnes",
        clone!(
            #[strong]
            ui,
            move || edit(&ui, |p| {
                let Some(svc) = p.services.iter_mut().find(|x| x.name == sg) else {
                    return;
                };
                let first_of = |name: &str, svc: &Service| {
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
                    .find(|x| x.name == tg)
                    .and_then(|tb| tb.foreign_keys.get(i))
                    .cloned()
                else {
                    return;
                };
                let mine = first_of(&tg, svc);
                let theirs = first_of(&f.ref_table, svc);
                if let Some(f) = table_mut(p, &sg, &tg).and_then(|tb| tb.foreign_keys.get_mut(i)) {
                    f.columns.push(mine);
                    f.ref_columns.push(theirs);
                }
            })
        ),
    );
    card.append(&b);

    let rules = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    let labels: Vec<String> = ACTIONS.iter().map(|(_, l)| (*l).to_string()).collect();
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
        let (sh, th) = (s.clone(), t.clone());
        rules.append(&labelled(
            title,
            &choice(
                &labels,
                shown,
                clone!(
                    #[strong]
                    ui,
                    move |v| {
                        if let Some((a, _)) = ACTIONS.iter().find(|(_, l)| *l == v) {
                            let a = *a;
                            edit(&ui, |p| {
                                if let Some(f) =
                                    table_mut(p, &sh, &th).and_then(|tb| tb.foreign_keys.get_mut(i))
                                {
                                    if on_delete {
                                        f.on_delete = a;
                                    } else {
                                        f.on_update = a;
                                    }
                                }
                            });
                        }
                    }
                ),
            ),
        ));
    }
    card.append(&rules);
    let _ = glib::ControlFlow::Continue;
    card
}
