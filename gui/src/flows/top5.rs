use std::path::{Path, PathBuf};

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::widgets as w;
use super::{Flow, Llm, Needs, each, filled, flag, num, opt};

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Top5 {
    topic: String,
    subject: String,
    channels: Vec<String>,
    since_midnight: bool,
    hours: f32,
    count: u32,
    format: String,
    lang: String,
    headline: String,
    title_bar: bool,
    audio: String,
    batch: u32,
    subscribe: bool,
    subscribe_text: String,
    intro: bool,
    intro_seconds: f32,
    clip_seconds: f32,
    card_seconds: f32,
    max_candidates: u32,
    scan_limit: u32,
    min_duration: f32,
    max_duration: f32,
    max_mb: f32,
    nvenc: bool,
    keep_work: bool,
    cached_only: bool,
    login: String,
    console: bool,
}

impl Default for Top5 {
    fn default() -> Self {
        Self {
            topic: "auto".into(),
            subject: String::new(),
            channels: Vec::new(),
            since_midnight: true,
            hours: 24.0,
            count: 5,
            format: "vertical".into(),
            lang: "English".into(),
            headline: String::new(),
            title_bar: false,
            audio: "music".into(),
            batch: 1,
            subscribe: true,
            subscribe_text: "Please subscribe for more!".into(),
            intro: false,
            intro_seconds: 3.0,
            clip_seconds: 30.0,
            card_seconds: 2.0,
            max_candidates: 25,
            scan_limit: 300,
            min_duration: 3.0,
            max_duration: 180.0,
            max_mb: 150.0,
            nvenc: false,
            keep_work: false,
            cached_only: false,
            login: "qr".into(),
            console: false,
        }
    }
}

fn logged_in(root: &Path) -> bool {
    root.join("topvideos").join("telegram.session").is_file()
}

