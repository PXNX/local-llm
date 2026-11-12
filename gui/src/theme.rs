//! The window's look: two palettes (dark and light), one set of text sizes, one set of frames.
//!
//! Everything that paints colours comes from here (or from [`crate::ui`], which re-exports them), so a
//! page can't drift into its own shades. The colours are functions, not constants, because the window
//! can switch between the dark and the light palette at run time; the active one is a process-wide
//! flag that [`install`] and [`set_mode`] write. egui's defaults are re-tuned at start-up and on every
//! switch: [`install`] sets the visuals and the spacing, the pages only compose.

use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{self, Color32, CornerRadius, FontId, Margin, Stroke, TextStyle};

/// Which palette the window paints with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Dark,
    Light,
}

struct Palette {
    bg: Color32,
    chrome: Color32,
    surface: Color32,
    surface_high: Color32,
    border: Color32,
    sunken: Color32,
    text: Color32,
    muted: Color32,
    faint: Color32,
    accent: Color32,
    accent_soft: Color32,
    accent_dim: Color32,
    hover: Color32,
    green: Color32,
    amber: Color32,
    red: Color32,
}

const DARK: Palette = Palette {
    bg: Color32::from_rgb(17, 20, 26),
    chrome: Color32::from_rgb(21, 25, 32),
    surface: Color32::from_rgb(27, 32, 41),
    surface_high: Color32::from_rgb(34, 40, 51),
    border: Color32::from_rgb(46, 54, 69),
    sunken: Color32::from_rgb(12, 15, 20),
    text: Color32::from_rgb(232, 236, 244),
    muted: Color32::from_rgb(150, 161, 179),
    faint: Color32::from_rgb(105, 115, 132),
    accent: Color32::from_rgb(88, 140, 255),
    accent_soft: Color32::from_rgb(40, 56, 88),
    accent_dim: Color32::from_rgb(47, 72, 120),
    hover: Color32::from_rgb(42, 49, 62),
    green: Color32::from_rgb(94, 196, 122),
    amber: Color32::from_rgb(235, 176, 65),
    red: Color32::from_rgb(235, 100, 92),
};

const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(243, 246, 251),
    chrome: Color32::from_rgb(255, 255, 255),
    surface: Color32::from_rgb(255, 255, 255),
    surface_high: Color32::from_rgb(238, 242, 248),
    border: Color32::from_rgb(213, 220, 231),
    sunken: Color32::from_rgb(244, 247, 251),
    text: Color32::from_rgb(26, 34, 48),
    muted: Color32::from_rgb(90, 102, 120),
    faint: Color32::from_rgb(139, 149, 165),
    accent: Color32::from_rgb(47, 107, 224),
    accent_soft: Color32::from_rgb(219, 231, 251),
    accent_dim: Color32::from_rgb(167, 191, 232),
    hover: Color32::from_rgb(228, 234, 243),
    green: Color32::from_rgb(31, 138, 76),
    amber: Color32::from_rgb(169, 105, 11),
    red: Color32::from_rgb(192, 57, 43),
};

/// True while the light palette is active.
static LIGHT_MODE: AtomicBool = AtomicBool::new(false);

fn palette() -> &'static Palette {
    if LIGHT_MODE.load(Ordering::Relaxed) { &LIGHT } else { &DARK }
}

pub fn mode() -> Mode {
    if LIGHT_MODE.load(Ordering::Relaxed) { Mode::Light } else { Mode::Dark }
}

/// Window background behind the page content.
pub fn bg() -> Color32 {
    palette().bg
}
/// Left navigation and the top bar - a shade off the background, separated by a border.
pub fn chrome() -> Color32 {
    palette().chrome
}
/// A card / group on the page.
pub fn surface() -> Color32 {
    palette().surface
}
/// One step lighter than a card: inputs, hovered rows, queue entries.
pub fn surface_high() -> Color32 {
    palette().surface_high
}
/// Card and panel borders.
pub fn border() -> Color32 {
    palette().border
}

pub fn text() -> Color32 {
    palette().text
}
pub fn muted() -> Color32 {
    palette().muted
}
pub fn faint() -> Color32 {
    palette().faint
}

pub fn accent() -> Color32 {
    palette().accent
}
/// A selected row's background: the accent, low over the card colour.
pub fn accent_soft() -> Color32 {
    palette().accent_soft
}
/// The same accent, muted, for 3D-ish edges (no alpha, so it looks the same on every surface).
pub fn accent_dim() -> Color32 {
    palette().accent_dim
}

