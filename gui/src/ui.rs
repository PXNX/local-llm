//! Small shared widgets: icon buttons, status lines, cards, page headers and tiles.

use eframe::egui::{self, Color32, Response, RichText};

use crate::icons::Icon;
use crate::theme::{self, muted, text};

pub use crate::theme::{accent, amber, green, red};

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
        .fill(accent())
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
    egui::Frame::new()
        .fill(theme::surface())
        .stroke(egui::Stroke::new(1.0, theme::border()))
        .corner_radius(theme::RADIUS)
        .inner_margin(theme::CARD_MARGIN)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A card's title row: icon, title, then whatever else the card wants on that row.
pub fn card_title_with(ui: &mut egui::Ui, icon: Icon, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add(icon.image(18.0, accent()));
        ui.label(RichText::new(title).size(15.5).color(text()));
        add(ui);
    });
    ui.add_space(2.0);
}

/// A card's title row with an optional one-line hint on the right.
pub fn card_title(ui: &mut egui::Ui, icon: Icon, title: &str, hint: Option<&str>) {
    card_title_with(ui, icon, title, |ui| {
        if let Some(hint) = hint {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(hint).small().color(muted()));
            });
        }
    });
}

/// The top of every page: icon, title, one line that says what the page is for.
pub fn page_header(ui: &mut egui::Ui, icon: Icon, title: &str, subtitle: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 15.0, theme::accent_soft());
        icon.image(19.0, accent()).paint_at(ui, egui::Rect::from_center_size(rect.center(), egui::vec2(19.0, 19.0)));
        ui.add_space(2.0);
        ui.label(RichText::new(title).size(21.0).color(text()));
    });
    ui.add_space(1.0);
    ui.label(RichText::new(subtitle).size(13.0).color(muted()));
    ui.add_space(10.0);
}

/// A small rounded label ("ready", "2 waiting").
pub fn pill(ui: &mut egui::Ui, text: impl Into<String>, color: Color32) -> Response {
    let text = text.into();
    let font = egui::FontId::proportional(11.5);
    let galley = ui.painter().layout_no_wrap(text, font, color);
    let padding = egui::vec2(8.0, 3.0);
    let size = galley.size() + padding * 2.0;
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, size.y / 2.0, color.gamma_multiply(0.18));
        ui.painter().galley(rect.min + padding, galley, color);
    }
    response
}

/// A clickable card for the start page: icon, title, one line of what it makes, and a status pill.
/// The caller passes the width - inside a grid the available width is the whole row, not the cell.
/// Returns true when clicked.
pub fn tile(ui: &mut egui::Ui, width: f32, icon: Icon, title: &str, blurb: &str, status: Option<(&str, Color32)>, selected: bool) -> bool {
    let height = 74.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if !ui.is_rect_visible(rect) {
        return response.clicked();
    }
    let hovered = response.hovered() || selected;
    let fill = if hovered { theme::surface_high() } else { theme::surface() };
    let stroke = if selected {
        egui::Stroke::new(1.5, accent())
    } else if response.hovered() {
        egui::Stroke::new(1.0, theme::accent_dim())
    } else {
        egui::Stroke::new(1.0, theme::border())
    };
    let painter = ui.painter();
    painter.rect(rect, theme::RADIUS, fill, stroke, egui::StrokeKind::Inside);
    let icon_rect = egui::Rect::from_min_size(rect.min + egui::vec2(16.0, 18.0), egui::vec2(26.0, 26.0));
    icon.image(24.0, if hovered { accent() } else { muted() }).paint_at(ui, icon_rect);
    let left = rect.left() + 54.0;
    let right = rect.right() - 16.0;
    painter.text(
        egui::pos2(left, rect.top() + 16.0),
        egui::Align2::LEFT_TOP,
        title,
        egui::FontId::proportional(15.0),
        text(),
    );
    let mut job = egui::text::LayoutJob::simple_singleline(blurb.to_owned(), egui::FontId::proportional(12.5), muted());
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - left).max(40.0) - 70.0);
    let galley = ui.painter().layout_job(job);
    painter.galley(egui::pos2(left, rect.top() + 40.0), galley, muted());
    if let Some((label, color)) = status {
        let galley = painter.layout_no_wrap(label.to_owned(), egui::FontId::proportional(11.5), color);
        let size = galley.size() + egui::vec2(16.0, 6.0);
        let pill = egui::Rect::from_min_size(egui::pos2(right - size.x, rect.top() + 14.0), size);
        painter.rect_filled(pill, size.y / 2.0, color.gamma_multiply(0.18));
        painter.galley(pill.min + egui::vec2(8.0, 3.0), galley, color);
    }
    response.clicked()
}


/// Numbered section heading ("1  Choose a photo").
pub fn step(ui: &mut egui::Ui, n: u32, text: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 11.0, accent());
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, n.to_string(), egui::FontId::proportional(13.0), Color32::WHITE);
        ui.label(RichText::new(text).strong().size(15.0));
    });
    ui.add_space(4.0);
}

/// The same, smaller, with room on the right for the button the step is about.
pub fn step_with(ui: &mut egui::Ui, n: u32, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 10.0, theme::accent_soft());
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, n.to_string(), egui::FontId::proportional(12.0), accent());
        ui.label(RichText::new(label).size(13.0).color(text()));
        add(ui);
    });
    ui.add_space(2.0);
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
        red()
    } else if frac > 0.7 {
        amber()
    } else {
        green()
    }
}
