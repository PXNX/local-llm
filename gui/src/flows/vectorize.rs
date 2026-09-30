use std::path::{Path, PathBuf};

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::widgets as w;
use super::{Flow, IMAGES, Llm, Needs, opt, required_file};

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Vectorize {
    image: String,
    style: String,
    mode: String,
    out: String,
}

impl Default for Vectorize {
    fn default() -> Self {
        Self { image: String::new(), style: "photo".into(), mode: "color".into(), out: String::new() }
    }
}

impl Flow for Vectorize {
    fn title(&self) -> &'static str {
        "Vectorize"
    }
    fn blurb(&self) -> &'static str {
        "PNG/JPG -> SVG trace with vtracer. Local, instant, no servers."
    }
    fn script(&self) -> &'static str {
        "vectorize/trace_svg.py"
    }

    fn ui(&mut self, ui: &mut egui::Ui, _root: &Path) {
        w::form(ui, "vectorize", |ui| {
            w::label(ui, "Image", "");
            w::file(ui, &mut self.image, IMAGES, "image to trace");
            ui.end_row();
            w::label(ui, "Style", "");
            w::choice(
                ui,
                "style",
                &mut self.style,
                &[("photo", "photo - smooth splines"), ("logo", "logo - flat polygons"), ("sketch", "sketch - fine detail")],
            );
            ui.end_row();
            w::label(ui, "Colors", "");
            w::choice(ui, "mode", &mut self.mode, &[("color", "color"), ("bw", "black & white")]);
            ui.end_row();
            w::label(ui, "Output .svg", "");
            w::file(ui, &mut self.out, ("SVG", &["svg"]), "default: next to the image");
            ui.end_row();
        });
    }

    fn args(&self) -> Result<Vec<String>, String> {
        required_file(&self.image, "an image")?;
        let mut a = vec![self.image.trim().to_owned()];
        opt(&mut a, "--style", &self.style);
        opt(&mut a, "--mode", &self.mode);
        opt(&mut a, "--out", &self.out);
        Ok(a)
    }

    fn needs(&self, _llm: &Llm) -> Needs {
        Needs::default()
    }

    fn out_dir(&self, root: &Path) -> PathBuf {
        let p = if self.out.trim().is_empty() { self.image.trim() } else { self.out.trim() };
        Path::new(p).parent().map(Path::to_path_buf).unwrap_or_else(|| root.to_path_buf())
    }

    fn drop_files(&mut self, files: Vec<PathBuf>) {
        if let Some(f) = files.into_iter().next() {
            self.image = f.display().to_string();
        }
    }
}
