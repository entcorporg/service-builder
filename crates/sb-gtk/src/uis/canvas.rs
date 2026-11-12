//! The Architecture page: services and topics as nodes, the arrows the project implies, and the inspector.
//!
//! Dragging a node moves it (one change in the history, at the end of the drag); dragging the background pans;
//! the wheel zooms around the pointer; a double click edits. The arrows are drawn, never edited: producers and
//! consumers are set in the forms, and the steps do the rest.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::cairo::{Context, FontSlant, FontWeight};
use gtk::glib::clone;
use gtk::prelude::*;
use sb_model::Position;

use super::edit::edit;
use super::editors::{service_form, topic_form};
use super::graph::{self, Edge, EdgeKind, Graph, HEADER, Id, Node, ROW, Tag, connect};
use super::ui::Shared;
use super::widgets::say;

#[derive(Clone, PartialEq)]
pub enum Sel {
    Service(String),
    Topic(String),
    /// The schemas (events, proto, API) in the inspector, the one of this name open.
    Messages(String),
}

enum Drag {
    Node {
        id: Id,
        /// Where the node was, in canvas units.
        origin: (f64, f64),
        /// Where it is now.
        now: (f64, f64),
    },
    Pan {
        origin: (f64, f64),
    },
    /// An arrow being drawn from the output handle of a node, up to this point of the canvas.
    Connect {
        from: Id,
        start: (f64, f64),
        to: (f64, f64),
    },
}

pub struct CanvasState {
    pub graph: Graph,
    pub offset: (f64, f64),
    pub zoom: f64,
    pub selected: Option<Sel>,
    drag: Option<Drag>,
    /// The cards of the inspector that are open, by id, so that an edit does not close them.
    pub open: std::collections::HashSet<String>,
    moved: bool,
    pointer: (f64, f64),
    /// The whole graph is fitted in the view at every paint, until the user moves or zooms it.
    auto_fit: bool,
}

pub struct Canvas {
    pub page: gtk::Box,
    pub area: gtk::DrawingArea,
    /// The tiles and the structure tree, in the left column.
    pub tiles: gtk::Box,
    pub tree: gtk::Box,
    /// Zoom buttons and the legend, over the bottom left corner of the canvas.
    pub controls: gtk::Box,
    pub minimap: gtk::DrawingArea,
    pub inspector: gtk::Box,
    pub state: Rc<RefCell<CanvasState>>,
}

// ---- colours (ui/src/styles.css) ----

fn rgb(cr: &Context, hex: u32, alpha: f64) {
    let c = |shift: u32| f64::from((hex >> shift) & 0xff) / 255.0;
    cr.set_source_rgba(c(16), c(8), c(0), alpha);
}

const BG: u32 = 0x0f1119;
const PANEL: u32 = 0x1b1f31;
const BORDER: u32 = 0x3a4270;
const TEXT: u32 = 0xe8eaf6;
const MUTED: u32 = 0x8f97bd;
const SERVICE: u32 = 0x7886ff;
const TOPIC: u32 = 0xffb04a;
const OK: u32 = 0x3ecf8e;
const ERR: u32 = 0xff6678;
const ACCENT_LINE: u32 = 0xff7a45;
const WARN: u32 = 0xffc53d;

fn tag_color(t: Tag) -> u32 {
    match t {
        Tag::Rpc => SERVICE,
        Tag::Get => OK,
        Tag::Post => TOPIC,
        Tag::OtherHttp => 0xc77dff,
        Tag::Kafka => OK,
        Tag::Mail => 0xff4d6d,
        Tag::Job => MUTED,
    }
}

