use std::path::{Path, PathBuf};

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::widgets as w;
use super::{Flow, IMAGES, Llm, Needs, WAN5B, each, filled, num, opt, out_or, required_file};

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Img2Video {
    image: String,
    prompts: Vec<String>,
    count: u32,
    seconds: f32,
    res: String,
    size: u32,
    steps: u32,
    cfg: f32,
    seed: Option<i64>,
    out: String,
}

impl Default for Img2Video {
    fn default() -> Self {
        Self {
            image: String::new(),
            prompts: Vec::new(),
            count: 1,
            seconds: 3.0,
            res: "480p".into(),
            size: 832,
            steps: 20,
            cfg: 5.0,
            seed: None,
            out: String::new(),
        }
    }
}

impl Flow for Img2Video {
    fn title(&self) -> &'static str {
        "Image to video"
    }
    fn blurb(&self) -> &'static str {
        "Still image -> short MP4 clip with Wan 2.2 5B; the vision LLM writes the motion prompt if you don't."
    }
    fn script(&self) -> &'static str {
        "img2video/make_video.py"
    }

    fn ui(&mut self, ui: &mut egui::Ui, _root: &Path) {
        w::form(ui, "img2video", |ui| {
            w::label(ui, "Start image", "");
            w::file(ui, &mut self.image, IMAGES, "jpg/png/webp");
            ui.end_row();
            w::label(ui, "Motion prompts", "empty = the vision LLM writes them");
            w::list(ui, "prompts", &mut self.prompts, "what moves + one camera move", None, false);
            ui.end_row();
            w::label(ui, "Clips", "each with its own prompt and seed");
            ui.add(egui::DragValue::new(&mut self.count).range(1..=10));
            ui.end_row();
            w::label(ui, "Length", "max ~5 s on 6 GB VRAM");
            ui.add(egui::Slider::new(&mut self.seconds, 1.0..=8.0).step_by(0.5).suffix(" s"));
            ui.end_row();
            w::label(ui, "Resolution", "Wan 2.2 5B generates up to 720p; higher is upscaled from 720p");
            ui.vertical(|ui| {
                w::choice(
                    ui,
                    "res",
                    &mut self.res,
                    &[
                        ("480p", "480p - fast"),
                        ("720p", "720p - native maximum, slow on 6 GB"),
                        ("1080p", "1080p - 720p upscaled"),
                        ("1440p", "1440p - 720p upscaled"),
                        ("4k", "4K - 720p upscaled"),
                        ("custom", "custom long side"),
                    ],
                );
                if self.res == "custom" {
                    ui.add(egui::Slider::new(&mut self.size, 384..=1280).step_by(32.0).suffix(" px"));
                }
            });
            ui.end_row();
            w::label(ui, "Steps", "12-15 faster but blurrier");
            ui.add(egui::DragValue::new(&mut self.steps).range(4..=50));
            ui.end_row();
            w::label(ui, "Prompt strength", "cfg");
            ui.add(egui::Slider::new(&mut self.cfg, 1.0..=10.0));
            ui.end_row();
            w::label(ui, "Seed", "");
            w::seed(ui, &mut self.seed);
            ui.end_row();
            w::label(ui, "Output folder", "");
            w::folder(ui, &mut self.out, "default: img2video/out");
            ui.end_row();
        });
    }

    fn args(&self) -> Result<Vec<String>, String> {
        required_file(&self.image, "a start image")?;
        let d = Img2Video::default();
        let mut a = vec![self.image.trim().to_owned()];
        each(&mut a, "--prompt", &self.prompts);
        num(&mut a, "--count", self.count, d.count);
        num(&mut a, "--seconds", self.seconds, d.seconds);
        if self.res == "custom" {
            num(&mut a, "--size", self.size, d.size);
        } else {
            opt(&mut a, "--res", &self.res);
        }
        num(&mut a, "--steps", self.steps, d.steps);
        num(&mut a, "--cfg", self.cfg, d.cfg);
        if let Some(s) = self.seed {
            a.extend(["--seed".into(), s.to_string()]);
        }
        opt(&mut a, "--out", &self.out);
        Ok(a)
    }

    fn needs(&self, llm: &Llm) -> Needs {
        let mut n = Needs::default();
        if (filled(&self.prompts).len() as u32) < self.count {
            n.llm(llm);
        }
        n.comfy(WAN5B);
        n
    }

    fn out_dir(&self, root: &Path) -> PathBuf {
        out_or(&self.out, root.join("img2video").join("out"))
    }

    fn drop_files(&mut self, files: Vec<PathBuf>) {
        if let Some(f) = files.into_iter().next() {
            self.image = f.display().to_string();
        }
    }
}
