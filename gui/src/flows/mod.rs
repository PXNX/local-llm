mod animate;
mod caricatures;
mod characters;
mod img2video;
mod soundfx;
mod stickers;
mod top5;
mod vectorize;
pub mod widgets;

use std::path::{Path, PathBuf};

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::models::ollama_id;
use crate::servers::Server;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum Provider {
    #[default]
    OpenRouter,
    Ollama,
}

impl Provider {
    pub fn from_env(value: &str, has_key: bool) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "ollama" => Provider::Ollama,
            "openrouter" => Provider::OpenRouter,
            _ if has_key => Provider::OpenRouter,
            _ => Provider::Ollama,
        }
    }

    pub fn env_value(self) -> &'static str {
        match self {
            Provider::OpenRouter => "openrouter",
            Provider::Ollama => "ollama",
        }
    }
}

/// How the flows reach the vision LLM (from `.env`).
pub struct Llm {
    pub provider: Provider,
    pub ollama_model: String,
}

/// What a run needs before it can start.
#[derive(Default)]
pub struct Needs {
    pub env: Vec<&'static str>,
    pub models: Vec<String>,
    pub servers: Vec<Server>,
}

impl Needs {
    fn llm(&mut self, llm: &Llm) {
        match llm.provider {
            Provider::OpenRouter => self.env.push("OPENROUTER_API_KEY"),
            Provider::Ollama => {
                self.models.push(ollama_id(&llm.ollama_model));
                self.servers.push(Server::Ollama);
            }
        }
    }

    fn comfy(&mut self, models: &[&str]) {
        self.servers.push(Server::Comfy);
        self.models.extend(models.iter().map(|m| m.to_string()));
    }
}

pub const FLUX1: &[&str] = &["flux1-schnell", "t5xxl-q5", "clip-l", "flux-ae"];
pub const WAN5B: &[&str] = &["wan22-5b", "umt5-xxl", "wan22-vae"];

