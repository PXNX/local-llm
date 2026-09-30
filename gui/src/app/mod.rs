mod flow_page;
mod health_panel;
mod home;
mod models_page;
mod queue_page;
mod settings_page;
mod setup;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use crate::config::{Config, Page};
use crate::envfile::EnvFile;
use crate::flows::{FlowId, Llm, Needs, Provider};
use crate::health::Health;
use crate::icons;
use crate::jobs::{self, JobHandle, Launch};
use crate::models::{Models, Status, ollama_id};
use crate::paths::{self, Paths};
use crate::servers::{self, Server, Servers};
use crate::ui::{self, AMBER, GREEN, RED};

/// Repo-bound state, created once the local-llm folder is known.
pub struct Repo {
    pub paths: Paths,
    pub env: EnvFile,
    pub models: Models,
}

pub struct App {
    ctx: egui::Context,
    cfg: Config,
    repo: Option<Repo>,
    servers: Servers,
    health: Health,
    /// Latest run of each flow (its log stays visible on the flow's page).
    jobs: HashMap<FlowId, JobHandle>,
    /// Only one flow runs at a time (GPU); the rest waits here and runs in order.
    running: Option<FlowId>,
    queue: VecDeque<(FlowId, Launch)>,
    /// Flow whose setup dialog is open.
    prompt: Option<FlowId>,
    prompted: Option<FlowId>,
    /// Flow whose "cancel this run?" dialog is open.
    confirm_cancel: Option<FlowId>,
    /// "Quit while a flow is running?" dialog is open.
    confirm_quit: bool,
    /// Set right before re-sending the close command, so that close isn't intercepted a second time.
    allow_close: bool,
    /// Unsaved text typed into env var fields (setup dialog and settings).
    env_edit: HashMap<&'static str, String>,
    free_models: Arc<Mutex<FreeModels>>,
    notice: Option<(String, bool, Instant)>,
    last_tick: Instant,
    saved_json: String,
    ollama_was_up: bool,
}

