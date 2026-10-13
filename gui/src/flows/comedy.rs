use std::path::{Path, PathBuf};

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::widgets as w;
use super::{FLUX1, Flow, Llm, Needs, flag, num, opt, out_or, required_file};
use crate::icons;
use crate::ui;

const SCRIPTS: (&str, &[&str]) = ("Sketch script", &["json"]);
const CHARACTERS_TIP: &str = "empty = the LLM picks 2-3 fitting leaders; a picture (e.g. from the Characters flow) is used instead of a caricature; personality (how they talk and act) shapes the dialogue, voice and animation";

/// Flow 12: political comedy cartoon sketches (comedy/make_comedy.py).
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Comedy {
    from_script: bool,
    topic: String,
    script: String,
    cast: Vec<String>,
    /// Optional picture per character, aligned with `cast` (empty = none).
    cast_images: Vec<String>,
    /// Optional personality per character, aligned with `cast` (e.g. "boastful, repeats himself").
    cast_personality: Vec<String>,
    lang: String,
    seconds: u32,
    write_only: bool,
    preview: bool,
    format: String,
    draw_missing: bool,
    use_screenshots: bool,
    limbs: bool,
    art: String,
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
            cast_images: Vec::new(),
            cast_personality: Vec::new(),
            lang: "English".into(),
            seconds: 60,
            write_only: false,
            preview: true,
            format: "landscape".into(),
            draw_missing: true,
            use_screenshots: false,
            limbs: false,
            art: "raster".into(),
            music: "auto".into(),
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

    fn ui(&mut self, ui: &mut egui::Ui, root: &Path) {
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
                w::label(ui, "Characters", CHARACTERS_TIP);
                self.cast_rows(ui, root);
                ui.end_row();
                w::label(ui, "Language", "Kokoro voices: English, Spanish, French, Italian, Portuguese, Hindi, Japanese, Chinese; others use the Windows voices");
                w::language(ui, "comedy-lang", &mut self.lang);
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
            w::label(ui, "Drawings as", "SVG: an existing .svg next to a drawing is used, else it is traced once and cached");
            w::choice(
                ui,
                "comedy-art",
                &mut self.art,
                &[("raster", "raster bitmaps (default)"), ("svg", "SVG vectors (sharp zooms, slower render)")],
            );
            ui.end_row();
            w::label(ui, "Music", "quiet bed, ducks under the dialogue; auto = a mood per scene picked by the LLM");
            w::choice(
                ui,
                "comedy-music",
                &mut self.music,
                &[("auto", "auto (fits each scene)"), ("calm", "calm"), ("funny", "bouncy"), ("none", "none")],
            );
            ui.end_row();
            w::label(ui, "Extras", "");
            ui.vertical(|ui| {
                ui.checkbox(&mut self.subtitles, "subtitles");
                ui.checkbox(&mut self.end_card, "end card (title + subscribe)");
            });
            ui.end_row();
            w::label(ui, "Animation", "splits each cutout at the neck automatically, no extra drawings needed");
            ui.checkbox(&mut self.limbs, "head moves independently (nod/bob while talking)");
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
            for (i, name) in self.cast.iter().enumerate() {
                let name = name.trim();
                if name.is_empty() {
                    continue;
                }
                let picture = self.cast_images.get(i).map(|p| p.trim()).filter(|p| !p.is_empty());
                let value = picture.map_or_else(|| name.to_owned(), |p| format!("{name}={p}"));
                a.extend(["--cast".into(), value]);
            }
            for (i, name) in self.cast.iter().enumerate() {
                let name = name.trim();
                let personality = self.cast_personality.get(i).map(|p| p.replace(['\r', '\n'], " ")).unwrap_or_default();
                let personality = personality.trim();
                if name.is_empty() || personality.is_empty() {
                    continue;
                }
                a.extend(["--personality".into(), format!("{name}={personality}")]);
            }
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
        flag(&mut a, "--limbs", self.limbs);
        if self.art != d.art {
            opt(&mut a, "--art", &self.art);
        }
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

impl Comedy {
    /// Character rows: name, optional picture (file or folder), remove; a personality line under each; then "add character".
    fn cast_rows(&mut self, ui: &mut egui::Ui, root: &Path) {
        self.cast_images.resize(self.cast.len(), String::new());
        self.cast_personality.resize(self.cast.len(), String::new());
        let start = root.join("characters").join("out");
        let mut remove = None;
        ui.vertical(|ui| {
            let rows = self.cast.iter_mut().zip(self.cast_images.iter_mut()).zip(self.cast_personality.iter_mut());
            for (i, ((name, image), personality)) in rows.enumerate() {
                ui.horizontal(|ui| {
                    ui.push_id(("cast", i), |ui| {
                        ui.add(egui::TextEdit::singleline(name).hint_text("e.g. Trump").desired_width(150.0));
                        if ui::button(ui, icons::FILE_SEARCH_OUTLINE, "picture").clicked()
                            && let Some(p) = pick_picture(&start)
                        {
                            *image = p;
                        }
                        if ui::button(ui, icons::FOLDER_SEARCH_OUTLINE, "folder").clicked()
                            && let Some(p) = pick_folder(&start)
                        {
                            *image = p;
                        }
                        if !image.is_empty() {
                            let shown = Path::new(image.as_str())
                                .file_name()
                                .map_or_else(|| image.clone(), |n| n.to_string_lossy().into_owned());
                            ui.add_sized(
                                [120.0, ui.spacing().interact_size.y],
                                egui::Label::new(shown).truncate(),
                            )
                            .on_hover_text(image.as_str());
                            if ui::tool(ui, icons::CLOSE, "clear picture").clicked() {
                                image.clear();
                            }
                        }
                        if ui::tool(ui, icons::DELETE_OUTLINE, "remove character").clicked() {
                            remove = Some(i);
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.push_id(("personality", i), |ui| {
                        ui.add_space(ui.spacing().indent);
                        ui.add(
                            egui::TextEdit::singleline(personality)
                                .hint_text("personality, e.g. boastful, repeats himself")
                                .desired_width(420.0),
                        );
                    });
                });
            }
            if ui::button(ui, icons::PLUS, "add character").clicked() {
                self.cast.push(String::new());
                self.cast_images.push(String::new());
                self.cast_personality.push(String::new());
            }
        });
        if let Some(i) = remove {
            self.cast.remove(i);
            self.cast_images.remove(i);
            self.cast_personality.remove(i);
        }
    }
}

/// File dialog for a character picture (PNG/WebP/SVG), starting in the Characters flow output.
fn pick_picture(start: &Path) -> Option<String> {
    let mut d = rfd::FileDialog::new().add_filter("Pictures", &["png", "webp", "svg"]);
    if start.is_dir() {
        d = d.set_directory(start);
    }
    d.pick_file().map(|p| p.display().to_string())
}

/// Folder dialog for a character's drawings, starting in the Characters flow output.
fn pick_folder(start: &Path) -> Option<String> {
    let mut d = rfd::FileDialog::new();
    if start.is_dir() {
        d = d.set_directory(start);
    }
    d.pick_folder().map(|p| p.display().to_string())
}
