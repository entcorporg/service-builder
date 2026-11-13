//! Every page of both samples, with every service, topic and message selected and every card open, must draw
//! without panicking; and the main edits (add, undo, redo) must change the project.

use egui_kittest::Harness;
use sb_egui::app::{App, Page};
use sb_egui::pages::canvas::{Sel, select};

const PAGES: [Page; 7] = [
    Page::Architecture,
    Page::Endpoint,
    Page::Flows,
    Page::Tables,
    Page::Mcp,
    Page::Pods,
    Page::Code,
];

fn harness(sample: &'static str) -> Harness<'static, App> {
    // SAFETY: set before the app starts its threads.
    unsafe {
        std::env::set_var("SB_NO_UPDATE", "1");
        std::env::set_var("SB_OPEN_ALL", "1");
        std::env::set_var("SB_NO_ENGINE", "1");
    }
    let mut h = Harness::builder()
        .with_size([1480.0, 920.0])
        .build_eframe(move |cc| {
            cc.egui_ctx
                .set_missing_glyph_policy(eframe::egui::epaint::text::MissingGlyphPolicy::Tofu);
            let mut app = App::new(cc, None);
            app.load_sample(sample);
            app
        });
    h.ctx
        .set_missing_glyph_policy(eframe::egui::epaint::text::MissingGlyphPolicy::Tofu);
    for _ in 0..4 {
        h.run_steps(2);
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    h
}

fn sweep(sample: &'static str) {
    let mut h = harness(sample);
    let p = h.state().project();
    let mut sels: Vec<Option<Sel>> = vec![None];
    sels.extend(
        p.services
            .iter()
            .map(|s| Some(Sel::Service(s.name.clone()))),
    );
    sels.extend(p.topics.iter().map(|t| Some(Sel::Topic(t.name.clone()))));
    sels.extend(
        p.messages
            .iter()
            .map(|m| Some(Sel::Messages(m.name.clone()))),
    );
    for sel in sels {
        select(h.state_mut(), sel);
        h.state_mut().page = Page::Architecture;
        h.run_steps(2);
    }
    for svc in &p.services {
        for r in &svc.routes {
            sb_egui::pages::endpoint::open(h.state_mut(), &svc.name, &r.name);
            h.run_steps(2);
        }
    }
    for page in PAGES {
        h.state_mut().page = page;
        h.run_steps(2);
    }
}

#[test]
fn orders_every_page() {
    sweep("orders");
}

#[test]
fn showcase_every_page() {
    sweep("showcase");
}

#[test]
fn edit_undo_redo() {
    let mut h = harness("orders");
    let before = h.state().project().services.len();
    sb_egui::pages::palette::add_service(h.state_mut(), sb_model::ServiceKind::Standard);
    h.run_steps(2);
    assert_eq!(h.state().project().services.len(), before + 1);
    assert!(h.state().state.dirty);
    h.state_mut().undo();
    h.run_steps(2);
    assert_eq!(h.state().project().services.len(), before);
    h.state_mut().redo();
    h.run_steps(2);
    assert_eq!(h.state().project().services.len(), before + 1);
    sb_egui::pages::palette::add_topic(h.state_mut());
    h.run_steps(2);
    for page in PAGES {
        h.state_mut().page = page;
        h.run_steps(2);
    }
}

#[test]
fn drag_a_node_then_undo_with_the_keyboard() {
    use eframe::egui::{Key, Modifiers, pos2};
    let mut h = harness("orders");
    let position = |h: &Harness<'static, App>| {
        let p = h.state().project();
        p.services
            .iter()
            .find(|s| s.name == "Payments")
            .map(|s| (s.position.x, s.position.y))
    };
    let before = position(&h);
    // The Payments node, where the orders sample is fitted in a 1480×920 window.
    h.hover_at(pos2(620.0, 454.0));
    h.run_steps(2);
    h.drag_at(pos2(620.0, 454.0));
    h.run_steps(1);
    for i in 1..=10 {
        h.hover_at(pos2(620.0 + 4.0 * i as f32, 454.0 + 12.0 * i as f32));
        h.run_steps(1);
    }
    h.drop_at(pos2(660.0, 574.0));
    h.run_steps(3);
    let moved = position(&h);
    assert_ne!(moved, before, "the drag moves the node");
    // The canvas has the pointer and maybe the focus: the shortcuts must still work.
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(position(&h), before, "Ctrl+Z undoes the move");
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    h.run_steps(2);
    assert_eq!(position(&h), moved, "Ctrl+Shift+Z redoes it");
}
