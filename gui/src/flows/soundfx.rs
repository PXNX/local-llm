use std::path::{Path, PathBuf};

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::widgets as w;
use super::{Flow, Llm, Needs, each, filled, num, opt, out_or};

const AUDIO: (&str, &[&str]) = ("Audio/video", &["wav", "mp3", "m4a", "flac", "ogg", "mp4", "mkv", "webm"]);

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct SoundFx {
    speech_mode: bool,
    count: u32,
    seed: i64,
    voice: String,
    speech: Vec<String>,
    denoise: f32,
    out: String,
}

impl Default for SoundFx {
    fn default() -> Self {
        Self { speech_mode: false, count: 3, seed: 0, voice: String::new(), speech: Vec::new(), denoise: 1.0, out: String::new() }
    }
}

impl Flow for SoundFx {
    fn title(&self) -> &'static str {
        "Sound effects"
    }
    fn blurb(&self) -> &'static str {
        "Procedural explosions / rocket launches, or isolate speech from recordings (Demucs)."
    }
    fn script(&self) -> &'static str {
        "soundfx/generate_sfx.py"
    }

    fn ui(&mut self, ui: &mut egui::Ui, _root: &Path) {
        w::form(ui, "soundfx", |ui| {
            w::label(ui, "Task", "");
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.speech_mode, false, "generate effects");
                ui.radio_value(&mut self.speech_mode, true, "isolate speech");
            });
            ui.end_row();
            if self.speech_mode {
                w::label(ui, "Recordings", "background noise/music is removed");
                w::list(ui, "speech", &mut self.speech, "audio or video file", Some(AUDIO), false);
                ui.end_row();
                w::label(ui, "Denoise", "0 = off, 1 = full");
                ui.add(egui::Slider::new(&mut self.denoise, 0.0..=1.0));
                ui.end_row();
            } else {
                w::label(ui, "Variations", "per sound");
                ui.add(egui::DragValue::new(&mut self.count).range(1..=20));
                ui.end_row();
                w::label(ui, "Seed", "");
                ui.add(egui::DragValue::new(&mut self.seed).range(0..=i64::from(i32::MAX)));
                ui.end_row();
                w::label(ui, "Voice clip", "optional: recording processed into slava_ukraini.wav");
                w::file(ui, &mut self.voice, AUDIO, "optional .wav");
                ui.end_row();
            }
            w::label(ui, "Output folder", "");
            w::folder(ui, &mut self.out, "default: soundfx/out");
            ui.end_row();
        });
    }

    fn args(&self) -> Result<Vec<String>, String> {
        let mut a = Vec::new();
        if self.speech_mode {
            let files = filled(&self.speech);
            if files.is_empty() {
                return Err("add at least one recording".into());
            }
            each(&mut a, "--speech", &files);
            num(&mut a, "--denoise", self.denoise, 1.0);
        } else {
            num(&mut a, "--count", self.count, 3);
            num(&mut a, "--seed", self.seed, 0);
            // the script's default voice path is machine-specific: pass an explicit (possibly missing) one
            a.extend(["--voice".into(), if self.voice.trim().is_empty() { "-".into() } else { self.voice.trim().into() }]);
        }
        opt(&mut a, "--out", &self.out);
        Ok(a)
    }

    fn needs(&self, _llm: &Llm) -> Needs {
        Needs::default()
    }

    fn out_dir(&self, root: &Path) -> PathBuf {
        out_or(&self.out, root.join("soundfx").join("out"))
    }

    fn drop_files(&mut self, files: Vec<PathBuf>) {
        if self.speech_mode {
            self.speech.retain(|s| !s.trim().is_empty());
            self.speech.extend(files.into_iter().map(|f| f.display().to_string()));
        } else if let Some(f) = files.into_iter().next() {
            self.voice = f.display().to_string();
        }
    }
}
