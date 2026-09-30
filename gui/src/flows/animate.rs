use std::path::{Path, PathBuf};

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::widgets as w;
use super::{Flow, Llm, Needs, WAN5B, filled, num, opt};

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Animate {
    inputs: Vec<String>,
    mode: String,
    motion: String,
    seconds: f32,
    prompt: String,
    steps: u32,
    seed: i64,
}

impl Default for Animate {
    fn default() -> Self {
        Self { inputs: Vec::new(), mode: "motion".into(), motion: "auto".into(), seconds: 2.0, prompt: String::new(), steps: 20, seed: 0 }
    }
}

impl Flow for Animate {
    fn title(&self) -> &'static str {
        "Animate stickers"
    }
    fn blurb(&self) -> &'static str {
        "Finished stickers -> looping Telegram video stickers (WebM, next to each sticker)."
    }
    fn script(&self) -> &'static str {
        "stickers/animate_stickers.py"
    }

    fn ui(&mut self, ui: &mut egui::Ui, root: &Path) {
        w::form(ui, "animate", |ui| {
            w::label(ui, "Stickers", "files or whole folders, e.g. stickers/out/<photo>");
            ui.vertical(|ui| {
                w::list(ui, "inputs", &mut self.inputs, "sticker .webp/.png or folder", Some(("Stickers", &["webp", "png"])), true);
                let out = root.join("stickers").join("out");
                if out.is_dir()
                    && crate::ui::button(ui, crate::icons::FOLDER_OPEN, "Add latest sticker folder").clicked()
                    && let Some(latest) = latest_dir(&out)
                {
                    self.inputs.push(latest.display().to_string());
                }
            });
            ui.end_row();

            w::label(ui, "Mode", "");
            w::choice(ui, "mode", &mut self.mode, &[("motion", "motion - instant loop, no AI"), ("ai", "ai - Wan 2.2 (ComfyUI, slow)")]);
            ui.end_row();

            if self.mode == "motion" {
                w::label(ui, "Motion", "auto matches the sticker's text/pose");
                w::choice(
                    ui,
                    "motion",
                    &mut self.motion,
                    &[("auto", "auto"), ("sway", "sway"), ("breathe", "breathe"), ("hop", "hop"), ("tremble", "tremble")],
                );
                ui.end_row();
            }

            w::label(ui, "Seconds", "max 3 for Telegram");
            ui.add(egui::Slider::new(&mut self.seconds, 0.5..=3.0).step_by(0.5));
            ui.end_row();

            if self.mode == "ai" {
                w::label(ui, "Prompt", "what should move (default: act out the sticker's pose)");
                ui.text_edit_singleline(&mut self.prompt);
                ui.end_row();
                w::label(ui, "Steps", "");
                ui.add(egui::DragValue::new(&mut self.steps).range(4..=50));
                ui.end_row();
                w::label(ui, "Seed", "");
                ui.add(egui::DragValue::new(&mut self.seed).range(0..=i64::from(i32::MAX)));
                ui.end_row();
            }
        });
    }

    fn args(&self) -> Result<Vec<String>, String> {
        let mut a = filled(&self.inputs);
        if a.is_empty() {
            return Err("add stickers or a sticker folder".into());
        }
        if let Some(missing) = a.iter().find(|p| !Path::new(p).exists()) {
            return Err(format!("not found: {missing}"));
        }
        a.extend(["--mode".into(), self.mode.clone()]);
        if self.mode == "motion" {
            opt(&mut a, "--motion", &self.motion);
        }
        num(&mut a, "--seconds", self.seconds, 2.0);
        if self.mode == "ai" {
            opt(&mut a, "--prompt", &self.prompt);
            num(&mut a, "--steps", self.steps, 20);
            num(&mut a, "--seed", self.seed, 0);
        }
        Ok(a)
    }

    fn needs(&self, _llm: &Llm) -> Needs {
        let mut n = Needs::default();
        if self.mode == "ai" {
            n.comfy(WAN5B);
        }
        n
    }

    fn out_dir(&self, root: &Path) -> PathBuf {
        filled(&self.inputs)
            .first()
            .map(PathBuf::from)
            .map(|p| if p.is_dir() { p } else { p.parent().map(Path::to_path_buf).unwrap_or(p) })
            .unwrap_or_else(|| root.join("stickers").join("out"))
    }

    fn drop_files(&mut self, files: Vec<PathBuf>) {
        self.inputs.retain(|s| !s.trim().is_empty());
        self.inputs.extend(files.into_iter().map(|f| f.display().to_string()));
    }
}

fn latest_dir(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
}
