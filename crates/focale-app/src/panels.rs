//! Editor panels: the fixed pipeline as ordered controls (PRD §7).
//!
//! Panel order mirrors PRD §3 stage order exactly and cannot be reordered.
//! One control per function — no duplicates anywhere in the app.

use eframe::egui::{self, CollapsingHeader, Sense, Slider, Ui, vec2};
use focale_core::params::tone::{CurvePoint, ToneCurve};
use focale_core::params::white_balance::WhiteBalanceParams;
use focale_core::params::{EditState, detail::SharpenMethod};
use focale_core::pipeline::RenderWarning;

/// Draws all stage panels; returns true when any parameter changed.
pub fn stage_panels(ui: &mut Ui, edit: &mut EditState, warnings: &[RenderWarning]) -> bool {
    let mut changed = false;
    changed |= optics_panel(ui, edit, warnings);
    changed |= white_balance_panel(ui, edit);
    changed |= tone_panel(ui, edit);
    changed |= color_panel(ui, edit);
    changed |= local_panel(ui, edit);
    changed |= detail_panel(ui, edit);
    changed |= retouch_panel(ui, edit);
    changed |= geometry_panel(ui, edit);
    changed |= finishing_panel(ui, edit);
    changed
}

fn optics_panel(ui: &mut Ui, edit: &mut EditState, warnings: &[RenderWarning]) -> bool {
    let mut changed = false;
    CollapsingHeader::new("Optics")
        .default_open(true)
        .show(ui, |ui| {
            let o = &mut edit.optics;
            changed |= ui.checkbox(&mut o.enabled, "Apply corrections").changed();
            ui.add_enabled_ui(o.enabled, |ui| {
                changed |= ui.checkbox(&mut o.vignetting, "Vignetting").changed();
                changed |= ui
                    .checkbox(&mut o.chromatic_aberration, "Chromatic aberration")
                    .changed();
                changed |= ui.checkbox(&mut o.distortion, "Distortion").changed();
            });
            if warnings.contains(&RenderWarning::OpticsMetadataMissing) {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "⚠ No optics metadata in this file — stage skipped",
                );
            }
        });
    changed
}

fn white_balance_panel(ui: &mut Ui, edit: &mut EditState) -> bool {
    let mut changed = false;
    CollapsingHeader::new("White balance")
        .default_open(true)
        .show(ui, |ui| {
            let wb = &mut edit.white_balance;
            let mut mode = match wb {
                WhiteBalanceParams::AsShot => 0,
                WhiteBalanceParams::Temperature { .. } => 1,
                WhiteBalanceParams::Custom { .. } => 2,
            };
            let before = mode;
            egui::ComboBox::from_label("Mode")
                .selected_text(["As shot", "Temperature", "Custom"][mode])
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut mode, 0, "As shot");
                    ui.selectable_value(&mut mode, 1, "Temperature");
                    ui.selectable_value(&mut mode, 2, "Custom");
                });
            if mode != before {
                *wb = match mode {
                    0 => WhiteBalanceParams::AsShot,
                    1 => WhiteBalanceParams::Temperature {
                        kelvin: 5500.0,
                        tint: 0.0,
                    },
                    _ => WhiteBalanceParams::Custom {
                        red: 1.0,
                        blue: 1.0,
                    },
                };
                changed = true;
            }
            match wb {
                WhiteBalanceParams::AsShot => {}
                WhiteBalanceParams::Temperature { kelvin, tint } => {
                    changed |= ui
                        .add(
                            Slider::new(kelvin, 2000.0..=25000.0)
                                .logarithmic(true)
                                .text("Temperature (K)"),
                        )
                        .changed();
                    changed |= ui
                        .add(Slider::new(tint, -100.0..=100.0).text("Tint"))
                        .changed();
                }
                WhiteBalanceParams::Custom { red, blue } => {
                    changed |= ui.add(Slider::new(red, 0.2..=4.0).text("Red")).changed();
                    changed |= ui.add(Slider::new(blue, 0.2..=4.0).text("Blue")).changed();
                }
            }
        });
    changed
}

fn slider(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, label: &str) -> bool {
    let r = ui.add(Slider::new(value, range).text(label));
    // Double-click resets to zero (one gesture, no duplicate control).
    if r.double_clicked() {
        *value = 0.0;
        return true;
    }
    r.changed()
}

