//! Fonts and text sizes. Windows' own fonts are loaded as fallbacks (nothing embedded), so the log
//! shows Cyrillic, Arabic, symbols and emoji from Telegram captions and tool output.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, FontData, FontFamily, FontId, TextStyle};

pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let dir = std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from).join("Fonts");
    // (name, file, first choice for monospace)
    for (name, file, mono) in [
        ("consolas", "consola.ttf", true),
        ("segoe", "segoeui.ttf", false),
        ("symbols", "seguisym.ttf", false),
        ("emoji", "seguiemj.ttf", false),
    ] {
        let Ok(bytes) = std::fs::read(dir.join(file)) else { continue };
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_owned(bytes)));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            let list = fonts.families.entry(family.clone()).or_default();
            if mono && family == FontFamily::Monospace {
                list.insert(0, name.into());
            } else if !mono {
                list.push(name.into());
            }
        }
    }
    ctx.set_fonts(fonts);
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(11.5)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Heading, FontId::proportional(22.0)),
            (TextStyle::Monospace, FontId::monospace(12.5)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        style.spacing.interact_size.y = 24.0;
    });
}
