#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod envfile;
mod flows;
mod fonts;
mod health;
mod icons;
mod jobs;
mod models;
mod paths;
mod servers;
mod sys;
mod ui;

use eframe::egui;

fn main() -> eframe::Result {
    sys::lower_own_priority();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("local-llm")
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([860.0, 540.0])
            .with_drag_and_drop(true)
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).expect("valid icon png")),
        // no MSAA/depth/stencil buffers: the GUI keeps its GPU footprint minimal for ComfyUI/Ollama
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        ..Default::default()
    };
    eframe::run_native("local-llm", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
