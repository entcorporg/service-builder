//! The Architecture page: services and topics as nodes, the arrows the project implies, and the inspector.
//!
//! Dragging a node moves it (one change in the history, at the end of the drag); dragging the background pans;
//! the wheel zooms around the pointer; a double click edits. The arrows are drawn, never edited: producers and
//! consumers are set in the forms, and the steps do the rest.

use std::rc::Rc;
use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, Galley, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2,
    epaint::RectShape, text::LayoutJob,
};
use sb_app::Analysis;
use sb_model::{Position, Project};

use super::editors::{service_form, topic_form};
use super::graph::{self, Edge, EdgeKind, Graph, HEADER, Id, Node, ROW, Tag, connect};
use crate::app::App;
use crate::icons;
use crate::theme::{self, *};

#[derive(Clone, PartialEq, Debug)]
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
    pub selected: Option<Sel>,
    graph: Graph,
    /// The project and the analysis the graph was built from.
    built_from: Option<(Rc<Project>, Rc<Analysis>)>,
    offset: (f64, f64),
    zoom: f64,
    drag: Option<Drag>,
    /// The whole graph is fitted in the view at every paint, until the user moves or zooms it.
    auto_fit: bool,
}

impl Default for CanvasState {
    fn default() -> Self {
        CanvasState {
            selected: None,
            graph: Graph::default(),
            built_from: None,
            offset: (40.0, 40.0),
            zoom: 1.0,
            drag: None,
            auto_fit: true,
        }
    }
}

fn tag_color(t: Tag) -> Color32 {
    match t {
        Tag::Rpc => SERVICE,
        Tag::Get => OK,
        Tag::Post => TOPIC,
        Tag::OtherHttp => hex(0xc77dff),
        Tag::Kafka => OK,
        Tag::Mail => ACCENT2,
        Tag::Job => MUTED,
    }
}

/// Selects a service or a topic (or nothing), in the canvas, the tree and the inspector.
pub fn select(app: &mut App, sel: Option<Sel>) {
    app.canvas.selected = sel;
}

pub fn page(app: &mut App, ui: &mut Ui) {
    egui::Panel::left("palette")
        .exact_size(228.0)
        .resizable(false)
        .frame(
            egui::Frame::new()
                .fill(PANEL)
                .inner_margin(egui::Margin::symmetric(12, 14)),
        )
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    super::palette::tiles(app, ui);
                    ui.add_space(6.0);
                    super::palette::tree(app, ui);
                });
        });
    egui::Panel::right("inspector")
        .exact_size(410.0)
        .resizable(false)
        .frame(
            egui::Frame::new()
                .fill(PANEL)
                .inner_margin(egui::Margin::symmetric(16, 6)),
        )
        .show(ui, |ui| {
            // The panel keeps its 410 px whatever the steps inside ask for (GTK's `External` policy): the
            // inspector is drawn in a child of a fixed rectangle, whose size does not reach the panel.
            let rect = ui.available_rect_before_wrap();
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            child.set_clip_rect(rect.expand2(egui::vec2(16.0, 0.0)).intersect(ui.clip_rect()));
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_width(rect.width())
                .show(&mut child, |ui| {
                    // Room for the scroll bar on the right.
                    ui.set_max_width(rect.width() - 12.0);
                    super::inspector::show(app, ui)
                });
            ui.allocate_rect(rect, egui::Sense::hover());
        });
    egui::CentralPanel::no_frame().show(ui, |ui| stage(app, ui));
}

// ---- the stage: the graph and its overlays ----

/// Canvas units to screen.
struct View {
    origin: Pos2,
    offset: (f64, f64),
    zoom: f64,
}

impl View {
    fn pt(&self, x: f64, y: f64) -> Pos2 {
        self.origin
            + Vec2::new(
                (self.offset.0 + x * self.zoom) as f32,
                (self.offset.1 + y * self.zoom) as f32,
            )
    }
    fn s(&self, v: f64) -> f32 {
        (v * self.zoom) as f32
    }
    fn radius(&self, v: f64) -> CornerRadius {
        CornerRadius::same(self.s(v).round().clamp(0.0, 255.0) as u8)
    }
    /// Screen to canvas units.
    fn canvas(&self, p: Pos2) -> (f64, f64) {
        (
            (f64::from(p.x - self.origin.x) - self.offset.0) / self.zoom,
            (f64::from(p.y - self.origin.y) - self.offset.1) / self.zoom,
        )
    }
}

