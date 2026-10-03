use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use eframe::egui::{self, RichText};

use super::{App, FreeModels, or_default};
use crate::envfile::{self, EnvVar};
use crate::flows::Provider;
use crate::health;
use crate::icons;
use crate::paths;
use crate::servers::Server;
use crate::sys;
use crate::ui::{self, AMBER, GREEN, RED};

impl App {
    pub(super) fn settings_page(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.heading("Settings");
            ui.label(
                RichText::new(
                    "Keys and passwords are saved in the .env file of the local-llm folder. It is git-ignored, so it is never uploaded.",
                )
                .weak(),
            );
            ui.add_space(8.0);
            ui::card(ui, |ui| self.llm_settings(ui));
            ui.add_space(8.0);
            ui::card(ui, |ui| self.secret_settings(ui));
            ui.add_space(8.0);
            ui::card(ui, |ui| self.storage_settings(ui));
            ui.add_space(8.0);
            ui::card(ui, |ui| self.install_settings(ui));
        });
    }

    fn section(ui: &mut egui::Ui, icon: icons::Icon, title: &str) {
        ui.horizontal(|ui| {
            ui.add(icon.image(20.0, ui::ACCENT));
            ui.label(RichText::new(title).strong().size(16.0));
        });
        ui.add_space(4.0);
    }

    fn env_field(&mut self, ui: &mut egui::Ui, var: &EnvVar) {
        let Some(r) = &self.repo else { return };
        let current = r.env.get(var.key);
        ui.label(var.label).on_hover_text(var.key);
        ui.vertical(|ui| {
            let value = self.env_edit.entry(var.key).or_insert(current);
            ui.add(egui::TextEdit::singleline(value).password(var.secret).desired_width(380.0).hint_text(var.key));
            ui.horizontal(|ui| {
                ui.label(RichText::new(var.help).small().weak());
                if !var.url.is_empty() {
                    ui.hyperlink_to(RichText::new("get it").small(), var.url);
                }
            });
        });
        ui.end_row();
    }

    fn text_field(&mut self, ui: &mut egui::Ui, key: &'static str, label: &str, default: &str, help: &str) {
        let Some(r) = &self.repo else { return };
        let current = or_default(&r.env.get(key), default);
        ui.label(label).on_hover_text(key);
        ui.vertical(|ui| {
            let value = self.env_edit.entry(key).or_insert(current);
            ui.add(egui::TextEdit::singleline(value).desired_width(380.0).hint_text(default));
            ui.label(RichText::new(help).small().weak());
        });
        ui.end_row();
    }

    fn save_button(&mut self, ui: &mut egui::Ui) {
        let Some(r) = &self.repo else { return };
        let changed: Vec<(&'static str, String)> =
            self.env_edit.iter().filter(|(k, v)| r.env.get(k) != v.trim()).map(|(k, v)| (*k, v.clone())).collect();
        let env_file = r.paths.env_file();
        ui.horizontal(|ui| {
            if ui::button_enabled(ui, !changed.is_empty(), icons::CHECK_CIRCLE, "Save").clicked() {
                self.save_env(&changed);
                self.env_edit.clear();
            }
            if ui::tool(ui, icons::FILE_DOCUMENT_EDIT, "open .env in the editor").clicked() {
                sys::open(&env_file);
            }
            ui::icon(ui, icons::SHIELD_LOCK, GREEN).on_hover_text(".env is git-ignored");
        });
    }

    fn llm_settings(&mut self, ui: &mut egui::Ui) {
        Self::section(ui, icons::ROBOT, "Vision AI (captions, clip ratings, motion prompts)");
        let llm = self.llm();
        let mut provider = llm.provider;
        ui.horizontal(|ui| {
            ui.radio_value(&mut provider, Provider::OpenRouter, "OpenRouter - free cloud models, no GPU memory used");
            ui.radio_value(&mut provider, Provider::OpenCode, "OpenCode Zen - free cloud models, no key");
            ui.radio_value(&mut provider, Provider::Ollama, "Ollama - local and offline");
        });
        if provider != llm.provider {
            self.save_env(&[("LLM_PROVIDER", provider.env_value().into())]);
        }
        egui::Grid::new("llm").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            if provider == Provider::OpenRouter {
                self.env_field(ui, envfile::var("OPENROUTER_API_KEY").expect("known var"));
                self.openrouter_model_field(ui);
                self.text_field(
                    ui,
                    "OPENROUTER_FALLBACKS",
                    "Fallback models",
                    "google/gemma-4-31b-it:free,dots-studio/dots-3-note-preview:free,nvidia/nemotron-3-nano-omni-30b-a3b-reasoning:free",
                    "up to 3 more models, comma separated - tried in order when the first fails or is rate limited",
                );
                self.text_field(
                    ui,
                    "LLM_LOCAL_FALLBACK",
                    "Then local Ollama",
                    "1",
                    "1 = use the local Ollama model when all OpenRouter models fail, 0 = give up",
                );
            }
            if provider == Provider::OpenCode {
                self.text_field(
                    ui,
                    "OPENCODE_MODEL",
                    "Model",
                    "opencode/muse-spark-1.3-contributor-free",
                    "a free OpenCode Zen model (e.g. opencode/mimo-v2.6-flash-free) - served by opencode-wrap, started when needed",
                );
                self.text_field(
                    ui,
                    "LLM_LOCAL_FALLBACK",
                    "Then local Ollama",
                    "1",
                    "1 = use the local Ollama model when the Zen model fails, 0 = give up",
                );
            }
            self.text_field(ui, "OLLAMA_MODEL", "Ollama model", "qwen3-vl:4b", "local vision model (downloaded under Models)");
        });
        self.save_button(ui);
    }

    fn openrouter_model_field(&mut self, ui: &mut egui::Ui) {
        let Some(r) = &self.repo else { return };
        let current = or_default(&r.env.get("OPENROUTER_MODEL"), "qwen/qwen3.8-27b:free");
        ui.label("Model");
        ui.horizontal(|ui| {
            let value = self.env_edit.entry("OPENROUTER_MODEL").or_insert(current);
            ui.add(egui::TextEdit::singleline(value).desired_width(300.0));
            let state = &mut *self.free_models.lock().unwrap();
            match state {
                FreeModels::Loaded(list) => {
                    egui::ComboBox::from_id_salt("free").selected_text("pick a free model").show_ui(ui, |ui| {
                        for m in list.iter() {
                            ui.selectable_value(value, m.clone(), m.as_str());
                        }
                    });
                }
                FreeModels::Loading => {
                    ui.spinner();
                }
                FreeModels::Failed(e) => {
                    ui::status(ui, icons::ALERT_CIRCLE, RED, "list failed").on_hover_text(e.as_str());
                }
                FreeModels::NotLoaded => {
                    if ui::button(ui, icons::CLOUD_OUTLINE, "List free vision models").clicked() {
                        *state = FreeModels::Loading;
                        fetch_free_models(self.free_models.clone(), ui.ctx().clone());
                    }
                }
            }
        });
        ui.end_row();
    }

    fn secret_settings(&mut self, ui: &mut egui::Ui) {
        Self::section(ui, icons::KEY, "Accounts");
        egui::Grid::new("secrets").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            for key in ["TELEGRAM_API_ID", "TELEGRAM_API_HASH", "TELEGRAM_PASSWORD", "HF_TOKEN"] {
                self.env_field(ui, envfile::var(key).expect("known var"));
            }
        });
        self.save_button(ui);
    }

    fn storage_settings(&mut self, ui: &mut egui::Ui) {
        let Some(r) = &self.repo else { return };
        let default = r.paths.default_models();
        let dir = self.models_dir();
        Self::section(ui, icons::HARDDISK, "Model storage");
        let mut change: Option<PathBuf> = None;
        ui.horizontal(|ui| {
            ui.label("Models folder:").on_hover_text("MODELS_DIR in .env - also used by the .bat files");
            if ui.link(dir.display().to_string()).clicked() {
                sys::open(&dir);
            }
            if let Some(f) = sys::free_space(&dir) {
                ui.label(RichText::new(format!("{} free", sys::human(f))).weak());
            }
            if ui::button(ui, icons::FOLDER_COG, "Change").clicked()
                && let Some(d) = rfd::FileDialog::new().set_title("One folder for all models").pick_folder()
            {
                change = Some(d);
            }
            if dir != default && ui::tool(ui, icons::REFRESH, "back to the default folder (models in the local-llm folder)").clicked() {
                change = Some(default.clone());
            }
        });
        ui.label(
            RichText::new(format!(
                "Everything in one place: ComfyUI's folders (checkpoints, diffusion_models, vae ...) directly in it,                  Ollama in '{}', Hugging Face in '{}', torch hub in '{}', rembg in '{}', Kokoro voices in 'kokoro'.                  Files in old folders are offered to be moved on the Models page, nothing is downloaded again.                  Restart ComfyUI and Ollama after a change.",
                paths::OLLAMA,
                paths::HUGGINGFACE,
                paths::TORCH,
                paths::U2NET
            ))
            .small()
            .weak(),
        );
        if let Some(new) = change.filter(|d| *d != dir) {
            self.cfg.models_prev = Some(dir);
            let value = if new == default { String::new() } else { new.display().to_string() };
            self.save_env(&[(paths::MODELS_KEY, value)]);
            if self.servers.up(Server::Ollama) || self.servers.up(Server::Comfy) {
                self.notify("Models folder changed - stop and start ComfyUI and Ollama so they use it", false);
            }
            self.save_config();
        }
    }

    fn install_settings(&mut self, ui: &mut egui::Ui) {
        let Some(r) = &self.repo else { return };
        let root = r.paths.root.clone();
        let python = r.paths.python();
        Self::section(ui, icons::TOOLS, "Installation");
        ui.horizontal(|ui| {
            ui.label("local-llm folder:");
            if ui.link(root.display().to_string()).clicked() {
                sys::open(&root);
            }
            if ui::tool(ui, icons::FOLDER_COG, "use another checkout").clicked()
                && let Some(dir) = rfd::FileDialog::new().pick_folder()
            {
                if paths::is_repo(&dir) {
                    self.open_repo(dir);
                } else {
                    self.notify("That folder is not a local-llm checkout", true);
                }
            }
        });
        ui.horizontal(|ui| {
            if python.is_file() {
                ui::status(ui, icons::CHECK_CIRCLE, GREEN, "ComfyUI + Python installed");
            } else {
                ui::status(ui, icons::ALERT, AMBER, "ComfyUI + Python not installed yet");
            }
            if ui::button(ui, icons::DOWNLOAD, "Run setup.bat")
                .on_hover_text("installs Ollama, ComfyUI portable and the Python packages (safe to re-run)")
                .clicked()
            {
                self.run_setup_bat();
            }
            if ui::button(ui, icons::SOURCE_PULL, "Update (git pull)").clicked() {
                let mut cmd = Command::new("cmd");
                cmd.args(["/K", "git", "pull", "--ff-only"]).current_dir(&root);
                sys::new_console(&mut cmd);
                if let Err(e) = cmd.spawn() {
                    self.notify(format!("could not run git: {e}"), true);
                }
            }
            self.desktop_shortcut_button(ui, &root);
        });
        match self.health.latest().and_then(|s| s.gpu) {
            Some(g) => {
                ui.horizontal(|ui| {
                    let text =
                        format!("{} · driver {}{}", g.name, g.driver, g.cuda.map(|(a, b)| format!(" · CUDA {a}.{b}")).unwrap_or_default());
                    match health::driver_ok(&g.driver) {
                        Some(true) => ui::status(ui, icons::CHECK_CIRCLE, GREEN, text),
                        _ => ui::status(ui, icons::ALERT, AMBER, format!("{text} - ComfyUI needs driver {}+", health::MIN_DRIVER)),
                    };
                    if ui::button(ui, icons::OPEN_IN_NEW, "NVIDIA drivers").clicked() {
                        sys::open_url("https://www.nvidia.com/en-us/drivers/");
                    }
                });
            }
            None => {
                ui::status(ui, icons::ALERT, AMBER, "no NVIDIA GPU / driver found");
            }
        }
        ui.label(
            RichText::new(
                "Ollama is started with low-VRAM settings (flash attention, q8 KV cache, one model at a time) so it fits next to ComfyUI.",
            )
            .small()
            .weak(),
        );
    }

    pub(super) fn run_setup_bat(&mut self) {
        let Some(r) = &self.repo else { return };
        let root = r.paths.root.clone();
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(root.join("setup.bat")).current_dir(&root);
        sys::new_console(&mut cmd);
        if let Err(e) = cmd.spawn() {
            self.notify(format!("could not run setup.bat: {e}"), true);
        }
    }

    fn desktop_shortcut_button(&mut self, ui: &mut egui::Ui, root: &std::path::Path) {
        if !ui::button(ui, icons::DESKTOP_TOWER, "Create desktop shortcut")
            .on_hover_text("local-llm.lnk on the Desktop, starting this exe in the local-llm folder (replaces an old one)")
            .clicked()
        {
            return;
        }
        match std::env::current_exe().map_err(|e| e.to_string()).and_then(|exe| sys::create_desktop_shortcut(&exe, root)) {
            Ok(lnk) => self.notify(format!("Shortcut created: {}", lnk.display()), false),
            Err(e) => self.notify(format!("could not create the shortcut: {e}"), true),
        }
    }
}

fn fetch_free_models(state: Arc<std::sync::Mutex<FreeModels>>, ctx: egui::Context) {
    std::thread::spawn(move || {
        let result = (|| -> Result<Vec<String>, String> {
            let mut resp = ureq::get("https://openrouter.ai/api/v1/models").call().map_err(|e| e.to_string())?;
            let v: serde_json::Value = resp.body_mut().with_config().limit(32 << 20).read_json().map_err(|e| e.to_string())?;
            let mut ids: Vec<String> = v["data"]
                .as_array()
                .ok_or("unexpected answer")?
                .iter()
                .filter(|m| m["id"].as_str().is_some_and(|id| id.ends_with(":free")))
                .filter(|m| m["architecture"]["input_modalities"].as_array().is_some_and(|a| a.iter().any(|x| x == "image")))
                .filter_map(|m| m["id"].as_str().map(str::to_owned))
                .collect();
            ids.sort();
            Ok(ids)
        })();
        *state.lock().unwrap() = match result {
            Ok(ids) => FreeModels::Loaded(ids),
            Err(e) => FreeModels::Failed(e),
        };
        ctx.request_repaint();
    });
}
