//! Local coding models for T3 Code / OpenCode / Pi, tuned for speed or for quality.
//!
//! The toggle (`LLM_LOCAL_PROFILE` in .env) picks one of two Ollama variants built from models that are
//! already downloaded - `ollama create` shares the weights, so nothing is downloaded or copied. Each
//! variant fixes the context size and Qwen's recommended sampling; the editor configs say whether the
//! model thinks before answering. The flows' local text prompts (common/llm.py) follow the same toggle.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

use crate::servers::{self, Server};
use crate::sys;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Profile {
    Speed,
    Quality,
}

impl Profile {
    pub const ALL: [Profile; 2] = [Profile::Speed, Profile::Quality];

    pub fn from_env(v: &str) -> Self {
        if v.trim().eq_ignore_ascii_case("quality") { Profile::Quality } else { Profile::Speed }
    }
    pub fn env_value(self) -> &'static str {
        match self {
            Profile::Speed => "speed",
            Profile::Quality => "quality",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Profile::Speed => "Speed",
            Profile::Quality => "Quality",
        }
    }
    /// .env key of the downloaded model this profile's variant is built from.
    pub fn base_key(self) -> &'static str {
        match self {
            Profile::Speed => "LLM_CODING_SPEED_MODEL",
            Profile::Quality => "LLM_CODING_QUALITY_MODEL",
        }
    }
    pub fn default_base(self) -> &'static str {
        match self {
            Profile::Speed => "qwen3:8b",
            Profile::Quality => "qwen3.8-blend:27b",
        }
    }
    /// The Ollama variant the editors are pointed at.
    pub fn variant(self) -> &'static str {
        match self {
            Profile::Speed => "local-coder:speed",
            Profile::Quality => "local-coder:quality",
        }
    }
    /// Context window: the KV cache of a smaller one leaves room for more layers on a small GPU.
    pub fn num_ctx(self) -> u32 {
        match self {
            Profile::Speed => 16384,
            Profile::Quality => 32768,
        }
    }
    pub fn thinking(self) -> bool {
        self == Profile::Quality
    }
    /// Qwen3's recommended sampling: non-thinking mode for speed, thinking mode for quality.
    fn parameters(self) -> Value {
        let (temperature, top_p) = if self.thinking() { (0.6, 0.95) } else { (0.7, 0.8) };
        json!({ "num_ctx": self.num_ctx(), "temperature": temperature, "top_p": top_p, "top_k": 20, "min_p": 0.0 })
    }
    pub fn summary(self) -> &'static str {
        match self {
            Profile::Speed => "small model, 16k context, answers without thinking first - quick replies and edits",
            Profile::Quality => "large model, 32k context, thinks before answering - better code, several times slower",
        }
    }
}

/// Builds (or rebuilds) the profile's variant from `base` and points OpenCode and Pi at it.
pub fn apply(profile: Profile, base: &str) -> Result<String, String> {
    let never = Default::default();
    servers::ensure(Server::Ollama, Path::new("."), &never, &mut |_| {})?;
    let resp = ureq::post("http://127.0.0.1:11434/api/create")
        .send_json(json!({ "model": profile.variant(), "from": base, "parameters": profile.parameters(), "stream": false }))
        .map_err(|e| format!("Ollama could not build {} from {base}: {e}", profile.variant()))?;
    if resp.status().as_u16() != 200 {
        return Err(format!("Ollama answered HTTP {} for {}", resp.status().as_u16(), profile.variant()));
    }
    let mut done = vec![];
    if let Some(p) = home(".config/opencode/opencode.json") {
        update_json(&p, |v| opencode_entry(v, profile, base))?;
        done.push("OpenCode");
    }
    if let Some(p) = home(".pi/agent/models.json") {
        update_json(&p, |v| pi_entry(v, profile, base))?;
        done.push("Pi");
    }
    Ok(format!("{} ready ({base}){}", profile.variant(), if done.is_empty() { String::new() } else { format!(", set up in {}", done.join(" and ")) }))
}

/// Loads the variant into memory now, so the first message in the editor doesn't wait for it.
pub fn preload(profile: Profile) {
    std::thread::spawn(move || {
        let _ = ureq::post("http://127.0.0.1:11434/api/generate").send_json(json!({ "model": profile.variant(), "keep_alive": "30m" }));
    });
}

fn display_name(profile: Profile, base: &str) -> String {
    format!("Local coder - {} ({base})", profile.label().to_lowercase())
}

fn opencode_entry(v: &mut Value, profile: Profile, base: &str) {
    let ollama = &mut v["provider"]["ollama"];
    if ollama.get("npm").is_none() {
        ollama["npm"] = json!("@ai-sdk/openai-compatible");
        ollama["name"] = json!("Ollama (local)");
        ollama["options"] = json!({ "baseURL": "http://localhost:11434/v1" });
    }
    let mut entry = json!({
        "name": display_name(profile, base),
        "limit": { "context": profile.num_ctx(), "output": 8192 },
    });
    if !profile.thinking() {
        // Ollama's OpenAI endpoint turns thinking off with reasoning_effort "none"
        entry["options"] = json!({ "reasoningEffort": "none" });
    }
    ollama["models"][profile.variant()] = entry;
}

