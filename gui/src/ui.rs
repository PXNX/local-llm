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

pub fn level_color(frac: f32) -> Color32 {
    if frac > 0.9 {
        RED
    } else if frac > 0.7 {
        AMBER
    } else {
        GREEN
    }
}
