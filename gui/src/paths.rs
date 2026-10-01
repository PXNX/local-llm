use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// `.env` key of the one folder that holds every model (ComfyUI, Ollama, Hugging Face, torch, rembg, Kokoro).
pub const MODELS_KEY: &str = "MODELS_DIR";

/// Locations inside a local-llm checkout.
#[derive(Clone)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    pub fn python(&self) -> PathBuf {
        self.root.join("ComfyUI_windows_portable").join("python_embeded").join("python.exe")
    }

    pub fn comfy(&self) -> PathBuf {
        self.root.join("ComfyUI_windows_portable").join("ComfyUI")
    }

    pub fn default_models(&self) -> PathBuf {
        self.root.join("models")
    }

    /// ComfyUI's own models folder: where downloads went before the one models folder.
    pub fn comfy_models(&self) -> PathBuf {
        self.comfy().join("models")
    }

    /// The models folder from `MODELS_DIR` (relative = inside the checkout), else `<repo>/models`.
    pub fn models(&self, value: &str) -> PathBuf {
        match value.trim() {
            "" => self.default_models(),
            v => self.root.join(v),
        }
    }

    pub fn env_file(&self) -> PathBuf {
        self.root.join(".env")
    }

    pub fn script(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }
}

/// Subfolders of the models folder for the tools that are not ComfyUI (whose folders sit directly in it).
pub const OLLAMA: &str = "ollama";
pub const HUGGINGFACE: &str = "huggingface";
pub const TORCH: &str = "torch";
pub const U2NET: &str = "u2net";

/// Env vars that point every tool at the models folder (the same as common/models-env.bat and common/storage.py).
pub fn models_env(dir: &Path) -> Vec<(&'static str, PathBuf)> {
    vec![
        (MODELS_KEY, dir.to_path_buf()),
        ("OLLAMA_MODELS", dir.join(OLLAMA)),
        ("HF_HOME", dir.join(HUGGINGFACE)),
        ("TORCH_HOME", dir.join(TORCH)),
        ("U2NET_HOME", dir.join(U2NET)),
    ]
}

static CURRENT: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Sets the models folder that servers and flows started from now on get.
pub fn set_models(dir: &Path) {
    *CURRENT.lock().unwrap() = Some(dir.to_path_buf());
}

pub fn current_env() -> Vec<(&'static str, PathBuf)> {
    CURRENT.lock().unwrap().as_deref().map(models_env).unwrap_or_default()
}

/// A folder where a tool kept its models before, and where they belong inside the models folder.
#[derive(Clone, Debug, PartialEq)]
pub struct OldStore {
    pub label: &'static str,
    pub from: PathBuf,
    pub to: PathBuf,
}

/// Case-insensitive on Windows: env vars and dialogs do not agree on drive letter case.
fn inside(p: &Path, dir: &Path) -> bool {
    let norm = |p: &Path| p.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect::<Vec<_>>();
    norm(p).starts_with(&norm(dir))
}

/// Folders the models were in before the models folder was changed (same layout) or set by an older GUI version.
#[derive(Clone, Copy, Default)]
pub struct Previous<'a> {
    pub models: Option<&'a Path>,
    pub ollama: Option<&'a Path>,
}

/// The old stores that exist and are not the models folder already. `home` is the user profile,
/// `var` reads the environment (injected for tests).
pub fn old_stores(paths: &Paths, dir: &Path, home: &Path, var: impl Fn(&str) -> Option<PathBuf>, prev: Previous) -> Vec<OldStore> {
    let store = |label, from: PathBuf, to: PathBuf| OldStore { label, from, to };
    let mut out = vec![
        store("ComfyUI", paths.comfy_models(), dir.to_path_buf()),
        store("Kokoro", paths.root.join("comedy").join("models"), dir.join("kokoro")),
    ];
    if let Some(m) = prev.models {
        out.push(store("previous models folder", m.to_path_buf(), dir.to_path_buf()));
    }
    let ollama = [Some(home.join(".ollama").join("models")), var("OLLAMA_MODELS"), prev.ollama.map(Path::to_path_buf)];
    for o in ollama.into_iter().flatten() {
        for sub in ["blobs", "manifests"] {
            out.push(store("Ollama", o.join(sub), dir.join(OLLAMA).join(sub)));
        }
    }
    let cache = home.join(".cache");
    // only the model stores of the shared caches: their tokens and settings stay for other programs
    out.push(store(
        "Hugging Face",
        var("HF_HOME").unwrap_or_else(|| cache.join("huggingface")).join("hub"),
        dir.join(HUGGINGFACE).join("hub"),
    ));
    out.push(store("torch", var("TORCH_HOME").unwrap_or_else(|| cache.join("torch")).join("hub"), dir.join(TORCH).join("hub")));
    out.push(store("rembg", var("U2NET_HOME").unwrap_or_else(|| home.join(".u2net")), dir.join(U2NET)));
    let mut seen: Vec<PathBuf> = Vec::new();
    out.retain(|s| {
        let keep = s.from.is_dir() && !inside(&s.from, dir) && !inside(dir, &s.from) && !seen.iter().any(|p| inside(&s.from, p));
        seen.push(s.from.clone());
        keep
    });
    out
}

