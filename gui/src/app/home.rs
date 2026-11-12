use std::collections::BTreeSet;

use eframe::egui::{self, RichText};

use super::App;
use crate::config::Page;
use crate::flows::{FlowId, Provider};
use crate::health;
use crate::icons;
use crate::models::Task;
use crate::sys;
use crate::ui::{self, amber, green};

impl App {
    /// Start page: the flows as tiles, what is still missing, how the machine is doing.
    pub(super) fn home_page(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui::page_header(
                ui,
                icons::HOME,
                "Start",
                "Pick a flow below or in the sidebar. Anything a flow is missing is listed further down - one click fixes it.",
            );
            self.queue_summary(ui);
            self.flow_tiles(ui);
            ui.add_space(12.0);
            self.checklist(ui);
            ui.add_space(12.0);
            self.health_section(ui);
        });
    }

    /// Every flow as a clickable card, two per row, with what it needs before it can run.
    fn flow_tiles(&mut self, ui: &mut egui::Ui) {
        let mut pick = None;
        let gap = 10.0;
        let width = ((ui.available_width() - gap) / 2.0).floor().max(200.0);
        for row in FlowId::ALL.chunks(2) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for id in row {
                    let (title, text) = {
                        let form = self.cfg.forms.get(*id);
                        (form.title(), form.blurb())
                    };
                    let (env, models) = self.missing(&self.needs(*id));
                    let missing = env.len() + models.len();
                    let status = if missing == 0 {
                        ("ready", green())
                    } else {
                        ("needs setup", amber())
                    };
                    let selected = self.cfg.page == Page::Flow(*id);
                    if ui::tile(ui, width, id.icon(), title, text, Some(status), selected) {
                        pick = Some(*id);
                    }
                }
            });
            ui.add_space(gap);
        }
        if let Some(id) = pick {
            self.select(Page::Flow(id));
        }
    }

    fn checklist(&mut self, ui: &mut egui::Ui) {
        let Some(r) = &self.repo else { return };
        self.poll_llama_router();
        let python_ok = r.paths.python().is_file();
        let llm = self.llm();
        let llm_ok = match llm.provider {
            Provider::OpenRouter => r.env.is_set("OPENROUTER_API_KEY"),
            Provider::Ollama => r.models.status(&crate::models::ollama_id(&llm.ollama_model)).usable(),
            Provider::OpenCode => crate::flows::opencode2_installed(),
            Provider::LlamaCpp => {
                matches!(self.coding.lock().unwrap().llama.status.as_ref(), Some(Ok(router)) if router.running)
            }
        };
        let needed: Vec<String> = FlowId::ALL.iter().flat_map(|id| self.needs(*id).models).collect::<BTreeSet<_>>().into_iter().collect();
        let missing: Vec<String> = needed.iter().filter(|m| !r.models.status(m).usable()).cloned().collect();
        let missing_bytes = r.models.missing_bytes(&missing);
        let gpu = self.health.latest().and_then(|s| s.gpu);
        let telegram_ok = r.env.is_set("TELEGRAM_API_ID") && r.env.is_set("TELEGRAM_API_HASH");

        let mut action = None;
        ui::card(ui, |ui| {
            ui::card_title(ui, icons::WRENCH, "Setup", Some("one click fixes each"));
            let row = |ui: &mut egui::Ui, ok: bool, optional: bool, title: &str, detail: String, button: Option<&str>| -> bool {
                let mut clicked = false;
                ui.horizontal(|ui| {
                    let (icon, color) = if ok {
                        (icons::CHECK_CIRCLE, green())
                    } else if optional {
                        (icons::INFORMATION_OUTLINE, amber())
                    } else {
                        (icons::ALERT_CIRCLE, amber())
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
                Provider::OpenCode if llm_ok => "OpenCode Zen (free cloud models, no key, no GPU memory used)".into(),
                Provider::OpenCode => "OpenCode 2 is missing - run opencode-wrap\\install-opencode2.bat".into(),
                Provider::LlamaCpp if llm_ok => format!(
                    "llama.cpp router, model {}",
                    crate::llamacpp::model_id(&r.env.get(crate::llamacpp::MODEL_KEY))
                ),
                Provider::LlamaCpp => "llama.cpp router is not running - start it on the Coding page".into(),
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
            let stray: u64 = r.models.move_plan().iter().map(|m| m.size).sum();
            let folder = self.models_dir();
            let folder_text = if stray == 0 {
                format!("every model is in {}", folder.display())
            } else {
                format!("{} still elsewhere - move it into {}", sys::human(stray), folder.display())
            };
            if row(ui, stray == 0, false, "One models folder", folder_text, Some("Move")) {
                action = Some(5);
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
            Some(5) => self.select(Page::Models),
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
