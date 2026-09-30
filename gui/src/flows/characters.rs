use std::path::{Path, PathBuf};

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::widgets as w;
use super::{Flow, Llm, Needs, filled, flag, num, opt, out_or};

const VIDEOS: (&str, &[&str]) = ("Videos", &["mp4", "mkv", "webm", "mov", "avi"]);

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Characters {
    sources: Vec<String>,
    max_videos: u32,
    max_duration: u32,
    interval: f32,
    min_score: f32,
    min_size: f32,
    cluster: f32,
    matching: f32,
    name_conf: f32,
    min_count: u32,
    dedupe: f32,
    only_known: bool,
    full_frame: bool,
    reprocess: bool,
    vectorize: bool,
    names: String,
    out: String,
}

impl Default for Characters {
    fn default() -> Self {
        Self {
            sources: vec![String::new()],
            max_videos: 10,
            max_duration: 600,
            interval: 1.0,
            min_score: 0.2,
            min_size: 0.15,
            cluster: 0.2,
            matching: 0.9,
            name_conf: 0.8,
            min_count: 3,
            dedupe: 0.93,
            only_known: false,
            full_frame: false,
            reprocess: false,
            vectorize: false,
            names: String::new(),
            out: String::new(),
        }
    }
}

impl Flow for Characters {
    fn title(&self) -> &'static str {
        "Characters"
    }
    fn blurb(&self) -> &'static str {
        "YouTube channel/playlist/video or local videos -> screenshots sorted into one folder per recurring character."
    }
    fn script(&self) -> &'static str {
        "characters/extract_characters.py"
    }

    fn ui(&mut self, ui: &mut egui::Ui, root: &Path) {
        w::form(ui, "characters", |ui| {
            w::label(ui, "Sources", "YouTube URLs and/or local video files");
            w::list(ui, "sources", &mut self.sources, "https://www.youtube.com/@channel or a video file", Some(VIDEOS), false);
            ui.end_row();
            w::label(ui, "Latest videos", "per channel/playlist");
            ui.add(egui::DragValue::new(&mut self.max_videos).range(1..=200));
            ui.end_row();
            w::label(ui, "Character names", "folder=name list, default characters/names.txt");
            ui.horizontal(|ui| {
                w::file(ui, &mut self.names, ("Text", &["txt"]), "default: characters/names.txt");
            });
            ui.end_row();
            w::label(ui, "Options", "");
            ui.vertical(|ui| {
                ui.checkbox(&mut self.only_known, "only characters from the name list");
                ui.checkbox(&mut self.full_frame, "save the whole frame instead of the crop");
                ui.checkbox(&mut self.vectorize, "afterwards cut out and trace everything to SVG");
                ui.checkbox(&mut self.reprocess, "also process already processed videos");
            });
            ui.end_row();
            w::label(ui, "Output folder", "");
            w::folder(ui, &mut self.out, "default: characters/out");
            ui.end_row();
        });
        if crate::ui::button(ui, crate::icons::TEXT_BOX_EDIT, "Edit names.txt").clicked() {
            crate::sys::open(&root.join("characters").join("names.txt"));
        }
        egui::CollapsingHeader::new("Advanced detection settings").show(ui, |ui| {
            w::form(ui, "characters-adv", |ui| {
                let f = |ui: &mut egui::Ui, text: &str, tip: &str, v: &mut f32, r: std::ops::RangeInclusive<f32>| {
                    w::label(ui, text, tip);
                    ui.add(egui::Slider::new(v, r));
                    ui.end_row();
                };
                w::label(ui, "Max duration (s)", "skip longer videos");
                ui.add(egui::DragValue::new(&mut self.max_duration).range(10..=36000));
                ui.end_row();
                f(ui, "Frame interval (s)", "seconds between sampled frames", &mut self.interval, 0.2..=10.0);
                f(ui, "Detector confidence", "", &mut self.min_score, 0.05..=0.9);
                f(ui, "Min size", "fraction of the frame height", &mut self.min_size, 0.02..=0.8);
                f(ui, "Cluster distance", "lower = stricter grouping", &mut self.cluster, 0.05..=0.6);
                f(ui, "Match threshold", "join an existing character folder", &mut self.matching, 0.5..=0.99);
                f(ui, "Name confidence", "", &mut self.name_conf, 0.3..=0.99);
                f(ui, "Dedupe", "skip near-identical screenshots", &mut self.dedupe, 0.5..=0.999);
                w::label(ui, "Min count", "screenshots needed for a new character");
                ui.add(egui::DragValue::new(&mut self.min_count).range(1..=100));
                ui.end_row();
            });
        });
        w::hint(ui, "Runs its own detector models on the GPU - stop ComfyUI first on 6 GB VRAM.");
    }

    fn args(&self) -> Result<Vec<String>, String> {
        let mut a = filled(&self.sources);
        if a.is_empty() {
            return Err("add a YouTube URL or a video file".into());
        }
        let d = Characters::default();
        num(&mut a, "--max-videos", self.max_videos, d.max_videos);
        num(&mut a, "--max-duration", self.max_duration, d.max_duration);
        num(&mut a, "--interval", self.interval, d.interval);
        num(&mut a, "--min-score", self.min_score, d.min_score);
        num(&mut a, "--min-size", self.min_size, d.min_size);
        num(&mut a, "--cluster", self.cluster, d.cluster);
        num(&mut a, "--match", self.matching, d.matching);
        num(&mut a, "--name-conf", self.name_conf, d.name_conf);
        num(&mut a, "--min-count", self.min_count, d.min_count);
        num(&mut a, "--dedupe", self.dedupe, d.dedupe);
        flag(&mut a, "--only-known", self.only_known);
        flag(&mut a, "--full-frame", self.full_frame);
        flag(&mut a, "--reprocess", self.reprocess);
        flag(&mut a, "--vectorize", self.vectorize);
        opt(&mut a, "--names", &self.names);
        opt(&mut a, "--out", &self.out);
        Ok(a)
    }

    fn needs(&self, _llm: &Llm) -> Needs {
        Needs::default()
    }

    fn out_dir(&self, root: &Path) -> PathBuf {
        out_or(&self.out, root.join("characters").join("out"))
    }

    fn drop_files(&mut self, files: Vec<PathBuf>) {
        self.sources.retain(|s| !s.trim().is_empty());
        self.sources.extend(files.into_iter().map(|f| f.display().to_string()));
    }
}
