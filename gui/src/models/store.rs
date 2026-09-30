//! On-disk helpers: Ollama's store, file identity, hashing, moving and ComfyUI's extra model paths.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use sha2::{Digest, Sha256};

/// A model in Ollama's store, read from its manifest files (works while Ollama is not running).
pub struct OllamaInstalled {
    /// sha256 of the GGUF weights layer.
    pub weights: Option<String>,
    pub complete: bool,
}

/// `name:tag` like `ollama list` prints it.
fn model_name(parts: &[String]) -> Option<String> {
    let (tag, rest) = parts.split_last()?;
    let name = match rest {
        [registry, ns, model] if registry == "registry.ollama.ai" && ns == "library" => model.clone(),
        [registry, rest @ ..] if registry == "registry.ollama.ai" => rest.join("/"),
        all => all.join("/"),
    };
    Some(format!("{name}:{tag}"))
}

pub fn scan_ollama(dir: &Path) -> HashMap<String, OllamaInstalled> {
    let mut out = HashMap::new();
    let manifests = dir.join("manifests");
    let mut stack = vec![manifests.clone()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Ok(rel) = path.strip_prefix(&manifests) else { continue };
            let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            let Some(name) = model_name(&parts) else { continue };
            let Ok(v) = fs::read_to_string(&path).map(|t| serde_json::from_str::<serde_json::Value>(&t).unwrap_or_default()) else {
                continue;
            };
            let layers = v["layers"].as_array().cloned().unwrap_or_default();
            let blob = |digest: &str| dir.join("blobs").join(digest.replace(':', "-"));
            let complete = !layers.is_empty() && layers.iter().all(|l| l["digest"].as_str().is_some_and(|d| blob(d).is_file()));
            let weights = layers
                .iter()
                .find(|l| l["mediaType"] == "application/vnd.ollama.image.model")
                .and_then(|l| l["digest"].as_str())
                .and_then(|d| d.strip_prefix("sha256:"))
                .map(str::to_owned);
            out.insert(name, OllamaInstalled { weights, complete });
        }
    }
    out
}

pub fn blob_path(ollama_dir: &Path, sha256: &str) -> PathBuf {
    ollama_dir.join("blobs").join(format!("sha256-{sha256}"))
}

/// True when both paths are the same file on disk (hard links), not just equal content.
#[cfg(windows)]
pub fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle};
    let id = |p: &Path| -> Option<(u32, u32, u32)> {
        let f = File::open(p).ok()?;
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        (unsafe { GetFileInformationByHandle(f.as_raw_handle() as _, &mut info) } != 0).then_some((
            info.dwVolumeSerialNumber,
            info.nFileIndexHigh,
            info.nFileIndexLow,
        ))
    };
    matches!((id(a), id(b)), (Some(x), Some(y)) if x == y)
}

#[cfg(not(windows))]
pub fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    matches!((fs::metadata(a), fs::metadata(b)), (Ok(x), Ok(y)) if x.dev() == y.dev() && x.ino() == y.ino())
}

pub fn sha256_file(p: &Path, done: &AtomicU64, stop: &AtomicBool) -> Result<String, String> {
    let mut f = File::open(p).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 4 << 20];
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        done.fetch_add(n as u64, Ordering::Relaxed);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Replaces `copy` by a hard link to `keep` (same bytes, one copy on disk).
pub fn link_over(keep: &Path, copy: &Path) -> Result<(), String> {
    let mut tmp = copy.as_os_str().to_owned();
    tmp.push(".link");
    let tmp = PathBuf::from(tmp);
    let _ = fs::remove_file(&tmp);
    fs::hard_link(keep, &tmp).map_err(|e| {
        if e.raw_os_error() == Some(17) {
            "the files are on different drives - use one models folder for both (Settings) first".to_owned()
        } else {
            e.to_string()
        }
    })?;
    fs::rename(&tmp, copy).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })
}

/// Every file below `dir` (for moving a whole store).
pub fn files_below(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() { stack.push(p) } else { out.push(p) }
        }
    }
    out
}