fn is_topic(id: Id) -> bool {
    matches!(id, Id::Topic(_))
}

fn selection_of(g: &Graph, id: Id) -> Option<Sel> {
    g.node(id).map(|n| match id {
        Id::Service(_) => Sel::Service(n.name.clone()),
        Id::Topic(_) => Sel::Topic(n.name.clone()),
    })
}

/// The node whose output handle is at this point of the canvas.
fn handle_at(g: &Graph, wx: f64, wy: f64) -> Option<(Id, (f64, f64))> {
    g.nodes
        .iter()
        .find(|n| (wx - (n.x + n.w)).abs() <= 9.0 && (wy - (n.y + n.h / 2.0)).abs() <= 9.0)
        .map(|n| (n.id, (n.x + n.w, n.y + n.h / 2.0)))
}

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

fn zoom_at(st: &mut CanvasState, factor: f64, at: (f64, f64)) {
    st.auto_fit = false;
    let z = (st.zoom * factor).clamp(0.25, 2.5);
    let k = z / st.zoom;
    st.offset = (
        at.0 - (at.0 - st.offset.0) * k,
        at.1 - (at.1 - st.offset.1) * k,
    );
    st.zoom = z;
}

fn stage(app: &mut App, ui: &mut Ui) {
    let rect = ui.available_rect_before_wrap();
    let resp = ui.allocate_rect(rect, Sense::click_and_drag());
    let local = |p: Pos2| (f64::from(p.x - rect.min.x), f64::from(p.y - rect.min.y));
    let view_of = |st: &CanvasState| View {
        origin: rect.min,
        offset: st.offset,
        zoom: st.zoom,
    };

    // ---- the mouse, on the graph as it was drawn ----
    let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
    if resp.hovered()
        && let Some(at) = resp.hover_pos()
    {
        let factor = 1.1f64.powf(f64::from(scroll) / 50.0) * f64::from(pinch);
        if (factor - 1.0).abs() > 1e-6 {
            zoom_at(&mut app.canvas, factor, local(at));
        }
    }
    if resp.drag_started_by(egui::PointerButton::Primary)
        && let Some(press) = ui.input(|i| i.pointer.press_origin())
    {
        let st = &mut app.canvas;
        let (wx, wy) = view_of(st).canvas(press);
        let handle = handle_at(&st.graph, wx, wy);
        let hit = st.graph.hit(wx, wy).and_then(|id| {
            st.graph
                .node(id)
                .map(|n| (id, (n.x, n.y), selection_of(&st.graph, id)))
        });
        st.drag = Some(if let Some((from, start)) = handle {
            Drag::Connect {
                from,
                start,
                to: start,
            }
        } else if let Some((id, origin, _)) = &hit {
            Drag::Node {
                id: *id,
                origin: *origin,
                now: *origin,
            }
        } else {
            Drag::Pan { origin: st.offset }
        });
        if handle.is_none()
            && let Some((_, _, sel)) = hit
        {
            select(app, sel);
        }
    }
    if resp.dragged()
        && let (Some(cur), Some(press)) = (
            resp.interact_pointer_pos(),
            ui.input(|i| i.pointer.press_origin()),
        )
    {
        let st = &mut app.canvas;
        let (dx, dy) = (f64::from(cur.x - press.x), f64::from(cur.y - press.y));
        let z = st.zoom;
        st.auto_fit = false;
        match &mut st.drag {
            Some(Drag::Connect { start, to, .. }) => *to = (start.0 + dx / z, start.1 + dy / z),
            Some(Drag::Node { origin, now, .. }) => *now = (origin.0 + dx / z, origin.1 + dy / z),
            Some(Drag::Pan { origin }) => {
                let o = *origin;
                st.offset = (o.0 + dx, o.1 + dy);
            }
            None => {}
        }
    }
    if resp.drag_stopped() {
        match app.canvas.drag.take() {
            Some(Drag::Node { id, now, .. }) => {
                let at = Position {
                    x: now.0.round(),
                    y: now.1.round(),
                };
                app.edit(move |p| match id {
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
                let target = app.canvas.graph.hit(to.0, to.1);
                if let Some(target) = target.filter(|t| *t != from) {
                    let mut message = None;
                    app.edit(|p| message = Some(connect(p, from, target)));
                    match message {
                        Some(Ok(text)) => app.say(&text),
                        Some(Err(e)) => app.say(&e),
                        None => {}
                    }
                } else {
                    app.say(
                        "Lâchez la flèche sur un topic (depuis un service) ou sur un service (depuis un topic).",
                    );
                }
            }
            _ => {}
        }
    }
    if (resp.clicked() || resp.double_clicked())
        && let Some(at) = resp.interact_pointer_pos()
    {
        let st = &app.canvas;
        let (wx, wy) = view_of(st).canvas(at);
        let on_handle = handle_at(&st.graph, wx, wy).is_some();
        let hit = st.graph.hit(wx, wy);
        if resp.double_clicked() {
            match hit {
                Some(Id::Service(i)) => service_form(app, Some(i)),
                Some(Id::Topic(i)) => topic_form(app, Some(i)),
                None => {}
            }
        } else if !on_handle {
            // A click on the background: nothing is selected any more.
            let sel = hit.and_then(|id| selection_of(&st.graph, id));
            select(app, sel);
        }
    }
    if app.canvas.drag.is_none()
        && resp.hovered()
        && let Some(at) = resp.hover_pos()
    {
        let (wx, wy) = view_of(&app.canvas).canvas(at);
        if handle_at(&app.canvas.graph, wx, wy).is_some() {
            ui.set_cursor_icon(egui::CursorIcon::Crosshair);
        }
    }
    if matches!(app.canvas.drag, Some(Drag::Connect { .. })) {
        ui.set_cursor_icon(egui::CursorIcon::Crosshair);
    }

    // ---- the graph, rebuilt when the project or the analysis changed ----
    let (project, analysis) = (app.project(), app.analysis());
    let fresh = matches!(&app.canvas.built_from,
        Some((p, a)) if Rc::ptr_eq(p, &project) && Rc::ptr_eq(a, &analysis));
    if !fresh {
        app.canvas.graph = graph::build(&project, &analysis.diagnostics);
        app.canvas.built_from = Some((project.clone(), analysis));
    }
    // A selected element that was deleted or renamed is not selected any more.
    let gone = match &app.canvas.selected {
        Some(Sel::Service(n)) => !project.services.iter().any(|s| s.name == *n),
        Some(Sel::Topic(n)) => !project.topics.iter().any(|t| t.name == *n),
        _ => false,
    };
    if gone {
        app.canvas.selected = None;
    }
    if app.canvas.auto_fit && rect.width() > 10.0 {
        fit(
            &mut app.canvas,
            f64::from(rect.width()),
            f64::from(rect.height()),
        );
    }

    // ---- painting ----
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, BG);
    dots(&painter, rect, &app.canvas);
    paint_graph(&painter, &view_of(&app.canvas), &app.canvas);

    // ---- the overlays, over the canvas ----
    let pad = 14.0;
    let legend = legend(ui, rect, pad);
    let zoom_rect = Rect::from_min_size(
        Pos2::new(rect.left() + pad, legend.top() - 8.0 - 88.0),
        Vec2::new(34.0, 88.0),
    );
    let size = (f64::from(rect.width()), f64::from(rect.height()));
    zoom_buttons(ui, zoom_rect, &mut app.canvas, (size.0 / 2.0, size.1 / 2.0));
    let mini = Rect::from_min_size(
        Pos2::new(rect.right() - pad - 200.0, rect.bottom() - pad - 150.0),
        Vec2::new(200.0, 150.0),
    );
    ui.interact(
        mini,
        ui.make_persistent_id("minimap"),
        Sense::click_and_drag(),
    );
    minimap(&painter, mini, &app.canvas, size);
}

/// The dots of the grid.
fn dots(p: &egui::Painter, rect: Rect, st: &CanvasState) {
    let step = 24.0 * st.zoom;
    if step <= 6.0 {
        return;
    }
    let color = alpha(BORDER, 230);
    let (ox, oy) = (st.offset.0.rem_euclid(step), st.offset.1.rem_euclid(step));
    let mut y = oy;
    while y < f64::from(rect.height()) {
        let mut x = ox;
        while x < f64::from(rect.width()) {
            let at = rect.min + Vec2::new(x as f32, y as f32);
            p.rect_filled(Rect::from_min_size(at, Vec2::splat(1.5)), 0.0, color);
            x += step;
        }
        y += step;
    }
}

fn paint_graph(p: &egui::Painter, v: &View, st: &CanvasState) {
    let moved = |n: &Node| match &st.drag {
        Some(Drag::Node { id, now, .. }) if *id == n.id => *now,
        _ => (n.x, n.y),
    };
    let anchor = |id: Id| st.graph.node(id).map(|n| (moved(n), n.w, n.h));
    for e in &st.graph.edges {
        if let (Some(a), Some(b)) = (anchor(e.from), anchor(e.to)) {
            edge(p, v, e, a, b);
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
        node(p, v, n, x, y, selected(n) || target == Some(n.id));
        // The output handle, where an arrow starts.
        let color = if is_topic(n.id) { TOPIC } else { SERVICE };
        p.circle_filled(v.pt(x + n.w, y + n.h / 2.0), v.s(5.0), alpha(color, 230));
    }
    if let Some(Drag::Connect { from, start, to }) = &st.drag
        && st.graph.node(*from).is_some()
    {
        let stroke = Stroke::new(v.s(2.0), alpha(ACCENT, 242));
        let pts = [v.pt(start.0, start.1), v.pt(to.0, to.1)];
        p.extend(Shape::dashed_line(&pts, stroke, v.s(6.0), v.s(4.0)));
    }
}

fn layout(
    p: &egui::Painter,
    v: &View,
    s: &str,
    size: f64,
    mono: bool,
    color: Color32,
    max: f64,
) -> Arc<Galley> {
    let font = if mono {
        theme::mono(v.s(size))
    } else {
        theme::prop(v.s(size))
    };
    let mut job = LayoutJob::simple_singleline(s.to_string(), font, color);
    job.wrap.max_width = v.s(max).max(1.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    p.layout_job(job)
}

/// The text of `size` canvas units, left-aligned at `x`, centred vertically on `cy`, cut with an ellipsis at `max`
/// canvas units wide.
struct Label<'a> {
    x: f64,
    cy: f64,
    text: &'a str,
    size: f64,
    mono: bool,
    color: Color32,
    max: f64,
}

impl Label<'_> {
    fn draw(&self, p: &egui::Painter, v: &View) {
        let g = layout(p, v, self.text, self.size, self.mono, self.color, self.max);
        let at = v.pt(self.x, self.cy) - Vec2::new(0.0, g.size().y / 2.0);
        p.galley(at, g, self.color);
    }
}

/// The width of the text in canvas units.
fn width_of(p: &egui::Painter, v: &View, s: &str, size: f64, mono: bool) -> f64 {
    f64::from(layout(p, v, s, size, mono, TEXT, 10_000.0).size().x) / v.zoom
}

type Anchor = ((f64, f64), f64, f64);

fn edge(p: &egui::Painter, v: &View, e: &Edge, a: Anchor, b: Anchor) {
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
    let line = alpha(color, 230);
    let stroke = Stroke::new(v.s(1.6), line);
    if dashed {
        let sampled: Vec<Pos2> = (0..=48)
            .map(|i| {
                let t = f64::from(i) / 48.0;
                let u = 1.0 - t;
                let f = |a: f64, b: f64, c: f64, d: f64| {
                    u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
                };
                v.pt(f(p0.0, c0.0, c1.0, p1.0), f(p0.1, c0.1, c1.1, p1.1))
            })
            .collect();
        p.extend(Shape::dashed_line(&sampled, stroke, v.s(6.0), v.s(5.0)));
    } else {
        p.add(egui::epaint::CubicBezierShape::from_points_stroke(
            [
                v.pt(p0.0, p0.1),
                v.pt(c0.0, c0.1),
                v.pt(c1.0, c1.1),
                v.pt(p1.0, p1.1),
            ],
            false,
            Color32::TRANSPARENT,
            stroke,
        ));
    }
    // The head follows the last bit of the curve.
    let angle = (p1.1 - c1.1).atan2(p1.0 - c1.0);
    let back = |da: f64| {
        v.pt(
            p1.0 - 9.0 * (angle + da).cos(),
            p1.1 - 9.0 * (angle + da).sin(),
        )
    };
    p.add(Shape::convex_polygon(
        vec![v.pt(p1.0, p1.1), back(-0.4), back(0.4)],
        line,
        Stroke::NONE,
    ));
    if !e.label.is_empty() {
        Label {
            x: (p0.0 + p1.0) / 2.0 + 6.0,
            cy: (p0.1 + p1.1) / 2.0 - 3.5,
            text: &e.label,
            size: 10.0,
            mono: true,
            color,
            max: 10_000.0,
        }
        .draw(p, v);
    }
}

fn node(p: &egui::Painter, v: &View, n: &Node, x: f64, y: f64, selected: bool) {
    let topic = is_topic(n.id);
    let accent = if topic { TOPIC } else { SERVICE };
    let rect = Rect::from_min_size(v.pt(x, y), Vec2::new(v.s(n.w), v.s(n.h)));
    let radius = v.radius(10.0);
    p.rect_filled(rect, radius, PANEL2);

    // Header: tinted with the colour of the kind, clipped to the rounded corners.
    let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), v.s(HEADER)));
    p.add(RectShape::filled(
        header,
        CornerRadius {
            sw: 0,
            se: 0,
            ..radius
        },
        alpha(accent, 41),
    ));
    let (border, width) = if selected {
        (accent, 2.0)
    } else {
        (BORDER_STRONG, 1.0)
    };
    p.rect_stroke(
        rect,
        radius,
        Stroke::new(v.s(width), border),
        StrokeKind::Middle,
    );

    let badge_w = if n.badge.is_empty() { 0.0 } else { 52.0 };
    Label {
        x: x + 14.0,
        cy: y + 17.5,
        text: &n.name,
        size: 13.0,
        mono: topic,
        color: if topic { TOPIC } else { TEXT },
        max: n.w - 28.0 - badge_w - 40.0,
    }
    .draw(p, v);

    let mut right = x + n.w - 12.0;
    if !n.badge.is_empty() {
        right -= width_of(p, v, &n.badge, 11.0, true);
        Label {
            x: right,
            cy: y + 18.0,
            text: &n.badge,
            size: 11.0,
            mono: true,
            color: MUTED,
            max: 10_000.0,
        }
        .draw(p, v);
        right -= 8.0;
    }
    for (count, color) in [(n.errors, ERR), (n.warnings, WARN)] {
        if count > 0 {
            right -= 20.0;
            let c = v.pt(right + 8.0, y + 17.0);
            p.circle_filled(c, v.s(8.0), color);
            let g = layout(p, v, &count.to_string(), 10.0, false, INK, 100.0);
            p.galley(c - g.size() / 2.0, g, INK);
            right -= 2.0;
        }
    }

    if topic {
        Label {
            x: x + 14.0,
            cy: y + HEADER + 14.0,
            text: &n.meta[0],
            size: 12.0,
            mono: true,
            color: TEXT,
            max: n.w - 28.0,
        }
        .draw(p, v);
        Label {
            x: x + 14.0,
            cy: y + HEADER + 32.0,
            text: &n.meta[1],
            size: 10.5,
            mono: false,
            color: MUTED,
            max: n.w - 28.0,
        }
        .draw(p, v);
        return;
    }
    if n.rows.is_empty() {
        Label {
            x: x + 14.0,
            cy: y + HEADER + 14.0,
            text: "vide",
            size: 11.0,
            mono: false,
            color: MUTED,
            max: 10_000.0,
        }
        .draw(p, v);
    }
    for (i, r) in n.rows.iter().enumerate() {
        let ry = y + HEADER + 6.0 + i as f64 * ROW;
        let color = tag_color(r.tag);
        let g = layout(p, v, r.label, 9.0, true, color, 10_000.0);
        let tw = f64::from(g.size().x) / v.zoom + 10.0;
        let pill = Rect::from_min_size(v.pt(x + 12.0, ry + 2.0), Vec2::new(v.s(tw), v.s(15.0)));
        p.rect_filled(pill, v.radius(4.0), alpha(color, 46));
        p.galley(pill.center() - g.size() / 2.0, g, color);
        Label {
            x: x + 12.0 + tw + 8.0,
            cy: ry + 10.5,
            text: &r.text,
            size: 11.5,
            mono: true,
            color: TEXT,
            max: n.w - tw - 44.0,
        }
        .draw(p, v);
    }
}

