use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use super::{App, or_default};
use crate::coding::{self, Editor, Profile};
use crate::config::Page;
use crate::flows::Provider;
use crate::icons;
use crate::llamacpp::{self, Router};
use crate::models::{Dl, Task, ollama_id};
use crate::sys;
use crate::ui::{self, AMBER, GREEN, RED};

/// What the Coding page knows about Ollama, filled in by background threads.
#[derive(Default)]
pub struct CodingState {
    /// Models Ollama has (None until listed).
    pub installed: Option<Vec<String>>,
    /// The profile being set up right now.
    pub busy: Option<Profile>,
    pub last: Option<Result<String, String>>,
    /// The llama.cpp router (the GGUF model Pi/T3 Code can use next to Ollama).
    pub llama: LlamaState,
}

#[derive(Default)]
pub struct LlamaState {
    /// Last answer from the router; None while the first request is out.
    pub status: Option<Result<Router, String>>,
    pub polling: bool,
    pub polled: Option<Instant>,
    /// The action running right now ("starting router", ...).
    pub busy: Option<&'static str>,
    pub last: Option<Result<String, String>>,
}

pub type Shared = Arc<Mutex<CodingState>>;

impl App {
    pub(super) fn profile(&self) -> Profile {
        self.repo.as_ref().map_or(Profile::Speed, |r| Profile::from_env(&r.env.get("LLM_LOCAL_PROFILE")))
    }

    pub(super) fn set_profile(&mut self, p: Profile) {
        if p != self.profile() {
            self.save_env(&[("LLM_LOCAL_PROFILE", p.env_value().into())]);
        }
    }

    /// Which LLM the flows ask for text and images (`LLM_PROVIDER` in .env).
    pub(super) fn flow_provider(&self) -> Provider {
        self.repo
            .as_ref()
            .map_or(Provider::Ollama, |r| Provider::from_env(&r.env.get("LLM_PROVIDER"), r.env.is_set("OPENROUTER_API_KEY")))
    }

    fn set_flow_provider(&mut self, p: Provider) {
        if p != self.flow_provider() {
            self.save_env(&[("LLM_PROVIDER", p.env_value().into())]);
        }
    }

    fn flow_provider_toggle(&mut self, ui: &mut egui::Ui) {
        let current = self.flow_provider();
        let mut pick = current;
        egui::ComboBox::from_id_salt("flow-provider").selected_text(current.label()).width(200.0).show_ui(ui, |ui| {
            for p in [Provider::Ollama, Provider::OpenCode, Provider::LlamaCpp, Provider::OpenRouter] {
                ui.selectable_value(&mut pick, p, p.label());
            }
        });
        if pick != current {
            self.set_flow_provider(pick);
        }
    }

    /// What the next flow run will use, with the model it will ask.
    fn flow_summary(&self) -> String {
        let env = |key: &str, default: &str| self.repo.as_ref().map_or_else(|| default.to_owned(), |r| or_default(&r.env.get(key), default));
        match self.flow_provider() {
            Provider::Ollama => format!("{} via Ollama - the tuned model above", self.base(self.profile())),
            Provider::OpenCode => format!("{} via OpenCode Zen - free, cloud, no GPU memory", env("OPENCODE_MODEL", "opencode/deepseek-v4.1-flash-free")),
            Provider::LlamaCpp => {
                let (_, model, _) = self.llama_config();
                format!("{} via the llama.cpp router - start it above, it is unloaded after each run", llamacpp::display_name(&model))
            }
            Provider::OpenRouter => format!("{} via OpenRouter", env("OPENROUTER_MODEL", "qwen/qwen3.8-27b:free")),
        }
    }