/// Rename (instant on the same drive), else copy + delete. `done` counts bytes of finished work.
pub fn move_file(src: &Path, dst: &Path, done: &AtomicU64, stop: &AtomicBool) -> Result<(), String> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let len = fs::metadata(src).map_err(|e| e.to_string())?.len();
    if let Ok(m) = fs::metadata(dst) {
        if m.len() == len {
            // already there (e.g. an earlier, interrupted move): drop the second copy
            fs::remove_file(src).map_err(|e| e.to_string())?;
            done.fetch_add(len, Ordering::Relaxed);
            return Ok(());
        }
        return Err(format!("{} already exists with another size", dst.display()));
    }
    match fs::rename(src, dst) {
        Ok(()) => {
            done.fetch_add(len, Ordering::Relaxed);
            Ok(())
        }
        Err(e) if e.raw_os_error() == Some(17) => {
            let part = dst.with_extension("moving");
            let copied = (|| -> Result<(), String> {
                let mut r = File::open(src).map_err(|e| e.to_string())?;
                let mut w = File::create(&part).map_err(|e| e.to_string())?;
                let mut buf = vec![0u8; 8 << 20];
                loop {
                    if stop.load(Ordering::Relaxed) {
                        return Err("cancelled".into());
                    }
                    let n = r.read(&mut buf).map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    w.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                    done.fetch_add(n as u64, Ordering::Relaxed);
                }
                w.sync_all().map_err(|e| e.to_string())
            })();
            if let Err(e) = copied {
                let _ = fs::remove_file(&part);
                return Err(e);
            }
            fs::rename(&part, dst).map_err(|e| e.to_string())?;
            fs::remove_file(src).map_err(|e| e.to_string())
        }
        Err(e) => Err(format!("{}: {e} (is ComfyUI/Ollama still using it?)", src.display())),
    }
}

const MARK: &str = "# written by the local-llm GUI";

/// Makes ComfyUI also load models from a custom folder (ComfyUI/extra_model_paths.yaml).
pub fn write_extra_paths(comfy: &Path, models_dir: &Path) -> Result<(), String> {
    let file = comfy.join("extra_model_paths.yaml");
    if let Ok(existing) = fs::read_to_string(&file)
        && !existing.starts_with(MARK)
    {
        return Err(format!("{} exists and was not written by this app - add the folder there by hand", file.display()));
    }
    let base = models_dir.display().to_string().replace('\\', "/");
    let dirs = [
        "checkpoints",
        "diffusion_models",
        "unet",
        "text_encoders",
        "clip",
        "vae",
        "loras",
        "photomaker",
        "upscale_models",
        "controlnet",
        "clip_vision",
    ];
    let mut yaml = format!("{MARK}\nlocal_llm:\n    base_path: \"{base}\"\n");
    for d in dirs {
        yaml.push_str(&format!("    {d}: {d}\n"));
    }
    fs::write(&file, yaml).map_err(|e| e.to_string())
}

pub fn remove_extra_paths(comfy: &Path) {
    let file = comfy.join("extra_model_paths.yaml");
    if fs::read_to_string(&file).is_ok_and(|t| t.starts_with(MARK)) {
        let _ = fs::remove_file(file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("llm-gui-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn ollama_names() {
        let n = |p: &[&str]| model_name(&p.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
        assert_eq!(n(&["registry.ollama.ai", "library", "qwen3-vl", "4b"]), "qwen3-vl:4b");
        assert_eq!(n(&["registry.ollama.ai", "someone", "model", "latest"]), "someone/model:latest");
        assert_eq!(n(&["hf.co", "JetBrains", "Qwen3.8-3.6-27B-blend-GGUF", "IQ3_S"]), "hf.co/JetBrains/Qwen3.8-3.6-27B-blend-GGUF:IQ3_S");
    }

    #[test]
    fn reads_an_ollama_store() {
        let d = tmp("ollama");
        let m = d.join("manifests/registry.ollama.ai/library/tiny/1b");
        fs::create_dir_all(m.parent().unwrap()).unwrap();
        fs::write(&m, r#"{"layers":[{"mediaType":"application/vnd.ollama.image.model","digest":"sha256:abc"},{"mediaType":"x","digest":"sha256:def"}]}"#).unwrap();
        fs::create_dir_all(d.join("blobs")).unwrap();
        fs::write(d.join("blobs/sha256-abc"), b"w").unwrap();
        let s = scan_ollama(&d);
        assert_eq!(s["tiny:1b"].weights.as_deref(), Some("abc"));
        assert!(!s["tiny:1b"].complete, "a layer blob is missing");
        fs::write(d.join("blobs/sha256-def"), b"x").unwrap();
        assert!(scan_ollama(&d)["tiny:1b"].complete);
    }

    #[test]
    fn hashing_linking_and_moving() {
        let d = tmp("link");
        let (a, b) = (d.join("a.bin"), d.join("b.bin"));
        fs::write(&a, b"hello").unwrap();
        fs::write(&b, b"hello").unwrap();
        let (done, stop) = (AtomicU64::new(0), AtomicBool::new(false));
        assert_eq!(sha256_file(&a, &done, &stop).unwrap(), "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
        assert!(!same_file(&a, &b));
        link_over(&a, &b).unwrap();
        assert!(same_file(&a, &b));
        let c = d.join("sub/c.bin");
        move_file(&b, &c, &done, &stop).unwrap();
        assert!(!b.exists() && same_file(&a, &c));
    }
}