pub fn green() -> Color32 {
    palette().green
}
pub fn amber() -> Color32 {
    palette().amber
}
pub fn red() -> Color32 {
    palette().red
}

/// Corner radius of cards and groups.
pub const RADIUS: u8 = 10;
/// Inner margin of a card.
pub const CARD_MARGIN: i8 = 14;

/// Applies the palette, text sizes and spacing to the whole window. Called once before the first
/// frame, with the mode saved in the config, and again whenever the mode is switched.
pub fn install(ctx: &egui::Context, mode: Mode) {
    LIGHT_MODE.store(mode == Mode::Light, Ordering::Relaxed);
    // Written to both theme slots: egui asks for the one the OS reports, and `set_visuals` alone only
    // touches whichever is active at that moment (a window that starts light would keep light widgets).
    ctx.set_theme(match mode {
        Mode::Dark => egui::Theme::Dark,
        Mode::Light => egui::Theme::Light,
    });
    ctx.set_visuals_of(egui::Theme::Dark, visuals(Mode::Dark));
    ctx.set_visuals_of(egui::Theme::Light, visuals(Mode::Light));
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(11.5)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Heading, FontId::proportional(21.0)),
            (TextStyle::Monospace, FontId::monospace(12.5)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size.y = 26.0;
        style.spacing.menu_margin = Margin::same(8);
        style.spacing.indent = 18.0;
        style.spacing.window_margin = Margin::same(14);
        // a slim bar that doesn't steal width from the content
        style.spacing.scroll.floating = true;
        style.spacing.scroll.bar_width = 10.0;
        style.spacing.scroll.floating_width = 5.0;
        style.spacing.scroll.bar_inner_margin = 2.0;
        style.spacing.scroll.handle_min_length = 24.0;
    });
}

/// Switches the palette and restyles every open widget; the caller saves the choice.
pub fn set_mode(ctx: &egui::Context, mode: Mode) {
    install(ctx, mode);
    ctx.request_repaint();
}

fn visuals(mode: Mode) -> egui::Visuals {
    let p = match mode {
        Mode::Dark => &DARK,
        Mode::Light => &LIGHT,
    };
    let mut v = match mode {
        Mode::Dark => egui::Visuals::dark(),
        Mode::Light => egui::Visuals::light(),
    };
    v.panel_fill = p.bg;
    v.window_fill = p.surface;
    v.extreme_bg_color = p.sunken;
    v.faint_bg_color = p.surface_high;
    v.override_text_color = Some(p.text);
    v.weak_text_color = Some(p.muted);
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.accent_soft;
    v.selection.stroke = Stroke::new(1.0, p.text);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(10);

    let rounding = CornerRadius::same(8);
    v.widgets.noninteractive = egui::style::WidgetVisuals {
        bg_fill: p.surface,
        weak_bg_fill: p.surface,
        bg_stroke: Stroke::new(1.0, p.border),
        corner_radius: rounding,
        fg_stroke: Stroke::new(1.0, p.muted),
        expansion: 0.0,
    };
    v.widgets.inactive = egui::style::WidgetVisuals {
        bg_fill: p.surface_high,
        weak_bg_fill: p.surface_high,
        bg_stroke: Stroke::new(1.0, p.border),
        corner_radius: rounding,
        fg_stroke: Stroke::new(1.0, p.text),
        expansion: 0.0,
    };
    v.widgets.hovered = egui::style::WidgetVisuals {
        bg_fill: p.hover,
        weak_bg_fill: p.hover,
        bg_stroke: Stroke::new(1.0, p.accent_dim),
        corner_radius: rounding,
        fg_stroke: Stroke::new(1.0, p.text),
        expansion: 0.0,
    };
    v.widgets.active = egui::style::WidgetVisuals {
        bg_fill: p.accent_soft,
        weak_bg_fill: p.accent_soft,
        bg_stroke: Stroke::new(1.0, p.accent),
        corner_radius: rounding,
        fg_stroke: Stroke::new(1.0, p.text),
        expansion: 0.0,
    };
    v.widgets.open = v.widgets.active;
    v
}

/// The frame of the top bar and the sidebar: chrome colour, square corners, one border on the inside edge.
/// (Panels take their frame from `App::ui`; this keeps the same look when a page needs one.)
#[allow(dead_code)]
pub fn chrome_frame(edges: Margin, stroke: Stroke) -> egui::Frame {
    egui::Frame::new().fill(chrome()).inner_margin(edges).stroke(stroke).corner_radius(0)
}
