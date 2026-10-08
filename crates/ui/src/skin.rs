//! Faceplate drawing ported from the former Swift deck. Coordinates remain in faceplate units.
use crate::layout::*;
use eframe::egui::{self, *};
pub const DIVIDER: Color32 = Color32::from_rgb(54, 58, 64);
const PRESS: Color32 = Color32::from_rgb(48, 52, 59);
const DISC: Color32 = Color32::from_rgb(37, 40, 45);
fn oval(p: &Painter, r: Rect, fill: Color32, stroke: Stroke, rotation: f32) {
    let points = (0..48)
        .map(|i| {
            let a = i as f32 * std::f32::consts::TAU / 48.;
            let v = Vec2::new(a.cos() * r.width() / 2., a.sin() * r.height() / 2.);
            r.center() + egui::emath::Rot2::from_angle(rotation) * v
        })
        .collect();
    p.add(Shape::convex_polygon(points, fill, stroke));
}
fn circle(p: &Painter, r: Rect, fill: Color32, color: Color32, s: f32) {
    p.circle(
        r.center(),
        r.width() / 2. - s / 2.,
        fill,
        Stroke::new(s, color),
    );
}
pub fn text(p: &Painter, at: Pos2, label: &str, size: f32, color: Color32) {
    p.text(
        at,
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(size),
        color,
    );
}
fn glyph(p: &Painter, r: Rect, name: &str, color: Color32) {
    let unit = r.width() / 24.;
    let pos = |x: f32, y: f32| r.min + Vec2::new(x, y) * unit;
    let tri = |base: f32, tip: f32| {
        p.add(Shape::convex_polygon(
            vec![pos(base, 5.), pos(tip, 12.), pos(base, 19.)],
            color,
            Stroke::NONE,
        ));
    };
    let bar = |x: f32, w: f32| {
        p.rect_filled(
            Rect::from_min_size(pos(x, 5.), Vec2::new(w, 14.) * unit),
            0.,
            color,
        );
    };
    match name {
        "previous" => {
            bar(3., 2.);
            tri(13., 6.);
            tri(21., 14.);
        }
        "next" => {
            bar(19., 2.);
            tri(11., 18.);
            tri(3., 10.);
        }
        "search-back" => {
            tri(12., 3.);
            tri(21., 12.);
        }
        "search-forward" => {
            tri(12., 21.);
            tri(3., 12.);
        }
        _ => {
            tri(2., 12.);
            bar(14., 3.);
            bar(19., 3.);
        }
    }
}
fn key(p: &Painter, r: Rect, name: &str, fill: Color32, s: f32) {
    // Four independent radii: the inside corner wraps around the selector.
    let big = r.width() * 0.7;
    let radii = match name {
        "back" => [8., 8., big / s, 8.],
        "tag" => [8., 8., 8., big / s],
        "track-filter" => [8., big / s, 8., 8.],
        _ => [big / s, 8., 8., 8.],
    };
    let corners = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
    ];
    let centres = [
        Vec2::new(1., 1.),
        Vec2::new(-1., 1.),
        Vec2::new(-1., -1.),
        Vec2::new(1., -1.),
    ];
    let mut points = Vec::new();
    for i in 0..4 {
        let radius = radii[i] * s;
        let c = corners[i] + centres[i] * radius;
        for j in 0..=12 {
            let a = (180. + i as f32 * 90. + j as f32 * 7.5).to_radians();
            points.push(c + Vec2::angled(a) * radius);
        }
    }
    p.add(Shape::convex_polygon(points, fill, Stroke::new(s, LINE)));
}
pub fn button(p: &Painter, r: Rect, c: &Control, held: bool, lit: bool, player: bool, s: f32) {
    let fill = if held { PRESS } else { FILL };
    let lamp = c.lamp.unwrap_or(INK);
    let lens = |inner: Rect| {
        circle(
            p,
            inner,
            if lit { lamp } else { fill },
            if lit { lamp } else { lamp.gamma_multiply(0.62) },
            s,
        )
    };
    match c.name {
        "back" | "tag" | "track-filter" | "shortcut" => key(p, r, c.name, fill, s),
        "load" => {
            p.circle_filled(r.center(), r.width() / 2., PAPER);
            circle(
                p,
                r.shrink(r.width() * 0.03),
                PAPER,
                if lit { INK } else { MUTED },
                s,
            );
            let knob = r.shrink(r.width() * 0.13);
            circle(p, knob, PAPER, LINE, s);
            for i in 0..36 {
                let a = (i as f32 * 10.).to_radians();
                let d = Vec2::new(a.sin(), -a.cos()) * knob.width() * 0.43;
                p.line_segment(
                    [r.center() + d * 0.62, r.center() + d],
                    Stroke::new(s, MUTED),
                );
            }
            circle(p, knob.shrink(knob.width() * 0.26), fill, LINE, s);
        }
        "cue" | "play" => {
            circle(p, r, fill, if lit { lamp } else { LINE }, s);
            let color = if lit { lamp } else { MUTED };
            if c.name == "cue" {
                medium_text(
                    p,
                    r.center(),
                    "CUE",
                    if player { 24. } else { 18. } * s,
                    color,
                );
            } else {
                glyph(p, r.shrink(r.width() * 0.3), "play", color);
            }
        }
        "reverse" | "vinyl" => {
            p.rect(r, s * 4., fill, Stroke::new(s, LINE), StrokeKind::Inside);
            let inner = r.shrink(5. * s);
            p.rect(
                inner,
                4. * s,
                if lit { lamp } else { fill },
                Stroke::new(s, if lit { lamp } else { lamp.gamma_multiply(0.62) }),
                StrokeKind::Inside,
            );
            medium_text(
                p,
                r.center(),
                c.label,
                if player { 16. } else { 12. } * s,
                if lit { PAPER } else { lamp.gamma_multiply(0.8) },
            );
        }
        _ => {
            circle(p, r, fill, LINE, s);
            match c.name {
                "loop-in" | "loop-out" => lens(r.shrink(r.width() * 0.15)),
                "reloop" | "quantize" | "master-tempo" => lens(r.shrink(r.width() * 0.31)),
                "previous" | "next" | "search-back" | "search-forward" => {
                    glyph(p, r.shrink(r.width() * 0.25), c.name, INK)
                }
                _ => {}
            }
        }
    }
}
pub fn knob(p: &Painter, r: Rect, value: i32, held: bool, s: f32) {
    circle(p, r, if held { PRESS } else { FILL }, LINE, s);
    let a = (value as f32 / 255. * 270. - 135.).to_radians();
    let v = Vec2::new(a.sin(), -a.cos()) * r.width() / 2.;
    p.line_segment(
        [r.center() + v * 0.08, r.center() + v * 0.84],
        Stroke::new((r.width() * 0.05).max(1.5 * s), LINE),
    );
}
pub fn usb(p: &Painter, r: Rect, rgb: [u8; 3], s: f32) {
    let port = Rect::from_min_size(r.min, Vec2::new(r.width(), r.height() * 0.82));
    p.rect_stroke(port, 5. * s, Stroke::new(s, MUTED), StrokeKind::Inside);
    p.rect_stroke(
        port.shrink2(port.size() * Vec2::new(0.22, 0.2)),
        0.,
        Stroke::new(s, MUTED),
        StrokeKind::Inside,
    );
    let lamp = Rect::from_min_size(
        r.min + Vec2::new(r.width() * 0.12, r.height() * 0.94),
        Vec2::new(r.width() * 0.76, r.height() * 0.06),
    );
    p.rect(
        lamp,
        2. * s,
        Color32::from_rgb(rgb[0], rgb[1], rgb[2]),
        Stroke::new(
            s,
            if rgb == [0; 3] {
                MUTED
            } else {
                Color32::from_rgb(rgb[0], rgb[1], rgb[2])
            },
        ),
        StrokeKind::Inside,
    );
}
pub fn jog(p: &Painter, r: Rect, state: &serde_json::Value, touched: bool, player: bool, s: f32) {
    let u = r.width() / 200.;
    let centre = r.center();
    let polar = |radius: f32, a: f32| {
        centre + Vec2::new(a.to_radians().sin(), -a.to_radians().cos()) * radius * u
    };
    p.circle_filled(centre, 99. * u, DISC);
    for i in 0..20 {
        let a = i as f32 * 18.;
        oval(
            p,
            Rect::from_center_size(polar(87., a), Vec2::new(15.2, 10.4) * u),
            PAPER,
            Stroke::new(s, LINE),
            a.to_radians(),
        );
        for (radius, offset) in [(80.3, 0.), (84.6, -1.9), (84.6, 1.9)] {
            p.circle(
                polar(radius, a + 9. + offset),
                1.2 * u,
                PAPER,
                Stroke::new(s, MUTED),
            );
        }
    }
    p.circle_stroke(centre, 99. * u, Stroke::new(s, LINE));
    p.circle(
        centre,
        73. * u,
        if touched { PRESS } else { PAPER },
        Stroke::NONE,
    );
    p.circle_stroke(centre, 73.6 * u, Stroke::new(s, INK));
    let display = if player { 1.305 } else { 1. };
    p.circle(
        centre,
        29.5 * display * u,
        Color32::BLACK,
        Stroke::new(s, MUTED),
    );
    if let Some(position) = state["position"].as_f64() {
        let turn = (position as f32 - 1.) / 135. * 360.;
        for i in 5..60 {
            p.line_segment(
                [
                    polar(
                        if i % 5 == 0 { 14.5 } else { 16.5 } * display,
                        turn + i as f32 * 6.,
                    ),
                    polar(23.5 * display, turn + i as f32 * 6.),
                ],
                Stroke::new(1.05 * u * display, INK),
            );
        }
        p.line_segment(
            [
                polar(13. * display, turn + 30.),
                polar(25. * display, turn + 30.),
            ],
            Stroke::new(2. * u * display, INK),
        );
    }
    if state["touch"] == true {
        let points = (0..=12)
            .map(|i| polar(27.2 * display, -42. + i as f32 * 7.))
            .collect();
        p.add(Shape::line(points, Stroke::new(1.6 * u * display, INK)));
    }
    let vinyl = state["vinyl"] == true;
    p.circle(
        centre,
        9. * display * u,
        if vinyl { BLUE } else { Color32::BLACK },
        Stroke::new(s, if vinyl { BLUE } else { MUTED }),
    );
    text(
        p,
        centre,
        "VINYL",
        3.5 * display * u,
        if vinyl { PAPER } else { MUTED },
    );
}
pub fn fader(p: &Painter, r: Rect, value: i32, s: f32) -> Rect {
    p.rect_stroke(r, 6. * s, Stroke::new(s, DIVIDER), StrokeKind::Inside);
    p.line_segment(
        [
            r.center_top() + Vec2::new(0., r.height() * 0.05),
            r.center_bottom() - Vec2::new(0., r.height() * 0.05),
        ],
        Stroke::new(s, MUTED),
    );
    let track = r.shrink2(Vec2::new(0., r.height() * 0.08));
    let y = egui::lerp(track.top()..=track.bottom(), value as f32 / 65535.);
    let cap = Rect::from_center_size(
        Pos2::new(r.center().x, y),
        Vec2::new(r.width() * 0.88, r.width() * 0.46),
    );
    p.rect(cap, 4. * s, DISC, Stroke::new(s, LINE), StrokeKind::Inside);
    p.line_segment(
        [cap.left_center(), cap.right_center()],
        Stroke::new(2. * s, Color32::WHITE),
    );
    track
}
pub fn host_button(ui: &mut Ui, r: Rect, label: &str, on: bool, size: f32) -> Response {
    let response = ui.interact(r, ui.id().with(label), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
    let p = ui.painter();
    p.rect(
        r,
        4.,
        if on {
            LINE
        } else if response.is_pointer_button_down_on() {
            FILL
        } else {
            Color32::TRANSPARENT
        },
        Stroke::new(1., DIVIDER),
        StrokeKind::Inside,
    );
    let tone = if on { PAPER } else { INK };
    if ["Mute", "Keys", "Load Music…", "USB Folder…"].contains(&label) {
        let c = r.left_center() + Vec2::new(16., 0.);
        let stroke = Stroke::new(1., tone);
        match label {
            "USB Folder…" => {
                let points = vec![
                    c + Vec2::new(-7., 4.),
                    c + Vec2::new(-6., -1.),
                    c + Vec2::new(-3., -5.),
                    c + Vec2::new(4., -5.),
                    c + Vec2::new(7., 1.),
                    c + Vec2::new(7., 4.),
                    c + Vec2::new(-7., 4.),
                ];
                p.add(Shape::line(points, stroke));
                p.line_segment([c + Vec2::new(-6., 0.), c + Vec2::new(6., 0.)], stroke);
                p.circle_filled(c + Vec2::new(4., 2.), 0.7, tone);
            }
            "Load Music…" => {
                p.line_segment(
                    [c + Vec2::new(-1., 5.), c + Vec2::new(-1., -5.)],
                    Stroke::new(2., tone),
                );
                p.line_segment(
                    [c + Vec2::new(-1., -5.), c + Vec2::new(4., -6.)],
                    Stroke::new(2., tone),
                );
                p.circle_filled(c + Vec2::new(-3., 5.), 2., tone);
            }
            "Keys" => {
                p.rect_stroke(
                    Rect::from_center_size(c, Vec2::new(15., 9.)),
                    1.,
                    stroke,
                    StrokeKind::Inside,
                );
                for row in 0..2 {
                    for col in 0..5 {
                        p.circle_filled(
                            c + Vec2::new(-5. + col as f32 * 2.5, -2. + row as f32 * 2.),
                            0.5,
                            tone,
                        );
                    }
                }
                p.line_segment([c + Vec2::new(-4., 2.), c + Vec2::new(4., 2.)], stroke);
            }
            _ => {
                p.add(Shape::line(
                    vec![
                        c + Vec2::new(-5., -2.),
                        c + Vec2::new(-2., -2.),
                        c + Vec2::new(2., -6.),
                        c + Vec2::new(2., 6.),
                        c + Vec2::new(-2., 2.),
                        c + Vec2::new(-5., 2.),
                        c + Vec2::new(-5., -2.),
                    ],
                    stroke,
                ));
                p.line_segment([c + Vec2::new(-7., -7.), c + Vec2::new(7., 7.)], stroke);
            }
        }
        p.text(
            r.left_center() + Vec2::new(32., 0.),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(size),
            tone,
        );
    } else {
        text(p, r.center(), label, size, tone);
    }
    response
}

/// Read the host's installed fonts; system font files are never shipped with the application.
pub fn install_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    let candidates = if cfg!(target_os = "macos") {
        [
            "/System/Library/Fonts/SFNS.ttf",
            "/System/Library/Fonts/SFNSMono.ttf",
        ]
    } else if cfg!(target_os = "windows") {
        [
            "C:/Windows/Fonts/segoeui.ttf",
            "C:/Windows/Fonts/consola.ttf",
        ]
    } else {
        [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        ]
    };
    for (i, path) in candidates.iter().enumerate() {
        if let Ok(bytes) = std::fs::read(path) {
            let name = format!("system-{i}");
            fonts.font_data.insert(
                name.clone(),
                std::sync::Arc::new(FontData::from_owned(bytes)),
            );
            fonts
                .families
                .get_mut(&if i == 0 {
                    FontFamily::Proportional
                } else {
                    FontFamily::Monospace
                })
                .unwrap()
                .insert(0, name);
        }
    }
    for (name, weight) in [("deck-medium", 500.), ("deck-semibold", 600.)] {
        let mut family = fonts.families[&FontFamily::Proportional].clone();
        if let Some(data) = fonts.font_data.get("system-0") {
            let mut data = (**data).clone();
            data.tweak.coords = egui::epaint::text::VariationCoords::new([(b"wght", weight)]);
            fonts
                .font_data
                .insert(name.to_owned(), std::sync::Arc::new(data));
            family.insert(0, name.to_owned());
        }
        fonts.families.insert(FontFamily::Name(name.into()), family);
    }
    ctx.set_fonts(fonts);
}
pub fn weighted_font(p: &Painter, size: f32, semibold: bool) -> FontId {
    let family = FontFamily::Name(
        if semibold {
            "deck-semibold"
        } else {
            "deck-medium"
        }
        .into(),
    );
    let available = p.ctx().fonts(|f| f.families().contains(&family));
    FontId::new(
        size,
        if available {
            family
        } else {
            FontFamily::Proportional
        },
    )
}
pub fn medium_text(p: &Painter, at: Pos2, label: &str, size: f32, color: Color32) {
    p.text(
        at,
        Align2::CENTER_CENTER,
        label,
        weighted_font(p, size, false),
        color,
    );
}

pub fn key_tag(p: &Painter, at: Pos2, label: &str) {
    let galley = p.layout_no_wrap(label.to_owned(), weighted_font(p, 11., true), INK);
    let tag = Rect::from_center_size(at, galley.size() + Vec2::new(10., 2.));
    p.rect(tag, 3., PAPER, Stroke::new(1., LINE), StrokeKind::Inside);
    p.galley(tag.min + Vec2::new(5., 1.), galley, INK);
}