#[derive(Default)]
pub enum FreeModels {
    #[default]
    NotLoaded,
    Loading,
    Loaded(Vec<String>),
    Failed(String),
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        crate::fonts::install(&cc.egui_ctx);
        let cfg = Config::load();
        let saved_json = cfg.to_json();
        let mut app = Self {
            ctx: cc.egui_ctx.clone(),
            servers: Servers::spawn(cc.egui_ctx.clone()),
            health: Health::spawn(cc.egui_ctx.clone()),
            repo: None,
            jobs: HashMap::new(),
            running: None,
            queue: VecDeque::new(),
            prompt: None,
            prompted: None,
            confirm_cancel: None,
            confirm_quit: false,
            allow_close: false,
            env_edit: HashMap::new(),
            free_models: Arc::default(),
            notice: None,
            last_tick: Instant::now(),
            saved_json,
            ollama_was_up: false,
            cfg,
        };
        if let Some(root) = paths::find_repo(app.cfg.repo.as_deref()) {
            app.open_repo(root);
        }
        app
    }

    fn open_repo(&mut self, root: PathBuf) {
        let paths = Paths { root: root.clone() };
        let env = EnvFile::load(&paths.env_file());
        let models = Models::new(&self.ctx, &root);
        self.cfg.repo = Some(root);
        self.repo = Some(Repo { paths, env, models });
        self.configure_models();
        self.env_edit.clear();
    }

    fn ollama_dir(&self) -> PathBuf {
        self.cfg.ollama_dir.clone().unwrap_or_else(servers::ollama_default_dir)
    }

    /// New downloads go to the custom folder if set; all folders are searched so nothing is fetched twice.
    fn configure_models(&self) {
        let Some(r) = &self.repo else { return };
        servers::set_ollama_models(self.cfg.ollama_dir.clone());
        let default = r.paths.default_models();
        let mut roots = Vec::new();
        if let Some(custom) = self.cfg.models_dir.clone().filter(|c| *c != default) {
            roots.push(custom);
        }
        roots.push(default);
        r.models.configure(roots, self.ollama_dir(), r.env.get("HF_TOKEN"));
        r.models.track(&ollama_id(&self.llm().ollama_model));
        r.models.rescan();
    }

    pub fn llm(&self) -> Llm {
        match &self.repo {
            Some(r) => Llm {
                provider: Provider::from_env(&r.env.get("LLM_PROVIDER"), r.env.is_set("OPENROUTER_API_KEY")),
                ollama_model: Some(r.env.get("OLLAMA_MODEL")).filter(|m| !m.trim().is_empty()).unwrap_or_else(|| "qwen3-vl:4b".into()),
            },
            None => Llm { provider: Provider::OpenRouter, ollama_model: "qwen3-vl:4b".into() },
        }
    }

    pub fn needs(&self, id: FlowId) -> Needs {
        let mut n = self.cfg.forms.get(id).needs(&self.llm());
        n.servers.dedup();
        n
    }

    /// Env vars and models a flow still lacks.
    pub fn missing(&self, n: &Needs) -> (Vec<&'static str>, Vec<String>) {
        let Some(r) = &self.repo else { return (Vec::new(), Vec::new()) };
        let env = n.env.iter().copied().filter(|k| !r.env.is_set(k)).collect();
        let models = n.models.iter().filter(|m| !r.models.status(m).usable()).cloned().collect();
        (env, models)
    }

    pub fn ready(&self, id: FlowId) -> bool {
        let (env, models) = self.missing(&self.needs(id));
        env.is_empty() && models.is_empty()
    }

    pub fn notify(&mut self, text: impl Into<String>, error: bool) {
        self.notice = Some((text.into(), error, Instant::now()));
    }

    fn running_job(&self) -> Option<(FlowId, &JobHandle)> {
        let id = self.running?;
        self.jobs.get(&id).map(|h| (id, h))
    }

    /// Starts the next queued run when nothing is running. Called every frame; a finished job repaints.
    fn pump_queue(&mut self) {
        if self.running_job().is_some_and(|(_, h)| h.job.lock().unwrap().running()) {
            return;
        }
        self.running = None;
        if let Some((id, launch)) = self.queue.pop_front() {
            self.jobs.insert(id, jobs::start(&self.ctx, launch));
            self.running = Some(id);
        }
    }

    /// Cheap periodic work, run at most every 2 s and only while frames are drawn.
    fn tick(&mut self) {
        if self.last_tick.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_tick = Instant::now();
        let mut reconfigure = false;
        if let Some(r) = &mut self.repo {
            if r.env.reload_if_changed() {
                self.env_edit.clear();
                reconfigure = true;
            }
        } else if let Some(root) = paths::find_repo(self.cfg.repo.as_deref()) {
            self.open_repo(root);
        }
        let up = self.servers.up(Server::Ollama);
        if up != self.ollama_was_up {
            self.ollama_was_up = up;
            reconfigure = true;
        }
        if reconfigure {
            self.configure_models();
        }
        self.save_config();
    }

    fn save_config(&mut self) {
        let json = self.cfg.to_json();
        if json != self.saved_json {
            Config::save_json(&json);
            self.saved_json = json;
        }
    }

    fn select(&mut self, page: Page) {
        self.cfg.page = page;
        if let Page::Flow(id) = page {
            // ask for missing env vars / models right when the flow is picked (once per visit)
            if self.prompted != Some(id) && !self.ready(id) {
                self.prompt = Some(id);
            }
            self.prompted = Some(id);
        } else {
            self.prompted = None;
        }
        if page == Page::Models
            && let Some(r) = &self.repo
        {
            r.models.rescan();
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("local-llm").size(18.0).strong());
            ui.separator();
            for server in [Server::Ollama, Server::Comfy] {
                let up = self.servers.up(server);
                ui::dot(ui, if up { GREEN } else { Color32::GRAY });
                ui.label(server.name()).on_hover_text(if up {
                    "running"
                } else {
                    "not running - started automatically when a flow needs it"
                });
                if up {
                    if server == Server::Comfy && ui::tool(ui, icons::OPEN_IN_NEW, "open ComfyUI in the browser").clicked() {
                        crate::sys::open_url("http://127.0.0.1:8188");
                    }
                } else if let Some(r) = &self.repo
                    && ui::tool(ui, icons::POWER, &format!("start {}", server.name())).clicked()
                {
                    let root = r.paths.root.clone();
                    if let Err(e) = servers::start(server, &root) {
                        self.notify(e, true);
                    }
                }
                ui.add_space(6.0);
            }
            ui.separator();
            let llm = self.llm();
            let (icon, text) = match (llm.provider, &self.repo) {
                (Provider::OpenRouter, Some(r)) => {
                    (icons::CLOUD_OUTLINE, or_default(&r.env.get("OPENROUTER_MODEL"), "qwen/qwen3.8-27b:free"))
                }
                _ => (icons::DESKTOP_TOWER, llm.ollama_model.clone()),
            };
            let weak = ui.visuals().weak_text_color();
            ui::icon(ui, icon, weak);
            if ui.link(text).on_hover_text("vision LLM - change in Settings").clicked() {
                self.select(Page::Settings);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !self.queue.is_empty()
                    && ui.link(format!("+{} queued", self.queue.len())).on_hover_text("see the queue on Start").clicked()
                {
                    self.select(Page::Home);
                }
                if let Some((id, h)) = self.running_job() {
                    let mut j = h.job.lock().unwrap();
                    let frac = j.progress.fraction();
                    let eta = j.eta();
                    let text =
                        format!("{} {}{}", j.title, mmss(j.elapsed()), eta.map(|e| format!(" · {} left", mmss(e))).unwrap_or_default());
                    drop(j);
                    let bar = egui::ProgressBar::new(frac.unwrap_or(0.0)).desired_width(260.0).text(text).animate(frac.is_none());
                    if ui.add(bar).on_hover_text("open the running flow").clicked() {
                        self.select(Page::Flow(id));
                    }
                    ui.ctx().request_repaint_after(Duration::from_secs(1));
                }
                if let Some(r) = &self.repo
                    && r.models.busy()
                    && ui::tool(ui, icons::PROGRESS_DOWNLOAD, "models are downloading - open Models").clicked()
                {
                    self.select(Page::Models);
                }
            });
        });
    }

    fn nav(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        let mut clicked = None;
        let text = ui.visuals().text_color();
        let nav_item = |ui: &mut egui::Ui, selected: bool, icon: icons::Icon, label: &str| -> bool {
            let b = egui::Button::image_and_text(icon.image(18.0, text), label)
                .selected(selected)
                .frame_when_inactive(false)
                .min_size(egui::vec2(ui.available_width(), 28.0));
            ui.add(b).clicked()
        };
        let start = if self.queue.is_empty() { "Start".to_owned() } else { format!("Start  ({} queued)", self.queue.len()) };
        if nav_item(ui, self.cfg.page == Page::Home, icons::HOME, &start) {
            clicked = Some(Page::Home);
        }
        ui.add_space(4.0);
        ui.label(RichText::new("CREATE").small().weak());
        for id in FlowId::ALL {
            let title = self.cfg.forms.get(id).title();
            let running = self.running == Some(id);
            let queued = self.queue.iter().any(|(q, _)| *q == id);
            let label = if running {
                format!("{title}  ▶")
            } else if queued {
                format!("{title}  ⏳")
            } else if !self.ready(id) {
                format!("{title}  ⚠")
            } else {
                title.to_owned()
            };
            if nav_item(ui, self.cfg.page == Page::Flow(id), id.icon(), &label) {
                clicked = Some(Page::Flow(id));
            }
        }
        ui.add_space(4.0);
        ui.label(RichText::new("MANAGE").small().weak());
        for (page, icon, label) in [(Page::Models, icons::DATABASE, "Models"), (Page::Settings, icons::COG, "Settings")] {
            if nav_item(ui, self.cfg.page == page, icon, label) {
                clicked = Some(page);
            }
        }
        if let Some(p) = clicked {
            self.select(p);
        }
    }

    /// Asked once when the window is closed while a flow is running or queued.
    fn confirm_quit_dialog(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut quit = false;
        let modal = egui::Modal::new(egui::Id::new("confirm_quit")).show(ctx, |ui| {
            ui.set_width(320.0);
            ui.heading("Quit local-llm?");
            ui.label("A flow is still running - quitting stops it and loses its progress.");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::image_and_text(icons::STOP.image(16.0, RED), RichText::new("Quit anyway").color(RED))).clicked() {
                    quit = true;
                }
                if ui.button("Keep running").clicked() {
                    close = true;
                }
            });
        });
        if modal.should_close() {
            close = true;
        }
        if quit {
            self.confirm_quit = false;
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if close {
            self.confirm_quit = false;
        }
    }

    fn notice_ui(&mut self, ui: &mut egui::Ui) {
        if let Some((text, error, at)) = &self.notice {
            if at.elapsed() > Duration::from_secs(8) {
                self.notice = None;
                return;
            }
            ui.ctx().request_repaint_after(Duration::from_secs(1));
            let (icon, color) = if *error { (icons::ALERT_CIRCLE, RED) } else { (icons::CHECK_CIRCLE, GREEN) };
            ui::status(ui, icon, color, text.as_str());
        }
    }
}

