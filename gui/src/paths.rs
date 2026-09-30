use std::path::{Path, PathBuf};

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
        self.comfy().join("models")
    }

    pub fn env_file(&self) -> PathBuf {
        self.root.join(".env")
    }

    pub fn script(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }
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
