//! Small shared widgets: icon buttons, status lines, cards.

use eframe::egui::{self, Color32, Response, RichText};

use crate::icons::Icon;

pub const GREEN: Color32 = Color32::from_rgb(90, 190, 110);
pub const AMBER: Color32 = Color32::from_rgb(230, 170, 60);
pub const RED: Color32 = Color32::from_rgb(230, 90, 80);
pub const ACCENT: Color32 = Color32::from_rgb(70, 130, 220);

pub fn icon(ui: &mut egui::Ui, icon: Icon, color: Color32) -> Response {
    ui.add(icon.image(16.0, color))
}

pub fn button(ui: &mut egui::Ui, icon: Icon, text: &str) -> Response {
    let color = ui.visuals().text_color();
    ui.add(egui::Button::image_and_text(icon.image(16.0, color), text))
}

pub fn button_enabled(ui: &mut egui::Ui, enabled: bool, icon: Icon, text: &str) -> Response {
    let color = ui.visuals().text_color();
    ui.add_enabled(enabled, egui::Button::image_and_text(icon.image(16.0, color), text))
}

/// The one big button of a page.
pub fn primary(ui: &mut egui::Ui, enabled: bool, icon: Icon, text: &str) -> Response {
    let b = egui::Button::image_and_text(icon.image(18.0, Color32::WHITE), RichText::new(text).strong().size(15.0).color(Color32::WHITE))
        .fill(ACCENT)
        .min_size(egui::vec2(140.0, 34.0));
    ui.add_enabled(enabled, b)
}

/// Icon-only button with a tooltip.
pub fn tool(ui: &mut egui::Ui, icon: Icon, tip: &str) -> Response {
    let color = ui.visuals().text_color();
    ui.add(egui::Button::image(icon.image(14.0, color)).frame(false)).on_hover_text(tip)
}

pub fn status(ui: &mut egui::Ui, icon: Icon, color: Color32, text: impl Into<String>) -> Response {
    ui.horizontal(|ui| {
        self::icon(ui, icon, color);
        ui.colored_label(color, text.into());
    })
    .response
}

pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::group(ui.style())
        .inner_margin(12.0)
        .corner_radius(6.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Numbered section heading ("1  Choose a photo").
pub fn step(ui: &mut egui::Ui, n: u32, text: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 11.0, ACCENT);
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, n.to_string(), egui::FontId::proportional(13.0), Color32::WHITE);
        ui.label(RichText::new(text).strong().size(15.0));
    });
    ui.add_space(4.0);
}

/// Painted, because the default fonts have no filled-circle glyph.
pub fn dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

/// Rounded progress bar: eases towards `frac`, or with `None` (progress unknown) a band sweeps
/// across. The text is drawn on the bar, white over the filled part, with the percentage at the end.
pub fn progress(ui: &mut egui::Ui, frac: Option<f32>, width: f32, text: &str, fill: Color32) -> Response {
    let height = 20.0;
    let radius = height / 2.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width.max(4.0 * height), height), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let painter = ui.painter();
    let visuals = ui.visuals();
    painter.rect_filled(rect, radius, visuals.extreme_bg_color);
    painter.rect_stroke(rect, radius, egui::Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color), egui::StrokeKind::Inside);

    let bar = match frac {
        Some(f) => {
            let f = ui.ctx().animate_value_with_time(response.id.with("progress"), f.clamp(0.0, 1.0), 0.4);
            // at least a circle, so the rounded ends stay round right from the start
            (f > 0.0).then(|| egui::Rect::from_min_size(rect.min, egui::vec2((rect.width() * f).max(height), height)))
        }
        None => {
            let band = rect.width() * 0.35;
            let t = (ui.input(|i| i.time) * 0.5).fract() as f32;
            let left = rect.left() - band + t * (rect.width() + band);
            ui.ctx().request_repaint();
            Some(egui::Rect::from_x_y_ranges(left.max(rect.left())..=(left + band).min(rect.right()), rect.y_range()))
        }
    };
    if let Some(bar) = bar {
        painter.rect_filled(bar, radius, fill);
        // a lighter top half for a little depth
        let gloss = egui::Rect::from_min_max(bar.min + egui::vec2(radius * 0.5, 2.0), egui::pos2(bar.max.x - radius * 0.5, bar.center().y));
        if gloss.width() > 0.0 {
            painter.rect_filled(gloss, radius / 2.0, Color32::from_white_alpha(28));
        }
    }

    let font = egui::FontId::proportional(12.5);
    let pad = 9.0;
    let pct = frac.map(|f| painter.layout_no_wrap(format!("{:.0}%", f.clamp(0.0, 1.0) * 100.0), font.clone(), Color32::WHITE));
    let pct_w = pct.as_ref().map_or(0.0, |g| g.size().x + pad);
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, Color32::WHITE);
    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 2.0 * pad - pct_w);
    let label = painter.layout_job(job);
    let y = rect.center().y - label.size().y / 2.0;
    let on_track = visuals.text_color();
    // each text twice: clipped to the filled part in white, to the rest in the normal text color
    let (from, to) = bar.map_or((rect.left(), rect.left()), |b| (b.left().max(rect.left()), b.right().min(rect.right())));
    let span = |a: f32, b: f32| egui::Rect::from_x_y_ranges(a..=b, rect.y_range());
    for (clip, color) in [(span(rect.left(), from), on_track), (span(to, rect.right()), on_track), (span(from, to), Color32::WHITE)] {
        if clip.width() <= 0.0 {
            continue;
        }
        let p = painter.with_clip_rect(clip.intersect(painter.clip_rect()));
        p.galley_with_override_text_color(egui::pos2(rect.left() + pad, y), label.clone(), color);
        if let Some(g) = &pct {
            p.galley_with_override_text_color(egui::pos2(rect.right() - pad - g.size().x, y), g.clone(), color);
        }
    }
    response
}

pub fn level_color(frac: f32) -> Color32 {
    if frac > 0.9 {
        RED
    } else if frac > 0.7 {
        AMBER
    } else {
        GREEN
    }
}
