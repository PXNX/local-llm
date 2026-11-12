use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use eframe::egui::{self, RichText};

use super::{App, FreeModels, or_default, setup};
use crate::envfile;
use crate::flows::Provider;
use crate::health;
use crate::icons;
use crate::paths;
use crate::servers::Server;
use crate::sys;
use crate::taskbar::sound::{self, Cue};
use crate::theme;
use crate::ui::{self, amber, green, red};

/// Result of the "Check" button next to the OpenRouter key (checked against the API, in a thread).
#[derive(Clone, Default)]
pub(super) enum KeyCheck {
    #[default]
    Idle,
    Checking,
    Ok(String),
    Failed(String),
}

impl App {
    pub(super) fn settings_page(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui::page_header(
                ui,
                icons::COG,
                "Settings",
                "Keys and passwords are saved in the .env file of the local-llm folder. It is git-ignored, so it is never uploaded.",
            );
            ui::card(ui, |ui| self.appearance_settings(ui));
            ui.add_space(8.0);
            ui::card(ui, |ui| self.llm_settings(ui));
            ui.add_space(8.0);
            ui::card(ui, |ui| self.openrouter_settings(ui));
            ui.add_space(8.0);
            ui::card(ui, |ui| self.telegram_settings(ui));
            ui.add_space(8.0);
            ui::card(ui, |ui| self.huggingface_settings(ui));
            ui.add_space(8.0);
            ui::card(ui, |ui| self.queue_settings(ui));
            ui.add_space(8.0);
            ui::card(ui, |ui| self.storage_settings(ui));
            ui.add_space(8.0);
            ui::card(ui, |ui| self.install_settings(ui));
        });
    }

    fn appearance_settings(&mut self, ui: &mut egui::Ui) {
        ui::card_title(ui, icons::PALETTE, "Appearance", Some("saved with the window"));
        ui.horizontal(|ui| {
            let mut mode = theme::mode();
            for (m, icon, text) in [
                (theme::Mode::Dark, icons::WEATHER_NIGHT, "Dark"),
                (theme::Mode::Light, icons::WEATHER_SUNNY, "Light"),
            ] {
                let selected = mode == m;
                let fg = if selected { egui::Color32::WHITE } else { ui.visuals().text_color() };
                let button = egui::Button::image_and_text(icon.image(18.0, fg), RichText::new(text).color(fg))
                    .fill(if selected { ui::accent() } else { ui.visuals().widgets.inactive.bg_fill })
                    .corner_radius(8)
                    .min_size(egui::vec2(132.0, 34.0));
                if ui.add(button).clicked() && !selected {
                    mode = m;
                    theme::set_mode(ui.ctx(), m);
                    self.cfg.dark = m == theme::Mode::Dark;
                    self.save_config();
                }
            }
            ui.label(RichText::new("switch any time - nothing else changes").small().color(theme::muted()));
        });
    }

    /// Green "connected" pill, or an amber one that says what is still missing.
    fn connect_pill(ui: &mut egui::Ui, ok: bool, done: &str, missing: &str) {
        if ok {
            ui::pill(ui, done, green());
        } else {
            ui::pill(ui, missing, amber());
        }
    }

    fn queue_settings(&mut self, ui: &mut egui::Ui) {
        Self::section(ui, icons::PLAYLIST_PLAY, "Queue");
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.cfg.sounds, "Play sounds")
                .on_hover_text("a ding when a run finishes, a low tone when it fails, a little fanfare when the whole queue is through");
            if self.cfg.sounds {
                for (label, cue) in [("Run done", Cue::RunDone), ("Run failed", Cue::RunFailed), ("Queue done", Cue::QueueDone)] {
                    if ui.small_button(format!("▶ {label}")).on_hover_text("play it").clicked() {
                        sound::play(cue);
                    }
                }
            }
        });
    }

    fn section(ui: &mut egui::Ui, icon: icons::Icon, title: &str) {
        ui::card_title(ui, icon, title, None);
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

    /// A key / password field: masked unless the eye was clicked, monospace so nothing is easy to misread.
    fn secret_field(&mut self, ui: &mut egui::Ui, key: &'static str, hint: &str, width: f32) {
        let Some(r) = &self.repo else { return };
        let saved = r.env.get(key);
        let secret = envfile::var(key).is_none_or(|v| v.secret);
        let shown = self.revealed.contains(key);
        let mut toggle = false;
        ui.horizontal(|ui| {
            let value = self.env_edit.entry(key).or_insert_with(|| saved.clone());
            ui.add(
                egui::TextEdit::singleline(value)
                    .password(secret && !shown)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(width)
                    .hint_text(hint),
            );
            if secret {
                let (icon, tip) = if shown { (icons::EYE_OFF, "hide it again") } else { (icons::EYE, "show what is typed") };
                if ui::tool(ui, icon, tip).clicked() {
                    toggle = true;
                }
            }
        });
        if toggle {
            if shown {
                self.revealed.remove(key);
            } else {
                self.revealed.insert(key);
            }
        }
    }

    /// The value as it would be saved right now: what is typed in the field, else what is in `.env`.
    fn edited(&self, key: &'static str) -> String {
        let saved = self.repo.as_ref().map(|r| r.env.get(key)).unwrap_or_default();
        self.env_edit.get(key).cloned().unwrap_or(saved)
    }

    fn save_button(&mut self, ui: &mut egui::Ui) {
        let Some(r) = &self.repo else { return };
        let changed: Vec<(&'static str, String)> =
            self.env_edit.iter().filter(|(k, v)| r.env.get(k) != v.trim()).map(|(k, v)| (*k, v.clone())).collect();
        let env_file = r.paths.env_file();
        let count = changed.len();
        ui.horizontal(|ui| {
            let text = if count > 1 { format!("Save {count} changes") } else { "Save".to_owned() };
            if ui::button_enabled(ui, count > 0, icons::CHECK_CIRCLE, &text).clicked() {
                self.save_env(&changed);
                self.env_edit.clear();
            }
            if count > 0 {
                ui.label(RichText::new("not saved yet").small().color(amber()));
            }
            if ui::tool(ui, icons::FILE_DOCUMENT_EDIT, "open .env in the editor").clicked() {
                sys::open(&env_file);
            }
            ui::icon(ui, icons::SHIELD_LOCK, green()).on_hover_text(".env is git-ignored");
        });
    }

    fn llm_settings(&mut self, ui: &mut egui::Ui) {
        Self::section(ui, icons::ROBOT, "Vision AI (captions, clip ratings, motion prompts)");
        let llm = self.llm();
        let mut provider = llm.provider;
        ui.horizontal_wrapped(|ui| {
            ui.radio_value(&mut provider, Provider::OpenRouter, "OpenRouter - free cloud models, no GPU memory used");
            ui.radio_value(&mut provider, Provider::OpenCode, "OpenCode Zen - free cloud models, no key");
            ui.radio_value(&mut provider, Provider::Ollama, "Ollama - local and offline");
            ui.radio_value(&mut provider, Provider::LlamaCpp, "llama.cpp - the GGUF router from the Coding page");
        });
        if provider != llm.provider {
            self.save_env(&[("LLM_PROVIDER", provider.env_value().into())]);
        }
        if provider == Provider::OpenRouter {
            let connected = self.repo.as_ref().is_some_and(|r| r.env.is_set("OPENROUTER_API_KEY"));
            if connected {
                ui::status(ui, icons::CLOUD_CHECK, green(), "key saved - models below, the key itself is in the OpenRouter card");
            } else {
                ui::status(ui, icons::ALERT_CIRCLE, amber(), "no OpenRouter key yet - paste it in the OpenRouter card below");
            }
            ui.add_space(4.0);
        }
        egui::Grid::new("llm").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            if provider == Provider::OpenRouter {
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
                    "opencode/deepseek-v4.1-flash-free",
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
            if provider == Provider::LlamaCpp {
                self.text_field(
                    ui,
                    "LLAMA_CPP_MODEL",
                    "Model",
                    "nvidia/NVIDIA-Nemotron-3.5-Bolt-E4B-4GB:Q4_K_M",
                    "the GGUF the router loads - Start router on the Coding page loads it and frees it again after a run",
                );
                self.text_field(
                    ui,
                    "LLAMA_CPP_URL",
                    "Router URL",
                    "http://127.0.0.1:8080",
                    "llama serve endpoint; the Coding page starts it hidden in the background",
                );
                self.text_field(
                    ui,
                    "LLM_LOCAL_FALLBACK",
                    "Then local Ollama",
                    "1",
                    "1 = use the local Ollama model when the router is not running or the model fails, 0 = give up",
                );
            }
            self.text_field(ui, "OLLAMA_MODEL", "Ollama model", "qwen3-vl:4b", "local vision model (downloaded under Models)");
        });
        self.save_button(ui);
    }

    /// The key that connects the free cloud models: paste, check, done.
    fn openrouter_settings(&mut self, ui: &mut egui::Ui) {
        let Some(r) = &self.repo else { return };
        let saved = r.env.get("OPENROUTER_API_KEY").trim().to_owned();
        let connected = !saved.is_empty();
        ui::card_title_with(ui, icons::CLOUD_OUTLINE, "OpenRouter", |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                Self::connect_pill(ui, connected, "connected", "no key yet");
            });
        });
        ui.label(
            RichText::new(
                "Free cloud vision models for captions, clip ratings and motion prompts - no GPU memory used. A key takes a minute: log in on openrouter.ai, open Keys, create one and paste it here. The key is only sent to openrouter.ai.",
            )
            .small()
            .color(theme::muted()),
        );
        ui.add_space(8.0);
        ui.label("API key").on_hover_text("OPENROUTER_API_KEY in .env");
        self.secret_field(ui, "OPENROUTER_API_KEY", "sk-or-v1-...", 380.0);
        ui.add_space(2.0);
        if connected {
            ui.label(RichText::new(format!("saved: {}", mask_key(&saved))).small().color(theme::muted())).on_hover_text("the full key stays in .env");
        } else {
            ui.label(RichText::new("nothing saved yet").small().color(theme::muted()));
        }
        ui.add_space(6.0);
        self.save_button(ui);
        ui.add_space(2.0);
        self.key_check_ui(ui);
        ui.add_space(10.0);
        self.openrouter_model_row(ui);
    }

    /// "Check key" asks OpenRouter itself, so a typo or an expired key shows up before a flow uses it.
    fn key_check_ui(&mut self, ui: &mut egui::Ui) {
        let Some(r) = &self.repo else { return };
        let saved = r.env.get("OPENROUTER_API_KEY").trim().to_owned();
        let state = self.key_check.lock().unwrap().clone();
        ui.horizontal(|ui| {
            let can = !saved.is_empty() && !matches!(state, KeyCheck::Checking);
            if ui::button_enabled(ui, can, icons::CLOUD_CHECK, "Check key")
                .on_hover_text("asks OpenRouter whether the saved key works - save a new key first")
                .clicked()
            {
                *self.key_check.lock().unwrap() = KeyCheck::Checking;
                check_openrouter_key(saved.clone(), self.key_check.clone(), ui.ctx().clone());
            }
            if ui::button(ui, icons::OPEN_IN_NEW, "Get a free key").clicked() {
                sys::open_url("https://openrouter.ai/keys");
            }
            match &state {
                KeyCheck::Idle => {
                    ui.label(RichText::new("not checked yet").small().color(theme::muted()));
                }
                KeyCheck::Checking => {
                    ui.spinner();
                    ui.label(RichText::new("asking openrouter.ai ...").small().color(theme::muted()));
                }
                KeyCheck::Ok(text) => {
                    ui::status(ui, icons::CHECK_CIRCLE, green(), text.as_str());
                }
                KeyCheck::Failed(text) => {
                    ui::status(ui, icons::ALERT_CIRCLE, red(), text.as_str());
                }
            }
        });
    }

    fn openrouter_model_row(&mut self, ui: &mut egui::Ui) {
        let Some(r) = &self.repo else { return };
        let current = or_default(&r.env.get("OPENROUTER_MODEL"), "qwen/qwen3.8-27b:free");
        ui.label("Model").on_hover_text("OPENROUTER_MODEL in .env");
        ui.horizontal(|ui| {
            let value = self.env_edit.entry("OPENROUTER_MODEL").or_insert(current);
            ui.add(egui::TextEdit::singleline(value).desired_width(300.0).hint_text("qwen/qwen3.8-27b:free"));
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
                    ui::status(ui, icons::ALERT_CIRCLE, red(), "list failed").on_hover_text(e.as_str());
                }
                FreeModels::NotLoaded => {
                    if ui::button(ui, icons::CLOUD_OUTLINE, "List free vision models").clicked() {
                        *state = FreeModels::Loading;
                        let (state, ctx) = (self.free_models.clone(), ui.ctx().clone());
                        fetch_free_models(state, ctx);
                    }
                }
            }
        });
        ui.label(
            RichText::new("only models that take images can caption or rate - the :free ones need no credit")
                .small()
                .color(theme::muted()),
        );
    }

    /// Telegram is a two-value login: everything else (phone, code, 2FA) happens in the flow's login window.
    fn telegram_settings(&mut self, ui: &mut egui::Ui) {
        let Some(r) = &self.repo else { return };
        let complete = telegram_problem(&self.edited("TELEGRAM_API_ID"), &self.edited("TELEGRAM_API_HASH")).is_none();
        let twofa = !r.env.get("TELEGRAM_PASSWORD").trim().is_empty();
        ui::card_title_with(ui, icons::SEND, "Telegram", |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                Self::connect_pill(ui, complete, "ready", "not set up");
            });
        });
        ui.label(
            RichText::new("Top 5 videos follows the channels you are in and rates today's clips. It needs your own API ID and hash - both are free and stay in .env.")
                .small()
                .color(theme::muted()),
        );
        ui.add_space(8.0);
        ui::step_with(ui, 1, "Open my.telegram.org and log in with your phone number", |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui::button(ui, icons::OPEN_IN_NEW, "my.telegram.org").clicked() {
                    sys::open_url("https://my.telegram.org");
                }
            });
        });
        ui::step_with(ui, 2, "API development tools > create an app (any name and platform)", |_| {});
        ui::step_with(ui, 3, "Copy App api_id and App api_hash below - phone and code are asked on the first run", |_| {});
        ui.add_space(6.0);
        egui::Grid::new("telegram").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("api_id").on_hover_text("the number from my.telegram.org");
            self.secret_field(ui, "TELEGRAM_API_ID", "1234567", 200.0);
            ui.end_row();
            ui.label("api_hash").on_hover_text("the 32-character string next to the api_id");
            self.secret_field(ui, "TELEGRAM_API_HASH", "0123456789abcdef0123456789abcdef", 380.0);
            ui.end_row();
            ui.label("2FA password").on_hover_text("TELEGRAM_PASSWORD - only needed with two-step verification");
            self.secret_field(ui, "TELEGRAM_PASSWORD", "optional", 200.0);
            ui.end_row();
        });
        match telegram_problem(&self.edited("TELEGRAM_API_ID"), &self.edited("TELEGRAM_API_HASH")) {
            None => {
                ui::status(ui, icons::CHECK_CIRCLE, green(), "complete - Top 5 videos logs in when it first reads a channel");
            }
            Some(problem) => {
                ui::status(ui, icons::INFORMATION_OUTLINE, amber(), problem);
            }
        }
        ui.add_space(4.0);
        self.save_button(ui);
        if complete && !twofa {
            ui.label(
                RichText::new("no 2FA password saved - if your account uses one, the login window asks for it once")
                    .small()
                    .color(theme::muted()),
            );
        }
    }

    fn huggingface_settings(&mut self, ui: &mut egui::Ui) {
        ui::card_title(ui, icons::KEY, "Hugging Face", Some("only for gated models"));
        ui.label(
            RichText::new("Some checkpoints (FLUX.2 klein) are gated: accept their license on the model page, then paste a read token here so the download can run.")
                .small()
                .color(theme::muted()),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("Token");
            ui.hyperlink_to(RichText::new("huggingface.co/settings/tokens").small(), "https://huggingface.co/settings/tokens");
        });
        self.secret_field(ui, "HF_TOKEN", "hf_...", 380.0);
        ui.add_space(6.0);
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
            ui.label(format!("local-llm GUI v{}", setup::VERSION));
            if ui::button(ui, icons::GITHUB, "GitHub").on_hover_text(setup::REPO_WEB).clicked() {
                sys::open_url(setup::REPO_WEB);
            }
            if ui::button(ui, icons::OPEN_IN_NEW, "Releases").on_hover_text("newer local-llm.exe builds").clicked() {
                sys::open_url(setup::RELEASES_URL);
            }
        });
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
                ui::status(ui, icons::CHECK_CIRCLE, green(), "ComfyUI + Python installed");
            } else {
                ui::status(ui, icons::ALERT, amber(), "ComfyUI + Python not installed yet");
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
                self.launch_once("git-pull", &mut cmd, "git pull");
            }
            self.desktop_shortcut_button(ui, &root);
        });
        match self.health.latest().and_then(|s| s.gpu) {
            Some(g) => {
                ui.horizontal(|ui| {
                    let text =
                        format!("{} · driver {}{}", g.name, g.driver, g.cuda.map(|(a, b)| format!(" · CUDA {a}.{b}")).unwrap_or_default());
                    match health::driver_ok(&g.driver) {
                        Some(true) => ui::status(ui, icons::CHECK_CIRCLE, green(), text),
                        _ => ui::status(ui, icons::ALERT, amber(), format!("{text} - ComfyUI needs driver {}+", health::MIN_DRIVER)),
                    };
                    if ui::button(ui, icons::OPEN_IN_NEW, "NVIDIA drivers").clicked() {
                        sys::open_url("https://www.nvidia.com/en-us/drivers/");
                    }
                });
            }
            None => {
                ui::status(ui, icons::ALERT, amber(), "no NVIDIA GPU / driver found");
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
        self.launch_once("setup", &mut cmd, "setup.bat");
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

/// `sk-or-v1-…3f9a`: enough to tell two keys apart, never the whole secret.
fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.trim().chars().collect();
    if chars.len() <= 12 {
        return "…".to_owned();
    }
    let head: String = chars[..6].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

/// What is still wrong with the two Telegram values, or None when they look usable.
fn telegram_problem(api_id: &str, api_hash: &str) -> Option<String> {
    let id = api_id.trim();
    let hash = api_hash.trim();
    if id.is_empty() && hash.is_empty() {
        return Some("nothing entered yet - see the three steps above".to_owned());
    }
    if id.is_empty() {
        return Some("api_id is missing".to_owned());
    }
    if !id.chars().all(|c| c.is_ascii_digit()) || !(5..=12).contains(&id.len()) {
        return Some("api_id is the number from my.telegram.org (5-12 digits, no spaces)".to_owned());
    }
    if hash.is_empty() {
        return Some("api_hash is missing".to_owned());
    }
    if hash.len() != 32 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some("api_hash is the 32-character string next to the api_id".to_owned());
    }
    None
}

/// Asks OpenRouter whether the key works and adds what the answer says about the account.
fn check_openrouter_key(key: String, state: Arc<std::sync::Mutex<KeyCheck>>, ctx: egui::Context) {
    std::thread::spawn(move || {
        let result = (|| -> Result<String, String> {
            let response = ureq::get("https://openrouter.ai/api/v1/key").header("Authorization", &format!("Bearer {key}")).call();
            let mut resp = match response {
                Ok(resp) => resp,
                Err(ureq::Error::StatusCode(401)) => return Err("the key was rejected (401) - create a new one on openrouter.ai".to_owned()),
                Err(e) => return Err(e.to_string()),
            };
            let v: serde_json::Value = resp.body_mut().with_config().limit(1 << 20).read_json().map_err(|e| e.to_string())?;
            let data = &v["data"];
            let label = data["label"].as_str().unwrap_or("").trim().to_owned();
            let tier = if data["is_free_tier"].as_bool().unwrap_or(false) { "free tier" } else { "paid tier" };
            let mut text = if label.is_empty() { format!("key works · {tier}") } else { format!("key works · {tier} · {label}") };
            if let (Some(used), Some(limit)) = (data["usage"].as_f64(), data["limit"].as_f64())
                && limit > 0.0
            {
                text.push_str(&format!(" · ${used:.2} of ${limit:.2} used"));
            }
            Ok(text)
        })();
        *state.lock().unwrap() = match result {
            Ok(text) => KeyCheck::Ok(text),
            Err(e) => KeyCheck::Failed(e),
        };
        ctx.request_repaint();
    });
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