    /// Speed / Quality segmented toggle (top bar and Coding page).
    pub(super) fn profile_toggle(&mut self, ui: &mut egui::Ui) {
        let current = self.profile();
        let mut pick = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for p in Profile::ALL {
                let icon = if p == Profile::Speed { icons::ROCKET_LAUNCH } else { icons::DIAMOND_STONE };
                let on = p == current;
                let color = if on { egui::Color32::WHITE } else { ui.visuals().text_color() };
                let b = egui::Button::image_and_text(icon.image(15.0, color), RichText::new(p.label()).color(color))
                    .fill(if on { ui::ACCENT } else { ui.visuals().widgets.inactive.weak_bg_fill })
                    .corner_radius(if p == Profile::Speed { egui::CornerRadius { nw: 6, sw: 6, ne: 0, se: 0 } } else { egui::CornerRadius { nw: 0, sw: 0, ne: 6, se: 6 } });
                let tip = format!("local LLM tuned for {}: {}", p.label().to_lowercase(), p.summary());
                if ui.add(b).on_hover_text(tip).clicked() {
                    pick = Some(p);
                }
            }
        });
        if let Some(p) = pick {
            self.set_profile(p);
        }
    }

    pub(super) fn coding_page(&mut self, ui: &mut egui::Ui) {
        if self.coding.lock().unwrap().installed.is_none() {
            self.list_ollama_models();
        }
        self.poll_llama_router();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.heading("Coding");
            ui.label(
                RichText::new(
                    "A local model for T3 Code, OpenCode and Pi, built from the models you already have - nothing is downloaded. \
                     The flows use the writer you pick below.",
                )
                .weak(),
            );
            ui.add_space(10.0);
            ui::card(ui, |ui| self.profile_card(ui));
            ui.add_space(8.0);
            ui::card(ui, |ui| self.llama_cpp_card(ui));
            ui.add_space(8.0);
            ui::card(ui, |ui| self.models_card(ui));
            ui.add_space(8.0);
            ui::card(ui, |ui| self.editors_card(ui));
            ui.add_space(8.0);
            ui::card(ui, |ui| self.console_card(ui));
        });
    }

    /// Endpoint, model and executable of the llama.cpp router (from .env, with defaults).
    pub(super) fn llama_config(&self) -> (String, String, String) {
        match &self.repo {
            Some(r) => (
                llamacpp::base_url(&r.env.get(llamacpp::URL_KEY)),
                llamacpp::model_id(&r.env.get(llamacpp::MODEL_KEY)),
                r.env.get(llamacpp::EXE_KEY),
            ),
            None => (llamacpp::DEFAULT_URL.into(), llamacpp::DEFAULT_MODEL.into(), String::new()),
        }
    }

    /// Writes the text typed into the model/URL fields to .env, so the next action uses it.
    fn flush_llama_env(&mut self) {
        let values: Vec<(&'static str, String)> = {
            let Some(r) = &self.repo else { return };
            [llamacpp::MODEL_KEY, llamacpp::URL_KEY]
                .into_iter()
                .filter_map(|key| {
                    let text = self.env_edit.get(key)?.trim();
                    (!text.is_empty() && text != r.env.get(key).trim()).then(|| (key, text.to_owned()))
                })
                .collect()
        };
        if values.is_empty() {
            return;
        }
        self.save_env(&values);
        {
            let mut s = self.coding.lock().unwrap();
            s.llama.polled = None;
            s.llama.status = None;
        }
        self.poll_llama_router();
    }

    fn llama_cpp_card(&mut self, ui: &mut egui::Ui) {
        let (url, model, _) = self.llama_config();
        let (status, busy, last) = {
            let s = self.coding.lock().unwrap();
            (s.llama.status.clone(), s.llama.busy, s.llama.last.clone())
        };
        let running = matches!(&status, Some(Ok(r)) if r.running);
        let loaded = matches!(&status, Some(Ok(r)) if r.loaded(&model));
        let loading = matches!(&status, Some(Ok(r)) if r.loading(&model));
        let failed = match &status {
            Some(Ok(r)) => r.failed(&model),
            _ => None,
        };

        ui.horizontal(|ui| {
            ui.label(RichText::new("llama.cpp router").strong());
            match &status {
                None => {
                    ui.spinner();
                    ui.label(RichText::new(format!("asking {url} ...")).weak());
                }
                Some(Err(e)) => {
                    ui::status(ui, icons::ALERT_CIRCLE, RED, e.as_str());
                }
                Some(Ok(_)) if loaded => {
                    ui::status(ui, icons::CHECK_CIRCLE, GREEN, format!("{} loaded", llamacpp::display_name(&model)));
                }
                Some(Ok(_)) if loading => {
                    ui::status(ui, icons::TIMER_SAND, AMBER, "loading the model ...");
                }
                Some(Ok(_)) if failed.is_some() => {
                    let code = failed.unwrap_or_default();
                    ui::status(ui, icons::ALERT_CIRCLE, RED, format!("model failed to load (exit code {code})"));
                }
                Some(Ok(r)) if r.running => {
                    ui::status(ui, icons::CIRCLE_OUTLINE, AMBER, "router running, model not loaded");
                }
                Some(Ok(_)) => {
                    ui::status(ui, icons::CIRCLE_OUTLINE, Color32::GRAY, "router not running");
                }
            }
            if let Some(what) = busy {
                ui.spinner();
                ui.label(RichText::new(what).weak());
            }
        });
        ui.label(
            RichText::new(
                "Runs `llama serve` hidden in the background and loads this GGUF - no console window, no Ollama. \
                 Pi gets a `llama-cpp` provider, so the model shows up in Pi and T3 Code as llama-cpp/<model>.",
            )
            .small()
            .weak(),
        );
        ui.add_space(6.0);

        egui::Grid::new("llama-cpp-config").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            let fields = [
                ("Model", llamacpp::MODEL_KEY, model.clone(), llamacpp::DEFAULT_MODEL, 560.0f32),
                ("Router URL", llamacpp::URL_KEY, url.clone(), llamacpp::DEFAULT_URL, 240.0f32),
            ];
            for (label, key, current, hint, width) in fields {
                ui.label(label);
                let mut text = self.env_edit.get(key).cloned().unwrap_or(current);
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .desired_width(width.min(ui.available_width()))
                        .hint_text(hint),
                );
                if resp.changed() {
                    self.env_edit.insert(key, text);
                }
                if resp.lost_focus() {
                    self.flush_llama_env();
                }
                ui.end_row();
            }
        });

        ui.add_space(6.0);
        let idle = busy.is_none();
        let mut clicked = None;
        ui.horizontal_wrapped(|ui| {
            if !running
                && ui::button_enabled(ui, idle, icons::POWER, "Start router")
                    .on_hover_text("starts llama.cpp hidden, loads the model and sets up Pi/OpenCode/T3 Code")
                    .clicked()
            {
                clicked = Some(0);
            }
            if running && !loaded && !loading
                && ui::button_enabled(ui, idle, icons::PLAY, "Load model").on_hover_text("loads the GGUF into memory").clicked()
            {
                clicked = Some(1);
            }
            if loaded
                && ui::button_enabled(ui, idle, icons::DELETE_OUTLINE, "Unload model")
                    .on_hover_text("frees the VRAM again, so other programs can use it")
                    .clicked()
            {
                clicked = Some(2);
            }
            if running
                && ui::button_enabled(ui, idle, icons::STOP, "Stop router").on_hover_text("stops llama serve").clicked()
            {
                clicked = Some(3);
            }
            if ui::button_enabled(ui, idle, icons::WRENCH, "Set up Pi + OpenCode")
                .on_hover_text("adds the llama-cpp provider and this model to Pi and OpenCode, and enables Pi in T3 Code")
                .clicked()
            {
                clicked = Some(4);
            }
            if busy.is_none() && ui::tool(ui, icons::REFRESH, "check the router again").clicked() {
                self.coding.lock().unwrap().llama.polled = None;
                self.poll_llama_router();
            }
        });
        if let Some(n) = clicked {
            self.flush_llama_env();
            match n {
                0 => self.llama_start(),
                1 => self.llama_load(),
                2 => self.llama_unload(),
                3 => self.llama_stop(),
                _ => self.llama_configure(),
            }
        }
        if let Some(result) = &last {
            match result {
                Ok(m) => {
                    ui::status(ui, icons::CHECK_CIRCLE, GREEN, m.as_str());
                }
                Err(e) => {
                    ui::status(ui, icons::ALERT_CIRCLE, RED, e.as_str());
                }
            }
        }
    }

    /// Runs one router action in the background and refreshes the status afterwards.
    fn llama_job(
        &mut self,
        what: &'static str,
        job: impl FnOnce(String, String, String) -> Result<String, String> + Send + 'static,
    ) {
        let (url, model, exe) = self.llama_config();
        {
            let mut s = self.coding.lock().unwrap();
            if s.llama.busy.is_some() {
                return;
            }
            s.llama.busy = Some(what);
            s.llama.last = None;
        }
        let state = self.coding.clone();
        let console = self.console.clone();
        let ctx = self.ctx.clone();
        crate::console::line(&console, format!("[gui  ] {what} ..."), false);
        std::thread::spawn(move || {
            let result = job(url.clone(), model, exe);
            let status = llamacpp::status(&url);
            let text = match &result {
                Ok(m) => m.clone(),
                Err(e) => e.clone(),
            };
            crate::console::line(&console, format!("[gui  ] {what}: {text}"), result.is_err());
            let mut s = state.lock().unwrap();
            s.llama.busy = None;
            s.llama.polled = Some(Instant::now());
            s.llama.status = Some(status);
            s.llama.last = Some(result);
            drop(s);
            ctx.request_repaint();
        });
    }

    fn llama_start(&mut self) {
        let console = self.console.clone();
        self.llama_job("starting router", move |url, model, exe| {
            if !llamacpp::status(&url)?.running {
                llamacpp::start(&url, &exe, &console)?;
                llamacpp::wait_until_up(&url, Duration::from_secs(120))?;
            }
            llamacpp::ensure_loaded(&url, &model, Duration::from_secs(300))?;
            let config = coding::llama_cpp_apply(&model, &url)?;
            Ok(format!("{} loaded on {url} - {config}", llamacpp::display_name(&model)))
        });
    }

    fn llama_load(&mut self) {
        self.llama_job("loading model", |url, model, _| {
            llamacpp::ensure_loaded(&url, &model, Duration::from_secs(300))?;
            let config = coding::llama_cpp_apply(&model, &url)?;
            Ok(format!("{} loaded - {config}", llamacpp::display_name(&model)))
        });
    }

    pub(super) fn llama_unload(&mut self) {
        self.llama_job("unloading model", |url, _, _| llamacpp::unload_all(&url));
    }

    pub(super) fn llama_stop(&mut self) {
        self.llama_job("stopping router", |url, _, _| llamacpp::stop(&url));
    }

    fn llama_configure(&mut self) {
        self.llama_job("setting up Pi/OpenCode", |url, model, _| coding::llama_cpp_apply(&model, &url));
    }

    /// Asks the router for its models every couple of seconds (the Coding page and the tray tip use it).
    pub(super) fn poll_llama_router(&self) {
        let (url, _, _) = self.llama_config();
        {
            let mut s = self.coding.lock().unwrap();
            let l = &mut s.llama;
            if l.polling || l.polled.is_some_and(|at| at.elapsed() < Duration::from_secs(2)) {
                return;
            }
            l.polling = true;
            l.polled = Some(Instant::now());
        }
        let state = self.coding.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let status = llamacpp::status(&url);
            let mut s = state.lock().unwrap();
            s.llama.polling = false;
            s.llama.status = Some(status);
            drop(s);
            ctx.request_repaint();
        });
    }

    /// Tooltip of the notification-area icon.
    pub(super) fn llama_tip(&self) -> String {
        let (url, model, _) = self.llama_config();
        let status = self.coding.lock().unwrap().llama.status.clone();
        match status {
            Some(Ok(r)) if r.loaded(&model) => format!("llama.cpp: {} ready", llamacpp::display_name(&model)),
            Some(Ok(r)) if r.loading(&model) => format!("llama.cpp: loading {} ...", llamacpp::display_name(&model)),
            Some(Ok(r)) if r.running => "llama.cpp: router running, no model loaded".into(),
            Some(Ok(_)) => format!("llama.cpp: not running ({url})"),
            Some(Err(_)) => "llama.cpp: no answer - open local-llm".into(),
            None => "llama.cpp: checking ...".into(),
        }
    }

    fn profile_card(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Tuned for").strong());
        self.profile_toggle(ui);
        ui.add_space(4.0);
        let p = self.profile();
        ui.label(p.summary());
        ui.label(
            RichText::new(format!(
                "Model {}, context {}k, thinking {}.",
                p.variant(),
                p.num_ctx() / 1024,
                if p.thinking() { "on" } else { "off" },
            ))
            .small()
            .weak(),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Flows use").strong());
            self.flow_provider_toggle(ui);
        });
        ui.label(RichText::new(self.flow_summary()).small().weak());
    }

    fn base(&self, p: Profile) -> String {
        self.repo.as_ref().map_or_else(|| p.default_base().to_owned(), |r| or_default(&r.env.get(p.base_key()), p.default_base()))
    }

    fn models_card(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Models").strong());
            if ui::tool(ui, icons::REFRESH, "list Ollama's models again").clicked() {
                self.list_ollama_models();
            }
        });
        let (installed, busy, last) = {
            let s = self.coding.lock().unwrap();
            (s.installed.clone(), s.busy, s.last.clone())
        };
        let Some(installed) = installed else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("asking Ollama ...");
            });
            return;
        };
        let bases: Vec<String> = installed.iter().filter(|m| !m.starts_with("local-coder:")).cloned().collect();
        let mut setup = None;
        egui::Grid::new("coding-models").num_columns(4).spacing([12.0, 8.0]).show(ui, |ui| {
            for p in Profile::ALL {
                ui.label(RichText::new(p.label()).strong());
                let mut base = self.base(p);
                let before = base.clone();
                egui::ComboBox::from_id_salt(("base", p.env_value())).selected_text(&base).width(260.0).show_ui(ui, |ui| {
                    for m in &bases {
                        ui.selectable_value(&mut base, m.clone(), m);
                    }
                });
                if base != before {
                    self.save_env(&[(p.base_key(), base.clone())]);
                }
                let have_base = installed.contains(&base);
                let built = installed.iter().any(|m| m == p.variant());
                if !have_base {
                    ui::status(ui, icons::ALERT, AMBER, "not downloaded");
                } else if built {
                    ui::status(ui, icons::CHECK_CIRCLE, GREEN, "set up");
                } else {
                    ui::status(ui, icons::CIRCLE_OUTLINE, AMBER, "not set up yet");
                }
                if busy == Some(p) {
                    ui.spinner();
                } else {
                    let label = if built { "Set up again" } else { "Set up" };
                    let tip = format!("builds {} from {base} and adds it to OpenCode and Pi", p.variant());
                    if ui::button_enabled(ui, have_base && busy.is_none(), icons::WRENCH, label).on_hover_text(tip).clicked() {
                        setup = Some((p, base));
                    }
                }
                ui.end_row();
            }
        });
        if bases.is_empty() {
            ui.label(RichText::new("Ollama has no models yet.").weak());
        }
        if let Some(models) = self.repo.as_ref().map(|r| r.models.clone()) {
            let suggested: Vec<_> = models.ollama.iter().filter(|m| m.suggested).cloned().collect();
            if !suggested.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new("Good free coding models (6-8 GB, not downloaded yet)").strong().small());
                for m in suggested {
                    let id = ollama_id(&m.model);
                    ui.horizontal(|ui| {
                        ui.label(&m.model);
                        if let Some(size) = m.size {
                            ui.label(RichText::new(sys::human(size)).weak().small());
                        }
                        if models.status(&id).usable() {
                            ui::status(ui, icons::CHECK_CIRCLE, GREEN, "downloaded");
                        } else if let Some(dl) = models.download(&id) {
                            match dl {
                                Dl::Failed(e) => {
                                    ui::status(ui, icons::ALERT_CIRCLE, RED, &e);
                                }
                                _ => {
                                    ui.spinner();
                                    ui.label(RichText::new("downloading ...").weak());
                                }
                            }
                        } else if ui::button(ui, icons::DOWNLOAD, "Download")
                            .on_hover_text("pulls it from ollama.com into the models folder; press refresh above when it is done")
                            .clicked()
                        {
                            models.enqueue(&id, Task::Download);
                        }
                    });
                }
            }
        }
        if ui.link("Download more coding models on the Models page").clicked() {
            self.select(Page::Models);
        }
        match &last {
            Some(Ok(m)) => {
                ui::status(ui, icons::CHECK_CIRCLE, GREEN, m.as_str());
            }
            Some(Err(e)) => {
                ui::status(ui, icons::ALERT_CIRCLE, RED, e.as_str());
            }
            None => {}
        }
        if let Some((p, base)) = setup {
            self.setup_profile(p, base);
        }
    }

    fn editors_card(&mut self, ui: &mut egui::Ui) {
        let p = self.profile();
        let root = self.repo.as_ref().map(|r| r.paths.root.clone());
        let ready = self.coding.lock().unwrap().installed.as_ref().is_some_and(|i| i.iter().any(|m| m == p.variant()));
        ui.label(RichText::new("Open").strong());
        ui.horizontal(|ui| {
            for e in Editor::ALL {
                let installed = e.exe().is_some();
                let tip = if installed { format!("loads {} into memory and opens {}", p.variant(), e.label()) } else { "not installed - run setup.bat".into() };
                if ui::button_enabled(ui, installed && ready, icons::OPEN_IN_NEW, e.label()).on_hover_text(tip).clicked()
                    && let Some(root) = &root
                {
                    coding::preload(p);
                    match e.command(p, root) {
                        Ok(mut cmd) => {
                            crate::console::command(&self.console, "gui", &cmd);
                            self.launch_once(e.label(), &mut cmd, e.label());
                            if e == Editor::T3Code {
                                self.notify(format!("T3 Code: pick provider OpenCode or Pi, then {}", p.variant()), false);
                            }
                        }
                        Err(err) => self.notify(err, true),
                    }
                }
            }
        });
        if !ready {
            ui.label(RichText::new(format!("Set up the {} model above first.", p.label().to_lowercase())).weak());
        }
    }

    fn console_card(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Console").strong());
            ui.label(RichText::new("llama.cpp, Ollama and the commands the GUI starts").small().weak());
            if ui::tool(ui, icons::DELETE_OUTLINE, "clear the console").clicked() {
                crate::console::clear(&self.console);
            }
        });
        let c = self.console.lock().unwrap();
        if c.lines.is_empty() {
            ui.label(RichText::new("Command lines and output appear here.").weak());
            return;
        }
        ui.allocate_ui(egui::vec2(ui.available_width(), 230.0), |ui| {
            super::flow_page::log_lines(ui, &c.lines);
        });
    }

    pub(super) fn list_ollama_models(&self) {
        let state = self.coding.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let names = ureq::get("http://127.0.0.1:11434/api/tags")
                .call()
                .ok()
                .and_then(|r| r.into_body().read_json::<serde_json::Value>().ok())
                .map(|v| v["models"].as_array().into_iter().flatten().filter_map(|m| m["name"].as_str().map(str::to_owned)).collect())
                .unwrap_or_default();
            state.lock().unwrap().installed = Some(names);
            ctx.request_repaint();
        });
    }

    fn setup_profile(&mut self, p: Profile, base: String) {
        let state = self.coding.clone();
        let console = self.console.clone();
        let ctx = self.ctx.clone();
        state.lock().unwrap().busy = Some(p);
        std::thread::spawn(move || {
            let result = {
                let mut log = |line: String| crate::console::line(&console, line, false);
                coding::apply(p, &base, &mut log)
            };
            match &result {
                Ok(m) => crate::console::line(&console, format!("[gui  ] {m}"), false),
                Err(e) => crate::console::line(&console, format!("[gui  ] {e}"), true),
            }
            {
                let mut s = state.lock().unwrap();
                // the variant shows up as "set up" without a manual refresh
                if result.is_ok()
                    && let Some(list) = &mut s.installed
                    && !list.iter().any(|m| m == p.variant())
                {
                    list.push(p.variant().to_owned());
                }
                s.busy = None;
                s.last = Some(result);
            }
            ctx.request_repaint();
        });
    }
}