pub fn mmss(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

pub fn or_default(value: &str, default: &str) -> String {
    if value.trim().is_empty() { default.to_owned() } else { value.trim().to_owned() }
}

pub fn status_text(s: &Status) -> (icons::Icon, String, Color32) {
    match s {
        Status::Present(_) => (icons::CHECK_CIRCLE, "on disk".into(), GREEN),
        Status::SizeDiffers { size, .. } => (icons::ALERT, format!("on disk, {} (size differs)", crate::sys::human(*size)), AMBER),
        Status::Linkable(p) => (
            icons::LINK_VARIANT,
            format!("same file found ({}) - linked, no download", p.file_name().unwrap_or_default().to_string_lossy()),
            AMBER,
        ),
        Status::Missing { partial: 0 } => (icons::CIRCLE_OUTLINE, "not downloaded".into(), RED),
        Status::Missing { partial } => (icons::CIRCLE_OUTLINE, format!("partly downloaded ({})", crate::sys::human(*partial)), RED),
        Status::Manual => (icons::ALERT_CIRCLE, "no public download - copy it in by hand".into(), RED),
        Status::Unknown => (icons::TIMER_SAND, "checking...".into(), Color32::GRAY),
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.tick();
        self.pump_queue();

        if ui.ctx().input(|i| i.viewport().close_requested()) && !self.allow_close {
            if self.running.is_some() || !self.queue.is_empty() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.confirm_quit = true;
            } else {
                self.allow_close = true;
            }
        }

        let dropped: Vec<PathBuf> = ui.ctx().input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty()
            && let Page::Flow(id) = self.cfg.page
        {
            self.cfg.forms.get_mut(id).drop_files(dropped);
        }

        if self.repo.is_none() {
            egui::CentralPanel::default().show(ui, |ui| self.setup_screen(ui));
            return;
        }

        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(4.0);
            self.top_bar(ui);
            ui.add_space(2.0);
        });
        egui::Panel::left("nav").resizable(false).exact_size(210.0).show(ui, |ui| self.nav(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            self.notice_ui(ui);
            match self.cfg.page {
                Page::Home => self.home_page(ui),
                Page::Flow(id) => self.flow_page(ui, id),
                Page::Models => self.models_page(ui),
                Page::Settings => self.settings_page(ui),
            }
        });
        if let Some(id) = self.prompt {
            let ctx = ui.ctx().clone();
            self.setup_dialog(&ctx, id);
        }
        if let Some(id) = self.confirm_cancel {
            let ctx = ui.ctx().clone();
            self.confirm_cancel_dialog(&ctx, id);
        }
        if self.confirm_quit {
            let ctx = ui.ctx().clone();
            self.confirm_quit_dialog(&ctx);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_config();
    }
}
