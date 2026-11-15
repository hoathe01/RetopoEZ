//! Look: "case file" - noir workspace, paper dossier, typewriter labels (see the design canvas).
use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, Painter, Pos2, RichText, Shape, Stroke, Vec2};
use std::sync::Arc;

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}
pub const NOIR: Color32 = rgb(0x0e, 0x0a, 0x07);
pub const VIEWPORT: Color32 = rgb(0x0b, 0x08, 0x07);
pub const PANEL: Color32 = rgb(0x17, 0x10, 0x0b);
pub const LEATHER: Color32 = rgb(0x2a, 0x1c, 0x12);
pub const RAIL: Color32 = rgb(0x11, 0x0c, 0x09);
pub const LINE: Color32 = rgb(0x3a, 0x2a, 0x1d);
pub const LAMP: Color32 = rgb(0xc9, 0xa4, 0x5c);
pub const PAPER: Color32 = rgb(0xdb, 0xca, 0xa4);
pub const PAPER_LIGHT: Color32 = rgb(0xea, 0xdd, 0xbc);
pub const INK: Color32 = rgb(0x23, 0x1a, 0x12);
pub const INK_DEEP: Color32 = rgb(0x1a, 0x12, 0x0b);
pub const INK_SOFT: Color32 = rgb(0x6b, 0x5a, 0x45);
pub const RED: Color32 = rgb(0xa8, 0x3f, 0x2e);
pub const CREAM: Color32 = rgb(0xe6, 0xd6, 0xbb);
pub const TITLE: Color32 = rgb(0xf2, 0xe6, 0xcc);
pub const MUTED: Color32 = rgb(0xa8, 0x97, 0x7f);
pub const GREEN: Color32 = rgb(0x6f, 0xcf, 0x7f);
pub const BLUE: Color32 = rgb(0x2f, 0x5f, 0xc4);
pub const ORANGE_DARK: Color32 = rgb(0xc4, 0x57, 0x0a);

fn fam(n: &str) -> FontFamily {
    FontFamily::Name(n.into())
}
pub fn disp(s: f32) -> FontId {
    FontId::new(s, fam("display")) // Oswald: committing buttons
}
pub fn serif(s: f32) -> FontId {
    FontId::new(s, fam("serif")) // Cinzel: headings, like the casebook titles
}
pub fn hand(s: f32) -> FontId {
    FontId::new(s, fam("hand")) // Caveat: handwritten margin notes
}
pub fn typew(s: f32) -> FontId {
    FontId::new(s, fam("type")) // Special Elite: labels, messages
}
pub fn mono(s: f32) -> FontId {
    FontId::new(s, fam("mono")) // IBM Plex Mono: numbers, key caps
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let fallback = fonts.families[&FontFamily::Proportional].clone(); // glyphs the faces lack
    let faces: [(&str, &'static [u8]); 5] = [
        ("serif", include_bytes!("../assets/Cinzel.ttf")),
        ("hand", include_bytes!("../assets/Caveat.ttf")),
        ("display", include_bytes!("../assets/Oswald.ttf")),
        ("type", include_bytes!("../assets/SpecialElite-Regular.ttf")),
        ("mono", include_bytes!("../assets/IBMPlexMono-Medium.ttf")),
    ];
    for (name, bytes) in faces {
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
        let mut list = vec![name.to_string()];
        list.extend(fallback.clone());
        fonts.families.insert(fam(name), list);
    }
    ctx.set_fonts(fonts);

    let mut v = egui::Visuals::dark();
    v.panel_fill = NOIR;
    v.window_fill = PANEL;
    v.extreme_bg_color = VIEWPORT;
    v.selection.bg_fill = LAMP.gamma_multiply(0.55);
    v.selection.stroke = Stroke::new(1.0, LAMP);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, CREAM);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
        w.corner_radius = 3.0.into();
    }
    v.widgets.inactive.weak_bg_fill = rgb(0x24, 0x1a, 0x12);
    v.widgets.inactive.bg_fill = rgb(0x24, 0x1a, 0x12);
    v.widgets.hovered.weak_bg_fill = rgb(0x33, 0x25, 0x19);
    v.widgets.hovered.bg_fill = rgb(0x33, 0x25, 0x19);
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
    v.extreme_bg_color = PAPER_LIGHT;
    v.selection.bg_fill = LAMP;
    v.selection.stroke = Stroke::new(1.0, INK);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, INK_SOFT);
    for (w, fill) in [(&mut v.widgets.inactive, PAPER_LIGHT), (&mut v.widgets.hovered, rgb(0xfb, 0xf3, 0xde)), (&mut v.widgets.active, LAMP)] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::new(1.5, INK);
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
        Kind::Amber => (LAMP, INK_DEEP, Stroke::new(1.5, INK_DEEP), disp(15.0)),
        Kind::Red => (RED, Color32::WHITE, Stroke::NONE, disp(17.0)),
        Kind::Ink => (INK, PAPER_LIGHT, Stroke::NONE, typew(12.0)),
        Kind::Ghost => (Color32::TRANSPARENT, INK, Stroke::new(1.5, INK), typew(12.0)),
        Kind::GhostDark => (Color32::TRANSPARENT, CREAM, Stroke::new(1.0, rgb(0x4a, 0x38, 0x27)), typew(12.0)),
    };
    ui.add_sized(size, egui::Button::new(RichText::new(text).font(font).color(fg)).fill(fill).stroke(stroke).corner_radius(3.0))
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
        Icon::Undo => {
            p.add(Shape::line(vec![at(-4., -8.), at(-9., -3.), at(-4., 2.)], s));
            p.add(Shape::line(vec![at(-9., -3.), at(3., -3.), at(9., 2.), at(3., 8.), at(0., 8.)], s));
        }
    }
}

