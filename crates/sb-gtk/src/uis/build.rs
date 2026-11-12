//! Builds the window and wires its parts together.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib::{self, clone};
use gtk::pango;
use gtk::prelude::*;
use sb_app::Analysis;

use super::edit::{guard, redo, save, undo};
use super::files::{pick_project, save_as};
use super::header::toolbar as header;
use super::project::{changed, watch_file};
use super::state::State;
use super::ui::{Shared, TITLE, Ui};
use super::updates::{check_for_update, offer_update};
use super::widgets::{list_page, scrolled};

pub fn build(app: &gtk::Application) {
    apply_theme();
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(TITLE)
        .default_width(1480)
        .default_height(920)
        .build();

    let stack = gtk::Stack::new();
    let canvas = super::canvas::create();
    stack.add_titled(&canvas.page, Some("architecture"), "Architecture");
    let services = list_page(&stack, "services", "Services");
    let contracts = list_page(&stack, "contracts", "Messages et topics");
    let flows = list_page(&stack, "flows", "Flows");
    let endpoint_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    endpoint_page.add_css_class("endpoint");
    let endpoint_tree = gtk::Box::new(gtk::Orientation::Vertical, 2);
    endpoint_tree.add_css_class("tree");
    let endpoint_side = gtk::Box::new(gtk::Orientation::Vertical, 8);
    endpoint_side.add_css_class("palette");
    endpoint_side.append(&{
        let t = gtk::Label::builder().label("STRUCTURE").xalign(0.0).build();
        t.add_css_class("palette-title");
        t
    });
    endpoint_side.append(&endpoint_tree);
    let endpoint_scroll = gtk::ScrolledWindow::new();
    endpoint_scroll.set_child(Some(&endpoint_side));
    endpoint_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    endpoint_scroll.set_width_request(228);
    endpoint_scroll.set_min_content_width(228);
    endpoint_scroll.set_max_content_width(228);
    endpoint_scroll.set_propagate_natural_width(false);
    endpoint_scroll.set_hexpand(false);
    endpoint_scroll.add_css_class("palette-scroll");
    endpoint_page.set_hexpand(true);
    let endpoint_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    endpoint_row.append(&endpoint_scroll);
    endpoint_row.append(&endpoint_page);
    stack.add_titled(&endpoint_row, Some("endpoint"), "API");
    let tables = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tables.set_vexpand(true);
    stack.add_titled(&tables, Some("tables"), "Tables");

    let files = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let code_path = gtk::Label::builder()
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(pango::EllipsizeMode::Start)
        .build();
    code_path.add_css_class("code-path");
    let code = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .top_margin(14)
        .bottom_margin(14)
        .left_margin(18)
        .right_margin(18)
        .pixels_below_lines(5)
        .build();
    code.add_css_class("code");
    let copy = gtk::Button::new();
    let copy_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    copy_content.append(&super::icons::icon("copy", 14, super::icons::TEXT));
    copy_content.append(&gtk::Label::new(Some("Copier")));
    copy.set_child(Some(&copy_content));
    copy.add_css_class("ghost");
    let code_head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    code_head.add_css_class("code-head");
    code_head.append(&code_path);
    code_head.append(&copy);
    let code_main = gtk::Box::new(gtk::Orientation::Vertical, 0);
    code_main.add_css_class("code-main");
    code_main.append(&code_head);
    code_main.append(&scrolled(&code, 0));
    let tree_scroll = scrolled(&files, 236);
    tree_scroll.add_css_class("code-tree");
    tree_scroll.set_hexpand(false);
    tree_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    tree_scroll.set_width_request(236);
    let split = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    split.append(&tree_scroll);
    split.append(&code_main);
    stack.add_titled(&split, Some("code"), "Code généré");

    let problems = gtk::ListBox::new();
    problems.set_selection_mode(gtk::SelectionMode::None);

    let mcp = gtk::Box::new(gtk::Orientation::Vertical, 16);
    mcp.set_margin_top(22);
    mcp.set_margin_bottom(48);
    mcp.set_margin_start(0);
    mcp.set_margin_end(0);
    // The web version centers a column of 812 px: the side margins follow the width of the page.
    let mcp_scroll = scrolled(&mcp, 0);
    mcp_scroll.add_tick_callback({
        let mcp = mcp.clone();
        move |scroll, _| {
            let side = ((scroll.width() - 812) / 2).max(0);
            if mcp.margin_start() != side {
                mcp.set_margin_start(side);
                mcp.set_margin_end(side);
            }
            glib::ControlFlow::Continue
        }
    });
    stack.add_titled(&mcp_scroll, Some("mcp"), "IA (MCP)");

    let title = gtk::Entry::new();
    let dirty_mark = gtk::Label::new(Some("●"));
    dirty_mark.add_css_class("dirty");
    dirty_mark.set_tooltip_text(Some("Modifications non enregistrées"));
    dirty_mark.set_visible(false);
    let file_label = gtk::Label::new(None);
    file_label.add_css_class("file");

    let toast = gtk::Label::builder()
        .halign(gtk::Align::End)
        .valign(gtk::Align::End)
        .margin_end(18)
        .margin_bottom(18)
        .wrap(true)
        .max_width_chars(60)
        .visible(false)
        .build();
    toast.add_css_class("toast");

    let update_button = gtk::Button::new();
    update_button.set_visible(false);

    let pill = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    pill.set_valign(gtk::Align::Center);
    pill.add_css_class("pill");
    let pill_text = gtk::Label::new(Some("Valide"));
    pill.append(&gtk::Image::new());
    pill.append(&pill_text);
    let export_button = gtk::Button::new();

    let problems_head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let problems_panel = gtk::Revealer::new();
    problems_panel.set_transition_type(gtk::RevealerTransitionType::SlideUp);

    let ui: Shared = Rc::new(Ui {
        window: window.clone(),
        state: RefCell::new(State {
            project: sb_model::sample::orders(),
            path: None,
            stamp: 0,
            analysis: Analysis {
                diagnostics: vec![],
                files: vec![],
            },
            generation: 0,
            update: None,
            dirty: false,
            undo: vec![],
            redo: vec![],
        }),
        title,
        dirty_mark,
        file_label,
        toast: toast.clone(),
        problems_head: problems_head.clone(),
        problems_panel: problems_panel.clone(),
        update_button: update_button.clone(),
        pill,
        pill_text,
        export_button,
        stack: stack.clone(),
        canvas,
        services,
        contracts,
        flows,
        tables,
        endpoint: RefCell::new(None),
        endpoint_page,
        endpoint_tree,
        table_sel: RefCell::new(None),
        files: files.clone(),
        current_file: RefCell::new(None),
        code_path,
        code: code.clone(),
        problems: problems.clone(),
        mcp,
        syncing: Cell::new(false),
    });

    stack.set_vexpand(true);
    let problems_scroll = scrolled(&problems, 0);
    problems_scroll.set_vexpand(false);
    problems_scroll.set_max_content_height(200);
    problems_scroll.set_propagate_natural_height(true);
    problems_panel.set_child(Some(&problems_scroll));
    let problems_button = gtk::Button::new();
    problems_button.add_css_class("problems-head");
    problems_button.set_child(Some(&problems_head));
    problems_button.connect_clicked(clone!(
        #[weak]
        problems_panel,
        move |_| problems_panel.set_reveal_child(!problems_panel.reveals_child())
    ));
    let bottom = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bottom.add_css_class("problems");
    bottom.set_vexpand(false);
    bottom.append(&problems_button);
    bottom.append(&problems_panel);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&header(&ui));
    root.append(&stack);
    root.append(&bottom);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&root));
    overlay.add_overlay(&toast);
    window.set_child(Some(&overlay));

    copy.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| super::codeview::copy(&ui)
    ));
    update_button.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| offer_update(&ui)
    ));

    // An AI client edits the file on disk: reload it when it changes.
    glib::timeout_add_seconds_local(
        2,
        clone!(
            #[strong]
            ui,
            move || {
                watch_file(&ui);
                glib::ControlFlow::Continue
            }
        ),
    );

    super::canvas::wire(&ui);
    shortcuts(&ui);
    // Closing with unsaved changes asks first; the dialog's answer closes the window for real.
    window.connect_close_request(clone!(
        #[strong]
        ui,
        move |_| {
            if !ui.state.borrow().dirty {
                return glib::Propagation::Proceed;
            }
            guard(&ui, |ui| {
                ui.state.borrow_mut().dirty = false;
                ui.window.close();
            });
            glib::Propagation::Stop
        }
    ));

    if let Some(path) = super::run::opening() {
        match sb_app::load_project(path) {
            Ok(project) => super::project::set_project(&ui, project, Some(path.clone())),
            Err(e) => super::widgets::fail(&ui, &e),
        }
    }

    window.present();
    // Development aid, with `SB_SCREENSHOT`: start with a service or a topic selected.
    if let Ok(name) = std::env::var("SB_SELECT") {
        let sel = if let Some(message) = name.strip_prefix("msg:") {
            ui.canvas.state.borrow_mut().open.insert(format!("msg:{message}"));
            super::canvas::Sel::Messages(message.to_string())
        } else if name.contains('.') {
            super::canvas::Sel::Topic(name)
        } else {
            super::canvas::Sel::Service(name)
        };
        super::canvas::select(&ui, Some(sel));
    }
    super::screenshot::schedule(&window, &stack);
    changed(&ui);
    check_for_update(&ui, false);
}

