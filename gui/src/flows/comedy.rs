use std::path::{Path, PathBuf};

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::widgets as w;
use super::{FLUX1, Flow, Llm, Needs, each, filled, flag, num, opt, out_or, required_file};

const SCRIPTS: (&str, &[&str]) = ("Sketch script", &["json"]);

/// Flow 12: political comedy cartoon sketches (comedy/make_comedy.py).
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Comedy {
    from_script: bool,
    topic: String,
    script: String,
    cast: Vec<String>,
    lang: String,
    seconds: u32,
    write_only: bool,
    preview: bool,
    format: String,
    draw_missing: bool,
    use_screenshots: bool,
    music: String,
    subtitles: bool,
    end_card: bool,
    handle: String,
    voice_engine: String,
    workers: u32,
    seed: Option<i64>,
    out: String,
}

impl Default for Comedy {
    fn default() -> Self {
        Self {
            from_script: false,
            topic: String::new(),
            script: String::new(),
            cast: Vec::new(),
            lang: "English".into(),
            seconds: 60,
            write_only: false,
            preview: true,
            format: "landscape".into(),
            draw_missing: true,
            use_screenshots: false,
            music: "calm".into(),
            subtitles: true,
            end_card: true,
            handle: String::new(),
            voice_engine: "auto".into(),
            workers: 0,
            seed: None,
            out: String::new(),
        }
    }
}