pub fn label(ui: &mut egui::Ui, t: impl Into<String>, font: FontId, col: Color32) {
    ui.label(RichText::new(t.into()).font(font).color(col));
}

/// Aged-paper look: darkened edges fading inwards.
pub fn edge_shade(p: &Painter, r: egui::Rect, depth: f32) {
    let dark = Color32::from_rgba_unmultiplied(60, 36, 14, 110);
    let clear = Color32::TRANSPARENT;
    let mut m = egui::Mesh::default();
    let mut quad = |a: Pos2, b: Pos2, c: Pos2, d: Pos2, ca: Color32, cb: Color32, cc: Color32, cd: Color32| {
        let i = m.vertices.len() as u32;
        for (p, c) in [(a, ca), (b, cb), (c, cc), (d, cd)] {
            m.colored_vertex(p, c);
        }
        m.add_triangle(i, i + 1, i + 2);
        m.add_triangle(i, i + 2, i + 3);
    };
    let (l, rr, t, b) = (r.left(), r.right(), r.top(), r.bottom());
    quad(Pos2::new(l, t), Pos2::new(rr, t), Pos2::new(rr, t + depth), Pos2::new(l, t + depth), dark, dark, clear, clear);
    quad(Pos2::new(l, b - depth), Pos2::new(rr, b - depth), Pos2::new(rr, b), Pos2::new(l, b), clear, clear, dark, dark);
    quad(Pos2::new(l, t), Pos2::new(l + depth, t), Pos2::new(l + depth, b), Pos2::new(l, b), dark, clear, clear, dark);
    quad(Pos2::new(rr - depth, t), Pos2::new(rr, t), Pos2::new(rr, b), Pos2::new(rr - depth, b), clear, dark, dark, clear);
    p.add(Shape::mesh(m));
}

/// Binder rings on the page edge.
pub fn rings(p: &Painter, r: egui::Rect) {
    for f in [0.2, 0.5, 0.8] {
        let c = Pos2::new(r.left() + 1.0, r.top() + r.height() * f);
        p.circle_stroke(c, 7.0, Stroke::new(3.5, rgb(0x14, 0x0e, 0x0a)));
        p.circle_stroke(c + Vec2::new(-0.8, -0.8), 7.0, Stroke::new(1.0, rgb(0x7a, 0x62, 0x40)));
    }
}