// ---- overlays ----

/// The legend of the colours, at the bottom left; returns its rectangle.
fn legend(ui: &Ui, stage: Rect, pad: f32) -> Rect {
    let items = [("publie", TOPIC), ("consomme", OK), ("appel gRPC", SERVICE)];
    let p = ui.painter();
    let galleys: Vec<Arc<Galley>> = items
        .iter()
        .map(|(t, _)| p.layout_no_wrap((*t).to_string(), theme::prop(11.0), MUTED))
        .collect();
    let content: f32 =
        galleys.iter().map(|g| 20.0 + g.size().x).sum::<f32>() + 14.0 * (items.len() - 1) as f32;
    let size = Vec2::new(content + 22.0, 29.0);
    let rect = Rect::from_min_size(
        Pos2::new(stage.left() + pad, stage.bottom() - pad - size.y),
        size,
    );
    p.rect_filled(rect, CornerRadius::same(8), alpha(PANEL, 217));
    p.rect_stroke(
        rect.shrink(0.5),
        CornerRadius::same(8),
        Stroke::new(1.0, BORDER),
        StrokeKind::Inside,
    );
    let mut x = rect.left() + 11.0;
    for (g, (_, color)) in galleys.into_iter().zip(items) {
        let icon = Rect::from_center_size(Pos2::new(x + 8.0, rect.center().y), Vec2::splat(16.0));
        icons::paint(ui, "minus", icon, color);
        x += 20.0;
        let w = g.size().x;
        p.galley(Pos2::new(x, rect.center().y - g.size().y / 2.0), g, MUTED);
        x += w + 14.0;
    }
    rect
}

