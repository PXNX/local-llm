use eframe::egui::{self, RichText};

use super::App;
use crate::health;
use crate::icons::{self, Icon};
use crate::sys::human;
use crate::ui::{self, AMBER};

impl App {
    /// CPU / RAM / GPU / VRAM, sampled every 2 s in the background (shown on the Start page).
    pub(super) fn health_section(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add(icons::HEART_PULSE.image(20.0, ui::ACCENT));
            ui.label(RichText::new("System").strong().size(16.0));
        });
        ui::card(ui, |ui| {
            let Some(s) = self.health.latest() else {
                ui.label(RichText::new("measuring...").weak());
                return;
            };
            egui::Grid::new("health").num_columns(3).spacing([12.0, 8.0]).show(ui, |ui| {
                let cpu = format!("{:.0} %", s.cpu);
                meter(ui, icons::CPU_6_4_BIT, "CPU", s.cpu / 100.0, cpu);
                let ram = s.ram_used as f32 / s.ram_total.max(1) as f32;
                meter(ui, icons::MEMORY, "RAM", ram, format!("{} of {}", human(s.ram_used), human(s.ram_total)));
                match &s.gpu {
                    Some(g) => {
                        meter(ui, icons::EXPANSION_CARD, "GPU", g.util as f32 / 100.0, format!("{} %", g.util));
                        let vram = g.vram_used as f32 / g.vram_total.max(1) as f32;
                        meter(ui, icons::CHIP, "VRAM", vram, format!("{} of {}", human(g.vram_used), human(g.vram_total)));
                    }
                    None => {
                        ui.label("GPU");
                        ui.label(RichText::new("no NVIDIA GPU found").weak());
                        ui.end_row();
                    }
                }
            });
            if let Some(g) = &s.gpu {
                ui.add_space(4.0);
                let text = format!(
                    "{} · driver {}{}{}",
                    g.name,
                    g.driver,
                    g.cuda.map(|(a, b)| format!(" · CUDA {a}.{b}")).unwrap_or_default(),
                    g.temp.map(|t| format!(" · {t} °C")).unwrap_or_default()
                );
                ui.label(RichText::new(text).small().weak());
                if health::driver_ok(&g.driver) == Some(false) {
                    ui.add(egui::Hyperlink::from_label_and_url(
                        RichText::new(format!(
                            "The NVIDIA driver is older than {} - ComfyUI needs a newer one (get it here)",
                            health::MIN_DRIVER
                        ))
                        .color(AMBER),
                        "https://www.nvidia.com/en-us/drivers/",
                    ));
                }
            }
        });
    }
}

fn meter(ui: &mut egui::Ui, icon: Icon, name: &str, frac: f32, text: String) {
    ui.horizontal(|ui| {
        let c = ui.visuals().text_color();
        ui.add(icon.image(18.0, c));
        ui.label(name);
    });
    let bar = egui::ProgressBar::new(frac.clamp(0.0, 1.0)).desired_width(320.0).fill(ui::level_color(frac));
    ui.add(bar);
    ui.label(text);
    ui.end_row();
}
