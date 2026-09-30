use std::process::Command;

use eframe::egui::{self, RichText};

use super::{App, status_text};
use crate::envfile;
use crate::flows::{FlowId, Provider};
use crate::icons;
use crate::models::Dl;
use crate::models::Task;
use crate::paths;
use crate::sys;
use crate::ui::{self, AMBER, GREEN};

pub const REPO_URL: &str = "https://github.com/PXNX/local-llm.git";

impl App {
    /// Dialog listing what a flow still lacks: env vars to type in and models to download.
    pub(super) fn setup_dialog(&mut self, ctx: &egui::Context, id: FlowId) {
        let needs = self.needs(id);
        let (env, models) = self.missing(&needs);
        let title = self.cfg.forms.get(id).title();
        let mut close = false;
        let mut run = false;
        let modal = egui::Modal::new(egui::Id::new("setup")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.heading(format!("Set up {title}"));
            if env.is_empty() && models.is_empty() {
                ui::status(ui, icons::CHECK_CIRCLE, GREEN, "Everything this flow needs is ready.");
            }

            if !env.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new("Settings (saved to .env, never committed)").strong());
                egui::Grid::new("env").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    for key in &env {
                        let var = envfile::var(key);
                        ui.label(var.map_or(*key, |v| v.label));
                        ui.vertical(|ui| {
                            let value = self.env_edit.entry(key).or_default();
                            ui.add(
                                egui::TextEdit::singleline(value)
                                    .password(var.is_some_and(|v| v.secret))
                                    .desired_width(320.0)
                                    .hint_text(*key),
                            );
                            if let Some(v) = var {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(v.help).small().weak());
                                    if !v.url.is_empty() {
                                        ui.hyperlink_to(RichText::new("get it").small(), v.url);
                                    }
                                });
                            }
                        });
                        ui.end_row();
                    }
                });
                if env.contains(&"OPENROUTER_API_KEY") && ui.link("or use the local Ollama model instead of OpenRouter").clicked() {
                    self.save_env(&[("LLM_PROVIDER", Provider::Ollama.env_value().into())]);
                }
            }

            if !models.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new("Models").strong());
                let r = self.repo.as_ref().expect("repo");
                let bytes = r.models.missing_bytes(&models);
                egui::Grid::new("models").num_columns(3).spacing([10.0, 4.0]).striped(true).show(ui, |ui| {
                    for m in &models {
                        let name = r.models.comfy_model(m).map_or_else(|| m.trim_start_matches("ollama:").to_owned(), |c| c.file.clone());
                        ui.label(name);
                        ui.label(r.models.size_of(m).map(sys::human).unwrap_or_else(|| "?".into()));
                        match r.models.download(m) {
                            Some(Dl::Queued) => {
                                ui.label("queued");
                            }
                            Some(Dl::Running { done, total, .. }) => {
                                let frac = total.map_or(0.0, |t| done as f32 / t.max(1) as f32);
                                ui.add(egui::ProgressBar::new(frac).desired_width(160.0).show_percentage());
                            }
                            Some(Dl::Failed(e)) => {
                                ui.colored_label(AMBER, e.clone()).on_hover_text(e);
                            }
                            None => {
                                let (icon, text, color) = status_text(&r.models.status(m));
                                ui::status(ui, icon, color, text);
                            }
                        }
                        ui.end_row();
                    }
                });
                let target = r.models.target_root().unwrap_or_default();
                let free = sys::free_space(&target);
                ui.label(
                    RichText::new(format!(
                        "{} to download into {}{}",
                        sys::human(bytes),
                        target.display(),
                        free.map(|f| format!(" ({} free)", sys::human(f))).unwrap_or_default()
                    ))
                    .small()
                    .weak(),
                );
                if free.is_some_and(|f| f < bytes) {
                    ui.colored_label(AMBER, "Not enough free space - choose another models folder in Settings.");
                }
                if r.models.needs_token(&models) && !r.env.is_set("HF_TOKEN") {
                    ui.horizontal(|ui| {
                        ui.label("Hugging Face token (gated model):");
                        let value = self.env_edit.entry("HF_TOKEN").or_default();
                        ui.add(egui::TextEdit::singleline(value).password(true).desired_width(240.0));
                        ui.hyperlink_to("get it", "https://huggingface.co/settings/tokens");
                    });
                }
                let r = self.repo.as_ref().expect("repo");
                if ui::button(ui, icons::DOWNLOAD, &format!("Download missing ({})", sys::human(bytes))).clicked() {
                    for m in &models {
                        r.models.enqueue(m, Task::Download);
                    }
                }
            }

            ui.add_space(10.0);
            ui.separator();
            ui.horizontal(|ui| {
                let typed = self.env_edit.iter().any(|(_, v)| !v.trim().is_empty());
                if ui::button_enabled(ui, typed, icons::CHECK_CIRCLE, "Save").clicked() {
                    self.save_typed_env();
                }
                if ui::primary(ui, env.is_empty() && models.is_empty(), icons::PLAY, "Run now").clicked() {
                    run = true;
                }
                if ui.button("Later").clicked() {
                    close = true;
                }
            });
            if self.repo.as_ref().is_some_and(|r| r.models.busy()) {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
            }
        });
        if modal.should_close() {
            close = true;
        }
        if run {
            self.prompt = None;
            self.enqueue(id);
        } else if close {
            self.prompt = None;
        }
    }

    /// Writes the non-empty values typed into env fields to .env.
    pub(super) fn save_typed_env(&mut self) {
        let typed: Vec<(&'static str, String)> = self.env_edit.drain().filter(|(_, v)| !v.trim().is_empty()).collect();
        self.save_env(&typed);
    }

    pub(super) fn save_env(&mut self, values: &[(&str, String)]) {
        let Some(r) = &mut self.repo else { return };
        for (k, v) in values {
            r.env.set(k, v);
        }
        match r.env.save() {
            Ok(()) => self.notify("Saved to .env", false),
            Err(e) => self.notify(format!("Could not write .env: {e}"), true),
        }
        self.configure_models();
    }

    /// First start: no local-llm checkout found next to the exe.
    pub(super) fn setup_screen(&mut self, ui: &mut egui::Ui) {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.heading("Where is your local-llm folder?");
            ui.add_space(8.0);
            ui.label("This app runs the flows of a local-llm checkout (scripts, ComfyUI, models).");
            ui.add_space(16.0);
            if ui.button("Choose the local-llm folder...").clicked()
                && let Some(dir) = rfd::FileDialog::new().pick_folder()
            {
                if paths::is_repo(&dir) {
                    self.open_repo(dir);
                } else {
                    self.notify(format!("{} is not a local-llm checkout (stickers/make_stickers.py missing)", dir.display()), true);
                }
            }
            ui.add_space(8.0);
            if ui.button("Download it from GitHub into...").clicked()
                && let Some(parent) = rfd::FileDialog::new().set_title("Folder to clone local-llm into").pick_folder()
            {
                let target = parent.join("local-llm");
                let mut cmd = Command::new("git");
                cmd.args(["clone", REPO_URL]).arg(&target);
                sys::new_console(&mut cmd);
                match cmd.spawn() {
                    Ok(_) => {
                        self.cfg.repo = Some(target.clone());
                        self.notify(format!("Cloning into {} - then run setup.bat there (Settings)", target.display()), false);
                    }
                    Err(e) => self.notify(format!("git not found ({e}) - install Git for Windows"), true),
                }
            }
            ui.add_space(12.0);
            self.notice_ui(ui);
        });
    }
}
