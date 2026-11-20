//! Every page of both samples, with every service, topic and message selected and every card open, must draw
//! without panicking; and the main edits (add, undo, redo) must change the project.

use egui_kittest::Harness;
use sb_egui::app::{App, Page};
use sb_egui::pages::canvas::{Sel, select};

const PAGES: [Page; 8] = [
    Page::Architecture,
    Page::Endpoint,
    Page::Mail,
    Page::Tables,
    Page::Mcp,
    Page::Infra,
    Page::Projects,
    Page::Code,
];

fn harness(sample: &'static str) -> Harness<'static, App> {
    // SAFETY: set before the app starts its threads.
    unsafe {
        std::env::set_var("SB_OPEN_ALL", "1");
        // Neither the projects nor the settings of the person who runs the tests.
        std::env::set_var("SB_NO_RESTORE", "1");
        let home = std::env::temp_dir().join(format!("sb-smoke-{}", std::process::id()));
        std::env::set_var("XDG_CONFIG_HOME", home.join("config"));
        std::env::set_var("XDG_DATA_HOME", home.join("data"));
        for var in [
            "SB_S3_ENDPOINT",
            "SB_S3_BUCKET",
            "SB_S3_ACCESS_KEY",
            "SB_S3_SECRET_KEY",
        ] {
            std::env::remove_var(var);
        }
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

/// A mail of the mail service opens the Mail page on it, which draws (the preview paints every kind of block, including
/// the columns); an edit of the design reaches the HTML and the text derived from it, and the editor follows an undo.
/// Leaving the page closes the editor.
#[test]
fn the_mail_page_opens_draws_derives_and_closes() {
    let mut h = harness("showcase");
    let si = h
        .state()
        .project()
        .services
        .iter()
        .position(|s| s.name == "Mailer")
        .unwrap();
    // The inspector of the mail service lists its mails (no form, no link to another page).
    select(h.state_mut(), Some(Sel::Service("Mailer".into())));
    h.run_steps(2);
    sb_egui::pages::mailcanvas::open(h.state_mut(), si, "accounts.registered");
    h.run_steps(3);
    assert_eq!(h.state().page, Page::Mail);
    assert!(h.state().mail_editor.is_some());
    h.state_mut().edit(|p| {
        let t = &mut p.services[si].templates[0];
        let sb_model::MailBlock::Heading { text, .. } = &mut t.design.as_mut().unwrap().blocks[0]
        else {
            panic!("the first block is a heading")
        };
        *text = "Bonjour {{ name }}".into();
    });
    h.run_steps(3);
    let p = h.state().project();
    let (html, text) = p.services[si].templates[0].bodies();
    assert!(html.contains("Bonjour {{ name }}</h1>"));
    assert!(text.starts_with("Bonjour {{ name }}\n"));
    assert!(
        sb_model::validate(&p)
            .iter()
            .all(|d| d.severity != sb_model::Severity::Error)
    );
    // Undone, the editor shows the mail as it was; nothing it held is written back over the undo.
    h.state_mut().undo();
    h.run_steps(3);
    let (html, _) = h.state().project().services[si].templates[0].bodies();
    assert!(!html.contains("Bonjour {{ name }}</h1>"));
    h.state_mut().page = Page::Architecture;
    h.run_steps(2);
    assert!(h.state().mail_editor.is_none());
    let (html, _) = h.state().project().services[si].templates[0].bodies();
    assert!(!html.contains("Bonjour {{ name }}</h1>"));
}

/// The inspector of the mail service declares senders inline, and the mail editor's rule picks one; both draw and the
/// project stays valid, the choice following an undo.
#[test]
fn the_mail_service_declares_senders_and_a_rule_picks_one() {
    let mut h = harness("showcase");
    let si = h
        .state()
        .project()
        .services
        .iter()
        .position(|s| s.name == "Mailer")
        .unwrap();
    select(h.state_mut(), Some(Sel::Service("Mailer".into())));
    h.state_mut()
        .edit(|p| sb_egui::pages::mail::add_sender(p, si));
    h.run_steps(3);
    assert_eq!(h.state().project().services[si].senders.len(), 1);
    h.state_mut().edit(|p| {
        p.services[si].consumers[0].mail.as_mut().unwrap().sender = Some("expediteur".into());
    });
    sb_egui::pages::mailcanvas::open(h.state_mut(), si, "accounts.registered");
    h.run_steps(3);
    assert!(
        sb_model::validate(&h.state().project())
            .iter()
            .all(|d| d.severity != sb_model::Severity::Error)
    );
    h.state_mut().undo();
    h.run_steps(3);
    assert!(
        h.state().project().services[si].consumers[0]
            .mail
            .as_ref()
            .unwrap()
            .sender
            .is_none()
    );
}

/// Edits made in the editor in quick succession are one entry of the history.
#[test]
fn grouped_edits_are_one_undo() {
    let mut h = harness("orders");
    let before = h.state().project().name.clone();
    for name in ["a", "ab", "abc"] {
        h.state_mut().edit_grouped("name", |p| p.name = name.into());
    }
    assert_eq!(h.state().project().name, "abc");
    h.state_mut().undo();
    h.run_steps(1);
    assert_eq!(h.state().project().name, before);
}
