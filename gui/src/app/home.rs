use std::collections::BTreeSet;

use eframe::egui::{self, RichText};

use super::App;
use crate::config::Page;
use crate::flows::{FlowId, Provider};
use crate::health;
use crate::icons;
use crate::models::Task;
use crate::sys;
use crate::ui::{self, AMBER, GREEN};

impl App {
    /// Start page: what runs (short), what is missing, how the machine is doing.
    pub(super) fn home_page(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.heading("Start");
            ui.label(
                RichText::new("Pick something to make on the left. Anything still missing is listed here - one click fixes it.").weak(),
            );
            ui.add_space(10.0);
            self.queue_summary(ui);
            self.checklist(ui);
            ui.add_space(10.0);
            self.health_section(ui);
        });
    }

    fn checklist(&mut self, ui: &mut egui::Ui) {
        let Some(r) = &self.repo else { return };
        let python_ok = r.paths.python().is_file();
        let llm = self.llm();
        let llm_ok = match llm.provider {
            Provider::OpenRouter => r.env.is_set("OPENROUTER_API_KEY"),
            Provider::Ollama => r.models.status(&crate::models::ollama_id(&llm.ollama_model)).usable(),
        };
        let needed: Vec<String> = FlowId::ALL.iter().flat_map(|id| self.needs(*id).models).collect::<BTreeSet<_>>().into_iter().collect();
        let missing: Vec<String> = needed.iter().filter(|m| !r.models.status(m).usable()).cloned().collect();
        let missing_bytes = r.models.missing_bytes(&missing);
        let gpu = self.health.latest().and_then(|s| s.gpu);
        let telegram_ok = r.env.is_set("TELEGRAM_API_ID") && r.env.is_set("TELEGRAM_API_HASH");

        let mut action = None;
        ui::card(ui, |ui| {
            ui.label(RichText::new("Setup").strong().size(16.0));
            ui.add_space(4.0);
            let row = |ui: &mut egui::Ui, ok: bool, optional: bool, title: &str, detail: String, button: Option<&str>| -> bool {
                let mut clicked = false;
                ui.horizontal(|ui| {
                    let (icon, color) = if ok {
                        (icons::CHECK_CIRCLE, GREEN)
                    } else if optional {
                        (icons::INFORMATION_OUTLINE, AMBER)
                    } else {
                        (icons::ALERT_CIRCLE, AMBER)
                    };
                    ui.add(icon.image(20.0, color));
                    ui.vertical(|ui| {
                        ui.label(RichText::new(title).strong());
                        ui.label(RichText::new(detail).small().weak());
                    });
                    if !ok && let Some(b) = button {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            clicked = ui::button(ui, icons::WRENCH, b).clicked();
                        });
                    }
                });
                ui.add_space(4.0);
                clicked
            };
            if row(
                ui,
                python_ok,
                false,
                "ComfyUI and Python",
                if python_ok { "installed".into() } else { "not installed yet - runs setup.bat (about 2 GB)".into() },
                Some("Install"),
            ) {
                action = Some(0);
            }
            let (gpu_ok, gpu_text) = match &gpu {
                Some(g) => match health::driver_ok(&g.driver) {
                    Some(true) => (true, format!("{} · driver {}", g.name, g.driver)),
                    _ => (
                        false,
                        format!("{} · driver {} is older than {} - ComfyUI needs a newer one", g.name, g.driver, health::MIN_DRIVER),
                    ),
                },
                None => (false, "no NVIDIA GPU detected - image and video flows need one".into()),
            };
            if row(ui, gpu_ok, false, "NVIDIA graphics driver", gpu_text, Some("Get driver")) {
                action = Some(1);
            }
            let llm_text = match llm.provider {
                Provider::OpenRouter if llm_ok => "OpenRouter key set (free cloud models, no GPU memory used)".into(),
                Provider::OpenRouter => "add a free OpenRouter key - or switch to the local Ollama model".into(),
                Provider::Ollama if llm_ok => format!("local model {}", llm.ollama_model),
                Provider::Ollama => format!("local model {} not downloaded yet", llm.ollama_model),
            };
            if row(ui, llm_ok, false, "Vision AI (captions, ratings, prompts)", llm_text, Some("Set up")) {
                action = Some(2);
            }
            let models_text = if missing.is_empty() {
                format!("all {} files the flows need are on disk", needed.len())
            } else {
                format!("{} of {} files missing, {} to download", missing.len(), needed.len(), sys::human(missing_bytes))
            };
            if row(ui, missing.is_empty(), false, "Models", models_text, Some("Download all")) {
                action = Some(3);
            }
            if row(
                ui,
                telegram_ok,
                true,
                "Telegram (only for Top 5 videos)",
                if telegram_ok { "API ID and hash set".into() } else { "optional - needed to read channels".into() },
                Some("Set up"),
            ) {
                action = Some(4);
            }
        });
        match action {
            Some(0) => self.run_setup_bat(),
            Some(1) => sys::open_url("https://www.nvidia.com/en-us/drivers/"),
            Some(2) | Some(4) => self.select(Page::Settings),
            Some(3) => {
                if let Some(r) = &self.repo {
                    for m in &missing {
                        r.models.enqueue(m, Task::Download);
                    }
                }
                self.select(Page::Models);
            }
            _ => {}
        }
    }
}
