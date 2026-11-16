//! Look: "case file" - noir workspace, paper dossier, typewriter labels (see the design canvas).
use eframe::egui::{self, Color32, FontId, Painter, Pos2, RichText, Shape, Stroke, Vec2};

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}
pub const NOIR: Color32 = rgb(0x1b, 0x1d, 0x21);
pub const VIEWPORT: Color32 = rgb(0x12, 0x14, 0x17);
pub const PANEL: Color32 = rgb(0x23, 0x26, 0x2b);
pub const LEATHER: Color32 = rgb(0x2b, 0x2f, 0x35);
pub const RAIL: Color32 = rgb(0x1e, 0x20, 0x24);
pub const LINE: Color32 = rgb(0x34, 0x38, 0x3e);
pub const LAMP: Color32 = rgb(0xe0, 0xa5, 0x3a);
pub const PAPER_LIGHT: Color32 = rgb(0x2d, 0x31, 0x37);
pub const INK: Color32 = rgb(0xe4, 0xe6, 0xea);
pub const INK_DEEP: Color32 = rgb(0x14, 0x16, 0x1a);
pub const INK_SOFT: Color32 = rgb(0x9a, 0xa0, 0xa8);
pub const RED: Color32 = rgb(0xd4, 0x50, 0x3c);
pub const CREAM: Color32 = rgb(0xd5, 0xd8, 0xdc);
pub const TITLE: Color32 = rgb(0xf1, 0xf3, 0xf5);
pub const MUTED: Color32 = rgb(0x8b, 0x91, 0x99);
pub const GREEN: Color32 = rgb(0x5f, 0xc7, 0x7a);
pub const BLUE: Color32 = rgb(0x6a, 0xa0, 0xff);
pub const ORANGE_DARK: Color32 = rgb(0xc4, 0x57, 0x0a);

// One clean sans family everywhere; the helpers keep their old names so call sites stay put.
pub fn disp(s: f32) -> FontId {
    FontId::proportional(s * 0.9)
}
pub fn serif(s: f32) -> FontId {
    FontId::proportional(s * 0.72)
}
pub fn hand(s: f32) -> FontId {
    FontId::proportional(s * 0.72)
}
pub fn typew(s: f32) -> FontId {
    FontId::proportional(s)
}
pub fn mono(s: f32) -> FontId {
    FontId::monospace(s)
}

pub fn install(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = NOIR;
    v.window_fill = PANEL;
    v.extreme_bg_color = VIEWPORT;
    v.selection.bg_fill = LAMP.gamma_multiply(0.55);
    v.selection.stroke = Stroke::new(1.0, LAMP);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, CREAM);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
        w.corner_radius = 4.0.into();
    }
    v.widgets.inactive.weak_bg_fill = rgb(0x2d, 0x31, 0x37);
    v.widgets.inactive.bg_fill = rgb(0x2d, 0x31, 0x37);
    v.widgets.hovered.weak_bg_fill = rgb(0x3a, 0x3f, 0x47);
    v.widgets.hovered.bg_fill = rgb(0x3a, 0x3f, 0x47);
    ctx.set_visuals(v);

    ctx.all_styles_mut(|st| {
        st.spacing.item_spacing = Vec2::new(8.0, 8.0);
        st.spacing.button_padding = Vec2::new(10.0, 6.0);
        st.text_styles.insert(egui::TextStyle::Body, typew(13.0));
        st.text_styles.insert(egui::TextStyle::Button, typew(12.0));
        st.text_styles.insert(egui::TextStyle::Small, mono(10.0));
        st.text_styles.insert(egui::TextStyle::Monospace, mono(12.0));
        st.text_styles.insert(egui::TextStyle::Heading, disp(22.0));
    });
}

/// Local overrides so egui widgets sit well on the paper dossier.
pub fn paper_visuals(ui: &mut egui::Ui) {
    let v = ui.visuals_mut();
    v.override_text_color = Some(INK);
    v.extreme_bg_color = VIEWPORT;
    v.selection.bg_fill = LAMP;
    v.selection.stroke = Stroke::new(1.0, INK_DEEP);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    for (w, fill) in [(&mut v.widgets.inactive, PAPER_LIGHT), (&mut v.widgets.hovered, rgb(0x3a, 0x3f, 0x47)), (&mut v.widgets.active, LAMP)] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::new(1.0, rgb(0x45, 0x4a, 0x52));
        w.fg_stroke = Stroke::new(1.5, INK);
    }
}

