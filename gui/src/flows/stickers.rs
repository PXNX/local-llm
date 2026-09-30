use std::path::{Path, PathBuf};

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::widgets as w;
use super::{FLUX1, Flow, IMAGES, Llm, Needs, each, flag, num, opt, out_or, required_file};

const ENGINES: &[(&str, &str)] = &[
    ("flux1", "FLUX.1 schnell (default, works)"),
    ("sdxl", "SDXL DreamShaper (broken on this ComfyUI)"),
    ("photomaker", "SDXL + PhotoMaker (broken on this ComfyUI)"),
    ("qwen-image21", "Qwen-Image 2.1 edit (large)"),
    ("flux2-klein", "FLUX.2 klein edit (large)"),
];

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq)]
enum Text {
    None,
    Llm,
    Own,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Stickers {
    image: String,
    count: u32,
    text: Text,
    lang: String,
    texts: Vec<String>,
    engine: String,
    strength: f32,
    stylize: bool,
    seed: Option<i64>,
    out: String,
}

impl Default for Stickers {
    fn default() -> Self {
        Self {
            image: String::new(),
            count: 3,
            text: Text::None,
            lang: "English".into(),
            texts: vec![String::new()],
            engine: "flux1".into(),
            strength: 0.55,
            stylize: true,
            seed: None,
            out: String::new(),
        }
    }
}

impl Stickers {
    fn engine_models(&self) -> &'static [&'static str] {
        match self.engine.as_str() {
            "sdxl" => &["dreamshaper-xl"],
            "photomaker" => &["dreamshaper-xl", "photomaker-v2"],
            "qwen-image21" => &["qwen-image21-uc", "qwen3vl-8b-te", "qwen-image21-vae"],
            "flux2-klein" => &["flux2-klein-9b", "flux2-klein-te", "flux2-vae"],
            _ => FLUX1,
        }
    }
}

impl Flow for Stickers {
    fn title(&self) -> &'static str {
        "Stickers"
    }
    fn blurb(&self) -> &'static str {
        "Photo of an animal or person -> transparent WebP reaction stickers (Telegram/WhatsApp)."
    }
    fn script(&self) -> &'static str {
        "stickers/make_stickers.py"
    }

    fn ui(&mut self, ui: &mut egui::Ui, _root: &Path) {
        w::form(ui, "stickers", |ui| {
            w::label(ui, "Photo", "drag & drop an image onto the window");
            w::file(ui, &mut self.image, IMAGES, "photo of the animal/person");
            ui.end_row();

            w::label(ui, "Text on stickers", "");
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.text, Text::None, "none");
                ui.radio_value(&mut self.text, Text::Llm, "LLM picks");
                ui.radio_value(&mut self.text, Text::Own, "my own");
            });
            ui.end_row();

            if self.text == Text::Own {
                w::label(ui, "Texts", "one sticker per text");
                w::list(ui, "texts", &mut self.texts, "e.g. Monday mood", None, false);
            } else {
                w::label(ui, "Count", "");
                ui.add(egui::DragValue::new(&mut self.count).range(1..=12));
            }
            ui.end_row();

            if self.text == Text::Llm {
                w::label(ui, "Language", "caption language");
                ui.text_edit_singleline(&mut self.lang);
                ui.end_row();
            }

            w::label(ui, "Stylize (ComfyUI)", "off: only cut out the original photo");
            ui.checkbox(&mut self.stylize, "draw as a cartoon sticker");
            ui.end_row();

            if self.stylize {
                w::label(ui, "Engine", "");
                w::choice(ui, "engine", &mut self.engine, ENGINES);
                ui.end_row();
                if matches!(self.engine.as_str(), "sdxl" | "photomaker") {
                    w::label(ui, "Strength", "img2img denoise: 0.3 close to photo .. 0.8 free");
                    ui.add(egui::Slider::new(&mut self.strength, 0.3..=0.8));
                    ui.end_row();
                }
            }

            w::label(ui, "Seed", "");
            w::seed(ui, &mut self.seed);
            ui.end_row();

            w::label(ui, "Output folder", "gets one subfolder per photo");
            w::folder(ui, &mut self.out, "default: stickers/out");
            ui.end_row();
        });
    }

    fn args(&self) -> Result<Vec<String>, String> {
        required_file(&self.image, "a photo")?;
        let mut a = vec![self.image.trim().to_owned()];
        match self.text {
            Text::Own => {
                if super::filled(&self.texts).is_empty() {
                    return Err("enter at least one text".into());
                }
                each(&mut a, "--text", &self.texts);
            }
            Text::None => {
                num(&mut a, "--count", self.count, 3);
                a.push("--no-text".into());
            }
            Text::Llm => {
                num(&mut a, "--count", self.count, 3);
                opt(&mut a, "--lang", &self.lang);
            }
        }
        if self.stylize {
            opt(&mut a, "--engine", &self.engine);
            if matches!(self.engine.as_str(), "sdxl" | "photomaker") {
                a.extend(["--strength".into(), format!("{:.2}", self.strength)]);
            }
        }
        flag(&mut a, "--no-stylize", !self.stylize);
        if let Some(s) = self.seed {
            a.extend(["--seed".into(), s.to_string()]);
        }
        opt(&mut a, "--out", &self.out);
        Ok(a)
    }

    fn needs(&self, llm: &Llm) -> Needs {
        let mut n = Needs::default();
        // make_stickers.py asks the LLM unless it has its own texts and nothing to draw
        if self.text != Text::Own || self.stylize {
            n.llm(llm);
        }
        if self.stylize {
            n.comfy(self.engine_models());
        }
        n
    }

    fn out_dir(&self, root: &Path) -> PathBuf {
        let stem = Path::new(self.image.trim()).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        out_or(&self.out, root.join("stickers").join("out")).join(stem)
    }

    fn drop_files(&mut self, files: Vec<PathBuf>) {
        if let Some(f) = files.into_iter().next() {
            self.image = f.display().to_string();
        }
    }
}