fn tone_panel(ui: &mut Ui, edit: &mut EditState) -> bool {
    let mut changed = false;
    CollapsingHeader::new("Tone")
        .default_open(true)
        .show(ui, |ui| {
            let t = &mut edit.tone;
            changed |= ui.checkbox(&mut t.enabled, "Enabled").changed();
            ui.add_enabled_ui(t.enabled, |ui| {
                changed |= slider(ui, &mut t.exposure, -5.0..=5.0, "Exposure (EV)");
                changed |= slider(ui, &mut t.contrast, -100.0..=100.0, "Contrast");
                changed |= slider(ui, &mut t.highlights, -100.0..=100.0, "Highlights");
                changed |= slider(ui, &mut t.shadows, -100.0..=100.0, "Shadows");
                changed |= slider(ui, &mut t.whites, -100.0..=100.0, "Whites");
                changed |= slider(ui, &mut t.blacks, -100.0..=100.0, "Blacks");
                ui.label("Point curve");
                changed |= curve_editor(ui, &mut t.curve);
            });
        });
    changed
}

/// Minimal point-curve editor: drag points, double-click to add, right-click
/// a point to remove (endpoints stay).
fn curve_editor(ui: &mut Ui, curve: &mut ToneCurve) -> bool {
    let mut changed = false;
    let size = vec2(ui.available_width().min(220.0), 140.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
    let to_screen = |p: CurvePoint| {
        egui::pos2(
            rect.left() + p.x * rect.width(),
            rect.bottom() - p.y * rect.height(),
        )
    };
    // Grid quarters.
    for i in 1..4 {
        let f = i as f32 / 4.0;
        let x = rect.left() + f * rect.width();
        let y = rect.top() + f * rect.height();
        let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            stroke,
        );
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            stroke,
        );
    }
    // Polyline through the points (UI sketch; the pipeline interpolates
    // monotone-cubic).
    let pts: Vec<egui::Pos2> = curve.points.iter().map(|p| to_screen(*p)).collect();
    painter.add(egui::Shape::line(
        pts.clone(),
        ui.visuals().widgets.active.fg_stroke,
    ));
    for p in &pts {
        painter.circle_filled(*p, 3.5, ui.visuals().widgets.active.fg_stroke.color);
    }
    let pointer = response.interact_pointer_pos();
    if let Some(pos) = pointer {
        let norm = CurvePoint {
            x: ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
            y: ((rect.bottom() - pos.y) / rect.height()).clamp(0.0, 1.0),
        };
        let nearest = curve
            .points
            .iter()
            .enumerate()
            .min_by(|a, b| {
                let da = (a.1.x - norm.x).abs();
                let db = (b.1.x - norm.x).abs();
                da.partial_cmp(&db).unwrap()
            })
            .map(|(i, p)| (i, *p));
        if response.double_clicked() {
            curve.points.push(norm);
            curve.points.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap());
            changed = true;
        } else if response.dragged() {
            if let Some((i, _)) = nearest {
                let lo = if i == 0 {
                    0.0
                } else {
                    curve.points[i - 1].x + 0.01
                };
                let hi = if i + 1 == curve.points.len() {
                    1.0
                } else {
                    curve.points[i + 1].x - 0.01
                };
                let is_endpoint = i == 0 || i + 1 == curve.points.len();
                curve.points[i] = CurvePoint {
                    x: if is_endpoint {
                        curve.points[i].x
                    } else {
                        norm.x.clamp(lo.min(hi), hi.max(lo))
                    },
                    y: norm.y,
                };
                changed = true;
            }
        } else if response.secondary_clicked()
            && let Some((i, p)) = nearest
        {
            let is_endpoint = i == 0 || i + 1 == curve.points.len();
            if !is_endpoint && ((p.x - norm.x).abs() * rect.width() < 12.0) {
                curve.points.remove(i);
                changed = true;
            }
        }
    }
    if ui.small_button("Reset curve").clicked() {
        *curve = ToneCurve::default();
        changed = true;
    }
    changed
}