type Binding = (&'static str, fn(&Shared));

fn shortcuts(ui: &Shared) {
    let controller = gtk::ShortcutController::new();
    controller.set_scope(gtk::ShortcutScope::Global);
    let bindings: [Binding; 5] = [
        ("<Control>s", save),
        ("<Control><Shift>s", save_as),
        ("<Control>o", |ui| guard(ui, pick_project)),
        ("<Control>z", undo),
        ("<Control><Shift>z", redo),
    ];
    for (keys, action) in bindings {
        let ui = ui.clone();
        controller.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string(keys),
            Some(gtk::CallbackAction::new(move |_, _| {
                action(&ui);
                glib::Propagation::Stop
            })),
        ));
    }
    ui.window.add_controller(controller);
}

/// The colours and shapes of the web version, over GTK's dark theme.
fn apply_theme() {
    if let Some(settings) = gtk::Settings::default() {
        prefer_dark(&settings);
    }
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let provider = gtk::CssProvider::new();
    provider.load_from_data(
        &[
            include_str!("../../data/css/00-theme.css"),
            include_str!("../../data/css/10-utilities.css"),
            include_str!("../../data/css/30-toolbar.css"),
            include_str!("../../data/css/40-widgets.css"),
            include_str!("../../data/css/50-architecture.css"),
            include_str!("../../data/css/55-inspector.css"),
            include_str!("../../data/css/60-tables.css"),
            include_str!("../../data/css/65-codeview.css"),
            include_str!("../../data/css/70-mcp.css"),
            include_str!("../../data/css/80-endpoint.css"),
        ]
        .concat(),
    );
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

/// GTK 4.20 replaced `gtk-application-prefer-dark-theme` (which now warns) by an interface colour scheme.
fn prefer_dark(settings: &gtk::Settings) {
    const DARK: i32 = 2;
    if let Some(spec) = settings.find_property("gtk-interface-color-scheme")
        && let Some(class) = glib::EnumClass::with_type(spec.value_type())
        && let Some(dark) = class.to_value(DARK)
    {
        settings.set_property_from_value("gtk-interface-color-scheme", &dark);
    } else {
        #[allow(deprecated)]
        settings.set_gtk_application_prefer_dark_theme(true);
    }
}
