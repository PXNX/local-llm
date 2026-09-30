//! Material Design Icons (pictogrammers.com, Pictogrammers Free License) as SVG images.
//! The SVG documents are assembled at compile time; egui_extras rasterizes each once per size.

use const_format::concatcp;
use eframe::egui::{self, Color32, ImageSource, load::Bytes};
use material_design_icons as mdi;

#[derive(Clone, Copy)]
pub struct Icon {
    uri: &'static str,
    svg: &'static str,
}

impl Icon {
    pub fn image(self, size: f32, color: Color32) -> egui::Image<'static> {
        egui::Image::new(ImageSource::Bytes { uri: self.uri.into(), bytes: Bytes::Static(self.svg.as_bytes()) })
            .fit_to_exact_size(egui::vec2(size, size))
            .tint(color)
    }
}

macro_rules! icons {
    ($($name:ident),* $(,)?) => {
        $(pub const $name: Icon = Icon {
            uri: concatcp!("bytes://mdi/", stringify!($name), ".svg"),
            svg: concatcp!("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24'><path fill='#ffffff' d='", mdi::$name, "'/></svg>"),
        };)*
    };
}

icons!(
    ACCOUNT_GROUP,
    ALERT,
    ALERT_CIRCLE,
    ANIMATION_PLAY,
    ARROW_UP,
    CHECK_CIRCLE,
    CHIP,
    CIRCLE_OUTLINE,
    CLOSE,
    CLOUD_OUTLINE,
    COG,
    CONTENT_COPY,
    CPU_6_4_BIT,
    CUBE_OUTLINE,
    DATABASE,
    DELETE_OUTLINE,
    DESKTOP_TOWER,
    DOWNLOAD,
    DRAMA_MASKS,
    EXPANSION_CARD,
    FILE_DOCUMENT_EDIT,
    FILE_MULTIPLE,
    FILE_SEARCH_OUTLINE,
    FOLDER_COG,
    FOLDER_MOVE,
    FOLDER_OPEN,
    FOLDER_PLUS,
    FOLDER_SEARCH_OUTLINE,
    HARDDISK,
    HEART_PULSE,
    HELP_CIRCLE_OUTLINE,
    HOME,
    INFORMATION_OUTLINE,
    KEY,
    LINK,
    LINK_VARIANT,
    MEMORY,
    MOVIE_OPEN_PLAY,
    OPEN_IN_NEW,
    PLAY,
    PLAYLIST_EDIT,
    PLUS,
    POWER,
    PROGRESS_DOWNLOAD,
    REFRESH,
    ROBOT,
    SHARE_VARIANT,
    SHIELD_LOCK,
    SOURCE_PULL,
    STICKER_EMOJI,
    STOP,
    SWAP_HORIZONTAL,
    TEXT_BOX_EDIT,
    TIMER_SAND,
    TOOLS,
    TROPHY,
    VECTOR_CURVE,
    WAVEFORM,
    WRENCH,
);
