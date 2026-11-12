//! Every page of both samples, with every service, topic and message selected and every card open, must draw
//! without panicking; and the main edits (add, undo, redo) must change the project.

use egui_kittest::Harness;
use sb_egui::app::{App, Page};
use sb_egui::pages::canvas::{Sel, select};

const PAGES: [Page; 6] = [Page::Architecture, Page::Endpoint, Page::Flows, Page::Tables, Page::Mcp, Page::Code];

fn harness(sample: &'static str) -> Harness<'static, App> {
    // SAFETY: set before the app starts its threads.
    unsafe {
        std::env::set_var("SB_NO_UPDATE", "1");
        std::env::set_var("SB_OPEN_ALL", "1");
    }
    let mut h = Harness::builder().with_size([1480.0, 920.0]).build_eframe(move |cc| {
        cc.egui_ctx.set_missing_glyph_policy(eframe::egui::epaint::text::MissingGlyphPolicy::Tofu);
        let mut app = App::new(cc, None);
        app.load_sample(sample);
        app
    });
    h.ctx.set_missing_glyph_policy(eframe::egui::epaint::text::MissingGlyphPolicy::Tofu);
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
    sels.extend(p.services.iter().map(|s| Some(Sel::Service(s.name.clone()))));
    sels.extend(p.topics.iter().map(|t| Some(Sel::Topic(t.name.clone()))));
    sels.extend(p.messages.iter().map(|m| Some(Sel::Messages(m.name.clone()))));
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