fn color_panel(ui: &mut Ui, edit: &mut EditState) -> bool {
    let mut changed = false;
    CollapsingHeader::new("Colour")
        .default_open(false)
        .show(ui, |ui| {
            let c = &mut edit.color;
            changed |= ui.checkbox(&mut c.enabled, "Enabled").changed();
            ui.add_enabled_ui(c.enabled, |ui| {
                changed |= slider(ui, &mut c.vibrance, -100.0..=100.0, "Vibrance");
                changed |= slider(ui, &mut c.saturation, -100.0..=100.0, "Saturation");
                CollapsingHeader::new("HSL").show(ui, |ui| {
                    for (i, name) in focale_core::params::color::HSL_BAND_NAMES
                        .iter()
                        .enumerate()
                    {
                        CollapsingHeader::new(*name)
                            .id_salt(("hsl", i))
                            .show(ui, |ui| {
                                changed |= slider(ui, &mut c.hsl.hue[i], -100.0..=100.0, "Hue");
                                changed |= slider(
                                    ui,
                                    &mut c.hsl.saturation[i],
                                    -100.0..=100.0,
                                    "Saturation",
                                );
                                changed |= slider(
                                    ui,
                                    &mut c.hsl.luminance[i],
                                    -100.0..=100.0,
                                    "Luminance",
                                );
                            });
                    }
                });
                CollapsingHeader::new("Grading").show(ui, |ui| {
                    for (label, wheel) in [
                        ("Shadows", &mut c.grading.shadows),
                        ("Midtones", &mut c.grading.midtones),
                        ("Highlights", &mut c.grading.highlights),
                    ] {
                        CollapsingHeader::new(label).show(ui, |ui| {
                            changed |= ui
                                .add(Slider::new(&mut wheel.hue, 0.0..=360.0).text("Hue"))
                                .changed();
                            changed |= slider(ui, &mut wheel.saturation, 0.0..=100.0, "Saturation");
                            changed |=
                                slider(ui, &mut wheel.luminance, -100.0..=100.0, "Luminance");
                        });
                    }
                    changed |= slider(ui, &mut c.grading.blending, 0.0..=100.0, "Blending");
                    changed |= slider(ui, &mut c.grading.balance, -100.0..=100.0, "Balance");
                });
            });
        });
    changed
}

fn local_panel(ui: &mut Ui, edit: &mut EditState) -> bool {
    let mut changed = false;
    CollapsingHeader::new("Local adjustments")
        .default_open(false)
        .show(ui, |ui| {
            let mut remove: Option<usize> = None;
            for (i, adj) in edit.local.iter_mut().enumerate() {
                CollapsingHeader::new(format!("{} ({})", adj.mask.name, i + 1))
                    .id_salt(("local", i))
                    .show(ui, |ui| {
                        changed |= ui.checkbox(&mut adj.enabled, "Enabled").changed();
                        let p = &mut adj.adjustments;
                        changed |= slider(ui, &mut p.exposure, -4.0..=4.0, "Exposure");
                        changed |= slider(ui, &mut p.contrast, -100.0..=100.0, "Contrast");
                        changed |= slider(ui, &mut p.highlights, -100.0..=100.0, "Highlights");
                        changed |= slider(ui, &mut p.shadows, -100.0..=100.0, "Shadows");
                        changed |= slider(ui, &mut p.whites, -100.0..=100.0, "Whites");
                        changed |= slider(ui, &mut p.blacks, -100.0..=100.0, "Blacks");
                        changed |= slider(ui, &mut p.temperature, -100.0..=100.0, "Temperature");
                        changed |= slider(ui, &mut p.tint, -100.0..=100.0, "Tint");
                        changed |= slider(ui, &mut p.vibrance, -100.0..=100.0, "Vibrance");
                        changed |= slider(ui, &mut p.saturation, -100.0..=100.0, "Saturation");
                        if ui.small_button("Remove adjustment").clicked() {
                            remove = Some(i);
                        }
                    });
            }
            if let Some(i) = remove {
                edit.local.remove(i);
                changed = true;
            }
            ui.label("Add masks with the viewport tools (toolbar).");
        });
    changed
}

fn detail_panel(ui: &mut Ui, edit: &mut EditState) -> bool {
    let mut changed = false;
    CollapsingHeader::new("Detail")
        .default_open(false)
        .show(ui, |ui| {
            let d = &mut edit.detail;
            changed |= ui.checkbox(&mut d.enabled, "Enabled").changed();
            ui.add_enabled_ui(d.enabled, |ui| {
                ui.label("Sharpening");
                let mut deconv = d.sharpen.method == SharpenMethod::Deconvolution;
                if ui
                    .checkbox(&mut deconv, "Deconvolution (vs unsharp)")
                    .changed()
                {
                    d.sharpen.method = if deconv {
                        SharpenMethod::Deconvolution
                    } else {
                        SharpenMethod::Unsharp
                    };
                    changed = true;
                }
                changed |= slider(ui, &mut d.sharpen.amount, 0.0..=150.0, "Amount");
                changed |= ui
                    .add(Slider::new(&mut d.sharpen.radius, 0.5..=3.0).text("Radius"))
                    .changed();
                changed |= slider(ui, &mut d.sharpen.masking, 0.0..=100.0, "Masking");
                ui.separator();
                ui.label("Noise reduction");
                let n = &mut d.noise_reduction;
                changed |= slider(ui, &mut n.luminance, 0.0..=100.0, "Luminance");
                changed |= slider(ui, &mut n.luminance_detail, 0.0..=100.0, "Luma detail");
                changed |= slider(ui, &mut n.chroma, 0.0..=100.0, "Chroma");
                changed |= slider(ui, &mut n.chroma_detail, 0.0..=100.0, "Chroma detail");
            });
        });
    changed
}

