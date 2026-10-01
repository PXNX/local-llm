use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::flows::{FlowId, Forms};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Page {
    Flow(FlowId),
    /// running, waiting and recently finished runs
    Queue,
    Models,
    Settings,
    /// also where an unknown saved page (older versions) lands
    #[serde(other)]
    Home,
}

/// GUI-only settings, kept outside the repo (%APPDATA%\local-llm-gui) so nothing private is ever committed.
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub repo: Option<PathBuf>,
    /// Older versions' custom ComfyUI folder: moved to MODELS_DIR in .env on start, then None.
    pub models_dir: Option<PathBuf>,
    /// Older versions' custom Ollama folder: becomes `ollama_prev` on start, then None.
    pub ollama_dir: Option<PathBuf>,
    /// An old Ollama folder whose models are offered to be moved into the models folder.
    pub ollama_prev: Option<PathBuf>,
    /// The models folder before it was changed, until its files are moved.
    pub models_prev: Option<PathBuf>,
    pub page: Page,
    pub forms: Forms,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            repo: None,
            models_dir: None,
            ollama_dir: None,
            ollama_prev: None,
            models_prev: None,
            page: Page::Home,
            forms: Forms::default(),
        }
    }
}

fn file() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("local-llm-gui").join("config.json"))
}

impl Config {
    pub fn load() -> Self {
        file().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Atomic write (temp file + rename), so a crash never leaves a half-written config.
    pub fn save_json(json: &str) {
        let Some(f) = file() else { return };
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = f.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(tmp, f);
        }
    }
}
