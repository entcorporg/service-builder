//! Editing the project: every change goes through `edit`, which keeps the history and the unsaved mark.

use gtk::glib::clone;
use sb_model::Project;

use super::files::save_as;
use super::project::{changed, refresh_title, stamp_of};
use super::ui::Shared;
use super::widgets::{fail, say};

const HISTORY: usize = 100;

/// Applies `change` to the project. A change that changes nothing leaves no trace.
pub fn edit(ui: &Shared, change: impl FnOnce(&mut Project)) {
    {
        let mut s = ui.state.borrow_mut();
        let before = s.project.clone();
        change(&mut s.project);
        // Field numbers are frozen as they are given, so that a later deletion never renumbers the others.
        s.project.freeze_field_numbers();
        if s.project == before {
            return;
        }
        s.undo.push(before);
        if s.undo.len() > HISTORY {
            s.undo.remove(0);
        }
        s.redo.clear();
        s.dirty = true;
    }
    changed(ui);
}

pub fn undo(ui: &Shared) {
    step(ui, true);
}

pub fn redo(ui: &Shared) {
    step(ui, false);
}

fn step(ui: &Shared, back: bool) {
    {
        let mut s = ui.state.borrow_mut();
        let s = &mut *s;
        let (from, to) = if back {
            (&mut s.undo, &mut s.redo)
        } else {
            (&mut s.redo, &mut s.undo)
        };
        let Some(previous) = from.pop() else {
            return;
        };
        to.push(std::mem::replace(&mut s.project, previous));
        s.dirty = true;
    }
    changed(ui);
}

/// Saves in place; a project that has no file yet asks for one.
pub fn save(ui: &Shared) {
    let Some(path) = ui.state.borrow().path.clone() else {
        return save_as(ui);
    };
    let saved = sb_app::save_project(&path, &ui.state.borrow().project);
    match saved {
        Ok(()) => {
            {
                let mut s = ui.state.borrow_mut();
                s.stamp = stamp_of(&path);
                s.dirty = false;
            }
            refresh_title(ui);
            say(ui, &format!("Enregistré dans {}", path.display()));
        }
        Err(e) => fail(ui, &e),
    }
}

/// Runs `then`, after asking what to do with the unsaved changes if there are any.
pub fn guard(ui: &Shared, then: impl FnOnce(&Shared) + 'static) {
    if !ui.state.borrow().dirty {
        return then(ui);
    }
    gtk::AlertDialog::builder()
        .message("Le projet a des modifications non enregistrées")
        .detail("Que faire de ces modifications ?")
        .buttons(["Annuler", "Abandonner", "Enregistrer"])
        .cancel_button(0)
        .default_button(2)
        .build()
        .choose(
            Some(&ui.window),
            gtk::gio::Cancellable::NONE,
            clone!(
                #[strong]
                ui,
                move |choice| match choice.ok() {
                    Some(1) => then(&ui),
                    Some(2) => {
                        save(&ui);
                        // A project without a file opens the file dialog: the user retries once it is saved.
                        if !ui.state.borrow().dirty {
                            then(&ui)
                        }
                    }
                    _ => {}
                }
            ),
        );
}
