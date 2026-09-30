use std::path::{Path, PathBuf};

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::widgets as w;
use super::{FLUX1, Flow, IMAGES, Llm, Needs, each, filled, flag, opt, out_or};

// the built-in lists of caricatures/make_caricatures.py
const EXPRESSIONS: &[&str] = &[
    "neutral expression, standing",
    "laughing with open mouth, arms raised",
    "angry and shouting, pointing finger",
    "shocked with wide eyes, hands on cheeks",
    "smug grin, arms crossed",
    "crying with big tears",
];
const VIEWS: &[&str] = &["side view", "three-quarter view", "on fire with black smoke clouds", "broken and damaged, cracks and debris"];

/// Flow 7 (people) and flow 8 (objects/buildings) share one script.
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Caricatures {
    things: bool,
    names: Vec<String>,
    photo: String,
    photo_name: String,
    features: String,
    strength: f32,
    builtin: Vec<bool>,
    custom: Vec<String>,
    cutout: bool,
    vectorize: bool,
    seed: Option<i64>,
    out: String,
}

impl Default for Caricatures {
    fn default() -> Self {
        Self::people()
    }
}

impl Caricatures {
    pub fn people() -> Self {
        Self::new(false)
    }

    pub fn things() -> Self {
        Self::new(true)
    }

    fn new(things: bool) -> Self {
        Self {
            things,
            names: vec![String::new()],
            photo: String::new(),
            photo_name: String::new(),
            features: String::new(),
            strength: 0.75,
            builtin: vec![true; if things { VIEWS.len() } else { EXPRESSIONS.len() }],
            custom: Vec::new(),
            cutout: false,
            vectorize: false,
            seed: None,
            out: String::new(),
        }
    }

    fn variants(&self) -> &'static [&'static str] {
        if self.things { VIEWS } else { EXPRESSIONS }
    }

    /// None = the script's full built-in list.
    fn chosen(&self) -> Option<Vec<String>> {
        let custom = filled(&self.custom);
        let all = self.builtin.iter().all(|b| *b) && self.builtin.len() == self.variants().len();
        if all && custom.is_empty() {
            return None;
        }
        let mut v: Vec<String> = self.variants().iter().zip(&self.builtin).filter(|(_, on)| **on).map(|(s, _)| s.to_string()).collect();
        v.extend(custom);
        Some(v)
    }
}

impl Flow for Caricatures {
    fn title(&self) -> &'static str {
        if self.things { "Objects" } else { "Caricatures" }
    }
    fn blurb(&self) -> &'static str {
        if self.things {
            "Objects/buildings -> flat 2D cartoons in several views and states (FLUX.1 schnell)."
        } else {
            "Famous people or a photo -> flat 2D political-satire caricatures in several expressions (FLUX.1 schnell)."
        }
    }
    fn script(&self) -> &'static str {
        "caricatures/make_caricatures.py"
    }

    fn ui(&mut self, ui: &mut egui::Ui, _root: &Path) {
        if self.builtin.len() != self.variants().len() {
            self.builtin = vec![true; self.variants().len()];
        }
        let id = if self.things { "objects" } else { "caricatures" };
        w::form(ui, id, |ui| {
            if self.things {
                w::label(ui, "Things", "one folder each");
                w::list(ui, "things", &mut self.names, "e.g. oil tanker, Kremlin", None, false);
                ui.end_row();
            } else {
                w::label(ui, "Famous people", "drawn from the model's knowledge");
                w::list(ui, "who", &mut self.names, "e.g. Donald Trump", None, false);
                ui.end_row();
                w::label(ui, "Or a photo", "img2img from a photo, e.g. of yourself");
                w::file(ui, &mut self.photo, IMAGES, "optional photo of a person");
                ui.end_row();
                if !self.photo.trim().is_empty() {
                    w::label(ui, "Name", "folder/file name for the photo");
                    ui.add(egui::TextEdit::singleline(&mut self.photo_name).hint_text("default: photo file name"));
                    ui.end_row();
                    w::label(ui, "Likeness", "0.5 close to the photo .. 0.9 free cartoon");
                    ui.add(egui::Slider::new(&mut self.strength, 0.5..=0.9));
                    ui.end_row();
                }
                w::label(ui, "Look hints", "added to the prompt");
                ui.add(egui::TextEdit::singleline(&mut self.features).hint_text("e.g. grey hair, rimless glasses"));
                ui.end_row();
            }

            w::label(ui, if self.things { "Views/states" } else { "Expressions" }, "one image each");
            ui.vertical(|ui| {
                let variants = self.variants();
                for (on, text) in self.builtin.iter_mut().zip(variants) {
                    ui.checkbox(on, *text);
                }
                w::list(ui, "custom", &mut self.custom, "own variant, e.g. waving a flag", None, false);
            });
            ui.end_row();

            w::label(ui, "Extras", "");
            ui.vertical(|ui| {
                ui.checkbox(&mut self.cutout, "transparent PNG cutout");
                ui.checkbox(&mut self.vectorize, "SVG trace (implies cutout)");
            });
            ui.end_row();
            w::label(ui, "Seed", "");
            w::seed(ui, &mut self.seed);
            ui.end_row();
            w::label(ui, "Output folder", "");
            w::folder(ui, &mut self.out, "default: caricatures/out");
            ui.end_row();
        });
    }

    fn args(&self) -> Result<Vec<String>, String> {
        let names = filled(&self.names);
        let mut a = Vec::new();
        if self.things {
            if names.is_empty() {
                return Err("enter at least one thing".into());
            }
            each(&mut a, "--thing", &names);
        } else {
            let photo = self.photo.trim();
            if names.is_empty() && photo.is_empty() {
                return Err("enter a famous person or choose a photo".into());
            }
            if !photo.is_empty() && !Path::new(photo).is_file() {
                return Err(format!("photo not found: {photo}"));
            }
            each(&mut a, "--who", &names);
            if !photo.is_empty() {
                a.extend(["--photo".into(), photo.into()]);
                opt(&mut a, "--name", &self.photo_name);
                a.extend(["--strength".into(), format!("{:.2}", self.strength)]);
            }
            opt(&mut a, "--features", &self.features);
        }
        if let Some(v) = self.chosen() {
            if v.is_empty() {
                return Err("tick at least one variant".into());
            }
            each(&mut a, "--variant", &v);
        }
        flag(&mut a, "--cutout", self.cutout || self.vectorize);
        flag(&mut a, "--vectorize", self.vectorize);
        if let Some(s) = self.seed {
            a.extend(["--seed".into(), s.to_string()]);
        }
        opt(&mut a, "--out", &self.out);
        Ok(a)
    }

    fn needs(&self, llm: &Llm) -> Needs {
        let mut n = Needs::default();
        if !self.things && !self.photo.trim().is_empty() {
            n.llm(llm);
        }
        n.comfy(FLUX1);
        n
    }

    fn out_dir(&self, root: &Path) -> PathBuf {
        out_or(&self.out, root.join("caricatures").join("out"))
    }

    fn drop_files(&mut self, files: Vec<PathBuf>) {
        if !self.things
            && let Some(f) = files.into_iter().next()
        {
            self.photo = f.display().to_string();
        }
    }
}