pub fn create() -> Canvas {
    let state = Rc::new(RefCell::new(CanvasState {
        graph: Graph::default(),
        offset: (40.0, 40.0),
        zoom: 1.0,
        selected: None,
        drag: None,
        open: Default::default(),
        moved: false,
        pointer: (0.0, 0.0),
        auto_fit: true,
    }));
    let area = gtk::DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .focusable(true)
        .build();
    let mini: Rc<std::cell::OnceCell<gtk::DrawingArea>> = Rc::default();
    area.set_draw_func({
        let state = state.clone();
        let mini = mini.clone();
        move |_, cr, w, h| {
            paint(cr, f64::from(w), f64::from(h), &mut state.borrow_mut());
            if let Some(m) = mini.get() {
                m.queue_draw();
            }
        }
    });

    let inspector = gtk::Box::new(gtk::Orientation::Vertical, 8);
    inspector.add_css_class("inspector");

    // The left column: tiles to add, then the structure.
    let tiles = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let tree = gtk::Box::new(gtk::Orientation::Vertical, 2);
    tree.add_css_class("tree");
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    column.add_css_class("palette");
    column.append(&tiles);
    column.append(&tree);
    let palette = gtk::ScrolledWindow::new();
    palette.set_child(Some(&column));
    palette.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    palette.set_width_request(228);
    palette.set_min_content_width(228);
    palette.set_max_content_width(228);
    palette.set_propagate_natural_width(false);
    palette.set_hexpand(false);
    palette.add_css_class("palette-scroll");

    let controls = gtk::Box::new(gtk::Orientation::Vertical, 8);
    controls.set_halign(gtk::Align::Start);
    controls.set_valign(gtk::Align::End);
    controls.set_margin_start(14);
    controls.set_margin_bottom(14);

    let minimap = gtk::DrawingArea::new();
    minimap.set_content_width(200);
    minimap.set_content_height(150);
    minimap.set_halign(gtk::Align::End);
    minimap.set_valign(gtk::Align::End);
    minimap.set_margin_end(14);
    minimap.set_margin_bottom(14);
    minimap.add_css_class("minimap");
    let _ = mini.set(minimap.clone());
    minimap.set_draw_func({
        let state = state.clone();
        let area = area.clone();
        move |_, cr, w, h| {
            minimap_paint(
                cr,
                f64::from(w),
                f64::from(h),
                &state.borrow(),
                (f64::from(area.width()), f64::from(area.height())),
            )
        }
    });

    let stage = gtk::Overlay::new();
    stage.set_child(Some(&area));
    stage.add_overlay(&controls);
    stage.add_overlay(&minimap);
    stage.set_hexpand(true);

    let inspector_scroll = gtk::ScrolledWindow::new();
    inspector_scroll.set_child(Some(&inspector));
    // `External`: the panel keeps its 410 px whatever the steps inside ask for.
    inspector_scroll.set_policy(gtk::PolicyType::External, gtk::PolicyType::Automatic);
    inspector_scroll.set_width_request(410);
    inspector_scroll.set_min_content_width(410);
    inspector_scroll.set_max_content_width(410);
    inspector_scroll.set_propagate_natural_width(false);
    inspector_scroll.set_hexpand(false);
    inspector_scroll.add_css_class("inspector-panel");

    let page = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    page.append(&palette);
    page.append(&stage);
    page.append(&inspector_scroll);
    page.set_vexpand(true);
    Canvas {
        page,
        area,
        tiles,
        tree,
        controls,
        minimap,
        inspector,
        state,
    }
}

/// The whole graph in a small frame, with the part of it the view shows.
fn minimap_paint(cr: &Context, w: f64, h: f64, st: &CanvasState, view: (f64, f64)) {
    rgb(cr, 0x151826, 1.0);
    round_rect(cr, 0.5, 0.5, w - 1.0, h - 1.0, 8.0);
    let _ = cr.fill_preserve();
    rgb(cr, 0x2a3050, 1.0);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    let Some((x0, y0, x1, y1)) = st.graph.bounds() else {
        return;
    };
    // The view, in canvas units, joins the graph so that both fit.
    let (vx0, vy0) = (-st.offset.0 / st.zoom, -st.offset.1 / st.zoom);
    let (vx1, vy1) = (vx0 + view.0 / st.zoom, vy0 + view.1 / st.zoom);
    let (bx0, by0, bx1, by1) = (x0.min(vx0), y0.min(vy0), x1.max(vx1), y1.max(vy1));
    let pad = 10.0;
    let k = ((w - 2.0 * pad) / (bx1 - bx0).max(1.0)).min((h - 2.0 * pad) / (by1 - by0).max(1.0));
    let (ox, oy) = (
        (w - (bx1 - bx0) * k) / 2.0 - bx0 * k,
        (h - (by1 - by0) * k) / 2.0 - by0 * k,
    );
    for n in &st.graph.nodes {
        rgb(
            cr,
            if matches!(n.id, Id::Topic(_)) {
                TOPIC
            } else {
                0x6e7bff
            },
            1.0,
        );
        cr.rectangle(
            ox + n.x * k,
            oy + n.y * k,
            (n.w * k).max(3.0),
            (n.h * k).max(3.0),
        );
        let _ = cr.fill();
    }
    rgb(cr, 0x7886ff, 0.9);
    cr.set_line_width(1.5);
    cr.rectangle(ox + vx0 * k, oy + vy0 * k, (vx1 - vx0) * k, (vy1 - vy0) * k);
    let _ = cr.stroke();
}