/// Zoom in, zoom out, fit: three square buttons in a frame.
fn zoom_buttons(ui: &mut Ui, rect: Rect, st: &mut CanvasState, centre: (f64, f64)) {
    ui.painter()
        .rect_filled(rect, CornerRadius::same(6), PANEL2);
    let buttons = [
        ("plus", "Zoom avant"),
        ("minus", "Zoom arrière"),
        ("graph", "Tout afficher"),
    ];
    let h = (rect.height() - 2.0) / 3.0;
    for (i, (name, tip)) in buttons.into_iter().enumerate() {
        let cell = Rect::from_min_size(
            Pos2::new(rect.left() + 1.0, rect.top() + 1.0 + h * i as f32),
            Vec2::new(rect.width() - 2.0, h),
        );
        let r = ui
            .interact(cell, ui.make_persistent_id(("zoom", i)), Sense::click())
            .on_hover_text(tip)
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if r.hovered() {
            let top = if i == 0 { 5 } else { 0 };
            let bottom = if i == 2 { 5 } else { 0 };
            let radius = CornerRadius {
                nw: top,
                ne: top,
                sw: bottom,
                se: bottom,
            };
            ui.painter().rect_filled(cell, radius, RAISED);
        }
        if i > 0 {
            ui.painter()
                .hline(cell.x_range(), cell.top(), Stroke::new(1.0, BORDER));
        }
        let icon = Rect::from_center_size(cell.center(), Vec2::splat(16.0));
        icons::paint(ui, name, icon, TEXT);
        if r.clicked() {
            match i {
                0 => zoom_at(st, 1.25, centre),
                1 => zoom_at(st, 0.8, centre),
                _ => st.auto_fit = true,
            }
        }
    }
    ui.painter().rect_stroke(
        rect.shrink(0.5),
        CornerRadius::same(6),
        Stroke::new(1.0, BORDER),
        StrokeKind::Inside,
    );
}