pub fn is_repo(p: &Path) -> bool {
    p.join("stickers").join("make_stickers.py").is_file() && p.join("common").join("llm.py").is_file()
}

/// The configured folder, else the first ancestor of the exe or the working directory that is a checkout.
pub fn find_repo(configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = configured.filter(|p| is_repo(p)) {
        return Some(p.to_path_buf());
    }
    let starts = [std::env::current_exe().ok(), std::env::current_dir().ok()];
    starts.into_iter().flatten().find_map(|s| s.ancestors().find(|a| is_repo(a)).map(Path::to_path_buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("llm-gui-paths-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn models_folder_and_env() {
        let p = Paths { root: PathBuf::from("C:/repo") };
        assert_eq!(p.models(""), PathBuf::from("C:/repo/models"));
        assert_eq!(p.models(" big "), PathBuf::from("C:/repo/big"));
        assert_eq!(p.models("D:/ai"), PathBuf::from("D:/ai"));
        let env = models_env(Path::new("D:/ai"));
        assert!(env.contains(&("OLLAMA_MODELS", PathBuf::from("D:/ai/ollama"))));
        assert!(env.contains(&("HF_HOME", PathBuf::from("D:/ai/huggingface"))));
        assert!(env.contains(&("TORCH_HOME", PathBuf::from("D:/ai/torch"))));
        assert!(env.contains(&("U2NET_HOME", PathBuf::from("D:/ai/u2net"))));
    }

    #[test]
    fn finds_old_stores() {
        let root = tmp("repo");
        let home = tmp("home");
        let paths = Paths { root: root.clone() };
        let dir = root.join("models");
        for d in [
            paths.comfy_models(),
            root.join("comedy/models"),
            home.join(".ollama/models/blobs"),
            home.join(".cache/huggingface/hub"),
            home.join(".u2net"),
            dir.join("ollama/manifests"),
        ] {
            std::fs::create_dir_all(d).unwrap();
        }
        // OLLAMA_MODELS already pointing into the models folder is no old store
        let var = |k: &str| (k == "OLLAMA_MODELS").then(|| dir.join("ollama"));
        let found = old_stores(&paths, &dir, &home, var, Previous::default());
        let pairs: Vec<(&str, PathBuf, PathBuf)> = found.iter().map(|s| (s.label, s.from.clone(), s.to.clone())).collect();
        assert_eq!(
            pairs,
            vec![
                ("ComfyUI", paths.comfy_models(), dir.clone()),
                ("Kokoro", root.join("comedy/models"), dir.join("kokoro")),
                ("Ollama", home.join(".ollama/models/blobs"), dir.join("ollama/blobs")),
                ("Hugging Face", home.join(".cache/huggingface/hub"), dir.join("huggingface/hub")),
                ("rembg", home.join(".u2net"), dir.join("u2net")),
            ]
        );
        // the models folder set to ComfyUI's own folder: that one is not moved
        let found = old_stores(&paths, &paths.comfy_models(), &home, |_| None, Previous::default());
        assert!(found.iter().all(|s| s.label != "ComfyUI"));
        // a changed models folder: the old one moves as a whole, unless the new one is inside it
        let new = home.join("big");
        let prev = Previous { models: Some(&dir), ollama: None };
        assert!(old_stores(&paths, &new, &home, |_| None, prev).iter().any(|s| s.from == dir && s.to == new));
        assert!(old_stores(&paths, &dir.join("sub"), &home, |_| None, prev).iter().all(|s| s.from != dir));
    }
}