#[derive(Clone, Copy)]
pub enum Kind {
    Amber,
    Red,
    Ink,
    Ghost,
    GhostDark,
}

pub fn button(ui: &mut egui::Ui, text: &str, kind: Kind, size: Vec2) -> egui::Response {
    let (fill, fg, stroke, font) = match kind {
        Kind::Amber => (LAMP, INK_DEEP, Stroke::NONE, disp(14.0)),
        Kind::Red => (RED, Color32::WHITE, Stroke::NONE, disp(16.0)),
        Kind::Ink => (rgb(0x3a, 0x3f, 0x47), TITLE, Stroke::NONE, typew(12.0)),
        Kind::Ghost | Kind::GhostDark => (Color32::TRANSPARENT, CREAM, Stroke::new(1.0, rgb(0x4a, 0x50, 0x58)), typew(12.0)),
    };
    ui.add_sized(size, egui::Button::new(RichText::new(text).font(font).color(fg)).fill(fill).stroke(stroke).corner_radius(4.0))
}

#[derive(Clone, Copy)]
pub enum Icon {
    Grid,
    Auto,
    Mirror,
    Export,
    Undo,
    Live,
    Relax,
    Pencil,
    Edit,
}

pub fn icon(p: &Painter, c: Pos2, ic: Icon, col: Color32) {
    let s = Stroke::new(1.7, col);
    let at = |x: f32, y: f32| Pos2::new(c.x + x, c.y + y);
    match ic {
        Icon::Grid => {
            p.add(Shape::closed_line(vec![at(-9., -9.), at(9., -9.), at(9., 9.), at(-9., 9.)], s));
            for k in [-3.0, 3.0] {
                p.line_segment([at(k, -9.), at(k, 9.)], s);
                p.line_segment([at(-9., k), at(9., k)], s);
            }
        }
        Icon::Auto => {
            p.line_segment([at(-9., 9.), at(0., 0.)], s);
            p.line_segment([at(4., -10.), at(4., 0.)], s);
            p.line_segment([at(-1., -5.), at(9., -5.)], s);
            p.line_segment([at(7., -8.), at(1., -2.)], Stroke::new(1.2, col));
        }
        Icon::Mirror => {
            p.line_segment([at(0., -10.), at(0., 10.)], s);
            p.add(Shape::closed_line(vec![at(-3., -6.), at(-10., 0.), at(-3., 6.)], s));
            p.add(Shape::closed_line(vec![at(3., -6.), at(10., 0.), at(3., 6.)], s));
        }
        Icon::Export => {
            p.line_segment([at(0., -10.), at(0., 4.)], s);
            p.add(Shape::line(vec![at(-6., -2.), at(0., 4.), at(6., -2.)], s));
            p.line_segment([at(-9., 10.), at(9., 10.)], s);
        }
        Icon::Relax => {
            p.add(Shape::line(vec![at(-10., 3.), at(-5., -4.), at(0., 3.), at(5., -4.), at(10., 3.)], s));
        }
        Icon::Live => {
            p.circle_stroke(c, 9.0, s);
            p.circle_filled(c, 3.5, col);
        }
        Icon::Pencil => {
            p.add(Shape::closed_line(vec![at(-9., 9.), at(-7., 3.), at(5., -9.), at(9., -5.), at(-3., 7.)], s));
            p.line_segment([at(2., -6.), at(6., -2.)], s);
        }
        Icon::Edit => {
            p.add(Shape::closed_line(vec![at(-9., -9.), at(-9., 5.), at(-5., 2.), at(-2., 9.), at(1., 8.), at(-2., 1.), at(3., 1.)], s));
            p.add(Shape::closed_line(vec![at(3., -9.), at(9., -9.), at(9., -3.), at(3., -3.)], Stroke::new(1.2, col)));
        }
        Icon::Undo => {
            p.add(Shape::line(vec![at(-4., -8.), at(-9., -3.), at(-4., 2.)], s));
            p.add(Shape::line(vec![at(-9., -3.), at(3., -3.), at(9., 2.), at(3., 8.), at(0., 8.)], s));
        }
    }
}

pub fn label(ui: &mut egui::Ui, t: impl Into<String>, font: FontId, col: Color32) {
    ui.label(RichText::new(t.into()).font(font).color(col));
}