impl Flow for Comedy {
    fn title(&self) -> &'static str {
        "Comedy sketch"
    }
    fn blurb(&self) -> &'static str {
        "Topic -> LLM-written political comedy cartoon (freeonis style) with the caricatures/objects of flows 7/8, \
         local voices, cuts, explosions and subtitles. Preview 480p 30 fps, final 1080p 60 fps."
    }
    fn script(&self) -> &'static str {
        "comedy/make_comedy.py"
    }

    fn ui(&mut self, ui: &mut egui::Ui, _root: &Path) {
        w::form(ui, "comedy", |ui| {
            w::label(ui, "Sketch", "write a new one, or render a script.json written earlier (edit it in between)");
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.from_script, false, "new from a topic");
                ui.radio_value(&mut self.from_script, true, "existing script.json");
            });
            ui.end_row();
            if self.from_script {
                w::label(ui, "Script", "comedy/out/<title>/script.json");
                w::file(ui, &mut self.script, SCRIPTS, "script.json");
                ui.end_row();
            } else {
                w::label(ui, "Topic", "what the sketch is about");
                ui.add(
                    egui::TextEdit::multiline(&mut self.topic)
                        .desired_rows(2)
                        .hint_text("e.g. Putin explains why the refineries keep burning"),
                );
                ui.end_row();
                w::label(ui, "Characters", "empty = the LLM picks 2-3 fitting leaders");
                w::list(ui, "cast", &mut self.cast, "e.g. Trump", None, false);
                ui.end_row();
                w::label(ui, "Language", "Kokoro voices: English, Spanish, French, Italian, Portuguese, Hindi, Japanese, Chinese; others use the Windows voices");
                ui.text_edit_singleline(&mut self.lang);
                ui.end_row();
                w::label(ui, "Length", "rough target");
                ui.add(egui::DragValue::new(&mut self.seconds).range(15..=300).suffix(" s"));
                ui.end_row();
                w::label(ui, "Only write", "stop after script.json/script.txt, to edit it before rendering");
                ui.checkbox(&mut self.write_only, "only write the script");
                ui.end_row();
            }
            w::label(ui, "Quality", "check the timing in the preview, then render the final video from its script.json");
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.preview, true, "preview 480p 30 fps (fast)");
                ui.radio_value(&mut self.preview, false, "final 1080p 60 fps");
            });
            ui.end_row();
            w::label(ui, "Format", "");
            w::choice(
                ui,
                "comedy-format",
                &mut self.format,
                &[("landscape", "landscape 16:9 (YouTube)"), ("vertical", "vertical 9:16 (Shorts/Reels)")],
            );
            ui.end_row();
            w::label(ui, "Drawings", "missing caricatures/objects/backgrounds are drawn with FLUX.1 (about 1 min each), else placeholders");
            ui.vertical(|ui| {
                ui.checkbox(&mut self.draw_missing, "draw missing ones in ComfyUI");
                ui.checkbox(&mut self.use_screenshots, "use flow 6 screenshots (the channel's own designs - not for publishing)");
            });
            ui.end_row();
            w::label(ui, "Music", "quiet bed, ducks under the dialogue");
            w::choice(ui, "comedy-music", &mut self.music, &[("calm", "calm"), ("funny", "bouncy"), ("none", "none")]);
            ui.end_row();
            w::label(ui, "Extras", "");
            ui.vertical(|ui| {
                ui.checkbox(&mut self.subtitles, "subtitles");
                ui.checkbox(&mut self.end_card, "end card (title + subscribe)");
            });
            ui.end_row();
            w::label(ui, "Channel name", "shown top right");
            ui.add(egui::TextEdit::singleline(&mut self.handle).hint_text("e.g. @mychannel"));
            ui.end_row();
        });
        egui::CollapsingHeader::new("Advanced").show(ui, |ui| {
            w::form(ui, "comedy-adv", |ui| {
                w::label(ui, "Voices", "");
                w::choice(
                    ui,
                    "comedy-voice",
                    &mut self.voice_engine,
                    &[("auto", "auto"), ("kokoro", "Kokoro-82M"), ("sapi", "Windows voices")],
                );
                ui.end_row();
                w::label(ui, "Render workers", "0 = half the CPU threads (max 6)");
                ui.add(egui::DragValue::new(&mut self.workers).range(0..=16));
                ui.end_row();
                w::label(ui, "Seed", "drawings and backgrounds");
                w::seed(ui, &mut self.seed);
                ui.end_row();
                w::label(ui, "Output folder", "");
                w::folder(ui, &mut self.out, "default: comedy/out");
                ui.end_row();
            });
        });
    }

    fn args(&self) -> Result<Vec<String>, String> {
        let d = Comedy::default();
        let mut a = Vec::new();
        if self.from_script {
            required_file(&self.script, "a script.json")?;
            opt(&mut a, "--script", &self.script);
        } else {
            if self.topic.trim().is_empty() {
                return Err("enter a topic".into());
            }
            opt(&mut a, "--topic", &self.topic.replace(['\r', '\n'], " "));
            each(&mut a, "--cast", &filled(&self.cast));
            opt(&mut a, "--lang", &self.lang);
            num(&mut a, "--seconds", self.seconds, d.seconds);
            flag(&mut a, "--write-only", self.write_only);
        }
        flag(&mut a, "--preview", self.preview);
        if self.format != d.format {
            opt(&mut a, "--format", &self.format);
        }
        flag(&mut a, "--no-generate", !self.draw_missing);
        flag(&mut a, "--use-screenshots", self.use_screenshots);
        if self.music != d.music {
            opt(&mut a, "--music", &self.music);
        }
        flag(&mut a, "--no-subtitles", !self.subtitles);
        flag(&mut a, "--no-end-card", !self.end_card);
        opt(&mut a, "--handle", &self.handle);
        if self.voice_engine != d.voice_engine {
            opt(&mut a, "--voice-engine", &self.voice_engine);
        }
        if self.workers > 0 {
            a.extend(["--workers".into(), self.workers.to_string()]);
        }
        if let Some(s) = self.seed {
            a.extend(["--seed".into(), s.to_string()]);
        }
        opt(&mut a, "--out", &self.out);
        Ok(a)
    }

    fn needs(&self, llm: &Llm) -> Needs {
        let mut n = Needs::default();
        if !self.from_script {
            n.llm(llm);
        }
        if !(self.write_only && !self.from_script) && self.voice_engine != "sapi" {
            n.models.extend(["kokoro-onnx".to_string(), "kokoro-voices".to_string()]);
        }
        if self.draw_missing && !(self.write_only && !self.from_script) {
            n.comfy(FLUX1);
        }
        n
    }

    fn out_dir(&self, root: &Path) -> PathBuf {
        if self.from_script
            && let Some(dir) = Path::new(self.script.trim()).parent().filter(|p| p.is_dir())
        {
            return dir.to_path_buf();
        }
        out_or(&self.out, root.join("comedy").join("out"))
    }

    fn drop_files(&mut self, files: Vec<PathBuf>) {
        if let Some(f) = files.into_iter().find(|f| f.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"))) {
            self.script = f.display().to_string();
            self.from_script = true;
        }
    }
}