pub trait Flow {
    fn title(&self) -> &'static str;
    fn blurb(&self) -> &'static str;
    fn script(&self) -> &'static str;
    fn ui(&mut self, ui: &mut egui::Ui, root: &Path);
    /// Command-line arguments, or what is missing in the form.
    fn args(&self) -> Result<Vec<String>, String>;
    fn needs(&self, llm: &Llm) -> Needs;
    fn out_dir(&self, root: &Path) -> PathBuf;
    fn drop_files(&mut self, _files: Vec<PathBuf>) {}
    /// Run in a visible console window (interactive login) instead of the log panel.
    fn console(&self, _root: &Path) -> bool {
        false
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum FlowId {
    Stickers,
    Animate,
    Caricatures,
    Objects,
    Img2Video,
    Top5,
    Characters,
    Vectorize,
    SoundFx,
}

impl FlowId {
    pub fn icon(self) -> crate::icons::Icon {
        use crate::icons::*;
        match self {
            FlowId::Stickers => STICKER_EMOJI,
            FlowId::Animate => ANIMATION_PLAY,
            FlowId::Caricatures => DRAMA_MASKS,
            FlowId::Objects => CUBE_OUTLINE,
            FlowId::Img2Video => MOVIE_OPEN_PLAY,
            FlowId::Top5 => TROPHY,
            FlowId::Characters => ACCOUNT_GROUP,
            FlowId::Vectorize => VECTOR_CURVE,
            FlowId::SoundFx => WAVEFORM,
        }
    }

    pub const ALL: [FlowId; 9] = [
        FlowId::Stickers,
        FlowId::Animate,
        FlowId::Caricatures,
        FlowId::Objects,
        FlowId::Img2Video,
        FlowId::Top5,
        FlowId::Characters,
        FlowId::Vectorize,
        FlowId::SoundFx,
    ];
}

/// Every form's last state, saved with the GUI config.
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Forms {
    stickers: stickers::Stickers,
    animate: animate::Animate,
    caricatures: caricatures::Caricatures,
    objects: caricatures::Caricatures,
    img2video: img2video::Img2Video,
    top5: top5::Top5,
    characters: characters::Characters,
    vectorize: vectorize::Vectorize,
    soundfx: soundfx::SoundFx,
}

impl Default for Forms {
    fn default() -> Self {
        Self {
            stickers: Default::default(),
            animate: Default::default(),
            caricatures: caricatures::Caricatures::people(),
            objects: caricatures::Caricatures::things(),
            img2video: Default::default(),
            top5: Default::default(),
            characters: Default::default(),
            vectorize: Default::default(),
            soundfx: Default::default(),
        }
    }
}

impl Forms {
    pub fn get(&self, id: FlowId) -> &dyn Flow {
        match id {
            FlowId::Stickers => &self.stickers,
            FlowId::Animate => &self.animate,
            FlowId::Caricatures => &self.caricatures,
            FlowId::Objects => &self.objects,
            FlowId::Img2Video => &self.img2video,
            FlowId::Top5 => &self.top5,
            FlowId::Characters => &self.characters,
            FlowId::Vectorize => &self.vectorize,
            FlowId::SoundFx => &self.soundfx,
        }
    }

    pub fn get_mut(&mut self, id: FlowId) -> &mut dyn Flow {
        match id {
            FlowId::Stickers => &mut self.stickers,
            FlowId::Animate => &mut self.animate,
            FlowId::Caricatures => &mut self.caricatures,
            FlowId::Objects => &mut self.objects,
            FlowId::Img2Video => &mut self.img2video,
            FlowId::Top5 => &mut self.top5,
            FlowId::Characters => &mut self.characters,
            FlowId::Vectorize => &mut self.vectorize,
            FlowId::SoundFx => &mut self.soundfx,
        }
    }
}

// ---------------------------------------------------------------- argument helpers
fn opt(a: &mut Vec<String>, flag: &str, value: &str) {
    if !value.trim().is_empty() {
        a.push(flag.into());
        a.push(value.trim().into());
    }
}

fn num<T: PartialEq + std::fmt::Display>(a: &mut Vec<String>, flag: &str, value: T, default: T) {
    if value != default {
        a.push(flag.into());
        a.push(value.to_string());
    }
}

fn flag(a: &mut Vec<String>, flag: &str, on: bool) {
    if on {
        a.push(flag.into());
    }
}

fn each(a: &mut Vec<String>, flag: &str, values: &[String]) {
    for v in values.iter().map(|v| v.trim()).filter(|v| !v.is_empty()) {
        a.push(flag.into());
        a.push(v.into());
    }
}

fn filled(values: &[String]) -> Vec<String> {
    values.iter().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()).collect()
}

fn required_file(path: &str, what: &str) -> Result<(), String> {
    if path.trim().is_empty() {
        Err(format!("choose {what}"))
    } else if !Path::new(path.trim()).exists() {
        Err(format!("{what} not found: {path}"))
    } else {
        Ok(())
    }
}

fn out_or(out: &str, default: PathBuf) -> PathBuf {
    if out.trim().is_empty() { default } else { PathBuf::from(out.trim()) }
}

const IMAGES: (&str, &[&str]) = ("Images", &["jpg", "jpeg", "png", "webp", "bmp"]);

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Checks the generated arguments against each script's real argparse (imports only, runs nothing).
    /// Needs the repo's embedded Python: `cargo test -- --ignored`
    #[test]
    #[ignore]
    fn args_match_the_scripts() {
        let root = crate::paths::find_repo(None).expect("run inside the local-llm checkout");
        let python = crate::paths::Paths { root: root.clone() }.python();
        let img = std::env::temp_dir().join("llm-gui-test.png");
        std::fs::write(&img, b"x").unwrap();
        let img = img.display().to_string();
        let cases: Vec<(Box<dyn Flow>, &str)> = vec![
            (Box::new(serde_json::from_value::<stickers::Stickers>(json!({"image": img})).unwrap()), "no text"),
            (Box::new(serde_json::from_value::<stickers::Stickers>(json!({"image": img, "text": "Own", "texts": ["Hi", "Yes"], "engine": "sdxl", "seed": 5, "out": "x"})).unwrap()), "own texts"),
            (Box::new(serde_json::from_value::<stickers::Stickers>(json!({"image": img, "text": "Llm", "lang": "German", "stylize": false, "count": 4})).unwrap()), "llm text"),
            (Box::new(serde_json::from_value::<animate::Animate>(json!({"inputs": [img], "mode": "ai", "prompt": "wave", "steps": 12, "seed": 3, "seconds": 2.5})).unwrap()), "ai"),
            (Box::new(serde_json::from_value::<animate::Animate>(json!({"inputs": [img], "motion": "hop"})).unwrap()), "motion"),
            (Box::new(serde_json::from_value::<caricatures::Caricatures>(json!({"names": ["Donald Trump", "Macron"]})).unwrap()), "who"),
            (Box::new(serde_json::from_value::<caricatures::Caricatures>(json!({"names": ["Trump"], "photo": img, "features": "tie", "builtin": [true, false, true, true, true, true], "custom": ["waving"], "vectorize": true, "seed": 1})).unwrap()), "photo"),
            (Box::new(serde_json::from_value::<caricatures::Caricatures>(json!({"things": true, "names": ["oil tanker"], "builtin": [true, true, false, false]})).unwrap()), "things"),
            (Box::new(serde_json::from_value::<img2video::Img2Video>(json!({"image": img, "prompts": ["waves"], "count": 2, "seconds": 4.0, "res": "4k", "steps": 12, "cfg": 4.5, "seed": 9})).unwrap()), "i2v"),
            (Box::new(serde_json::from_value::<top5::Top5>(json!({"topic": "cute", "subject": "cats", "channels": ["@a", "@b"], "since_midnight": false, "hours": 48.0, "format": "landscape", "headline": "Top", "title_bar": true, "audio": "original", "batch": 2, "intro": true, "subscribe": false, "clip_seconds": 20.0, "nvenc": true, "cached_only": true, "login": "code"})).unwrap()), "top5"),
            (Box::new(serde_json::from_value::<characters::Characters>(json!({"sources": ["https://youtube.com/@x", img], "max_videos": 3, "interval": 0.5, "matching": 0.8, "only_known": true, "vectorize": true, "names": img})).unwrap()), "chars"),
            (Box::new(serde_json::from_value::<vectorize::Vectorize>(json!({"image": img, "style": "logo", "mode": "bw", "out": "a.svg"})).unwrap()), "vec"),
            (Box::new(soundfx::SoundFx::default()), "sfx"),
            (Box::new(serde_json::from_value::<soundfx::SoundFx>(json!({"speech_mode": true, "speech": [img], "denoise": 0.5})).unwrap()), "speech"),
        ];
        let harness = r#"
import argparse, runpy, sys
orig = argparse.ArgumentParser.parse_args
def parse(self, args=None, namespace=None):
    orig(self, args, namespace)
    raise SystemExit(0)
argparse.ArgumentParser.parse_args = parse
script = sys.argv[1]
sys.argv = sys.argv[1:]
runpy.run_path(script, run_name="__main__")
"#;
        let mut failed = Vec::new();
        for (flow, name) in &cases {
            let args = flow.args().unwrap_or_else(|e| panic!("{name}: {e}"));
            let out = std::process::Command::new(&python)
                .args(["-s", "-c", harness])
                .arg(root.join(flow.script()))
                .args(&args)
                .current_dir(&root)
                .output()
                .unwrap();
            if !out.status.success() {
                failed.push(format!("{name} {args:?}: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("")));
            }
        }
        assert!(failed.is_empty(), "{failed:#?}");
    }
}