// ---- drawing ----

fn round_rect(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}

fn font(cr: &Context, size: f64, mono: bool, bold: bool) {
    cr.select_font_face(
        if mono { "monospace" } else { "sans-serif" },
        FontSlant::Normal,
        if bold {
            FontWeight::Bold
        } else {
            FontWeight::Normal
        },
    );
    cr.set_font_size(size);
}

/// The text cut with an ellipsis so that it fits `max` wide.
fn fitted(cr: &Context, text: &str, max: f64) -> String {
    let wide = |s: &str| cr.text_extents(s).map_or(0.0, |e| e.x_advance());
    if wide(text) <= max {
        return text.to_string();
    }
    let mut cut: String = text.to_string();
    while !cut.is_empty() && wide(&format!("{cut}…")) > max {
        cut.pop();
    }
    format!("{cut}…")
}

fn paint(cr: &Context, w: f64, h: f64, st: &mut CanvasState) {
    rgb(cr, BG, 1.0);
    let _ = cr.paint();
    if st.auto_fit && w > 10.0 {
        fit(st, w, h);
    }
    // Dots of the grid.
    let step = 24.0 * st.zoom;
    if step > 6.0 {
        rgb(cr, 0x2a3050, 0.9);
        let (ox, oy) = (st.offset.0.rem_euclid(step), st.offset.1.rem_euclid(step));
        let mut y = oy;
        while y < h {
            let mut x = ox;
            while x < w {
                cr.rectangle(x, y, 1.5, 1.5);
                x += step;
            }
            y += step;
        }
        let _ = cr.fill();
    }

    cr.translate(st.offset.0, st.offset.1);
    cr.scale(st.zoom, st.zoom);

    let moved = |n: &Node| match &st.drag {
        Some(Drag::Node { id, now, .. }) if *id == n.id => (now.0, now.1),
        _ => (n.x, n.y),
    };
    let anchor = |id: Id| st.graph.node(id).map(|n| (moved(n), n.w, n.h));
    for e in &st.graph.edges {
        if let (Some(a), Some(b)) = (anchor(e.from), anchor(e.to)) {
            edge(cr, e, a, b);
        }
    }
    let selected = |n: &Node| match (&st.selected, n.id) {
        (Some(Sel::Service(name)), Id::Service(_)) | (Some(Sel::Topic(name)), Id::Topic(_)) => {
            *name == n.name
        }
        _ => false,
    };
    let target = match &st.drag {
        Some(Drag::Connect { to, .. }) => st.graph.hit(to.0, to.1),
        _ => None,
    };
    for n in &st.graph.nodes {
        let (x, y) = moved(n);
        node(cr, n, x, y, selected(n) || target == Some(n.id));
        // The output handle, where an arrow starts.
        rgb(
            cr,
            if matches!(n.id, Id::Topic(_)) {
                TOPIC
            } else {
                SERVICE
            },
            0.9,
        );
        cr.arc(x + n.w, y + n.h / 2.0, 5.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    }
    if let Some(Drag::Connect { from, start, to }) = &st.drag
        && st.graph.node(*from).is_some()
    {
        rgb(cr, ACCENT_LINE, 0.95);
        cr.set_line_width(2.0);
        cr.set_dash(&[6.0, 4.0], 0.0);
        cr.move_to(start.0, start.1);
        cr.line_to(to.0, to.1);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
    }
}

type Anchor = ((f64, f64), f64, f64);

fn edge(cr: &Context, e: &Edge, a: Anchor, b: Anchor) {
    let ((ax, ay), aw, ah) = a;
    let ((bx, by), bw, bh) = b;
    let (color, dashed) = match e.kind {
        EdgeKind::Publish => (TOPIC, false),
        EdgeKind::Consume => (OK, false),
        EdgeKind::Rpc => (SERVICE, true),
    };
    // Horizontal arrows leave on the right and arrive on the left; calls leave at the bottom and arrive on top.
    let (p0, c0, c1, p1) = if e.kind == EdgeKind::Rpc {
        let (sx, sy) = (ax + aw / 2.0, ay + ah);
        let (tx, ty) = (bx + bw / 2.0, by);
        let d = ((ty - sy).abs() / 2.0).max(50.0);
        ((sx, sy), (sx, sy + d), (tx, ty - d), (tx, ty))
    } else {
        let (sx, sy) = (ax + aw, ay + ah / 2.0);
        let (tx, ty) = (bx, by + bh / 2.0);
        let d = ((tx - sx).abs() / 2.0).max(50.0);
        ((sx, sy), (sx + d, sy), (tx - d, ty), (tx, ty))
    };
    rgb(cr, color, 0.9);
    cr.set_line_width(1.6);
    cr.set_dash(if dashed { &[6.0, 5.0] } else { &[] }, 0.0);
    cr.move_to(p0.0, p0.1);
    cr.curve_to(c0.0, c0.1, c1.0, c1.1, p1.0, p1.1);
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
    // The head follows the last bit of the curve.
    let angle = (p1.1 - c1.1).atan2(p1.0 - c1.0);
    cr.move_to(p1.0, p1.1);
    cr.line_to(
        p1.0 - 9.0 * (angle - 0.4).cos(),
        p1.1 - 9.0 * (angle - 0.4).sin(),
    );
    cr.line_to(
        p1.0 - 9.0 * (angle + 0.4).cos(),
        p1.1 - 9.0 * (angle + 0.4).sin(),
    );
    cr.close_path();
    let _ = cr.fill();
    if !e.label.is_empty() {
        font(cr, 10.0, true, false);
        let (mx, my) = ((p0.0 + p1.0) / 2.0 + 6.0, (p0.1 + p1.1) / 2.0);
        cr.move_to(mx, my);
        rgb(cr, color, 1.0);
        let _ = cr.show_text(&e.label);
    }
}

fn node(cr: &Context, n: &Node, x: f64, y: f64, selected: bool) {
    let topic = matches!(n.id, Id::Topic(_));
    let accent = if topic { TOPIC } else { SERVICE };
    round_rect(cr, x, y, n.w, n.h, 10.0);
    rgb(cr, PANEL, 1.0);
    let _ = cr.fill_preserve();
    rgb(cr, if selected { accent } else { BORDER }, 1.0);
    cr.set_line_width(if selected { 2.0 } else { 1.0 });
    let _ = cr.stroke();

    // Header: tinted with the colour of the kind, clipped to the rounded corners.
    cr.save().ok();
    round_rect(cr, x, y, n.w, n.h, 10.0);
    cr.clip();
    rgb(cr, accent, 0.16);
    cr.rectangle(x, y, n.w, HEADER);
    let _ = cr.fill();
    cr.restore().ok();

    font(cr, 13.0, topic, true);
    rgb(cr, if topic { TOPIC } else { TEXT }, 1.0);
    let badge_w = if n.badge.is_empty() { 0.0 } else { 52.0 };
    let title = fitted(cr, &n.name, n.w - 28.0 - badge_w - 40.0);
    cr.move_to(x + 14.0, y + 22.0);
    let _ = cr.show_text(&title);

    let mut right = x + n.w - 12.0;
    if !n.badge.is_empty() {
        font(cr, 11.0, true, false);
        rgb(cr, MUTED, 1.0);
        let width = cr.text_extents(&n.badge).map_or(40.0, |e| e.x_advance());
        right -= width;
        cr.move_to(right, y + 22.0);
        let _ = cr.show_text(&n.badge);
        right -= 8.0;
    }
    for (count, color) in [(n.errors, ERR), (n.warnings, WARN)] {
        if count > 0 {
            right -= 20.0;
            rgb(cr, color, 1.0);
            cr.arc(right + 8.0, y + 17.0, 8.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            font(cr, 10.0, false, true);
            rgb(cr, 0x1a0d0a, 1.0);
            cr.move_to(right + 5.0, y + 21.0);
            let _ = cr.show_text(&count.to_string());
            right -= 2.0;
        }
    }

    if topic {
        font(cr, 12.0, true, false);
        rgb(cr, TEXT, 1.0);
        cr.move_to(x + 14.0, y + HEADER + 18.0);
        let message = fitted(cr, &n.meta[0], n.w - 28.0);
        let _ = cr.show_text(&message);
        font(cr, 10.5, false, false);
        rgb(cr, MUTED, 1.0);
        cr.move_to(x + 14.0, y + HEADER + 36.0);
        let meta = fitted(cr, &n.meta[1], n.w - 28.0);
        let _ = cr.show_text(&meta);
        return;
    }
    if n.rows.is_empty() {
        font(cr, 11.0, false, false);
        rgb(cr, MUTED, 1.0);
        cr.move_to(x + 14.0, y + HEADER + 18.0);
        let _ = cr.show_text("vide");
    }
    for (i, r) in n.rows.iter().enumerate() {
        let ry = y + HEADER + 6.0 + i as f64 * ROW;
        font(cr, 9.0, true, true);
        let tw = cr.text_extents(r.label).map_or(30.0, |e| e.x_advance()) + 10.0;
        round_rect(cr, x + 12.0, ry + 2.0, tw, 15.0, 4.0);
        rgb(cr, tag_color(r.tag), 0.18);
        let _ = cr.fill();
        rgb(cr, tag_color(r.tag), 1.0);
        cr.move_to(x + 17.0, ry + 13.0);
        let _ = cr.show_text(r.label);
        font(cr, 11.5, true, false);
        rgb(cr, TEXT, 1.0);
        cr.move_to(x + 12.0 + tw + 8.0, ry + 14.0);
        let text = fitted(cr, &r.text, n.w - tw - 44.0);
        let _ = cr.show_text(&text);
    }
}

// ---- view ----

/// Fits the whole graph in a view of this size.
fn fit(st: &mut CanvasState, w: f64, h: f64) {
    let Some((x0, y0, x1, y1)) = st.graph.bounds() else {
        st.offset = (40.0, 40.0);
        st.zoom = 1.0;
        return;
    };
    let (bw, bh) = ((x1 - x0).max(1.0), (y1 - y0).max(1.0));
    st.zoom = ((w - 80.0) / bw).min((h - 80.0) / bh).clamp(0.3, 1.2);
    st.offset = (
        (w - bw * st.zoom) / 2.0 - x0 * st.zoom,
        (h - bh * st.zoom) / 2.0 - y0 * st.zoom,
    );
}

fn zoom_at(ui: &Shared, factor: f64, at: (f64, f64)) {
    {
        let mut st = ui.canvas.state.borrow_mut();
        st.auto_fit = false;
        let z = (st.zoom * factor).clamp(0.25, 2.5);
        let k = z / st.zoom;
        st.offset = (
            at.0 - (at.0 - st.offset.0) * k,
            at.1 - (at.1 - st.offset.1) * k,
        );
        st.zoom = z;
    }
    ui.canvas.area.queue_draw();
    ui.canvas.minimap.queue_draw();
}

fn selection_of(ui: &Shared, id: Id) -> Option<Sel> {
    let s = ui.state.borrow();
    match id {
        Id::Service(i) => s
            .project
            .services
            .get(i)
            .map(|x| Sel::Service(x.name.clone())),
        Id::Topic(i) => s.project.topics.get(i).map(|x| Sel::Topic(x.name.clone())),
    }
}

fn index_of(ui: &Shared, sel: &Sel) -> Option<Id> {
    let s = ui.state.borrow();
    match sel {
        Sel::Service(n) => s
            .project
            .services
            .iter()
            .position(|x| x.name == *n)
            .map(Id::Service),
        Sel::Topic(n) => s
            .project
            .topics
            .iter()
            .position(|x| x.name == *n)
            .map(Id::Topic),
        // Not a node of the graph.
        Sel::Messages(_) => None,
    }
}

fn edit_node(ui: &Shared, id: Id) {
    match id {
        Id::Service(i) => service_form(ui, Some(i)),
        Id::Topic(i) => topic_form(ui, Some(i)),
    }
}

/// Wires the toolbar and the mouse. Called once, when the window is built.
pub fn wire(ui: &Shared) {
    let c = &ui.canvas;
    super::palette::fill_tiles(ui, &c.tiles);
    let square = |name: &str, tip: &str| {
        let b = gtk::Button::new();
        b.set_child(Some(&super::icons::icon(name, 16, super::icons::TEXT)));
        b.set_tooltip_text(Some(tip));
        b.add_css_class("zoom");
        b
    };
    let zoom_in = square("plus", "Zoom avant");
    zoom_in.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            let (w, h) = (ui.canvas.area.width(), ui.canvas.area.height());
            zoom_at(&ui, 1.25, (f64::from(w) / 2.0, f64::from(h) / 2.0))
        }
    ));
    let zoom_out = square("minus", "Zoom arrière");
    zoom_out.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            let (w, h) = (ui.canvas.area.width(), ui.canvas.area.height());
            zoom_at(&ui, 0.8, (f64::from(w) / 2.0, f64::from(h) / 2.0))
        }
    ));
    let fit_button = square("graph", "Tout afficher");
    fit_button.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            ui.canvas.state.borrow_mut().auto_fit = true;
            ui.canvas.area.queue_draw();
        }
    ));
    let zoom = gtk::Box::new(gtk::Orientation::Vertical, 0);
    zoom.add_css_class("zoom-box");
    for b in [&zoom_in, &zoom_out, &fit_button] {
        zoom.append(b);
    }
    zoom.set_halign(gtk::Align::Start);
    c.controls.append(&zoom);
    let legend = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    legend.add_css_class("legend");
    legend.set_halign(gtk::Align::Start);
    for (text, color) in [
        ("publie", "#ffb04a"),
        ("consomme", "#3ecf8e"),
        ("appel gRPC", "#7886ff"),
    ] {
        let item = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        item.append(&super::icons::icon("minus", 16, color));
        item.append(&gtk::Label::new(Some(text)));
        legend.append(&item);
    }
    c.controls.append(&legend);

    let drag = gtk::GestureDrag::new();
    drag.set_button(1);
    drag.connect_drag_begin(clone!(
        #[strong]
        ui,
        move |_, x, y| {
            ui.canvas.area.grab_focus();
            // An output handle starts an arrow, before anything else.
            let handle = {
                let st = ui.canvas.state.borrow();
                let (wx, wy) = ((x - st.offset.0) / st.zoom, (y - st.offset.1) / st.zoom);
                st.graph
                    .nodes
                    .iter()
                    .find(|n| {
                        (wx - (n.x + n.w)).abs() <= 9.0 && (wy - (n.y + n.h / 2.0)).abs() <= 9.0
                    })
                    .map(|n| (n.id, (n.x + n.w, n.y + n.h / 2.0)))
            };
            if let Some((from, start)) = handle {
                let mut st = ui.canvas.state.borrow_mut();
                st.moved = false;
                st.drag = Some(Drag::Connect {
                    from,
                    start,
                    to: start,
                });
                return;
            }
            let hit = {
                let st = ui.canvas.state.borrow();
                let (wx, wy) = ((x - st.offset.0) / st.zoom, (y - st.offset.1) / st.zoom);
                st.graph.hit(wx, wy).and_then(|id| {
                    st.graph
                        .node(id)
                        .map(|n| (id, (n.x, n.y), (wx - n.x, wy - n.y)))
                })
            };
            let mut st = ui.canvas.state.borrow_mut();
            st.moved = false;
            st.drag = Some(match hit {
                Some((id, origin, _)) => Drag::Node {
                    id,
                    origin,
                    now: origin,
                },
                None => Drag::Pan { origin: st.offset },
            });
            drop(st);
            if let Some((id, _, _)) = hit {
                let sel = selection_of(&ui, id);
                select(&ui, sel);
            }
        }
    ));
    drag.connect_drag_update(clone!(
        #[strong]
        ui,
        move |_, dx, dy| {
            let mut st = ui.canvas.state.borrow_mut();
            let z = st.zoom;
            if dx.abs() + dy.abs() > 3.0 {
                st.moved = true;
                st.auto_fit = false;
            }
            match &mut st.drag {
                Some(Drag::Connect { start, to, .. }) => *to = (start.0 + dx / z, start.1 + dy / z),
                Some(Drag::Node { origin, now, .. }) => {
                    *now = (origin.0 + dx / z, origin.1 + dy / z)
                }
                Some(Drag::Pan { origin }) => {
                    let o = *origin;
                    st.offset = (o.0 + dx, o.1 + dy)
                }
                None => {}
            }
            drop(st);
            ui.canvas.area.queue_draw();
        }
    ));
    drag.connect_drag_end(clone!(
        #[strong]
        ui,
        move |_, _, _| {
            let (drag, moved) = {
                let mut st = ui.canvas.state.borrow_mut();
                (st.drag.take(), st.moved)
            };
            match drag {
                Some(Drag::Node { id, now, .. }) if moved => {
                    let at = Position {
                        x: now.0.round(),
                        y: now.1.round(),
                    };
                    edit(&ui, move |p| match id {
                        Id::Service(i) => {
                            if let Some(s) = p.services.get_mut(i) {
                                s.position = at;
                            }
                        }
                        Id::Topic(i) => {
                            if let Some(t) = p.topics.get_mut(i) {
                                t.position = at;
                            }
                        }
                    });
                }
                Some(Drag::Connect { from, to, .. }) => {
                    let target = ui.canvas.state.borrow().graph.hit(to.0, to.1);
                    if let Some(target) = target.filter(|t| *t != from) {
                        let mut message = None;
                        edit(&ui, |p| message = Some(connect(p, from, target)));
                        match message {
                            Some(Ok(text)) => say(&ui, &text),
                            Some(Err(e)) => say(&ui, &e),
                            None => {}
                        }
                    } else if moved {
                        say(&ui, "Lâchez la flèche sur un topic (depuis un service) ou sur un service (depuis un topic).");
                    }
                }
                // A click on the background: nothing is selected any more.
                Some(Drag::Pan { .. }) if !moved => {
                    select(&ui, None);
                }
                _ => {}
            }
            ui.canvas.area.queue_draw();
        }
    ));
    c.area.add_controller(drag);

    let click = gtk::GestureClick::new();
    click.set_button(1);
    click.connect_pressed(clone!(
        #[strong]
        ui,
        move |_, n, x, y| {
            if n < 2 {
                return;
            }
            let hit = {
                let st = ui.canvas.state.borrow();
                st.graph
                    .hit((x - st.offset.0) / st.zoom, (y - st.offset.1) / st.zoom)
            };
            if let Some(id) = hit {
                edit_node(&ui, id);
            }
        }
    ));
    c.area.add_controller(click);

    let motion = gtk::EventControllerMotion::new();
    motion.connect_motion(clone!(
        #[strong]
        ui,
        move |_, x, y| ui.canvas.state.borrow_mut().pointer = (x, y)
    ));
    c.area.add_controller(motion);

    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    scroll.connect_scroll(clone!(
        #[strong]
        ui,
        move |_, _, dy| {
            let at = ui.canvas.state.borrow().pointer;
            zoom_at(&ui, 1.1f64.powf(-dy), at);
            gtk::glib::Propagation::Stop
        }
    ));
    c.area.add_controller(scroll);
    inspect(ui);
}

/// Selects a service or a topic (or nothing), in the canvas, the tree and the inspector.
pub fn select(ui: &Shared, sel: Option<Sel>) {
    ui.canvas.state.borrow_mut().selected = sel;
    inspect(ui);
    super::palette::draw_tree(ui);
    ui.canvas.area.queue_draw();
}

/// Rebuilds the graph from the project and the last analysis.
pub fn refresh(ui: &Shared) {
    let graph = {
        let s = ui.state.borrow();
        graph::build(&s.project, &s.analysis.diagnostics)
    };
    ui.canvas.state.borrow_mut().graph = graph;
    // A selected element that was deleted or renamed is not selected any more.
    let gone = ui
        .canvas
        .state
        .borrow()
        .selected
        .clone()
        .is_some_and(|sel| !matches!(sel, Sel::Messages(_)) && index_of(ui, &sel).is_none());
    if gone {
        ui.canvas.state.borrow_mut().selected = None;
    }
    inspect(ui);
    super::palette::draw_tree(ui);
    ui.canvas.area.queue_draw();
}

fn inspect(ui: &Shared) {
    super::inspector::inspect(ui);
}