impl Flow for Top5 {
    fn title(&self) -> &'static str {
        "Top 5 videos"
    }
    fn blurb(&self) -> &'static str {
        "Today's videos from Telegram channels, rated by the vision LLM -> animated Top-5 countdown MP4."
    }
    fn script(&self) -> &'static str {
        "topvideos/make_top5.py"
    }

    fn ui(&mut self, ui: &mut egui::Ui, root: &Path) {
        w::form(ui, "top5", |ui| {
            w::label(ui, "Topic", "auto picks whichever has the better clips");
            w::choice(ui, "topic", &mut self.topic, &[("auto", "auto"), ("funny", "funniest"), ("cute", "cutest")]);
            ui.end_row();
            w::label(ui, "Only about", "e.g. animals, cats, babies");
            ui.add(egui::TextEdit::singleline(&mut self.subject).hint_text("anything"));
            ui.end_row();
            w::label(ui, "Channels", "empty = topvideos/channels.txt");
            ui.vertical(|ui| {
                w::list(ui, "channels", &mut self.channels, "@username or numeric ID", None, false);
                if crate::ui::button(ui, crate::icons::PLAYLIST_EDIT, "Edit channels.txt").clicked() {
                    crate::sys::open(&root.join("topvideos").join("channels.txt"));
                }
            });
            ui.end_row();
            w::label(ui, "Time window", "");
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.since_midnight, "since midnight");
                if !self.since_midnight {
                    ui.add(egui::DragValue::new(&mut self.hours).range(1.0..=720.0).suffix(" h"));
                }
            });
            ui.end_row();
            w::label(ui, "Places", "");
            ui.add(egui::DragValue::new(&mut self.count).range(2..=10));
            ui.end_row();
            w::label(ui, "Format", "");
            w::choice(
                ui,
                "format",
                &mut self.format,
                &[("vertical", "vertical 720x1280 (Shorts/Reels)"), ("landscape", "landscape 1280x720")],
            );
            ui.end_row();
            w::label(ui, "Title language", "");
            ui.text_edit_singleline(&mut self.lang);
            ui.end_row();
            w::label(ui, "Headline", "own intro text");
            ui.add(egui::TextEdit::singleline(&mut self.headline).hint_text("Top 5 Funniest/Cutest Videos of Today"));
            ui.end_row();
            w::label(ui, "Audio", "");
            w::choice(
                ui,
                "audio",
                &mut self.audio,
                &[
                    ("music", "own generated soundtrack (copyright-free)"),
                    ("original", "the clips' own sound (may contain copyrighted music)"),
                    ("none", "silent"),
                ],
            );
            ui.end_row();
            w::label(ui, "Videos per run", "each with different clips");
            ui.add(egui::DragValue::new(&mut self.batch).range(1..=10));
            ui.end_row();
            w::label(ui, "Extras", "");
            ui.vertical(|ui| {
                ui.checkbox(&mut self.title_bar, "title bar on the clips");
                ui.checkbox(&mut self.intro, "intro card");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.subscribe, "subscribe prompt:");
                    if self.subscribe {
                        ui.text_edit_singleline(&mut self.subscribe_text);
                    }
                });
                ui.checkbox(&mut self.nvenc, "encode on the GPU (NVENC, driver >= 570)");
            });
            ui.end_row();
            w::label(ui, "Telegram login", "only needed once, the session is saved");
            ui.vertical(|ui| {
                if logged_in(root) {
                    ui.label("logged in (topvideos/telegram.session)");
                } else {
                    ui.label("not logged in yet - the run opens a console window for the login");
                }
                w::choice(ui, "login", &mut self.login, &[("qr", "QR code (scan in the Telegram app)"), ("code", "phone number + code")]);
                ui.checkbox(&mut self.console, "always run in a console window");
            });
            ui.end_row();
        });
        egui::CollapsingHeader::new("Advanced").show(ui, |ui| {
            w::form(ui, "top5-adv", |ui| {
                let f = |ui: &mut egui::Ui, text: &str, v: &mut f32, r: std::ops::RangeInclusive<f32>, suffix: &str| {
                    w::label(ui, text, "");
                    ui.add(egui::DragValue::new(v).range(r).suffix(suffix));
                    ui.end_row();
                };
                f(ui, "Clip length", &mut self.clip_seconds, 3.0..=180.0, " s");
                f(ui, "Rank card", &mut self.card_seconds, 0.5..=10.0, " s");
                f(ui, "Intro length", &mut self.intro_seconds, 1.0..=10.0, " s");
                f(ui, "Min video length", &mut self.min_duration, 0.0..=60.0, " s");
                f(ui, "Max video length", &mut self.max_duration, 5.0..=3600.0, " s");
                f(ui, "Max file size", &mut self.max_mb, 5.0..=2000.0, " MB");
                w::label(ui, "Rate at most", "clips downloaded and rated");
                ui.add(egui::DragValue::new(&mut self.max_candidates).range(2..=200));
                ui.end_row();
                w::label(ui, "Posts per channel", "");
                ui.add(egui::DragValue::new(&mut self.scan_limit).range(10..=5000));
                ui.end_row();
                w::label(ui, "Cache", "");
                ui.vertical(|ui| {
                    ui.checkbox(&mut self.cached_only, "only already downloaded and rated clips");
                    ui.checkbox(&mut self.keep_work, "keep intermediate cards/clips");
                });
                ui.end_row();
            });
        });
    }

    fn args(&self) -> Result<Vec<String>, String> {
        let d = Top5::default();
        let mut a = Vec::new();
        opt(&mut a, "--topic", &self.topic);
        opt(&mut a, "--subject", &self.subject);
        each(&mut a, "--channel", &filled(&self.channels));
        if !self.since_midnight {
            a.extend(["--hours".into(), self.hours.to_string()]);
        }
        num(&mut a, "--count", self.count, d.count);
        opt(&mut a, "--format", &self.format);
        opt(&mut a, "--lang", &self.lang);
        opt(&mut a, "--headline", &self.headline);
        flag(&mut a, "--title-bar", self.title_bar);
        if self.audio != d.audio {
            opt(&mut a, "--audio", &self.audio);
        }
        num(&mut a, "--batch", self.batch, d.batch);
        flag(&mut a, "--intro", self.intro);
        num(&mut a, "--intro-seconds", self.intro_seconds, d.intro_seconds);
        flag(&mut a, "--no-subscribe", !self.subscribe);
        if self.subscribe && self.subscribe_text.trim() != d.subscribe_text {
            opt(&mut a, "--subscribe-text", &self.subscribe_text);
        }
        num(&mut a, "--clip-seconds", self.clip_seconds, d.clip_seconds);
        num(&mut a, "--card-seconds", self.card_seconds, d.card_seconds);
        num(&mut a, "--max-candidates", self.max_candidates, d.max_candidates);
        num(&mut a, "--scan-limit", self.scan_limit, d.scan_limit);
        num(&mut a, "--min-duration", self.min_duration, d.min_duration);
        num(&mut a, "--max-duration", self.max_duration, d.max_duration);
        num(&mut a, "--max-mb", self.max_mb, d.max_mb);
        flag(&mut a, "--nvenc", self.nvenc);
        flag(&mut a, "--keep-work", self.keep_work);
        flag(&mut a, "--cached-only", self.cached_only);
        opt(&mut a, "--login", &self.login);
        Ok(a)
    }

    fn needs(&self, llm: &Llm) -> Needs {
        let mut n = Needs { env: vec!["TELEGRAM_API_ID", "TELEGRAM_API_HASH"], ..Default::default() };
        n.llm(llm);
        n
    }

    fn out_dir(&self, root: &Path) -> PathBuf {
        root.join("topvideos").join("out")
    }

    fn console(&self, root: &Path) -> bool {
        self.console || !logged_in(root)
    }
}