fn pi_entry(v: &mut Value, profile: Profile, base: &str) {
    let ollama = &mut v["providers"]["ollama"];
    if ollama.get("baseUrl").is_none() {
        *ollama = json!({ "name": "Ollama (local)", "baseUrl": "http://localhost:11434/v1", "api": "openai-completions", "apiKey": "ollama", "models": [] });
    }
    // Pi only sends reasoning_effort for reasoning models; without it Ollama lets Qwen think. Speed maps
    // every Pi thinking level to "none", so it never thinks whatever level Pi is set to.
    let mut entry = json!({
        "id": profile.variant(),
        "name": display_name(profile, base),
        "reasoning": true,
        "contextWindow": profile.num_ctx(),
        "maxTokens": 8192,
    });
    if !profile.thinking() {
        let none: serde_json::Map<String, Value> =
            ["off", "minimal", "low", "medium", "high", "xhigh", "max"].iter().map(|l| (l.to_string(), json!("none"))).collect();
        entry["thinkingLevelMap"] = Value::Object(none);
    }
    if !ollama["models"].is_array() {
        ollama["models"] = json!([]);
    }
    let models = ollama["models"].as_array_mut().expect("array");
    match models.iter_mut().find(|m| m["id"] == profile.variant()) {
        Some(m) => *m = entry,
        None => models.push(entry),
    }
}

/// Edits a JSON config in place; a missing file starts empty, one that doesn't parse is left alone.
fn update_json(path: &Path, edit: impl FnOnce(&mut Value)) -> Result<(), String> {
    let mut v = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{} is not valid JSON ({e}) - not changed", path.display()))?,
        Err(_) => json!({}),
    };
    edit(&mut v);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(&v).expect("json") + "\n";
    std::fs::write(path, text).map_err(|e| format!("could not write {}: {e}", path.display()))
}

fn home(rel: &str) -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(rel))
}

/// Where each editor is installed (by setup.bat / install-opencode2.bat).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Editor {
    T3Code,
    OpenCode,
    Pi,
}

impl Editor {
    pub const ALL: [Editor; 3] = [Editor::T3Code, Editor::OpenCode, Editor::Pi];

    pub fn label(self) -> &'static str {
        match self {
            Editor::T3Code => "T3 Code",
            Editor::OpenCode => "OpenCode (terminal)",
            Editor::Pi => "Pi (terminal)",
        }
    }
    pub fn exe(self) -> Option<PathBuf> {
        let p = match self {
            Editor::T3Code => std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join(r"Programs\t3code\T3 Code (Alpha).exe")),
            Editor::OpenCode => home(r".opencode2\bin\opencode.exe"),
            Editor::Pi => home(r".bun\bin\pi.exe"),
        }?;
        p.is_file().then_some(p)
    }

    /// T3 Code opens its own window (pick provider OpenCode or Pi and the model there); the terminal
    /// editors start in a new console with the variant selected.
    pub fn command(self, profile: Profile, root: &Path) -> Result<Command, String> {
        let exe = self.exe().ok_or_else(|| format!("{} is not installed - run setup.bat", self.label()))?;
        let mut cmd = Command::new(&exe);
        match self {
            Editor::T3Code => {}
            Editor::OpenCode => {
                cmd.args(["-m", &format!("ollama/{}", profile.variant())]);
                sys::new_console(&mut cmd);
            }
            Editor::Pi => {
                cmd.args(["--model", &format!("ollama/{}", profile.variant())]);
                sys::new_console(&mut cmd);
            }
        }
        cmd.current_dir(root);
        Ok(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_merge_into_existing_configs() {
        let mut oc = json!({ "model": "opencode/x", "provider": { "ollama": { "npm": "keep", "models": { "qwen3:8b": {} } } } });
        opencode_entry(&mut oc, Profile::Speed, "qwen3:8b");
        assert_eq!(oc["model"], "opencode/x");
        assert_eq!(oc["provider"]["ollama"]["npm"], "keep");
        assert!(oc["provider"]["ollama"]["models"]["qwen3:8b"].is_object());
        assert_eq!(oc["provider"]["ollama"]["models"]["local-coder:speed"]["limit"]["context"], 16384);
        assert_eq!(oc["provider"]["ollama"]["models"]["local-coder:speed"]["options"]["reasoningEffort"], "none");

        let mut pi = json!({ "providers": { "ollama": { "baseUrl": "u", "models": [{ "id": "local-coder:quality", "name": "old" }] } } });
        pi_entry(&mut pi, Profile::Quality, "big:27b");
        pi_entry(&mut pi, Profile::Speed, "qwen3:8b");
        let models = pi["providers"]["ollama"]["models"].as_array().unwrap();
        assert_eq!(models.len(), 2);
        assert!(models[0].get("thinkingLevelMap").is_none());
        assert_eq!(models[1]["thinkingLevelMap"]["low"], "none");
    }
}