/// The whole graph in a small frame, with the part of it the view shows.
fn minimap(p: &egui::Painter, rect: Rect, st: &CanvasState, view: (f64, f64)) {
    p.rect_filled(rect, CornerRadius::same(8), PANEL);
    p.rect_stroke(
        rect.shrink(0.5),
        CornerRadius::same(8),
        Stroke::new(1.0, BORDER),
        StrokeKind::Inside,
    );
    let Some((x0, y0, x1, y1)) = st.graph.bounds() else {
        return;
    };
    let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
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
    let at = |x: f64, y: f64| rect.min + Vec2::new((ox + x * k) as f32, (oy + y * k) as f32);
    let p = p.with_clip_rect(rect);
    for n in &st.graph.nodes {
        let color = if is_topic(n.id) { TOPIC } else { hex(0x6e7bff) };
        let size = Vec2::new((n.w * k).max(3.0) as f32, (n.h * k).max(3.0) as f32);
        p.rect_filled(Rect::from_min_size(at(n.x, n.y), size), 0.0, color);
    }
    let view = Rect::from_min_max(at(vx0, vy0), at(vx1, vy1));
    p.rect_stroke(
        view,
        0.0,
        Stroke::new(1.5, alpha(SERVICE, 230)),
        StrokeKind::Middle,
    );
}
