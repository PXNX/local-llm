use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// A variable the flows read from the repo's `.env`.
pub struct EnvVar {
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub url: &'static str,
    pub secret: bool,
}

pub const VARS: &[EnvVar] = &[
    EnvVar {
        key: "OPENROUTER_API_KEY",
        label: "OpenRouter API key",
        help: "Free vision models for captions, ratings and prompts.",
        url: "https://openrouter.ai/keys",
        secret: true,
    },
    EnvVar {
        key: "TELEGRAM_API_ID",
        label: "Telegram API ID",
        help: "my.telegram.org > API development tools.",
        url: "https://my.telegram.org",
        secret: false,
    },
    EnvVar {
        key: "TELEGRAM_API_HASH",
        label: "Telegram API hash",
        help: "Shown next to the API ID.",
        url: "https://my.telegram.org",
        secret: true,
    },
    EnvVar {
        key: "TELEGRAM_PASSWORD",
        label: "Telegram 2FA password",
        help: "Optional, otherwise asked in the login window.",
        url: "",
        secret: true,
    },
    EnvVar {
        key: "HF_TOKEN",
        label: "Hugging Face token",
        help: "Only for gated models (FLUX.2 klein): accept the license on the model page first.",
        url: "https://huggingface.co/settings/tokens",
        secret: true,
    },
];

pub fn var(key: &str) -> Option<&'static EnvVar> {
    VARS.iter().find(|v| v.key == key)
}

/// The repo's `.env`, edited in place so comments and unknown keys survive.
pub struct EnvFile {
    path: PathBuf,
    lines: Vec<String>,
    values: HashMap<String, String>,
    mtime: Option<SystemTime>,
}

impl EnvFile {
    pub fn load(path: &Path) -> Self {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let lines: Vec<String> = text.lines().map(str::to_owned).collect();
        let values = lines.iter().filter_map(|l| parse(l)).collect();
        Self { path: path.to_path_buf(), lines, values, mtime: mtime(path) }
    }

    /// Value from `.env`, else from the process environment (the scripts let real env vars win too).
    pub fn get(&self, key: &str) -> String {
        std::env::var(key).ok().filter(|v| !v.is_empty()).or_else(|| self.values.get(key).cloned()).unwrap_or_default()
    }

    pub fn is_set(&self, key: &str) -> bool {
        !self.get(key).trim().is_empty()
    }

    pub fn set(&mut self, key: &str, value: &str) {
        let value = value.trim();
        let quoted = if value.contains('#') || value.contains(' ') { format!("\"{value}\"") } else { value.to_owned() };
        let line = format!("{key}={quoted}");
        match self.lines.iter_mut().find(|l| parse(l).is_some_and(|(k, _)| k == key)) {
            Some(l) => *l = line,
            None => self.lines.push(line),
        }
        self.values.insert(key.to_owned(), value.to_owned());
    }

    pub fn save(&mut self) -> std::io::Result<()> {
        if !self.path.exists() {
            // start from the committed template (its comments explain the keys), without its sample values
            if let Ok(t) = std::fs::read_to_string(self.path.with_file_name(".env.example")) {
                self.lines = t.lines().map(|l| parse(l).map_or_else(|| l.to_owned(), |(k, _)| format!("{k}="))).collect();
                let values: Vec<(String, String)> = self.values.drain().collect();
                for (k, v) in values {
                    self.set(&k, &v);
                }
            }
        }
        let mut text = self.lines.join("\n");
        text.push('\n');
        std::fs::write(&self.path, text)?;
        self.mtime = mtime(&self.path);
        Ok(())
    }

    /// Picks up edits made outside the GUI (cheap: one metadata call).
    pub fn reload_if_changed(&mut self) -> bool {
        if mtime(&self.path) != self.mtime {
            *self = Self::load(&self.path);
            true
        } else {
            false
        }
    }
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn parse(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.starts_with('#') {
        return None;
    }
    let (k, v) = line.split_once('=')?;
    let v = v.trim();
    let v = v
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(v);
    Some((k.trim().to_owned(), v.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_in_place_and_creates_from_template() {
        let dir = std::env::temp_dir().join(format!("llm-gui-env-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".env.example"), "# telegram\nTELEGRAM_API_ID=1234567\nOPENROUTER_API_KEY=\n").unwrap();
        let path = dir.join(".env");

        let mut env = EnvFile::load(&path);
        assert!(!env.is_set("LOCAL_LLM_TEST_KEY"));
        env.set("OPENROUTER_API_KEY", "sk-test");
        env.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# telegram"), "template comments kept");
        assert!(text.contains("TELEGRAM_API_ID=\n"), "template sample values are not taken over");
        assert!(text.contains("OPENROUTER_API_KEY=sk-test"));

        let mut env = EnvFile::load(&path);
        env.set("LOCAL_LLM_TEST_KEY", "a b");
        env.save().unwrap();
        let env = EnvFile::load(&path);
        assert_eq!(env.get("LOCAL_LLM_TEST_KEY"), "a b");
        assert_eq!(env.get("OPENROUTER_API_KEY"), "sk-test");
        assert_eq!(std::fs::read_to_string(&path).unwrap().matches("OPENROUTER_API_KEY").count(), 1);
    }
}