fn retouch_panel(ui: &mut Ui, edit: &mut EditState) -> bool {
    let mut changed = false;
    CollapsingHeader::new("Retouch")
        .default_open(false)
        .show(ui, |ui| {
            let r = &mut edit.retouch;
            changed |= ui.checkbox(&mut r.enabled, "Enabled").changed();
            ui.label(format!("{} stroke(s)", r.strokes.len()));
            if !r.strokes.is_empty() && ui.small_button("Remove last stroke").clicked() {
                r.strokes.pop();
                changed = true;
            }
            ui.label("Use the heal/clone tool in the viewport toolbar.");
        });
    changed
}

fn geometry_panel(ui: &mut Ui, edit: &mut EditState) -> bool {
    let mut changed = false;
    CollapsingHeader::new("Geometry")
        .default_open(false)
        .show(ui, |ui| {
            let g = &mut edit.geometry;
            changed |= ui.checkbox(&mut g.enabled, "Enabled").changed();
            ui.add_enabled_ui(g.enabled, |ui| {
                changed |= slider(ui, &mut g.rotate, -45.0..=45.0, "Rotate (°)");
                changed |= slider(ui, &mut g.perspective.vertical, -100.0..=100.0, "Vertical");
                changed |= slider(
                    ui,
                    &mut g.perspective.horizontal,
                    -100.0..=100.0,
                    "Horizontal",
                );
                changed |= ui
                    .checkbox(&mut g.flip_horizontal, "Flip horizontal")
                    .changed();
                if g.crop.is_some() {
                    ui.horizontal(|ui| {
                        ui.label("Crop set");
                        if ui.small_button("Clear crop").clicked() {
                            g.crop = None;
                            changed = true;
                        }
                    });
                } else {
                    ui.label("Drag in the viewport with the crop tool to set a crop.");
                }
            });
        });
    changed
}

fn finishing_panel(ui: &mut Ui, edit: &mut EditState) -> bool {
    let mut changed = false;
    CollapsingHeader::new("Finishing")
        .default_open(false)
        .show(ui, |ui| {
            let f = &mut edit.finishing;
            changed |= ui.checkbox(&mut f.enabled, "Enabled").changed();
            ui.add_enabled_ui(f.enabled, |ui| {
                ui.label("Post-crop vignette");
                changed |= slider(ui, &mut f.vignette.amount, -100.0..=100.0, "Amount");
                changed |= slider(ui, &mut f.vignette.midpoint, 0.0..=100.0, "Midpoint");
                changed |= slider(ui, &mut f.vignette.roundness, -100.0..=100.0, "Roundness");
                changed |= slider(ui, &mut f.vignette.feather, 0.0..=100.0, "Feather");
                ui.separator();
                ui.label("Grain");
                changed |= slider(ui, &mut f.grain.amount, 0.0..=100.0, "Amount");
                changed |= slider(ui, &mut f.grain.size, 0.0..=100.0, "Size");
                changed |= slider(ui, &mut f.grain.roughness, 0.0..=100.0, "Roughness");
            });
        });
    changed
}

/// Rating/flag controls shown with the filmstrip; one place only.
pub fn rating_widget(ui: &mut Ui, rating: &mut u8) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        for star in 1..=5u8 {
            let filled = *rating >= star;
            let label = if filled { "★" } else { "☆" };
            if ui.selectable_label(false, label).clicked() {
                *rating = if *rating == star { 0 } else { star };
                changed = true;
            }
        }
    });
    changed
}

/// Draws warnings text for the status bar (missing optics metadata etc.).
pub fn warning_text(warnings: &[RenderWarning]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for w in warnings {
        match w {
            RenderWarning::OpticsMetadataMissing => parts.push("no optics metadata".into()),
            RenderWarning::CameraMatrixMissing => parts.push("no camera colour matrix".into()),
            RenderWarning::OlderPipelineVersion(v) => {
                parts.push(format!("edited with older pipeline v{v}"));
            }
        }
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warning_text_covers_every_variant() {
        assert_eq!(warning_text(&[]), "");
        assert_eq!(
            warning_text(&[
                RenderWarning::OpticsMetadataMissing,
                RenderWarning::OlderPipelineVersion(1),
            ]),
            "no optics metadata · edited with older pipeline v1"
        );
    }
}
