use super::*;

/// Runs a few headless frames of one page; returns the last output and whether it overflowed.
fn run_page(width: f32, page: Page, p: &mut SynthParams) -> (egui::FullOutput, Option<String>) {
    let ctx = egui::Context::default();
    widgets::apply_theme(&ctx);
    let mut st = SynthUiState { page, loaded: "Schranz Kick".into(), ..Default::default() };
    let dir = std::env::temp_dir().join("tstudio_layout_test_presets");
    let mut status = None;
    let mut out = None;
    let mut err = None;
    // several frames so the equal-height rows settle
    for _ in 0..5 {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 1400.0))), ..Default::default() };
        out = Some(ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let avail = ui.available_rect_before_wrap();
                header(ui, &mut st, p, &dir, &mut status);
                page_tabs(ui, &mut st, p);
                ui.add_space(GAP);
                match page {
                    Page::Sound => sound_page(ui, p),
                    Page::Kick => kick_page(ui, p),
                    Page::Filter => filter_page(ui, p),
                    Page::Mod => mod_page(ui, p),
                    Page::Fx => fx_page(ui, p),
                }
                let used = ui.min_rect();
                err = (used.right() > avail.right() + 1.0).then(|| format!("{:.0} > {:.0}", used.right(), avail.right()));
            });
        }));
    }
    (out.unwrap(), err)
}

fn preset(name: &str) -> SynthParams {
    SynthParams::presets().into_iter().find(|(n, _)| *n == name).map(|(_, p)| p).unwrap()
}

/// Nothing may stick out to the right (the old wrapped layout pushed LFOs off screen).
#[test]
fn pages_fit_the_width_at_all_sizes() {
    for width in [700.0f32, 1000.0, 1300.0, 1620.0, 2400.0] {
        for (page, name) in Page::ALL {
            for pr in ["Init", "Schranz Kick", "Acid Ladder"] {
                let mut p = preset(pr);
                let (_, err) = run_page(width, page, &mut p);
                assert!(err.is_none(), "{name} / {pr} at {width}px overflows: {}", err.unwrap());
            }
        }
    }
}

/// Debug aid (no screen control): `TSYNTH_SVG=dir cargo test … dump_svg -- --ignored`
/// writes every page as an SVG of egui's paint shapes.
#[test]
#[ignore]
fn dump_svg() {
    use egui::epaint::Shape;
    use std::fmt::Write as _;
    let dir = std::path::PathBuf::from(std::env::var("TSYNTH_SVG").expect("TSYNTH_SVG"));
    let width: f32 = std::env::var("TSYNTH_W").ok().and_then(|w| w.parse().ok()).unwrap_or(1620.0);
    let col = |c: Color32| if c.a() == 0 { "none".to_string() } else { format!("rgba({},{},{},{:.3})", c.r(), c.g(), c.b(), c.a() as f32 / 255.0) };
    fn walk(s: &Shape, out: &mut Vec<Shape>) {
        match s {
            Shape::Vec(v) => v.iter().for_each(|x| walk(x, out)),
            x => out.push(x.clone()),
        }
    }
    for (page, name) in Page::ALL {
        let mut p = preset(if page == Page::Kick { "Schranz Kick" } else { "Acid Ladder" });
        let (out, _) = run_page(width, page, &mut p);
        let mut svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"1400\" font-family=\"Segoe UI, sans-serif\"><rect width=\"100%\" height=\"100%\" fill=\"{}\"/>", col(BG_PANEL));
        let mut flat = vec![];
        for cs in &out.shapes {
            walk(&cs.shape, &mut flat);
        }
        for s in flat {
            match s {
                Shape::Rect(r) => {
                    let _ = write!(svg, "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/>", r.rect.left(), r.rect.top(), r.rect.width(), r.rect.height(), r.rounding.nw, col(r.fill), col(r.stroke.color), r.stroke.width);
                }
                Shape::Circle(c) => {
                    let _ = write!(svg, "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/>", c.center.x, c.center.y, c.radius, col(c.fill), col(c.stroke.color), c.stroke.width);
                }
                Shape::LineSegment { points, stroke } => {
                    let _ = write!(svg, "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/>", points[0].x, points[0].y, points[1].x, points[1].y, col(match stroke.color { egui::epaint::ColorMode::Solid(c) => c, _ => Color32::WHITE }), stroke.width);
                }
                Shape::Path(ps) => {
                    let pts: String = ps.points.iter().map(|q| format!("{},{} ", q.x, q.y)).collect();
                    let sc = match ps.stroke.color {
                        egui::epaint::ColorMode::Solid(c) => c,
                        _ => Color32::WHITE,
                    };
                    let tag = if ps.closed { "polygon" } else { "polyline" };
                    let _ = write!(svg, "<{tag} points=\"{pts}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\" stroke-linejoin=\"round\"/>", if ps.closed { col(ps.fill) } else { "none".into() }, col(sc), ps.stroke.width);
                }
                Shape::Text(t) => {
                    for row in &t.galley.rows {
                        let text: String = row.glyphs.iter().map(|g| g.chr).collect();
                        if text.trim().is_empty() {
                            continue;
                        }
                        let sec = row.glyphs.first().map(|g| g.section_index as usize).unwrap_or(0);
                        let fmt = t.galley.job.sections.get(sec).map(|s| s.format.clone()).unwrap_or_default();
                        let c = t.override_text_color.unwrap_or(if fmt.color == Color32::PLACEHOLDER { t.fallback_color } else { fmt.color });
                        let x = t.pos.x + row.glyphs.first().map(|g| g.pos.x).unwrap_or(row.rect.min.x);
                        let y = t.pos.y + row.rect.min.y;
                        let esc = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
                        let _ = write!(svg, "<text x=\"{x}\" y=\"{y}\" font-size=\"{}\" fill=\"{}\" dominant-baseline=\"text-before-edge\">{esc}</text>", fmt.font_id.size, col(c));
                    }
                }
                _ => {}
            }
        }
        svg.push_str("</svg>");
        std::fs::write(dir.join(format!("tsynth_{}.svg", name.replace([' ', '/'], "_"))), svg).unwrap();
    }
}
