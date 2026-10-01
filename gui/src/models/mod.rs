//! Model files for ComfyUI and Ollama: what is on disk, what is missing, and a download queue.
//!
//! Nothing is downloaded or stored twice:
//! - a finished file gets a `<file>.done` marker, the same convention as download-models.sh;
//! - a file already on disk under its upstream name or in another models folder is hard-linked;
//! - ComfyUI files and Ollama blobs with the same sha256 are hard-linked into one copy.
//!
//! Everything lives in one models folder (`paths::models_env`); files the tools stored elsewhere before
//! are offered to be moved into it.

mod download;
mod store;

use std::collections::{HashMap, VecDeque};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use eframe::egui;
use serde::Deserialize;

use crate::paths::OldStore;

pub use store::remove_extra_paths;

const EMBEDDED: &str = include_str!("../../../models.json");

#[derive(Deserialize, Clone)]
pub struct ComfyModel {
    pub id: String,
    pub group: String,
    pub used_by: String,
    pub dir: String,
    pub file: String,
    pub size: Option<u64>,
    pub url: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub gated: bool,
    #[serde(default)]
    pub note: String,
}

impl ComfyModel {
    /// File name on the server, when it differs from the name the workflows load.
    fn upstream_name(&self) -> Option<&str> {
        let name = self.url.as_deref()?.rsplit('/').next()?;
        (name != self.file).then_some(name)
    }
}

#[derive(Deserialize, Clone)]
pub struct OllamaModel {
    pub model: String,
    pub group: String,
    pub used_by: String,
    pub size: Option<u64>,
}

/// A model file that is neither a ComfyUI weight nor an Ollama model - e.g. the comedy flow's
/// Kokoro TTS voices - kept in a subfolder `dir` of the models folder.
#[derive(Deserialize, Clone)]
pub struct LocalModel {
    pub id: String,
    pub group: String,
    pub used_by: String,
    pub dir: String,
    pub file: String,
    pub size: Option<u64>,
    pub url: Option<String>,
    /// Repo-relative folder of older versions, still used until the file is moved.
    #[serde(default)]
    pub legacy_dir: Option<String>,
}

#[derive(Deserialize)]
struct Manifest {
    comfy: Vec<ComfyModel>,
    ollama: Vec<OllamaModel>,
    #[serde(default)]
    local: Vec<LocalModel>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Unknown,
    /// Not on disk; `partial` bytes of an interrupted download are kept in `<file>.part`.
    Missing {
        partial: u64,
    },
    /// Same bytes exist elsewhere (other name, other folder, Ollama blob): hard-linked, no download.
    Linkable(PathBuf),
    Present(PathBuf),
    /// On disk without a `.done` marker and with an unexpected size: unfinished download or another variant.
    SizeDiffers {
        path: PathBuf,
        size: u64,
    },
    /// No public download: copy it in by hand.
    Manual,
}

impl Status {
    pub fn usable(&self) -> bool {
        matches!(self, Status::Present(_) | Status::SizeDiffers { .. })
    }
}

/// How a model relates to the other tool's copy of the same bytes (sha256 match).
#[derive(Clone, Debug, PartialEq)]
pub enum Share {
    /// One copy on disk, used by both ComfyUI and Ollama.
    Shared(String),
    /// Two copies of the same bytes: "share" turns them into one.
    Duplicate { with: String, bytes: u64 },
}

#[derive(Clone, Debug)]
pub enum Dl {
    Queued,
    Running { done: u64, total: Option<u64>, rate: f64, verb: &'static str },
    Failed(String),
}

#[derive(Clone, Copy, PartialEq)]
pub enum Task {
    Download,
    /// Re-download over a file whose size is off.
    Replace,
    /// Verify a duplicate's sha256 and hard-link it to the other tool's copy.
    Share,
}

/// A file of an old store and its place in the models folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Move {
    pub label: &'static str,
    pub from: PathBuf,
    pub to: PathBuf,
    pub size: u64,
}

#[derive(Clone, Default)]
pub struct Moving {
    pub done: u64,
    pub total: u64,
    pub file: String,
    pub errors: Vec<String>,
    pub finished: bool,
}

#[derive(Default)]
struct Shared {
    status: HashMap<String, Status>,
    share: HashMap<String, Share>,
    dl: HashMap<String, (Dl, Arc<AtomicBool>)>,
    roots: Vec<PathBuf>,
    ollama_dir: PathBuf,
    local_dir: PathBuf,
    old: Vec<OldStore>,
    plan: Vec<Move>,
    token: String,
    scan_again: bool,
    scanning: bool,
    moving: Option<Moving>,
}

struct Queue {
    items: Mutex<VecDeque<(String, Task)>>,
    ready: Condvar,
}

#[derive(Clone)]
pub struct Models {
    pub comfy: Arc<Vec<ComfyModel>>,
    pub ollama: Arc<Vec<OllamaModel>>,
    pub local: Arc<Vec<LocalModel>>,
    repo: Arc<PathBuf>,
    shared: Arc<Mutex<Shared>>,
    queue: Arc<Queue>,
    ctx: egui::Context,
}

pub fn ollama_id(model: &str) -> String {
    format!("ollama:{model}")
}

impl Models {
    pub fn new(ctx: &egui::Context, repo: &Path) -> Self {
        let text = fs::read_to_string(repo.join("models.json")).unwrap_or_else(|_| EMBEDDED.to_owned());
        let manifest: Manifest =
            serde_json::from_str(&text).or_else(|_| serde_json::from_str(EMBEDDED)).expect("embedded models.json is valid");
        let m = Self {
            comfy: Arc::new(manifest.comfy),
            ollama: Arc::new(manifest.ollama),
            local: Arc::new(manifest.local),
            repo: Arc::new(repo.to_path_buf()),
            shared: Arc::default(),
            queue: Arc::new(Queue { items: Mutex::default(), ready: Condvar::new() }),
            ctx: ctx.clone(),
        };
        let worker = m.clone();
        std::thread::Builder::new().name("downloads".into()).spawn(move || worker.worker()).expect("spawn download worker");
        m
    }

    /// `roots[0]` (the models folder) receives new ComfyUI downloads; all roots are searched so nothing
    /// is fetched twice. Local models go below `local_dir`, `old` stores are offered to be moved.
    pub fn configure(&self, roots: Vec<PathBuf>, ollama_dir: PathBuf, local_dir: PathBuf, old: Vec<OldStore>, token: String) {
        let mut s = self.shared.lock().unwrap();
        s.roots = roots;
        s.ollama_dir = ollama_dir;
        s.local_dir = local_dir;
        s.old = old;
        s.token = token;
    }

    pub fn target_root(&self) -> Option<PathBuf> {
        self.shared.lock().unwrap().roots.first().cloned()
    }

    pub fn status(&self, id: &str) -> Status {
        self.shared.lock().unwrap().status.get(id).cloned().unwrap_or(Status::Unknown)
    }

    pub fn share(&self, id: &str) -> Option<Share> {
        self.shared.lock().unwrap().share.get(id).cloned()
    }

    pub fn download(&self, id: &str) -> Option<Dl> {
        self.shared.lock().unwrap().dl.get(id).map(|(d, _)| d.clone())
    }

    pub fn busy(&self) -> bool {
        let s = self.shared.lock().unwrap();
        s.dl.values().any(|(d, _)| !matches!(d, Dl::Failed(_))) || s.moving.as_ref().is_some_and(|m| !m.finished)
    }

    pub fn comfy_model(&self, id: &str) -> Option<&ComfyModel> {
        self.comfy.iter().find(|m| m.id == id)
    }

    pub fn local_model(&self, id: &str) -> Option<&LocalModel> {
        self.local.iter().find(|m| m.id == id)
    }

    pub fn missing_bytes(&self, ids: &[String]) -> u64 {
        ids.iter()
            .filter(|id| matches!(self.status(id), Status::Missing { .. } | Status::Unknown))
            .map(|id| self.size_of(id).unwrap_or(0))
            .sum()
    }

    /// Disk space "share" would free.
    pub fn duplicate_bytes(&self) -> u64 {
        self.shared
            .lock()
            .unwrap()
            .share
            .iter()
            .filter(|(id, _)| !id.starts_with("ollama:"))
            .map(|(_, s)| if let Share::Duplicate { bytes, .. } = s { *bytes } else { 0 })
            .sum()
    }

    pub fn size_of(&self, id: &str) -> Option<u64> {
        match id.strip_prefix("ollama:") {
            Some(name) => self.ollama.iter().find(|m| m.model == name).and_then(|m| m.size),
            None => self.comfy_model(id).and_then(|m| m.size).or_else(|| self.local_model(id).and_then(|m| m.size)),
        }
    }

    pub fn needs_token(&self, ids: &[String]) -> bool {
        ids.iter().any(|id| self.comfy_model(id).is_some_and(|m| m.gated) && matches!(self.status(id), Status::Missing { .. }))
    }

    /// Queues work for a model unless it is already queued/running.
    pub fn enqueue(&self, id: &str, task: Task) {
        {
            let mut s = self.shared.lock().unwrap();
            if s.dl.get(id).is_some_and(|(d, _)| !matches!(d, Dl::Failed(_))) {
                return;
            }
            s.dl.insert(id.to_owned(), (Dl::Queued, Arc::default()));
        }
        self.queue.items.lock().unwrap().push_back((id.to_owned(), task));
        self.queue.ready.notify_one();
        self.ctx.request_repaint();
    }

    pub fn cancel(&self, id: &str) {
        if let Some((_, stop)) = self.shared.lock().unwrap().dl.remove(id) {
            stop.store(true, Ordering::Relaxed);
        }
        self.ctx.request_repaint();
    }

    pub fn dismiss(&self, id: &str) {
        self.shared.lock().unwrap().dl.remove(id);
    }

    /// Re-reads what is on disk in a background thread (coalesces overlapping requests).
    pub fn rescan(&self) {
        {
            let mut s = self.shared.lock().unwrap();
            if s.scanning {
                s.scan_again = true;
                return;
            }
            s.scanning = true;
        }
        let m = self.clone();
        std::thread::spawn(move || {
            loop {
                let (roots, odir, local, old, tracked) = {
                    let s = m.shared.lock().unwrap();
                    let tracked: Vec<String> = s.status.keys().filter(|k| k.starts_with("ollama:")).cloned().collect();
                    (s.roots.clone(), s.ollama_dir.clone(), s.local_dir.clone(), s.old.clone(), tracked)
                };
                let (status, share) = m.scan(&roots, &odir, &local, tracked);
                let plan = plan_moves(&old);
                let mut s = m.shared.lock().unwrap();
                s.status = status;
                s.share = share;
                s.plan = plan;
                if !std::mem::take(&mut s.scan_again) {
                    s.scanning = false;
                    break;
                }
            }
            m.ctx.request_repaint();
        });
    }

    fn scan(
        &self,
        roots: &[PathBuf],
        odir: &Path,
        local: &Path,
        tracked: Vec<String>,
    ) -> (HashMap<String, Status>, HashMap<String, Share>) {
        let installed = store::scan_ollama(odir);
        let owner = |sha: &str| installed.iter().find(|(_, o)| o.weights.as_deref() == Some(sha)).map(|(n, _)| n.clone());
        let mut status = HashMap::new();
        let mut share = HashMap::new();
        for c in self.comfy.iter() {
            let mut st = scan_comfy(c, roots);
            if let Some(sha) = &c.sha256 {
                let blob = store::blob_path(odir, sha);
                if blob.is_file() {
                    let with = owner(sha).unwrap_or_else(|| "an Ollama model".into());
                    match &st {
                        Status::Missing { .. } => st = Status::Linkable(blob),
                        Status::Present(p) => {
                            let s = if store::same_file(p, &blob) {
                                Share::Shared(format!("Ollama {with}"))
                            } else {
                                Share::Duplicate { with: format!("Ollama {with}"), bytes: c.size.unwrap_or(0) }
                            };
                            let mirrored = match &s {
                                Share::Shared(_) => Share::Shared(format!("ComfyUI {}", c.file)),
                                Share::Duplicate { bytes, .. } => Share::Duplicate { with: format!("ComfyUI {}", c.file), bytes: *bytes },
                            };
                            if let Some(name) = owner(sha) {
                                share.insert(ollama_id(&name), mirrored);
                            }
                            share.insert(c.id.clone(), s);
                        }
                        _ => {}
                    }
                }
            }
            status.insert(c.id.clone(), st);
        }
        let names = self.ollama.iter().map(|o| ollama_id(&o.model)).chain(tracked);
        for id in names {
            let name = &id["ollama:".len()..];
            let found = installed.get(name).or_else(|| installed.get(&format!("{name}:latest")));
            let st = match found {
                Some(o) if o.complete => Status::Present(odir.join("manifests")),
                _ => Status::Missing { partial: 0 },
            };
            status.insert(id, st);
        }
        for m in self.local.iter() {
            status.insert(m.id.clone(), scan_local(m, local, &self.repo));
        }
        (status, share)
    }

    /// Makes sure a model id outside the manifest (custom OLLAMA_MODEL) is tracked.
    pub fn track(&self, id: &str) {
        let mut s = self.shared.lock().unwrap();
        if !s.status.contains_key(id) {
            s.status.insert(id.to_owned(), Status::Unknown);
            drop(s);
            self.rescan();
        }
    }

    // ------------------------------------------------------------ moving into one folder
    pub fn moving(&self) -> Option<Moving> {
        self.shared.lock().unwrap().moving.clone()
    }

    /// Files still in the old stores (found by the last scan).
    pub fn move_plan(&self) -> Vec<Move> {
        self.shared.lock().unwrap().plan.clone()
    }

    pub fn start_move(&self, plan: Vec<Move>) {
        let total = plan.iter().map(|m| m.size).sum();
        self.shared.lock().unwrap().moving = Some(Moving { total, ..Default::default() });
        let m = self.clone();
        std::thread::spawn(move || {
            let (done, stop) = (AtomicU64::new(0), AtomicBool::new(false));
            for Move { from, to, .. } in plan {
                let name = from.file_name().unwrap_or_default().to_string_lossy().into_owned();
                if let Some(mv) = &mut m.shared.lock().unwrap().moving {
                    mv.file = name.clone();
                }
                m.ctx.request_repaint();
                // `.done` markers are files of the store too and move along
                let result = store::move_file(&from, &to, &done, &stop);
                let mut s = m.shared.lock().unwrap();
                if let Some(mv) = &mut s.moving {
                    mv.done = done.load(Ordering::Relaxed);
                    if let Err(e) = result {
                        mv.errors.push(format!("{name}: {e}"));
                    }
                }
            }
            if let Some(mv) = &mut m.shared.lock().unwrap().moving {
                mv.finished = true;
                mv.done = mv.total;
            }
            m.rescan();
        });
    }

    pub fn clear_move(&self) {
        self.shared.lock().unwrap().moving = None;
    }

    // ------------------------------------------------------------ worker
    fn worker(&self) {
        loop {
            let (id, task) = {
                let mut q = self.queue.items.lock().unwrap();
                loop {
                    if let Some(item) = q.pop_front() {
                        break item;
                    }
                    q = self.queue.ready.wait(q).unwrap();
                }
            };
            let Some(stop) = self.shared.lock().unwrap().dl.get(&id).map(|(_, s)| s.clone()) else {
                continue; // cancelled while queued
            };
            let result = match task {
                Task::Share => self.share_copies(&id, &stop),
                _ => self.fetch(&id, task == Task::Replace, &stop),
            };
            {
                let mut s = self.shared.lock().unwrap();
                match result {
                    Ok(()) => {
                        s.dl.remove(&id);
                    }
                    Err(e) if !stop.load(Ordering::Relaxed) => {
                        s.dl.insert(id.clone(), (Dl::Failed(e), stop));
                    }
                    Err(_) => {}
                }
            }
            self.rescan();
        }
    }

    fn progress(&self, id: &str, verb: &'static str) -> impl Fn(u64, Option<u64>, f64) + '_ {
        let id = id.to_owned();
        move |done, total, rate| {
            if let Some(entry) = self.shared.lock().unwrap().dl.get_mut(&id) {
                entry.0 = Dl::Running { done, total, rate, verb };
            }
            self.ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn fetch(&self, id: &str, replace: bool, stop: &Arc<AtomicBool>) -> Result<(), String> {
        if let Some(name) = id.strip_prefix("ollama:") {
            let name = name.to_owned();
            return download::supervise(None, stop, &self.progress(id, "downloading"), move |d, t, s| {
                download::ollama_pull(&name, d, t, s)
            });
        }
        if let Some(model) = self.local_model(id).cloned() {
            return self.fetch_local(&model, replace, stop);
        }
        let model = self.comfy_model(id).cloned().ok_or("unknown model")?;
        let (roots, odir, token) = {
            let s = self.shared.lock().unwrap();
            (s.roots.clone(), s.ollama_dir.clone(), s.token.clone())
        };
        let root = roots.first().cloned().ok_or("no models folder")?;
        let dir = root.join(&model.dir);
        fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let dest = dir.join(&model.file);
        if !replace {
            let blob = model.sha256.as_deref().map(|sha| store::blob_path(&odir, sha)).filter(|b| b.is_file());
            if let Some(src) = find_linkable(&model, &roots).or(blob)
                && fs::hard_link(&src, &dest).is_ok()
            {
                mark_done(&dest);
                return Ok(());
            }
        }
        let url = model.url.clone().ok_or("no download URL - copy the file in by hand")?;
        if model.gated && token.is_empty() {
            return Err("gated model: set HF_TOKEN (Settings) after accepting the license on Hugging Face".into());
        }
        // an unfinished download-models.sh file (no .done) is continued in place, otherwise use .part
        let in_place = !replace && dest.exists();
        let part = if in_place { dest.clone() } else { dir.join(format!("{}.part", model.file)) };
        if replace {
            let _ = fs::remove_file(&part);
        }
        let expected = model.size;
        let target = part.clone();
        download::supervise(expected, stop, &self.progress(id, "downloading"), move |d, t, s| {
            download::http_get(&url, &token, &target, expected, d, t, s)
        })?;
        if !in_place {
            fs::rename(&part, &dest).map_err(|e| format!("rename: {e}"))?;
        }
        mark_done(&dest);
        Ok(())
    }

    /// A file in a subfolder of the models folder (e.g. the comedy flow's Kokoro voices): no roots,
    /// no gating, no cross-tool linking.
    fn fetch_local(&self, model: &LocalModel, replace: bool, stop: &Arc<AtomicBool>) -> Result<(), String> {
        let dir = self.shared.lock().unwrap().local_dir.join(&model.dir);
        fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let dest = dir.join(&model.file);
        let url = model.url.clone().ok_or("no download URL - copy the file in by hand")?;
        let in_place = !replace && dest.exists();
        let part = if in_place { dest.clone() } else { dir.join(format!("{}.part", model.file)) };
        if replace {
            let _ = fs::remove_file(&part);
        }
        let expected = model.size;
        let target = part.clone();
        download::supervise(expected, stop, &self.progress(&model.id, "downloading"), move |d, t, s| {
            download::http_get(&url, "", &target, expected, d, t, s)
        })?;
        if !in_place {
            fs::rename(&part, &dest).map_err(|e| format!("rename: {e}"))?;
        }
        mark_done(&dest);
        Ok(())
    }

    /// Hashes the ComfyUI copy; if it really is the published file, replaces it by a link to Ollama's blob.
    fn share_copies(&self, id: &str, stop: &AtomicBool) -> Result<(), String> {
        let comfy_id = match id.strip_prefix("ollama:") {
            // an Ollama row: find the ComfyUI model with the same weights
            Some(_) => {
                let with = match self.share(id) {
                    Some(Share::Duplicate { with, .. }) => with,
                    _ => return Err("nothing to share".into()),
                };
                self.comfy.iter().find(|c| with.ends_with(&c.file)).map(|c| c.id.clone()).ok_or("no matching ComfyUI file")?
            }
            None => id.to_owned(),
        };
        let model = self.comfy_model(&comfy_id).cloned().ok_or("unknown model")?;
        let sha = model.sha256.clone().ok_or("no sha256 in models.json")?;
        let Status::Present(path) = self.status(&comfy_id) else { return Err("ComfyUI file not on disk".into()) };
        let blob = store::blob_path(&self.shared.lock().unwrap().ollama_dir, &sha);
        let done = AtomicU64::new(0);
        let total = fs::metadata(&path).map_or(0, |m| m.len());
        let progress = self.progress(id, "verifying");
        let hash = std::thread::scope(|s| {
            let h = s.spawn(|| store::sha256_file(&path, &done, stop));
            while !h.is_finished() {
                progress(done.load(Ordering::Relaxed), Some(total), 0.0);
                std::thread::sleep(Duration::from_millis(250));
            }
            h.join().unwrap_or_else(|_| Err("hash thread died".into()))
        })?;
        if hash != sha {
            return Err("the ComfyUI file differs from the published one - kept both".into());
        }
        store::link_over(&blob, &path)
    }
}

fn scan_comfy(m: &ComfyModel, roots: &[PathBuf]) -> Status {
    for root in roots {
        let path = root.join(&m.dir).join(&m.file);
        let Ok(meta) = fs::metadata(&path) else { continue };
        let size = meta.len();
        let done = done_marker(&path).exists();
        if done || m.size.is_none_or(|s| s == size) {
            if !done && m.size.is_some() {
                mark_done(&path); // same convention as download-models.sh
            }
            return Status::Present(path);
        }
        return Status::SizeDiffers { path, size };
    }
    if let Some(src) = find_linkable(m, roots) {
        return Status::Linkable(src);
    }
    if m.url.is_none() {
        return Status::Manual;
    }
    let partial = roots.first().and_then(|r| fs::metadata(r.join(&m.dir).join(format!("{}.part", m.file))).ok()).map_or(0, |m| m.len());
    Status::Missing { partial }
}

/// Like `scan_comfy`, but for a file in one subfolder of the models folder (or its old repo folder).
fn scan_local(m: &LocalModel, local: &Path, repo: &Path) -> Status {
    let dir = local.join(&m.dir);
    let old = m.legacy_dir.as_ref().map(|d| repo.join(d).join(&m.file)).filter(|p| p.is_file() && !dir.join(&m.file).is_file());
    let path = old.unwrap_or_else(|| dir.join(&m.file));
    if let Ok(meta) = fs::metadata(&path) {
        let size = meta.len();
        let done = done_marker(&path).exists();
        if done || m.size.is_none_or(|s| s == size) {
            if !done && m.size.is_some() {
                mark_done(&path);
            }
            return Status::Present(path);
        }
        return Status::SizeDiffers { path, size };
    }
    if m.url.is_none() {
        return Status::Manual;
    }
    let partial = fs::metadata(dir.join(format!("{}.part", m.file))).map_or(0, |m| m.len());
    Status::Missing { partial }
}

/// Every file of the old stores with its place in the models folder (not ComfyUI's empty `put_..._here` placeholders).
fn plan_moves(old: &[OldStore]) -> Vec<Move> {
    let placeholder = |f: &Path| f.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("put_") && n.ends_with("_here"));
    let mut plan = Vec::new();
    for s in old {
        for f in store::files_below(&s.from).into_iter().filter(|f| !placeholder(f)) {
            let to = s.to.join(f.strip_prefix(&s.from).expect("below"));
            let size = fs::metadata(&f).map_or(0, |m| m.len());
            plan.push(Move { label: s.label, from: f, to, size });
        }
    }
    plan
}

/// The same file under its upstream name, or in another models folder, with the right size.
fn find_linkable(m: &ComfyModel, roots: &[PathBuf]) -> Option<PathBuf> {
    let names: Vec<&str> = std::iter::once(m.file.as_str()).chain(m.upstream_name()).collect();
    roots
        .iter()
        .flat_map(|r| names.iter().map(move |n| r.join(&m.dir).join(n)))
        .filter(|p| roots.first().is_none_or(|t| *p != t.join(&m.dir).join(&m.file)))
        .find(|p| fs::metadata(p).is_ok_and(|meta| m.size.is_none_or(|s| s == meta.len())))
}

fn done_marker(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".done");
    PathBuf::from(s)
}

fn mark_done(p: &Path) {
    let _ = File::create(done_marker(p));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("llm-gui-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn model(file: &str, url: Option<&str>, size: Option<u64>) -> ComfyModel {
        ComfyModel {
            id: file.into(),
            group: String::new(),
            used_by: String::new(),
            dir: "vae".into(),
            file: file.into(),
            size,
            url: url.map(str::to_owned),
            sha256: None,
            gated: false,
            note: String::new(),
        }
    }

    fn write(p: &Path, len: usize) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![7u8; len]).unwrap();
    }

    #[test]
    fn manifest_parses_and_ids_are_unique() {
        let m: Manifest = serde_json::from_str(EMBEDDED).unwrap();
        let mut ids: Vec<&str> = m.comfy.iter().map(|c| c.id.as_str()).chain(m.local.iter().map(|c| c.id.as_str())).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "comfy and local ids must not collide (fetch()/scan() key on them together)");
        assert!(m.comfy.iter().all(|c| c.url.is_some() || !c.note.is_empty()), "manual models need a note");
        assert!(m.comfy.iter().filter_map(|c| c.sha256.as_deref()).all(|s| s.len() == 64));
        assert!(m.local.iter().all(|c| c.url.is_some()), "local models need a download URL");
    }

    #[test]
    fn scan_states() {
        let root = tmp("scan");
        let roots = vec![root.clone()];
        let url = Some("https://x/upstream.safetensors");

        assert_eq!(scan_comfy(&model("a.bin", url, Some(10)), &roots), Status::Missing { partial: 0 });

        write(&root.join("vae/a.bin.part"), 4);
        assert_eq!(scan_comfy(&model("a.bin", url, Some(10)), &roots), Status::Missing { partial: 4 });

        write(&root.join("vae/b.bin"), 10);
        assert_eq!(scan_comfy(&model("b.bin", url, Some(10)), &roots), Status::Present(root.join("vae/b.bin")));
        assert!(root.join("vae/b.bin.done").exists(), "matching size gets a .done marker");

        write(&root.join("vae/c.bin"), 3);
        assert!(matches!(scan_comfy(&model("c.bin", url, Some(10)), &roots), Status::SizeDiffers { size: 3, .. }));
        File::create(root.join("vae/c.bin.done")).unwrap();
        assert!(matches!(scan_comfy(&model("c.bin", url, Some(10)), &roots), Status::Present(_)), ".done wins over size");

        write(&root.join("vae/upstream.safetensors"), 10);
        assert_eq!(scan_comfy(&model("d.bin", url, Some(10)), &roots), Status::Linkable(root.join("vae/upstream.safetensors")));
        assert_eq!(scan_comfy(&model("e.bin", url, Some(99)), &roots), Status::Missing { partial: 0 }, "wrong size is no link source");

        assert_eq!(scan_comfy(&model("f.bin", None, None), &roots), Status::Manual);
    }

    #[test]
    fn second_root_counts() {
        let custom = tmp("custom");
        let default = tmp("default");
        let roots = vec![custom.clone(), default.clone()];
        write(&default.join("vae/a.bin"), 10);
        assert_eq!(scan_comfy(&model("a.bin", Some("https://x/a.bin"), Some(10)), &roots), Status::Present(default.join("vae/a.bin")));
    }

    #[test]
    fn ollama_blob_with_the_same_sha_is_shared() {
        let comfy = tmp("share-comfy");
        let ollama = tmp("share-ollama");
        let sha = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"; // "hello"
        let manifest = ollama.join("manifests/registry.ollama.ai/library/tiny/1b");
        fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        fs::write(&manifest, format!(r#"{{"layers":[{{"mediaType":"application/vnd.ollama.image.model","digest":"sha256:{sha}"}}]}}"#))
            .unwrap();
        fs::create_dir_all(ollama.join("blobs")).unwrap();
        fs::write(ollama.join(format!("blobs/sha256-{sha}")), b"hello").unwrap();

        let ctx = egui::Context::default();
        let mut m = Models::new(&ctx, &comfy);
        let mut c = model("tiny.gguf", Some("https://x/tiny.gguf"), Some(5));
        c.sha256 = Some(sha.into());
        m.comfy = Arc::new(vec![c]);
        m.ollama = Arc::new(vec![OllamaModel { model: "tiny:1b".into(), group: String::new(), used_by: String::new(), size: Some(5) }]);
        let roots = vec![comfy.clone()];

        let (st, _) = m.scan(&roots, &ollama, &comfy, Vec::new());
        assert!(matches!(st["tiny.gguf"], Status::Linkable(_)), "missing ComfyUI file links to the Ollama blob");
        assert_eq!(st["ollama:tiny:1b"], Status::Present(ollama.join("manifests")));

        fs::create_dir_all(comfy.join("vae")).unwrap();
        fs::write(comfy.join("vae/tiny.gguf"), b"hello").unwrap();
        let (_, share) = m.scan(&roots, &ollama, &comfy, Vec::new());
        assert!(matches!(share["tiny.gguf"], Share::Duplicate { .. }));

        m.configure(roots.clone(), ollama.clone(), comfy.clone(), Vec::new(), String::new());
        {
            let mut s = m.shared.lock().unwrap();
            s.status.insert("tiny.gguf".into(), Status::Present(comfy.join("vae/tiny.gguf")));
            s.share = share;
        }
        m.share_copies("tiny.gguf", &AtomicBool::new(false)).unwrap();
        let (_, share) = m.scan(&roots, &ollama, &comfy, Vec::new());
        assert!(matches!(share["tiny.gguf"], Share::Shared(_)), "{share:?}");
        assert!(matches!(share["ollama:tiny:1b"], Share::Shared(_)));
    }

    #[test]
    fn local_models_and_old_stores() {
        let repo = tmp("local-repo");
        let dir = tmp("local-models");
        let m = LocalModel {
            id: "voice".into(),
            group: String::new(),
            used_by: String::new(),
            dir: "kokoro".into(),
            file: "v.bin".into(),
            size: Some(4),
            url: Some("https://x/v.bin".into()),
            legacy_dir: Some("comedy/models".into()),
        };
        assert_eq!(scan_local(&m, &dir, &repo), Status::Missing { partial: 0 });
        write(&repo.join("comedy/models/v.bin"), 4);
        assert_eq!(scan_local(&m, &dir, &repo), Status::Present(repo.join("comedy/models/v.bin")), "old folder still counts");

        let comfy = repo.join("ComfyUI/models");
        write(&comfy.join("vae/a.bin"), 3);
        write(&comfy.join("vae/put_vae_here"), 0);
        let old = vec![
            OldStore { label: "ComfyUI", from: comfy.clone(), to: dir.clone() },
            OldStore { label: "Kokoro", from: repo.join("comedy/models"), to: dir.join("kokoro") },
        ];
        let mut plan = plan_moves(&old);
        plan.sort_by(|a, b| a.to.cmp(&b.to));
        let got: Vec<(&str, PathBuf, u64)> = plan.iter().map(|p| (p.label, p.to.clone(), p.size)).collect();
        assert_eq!(
            got,
            vec![
                ("Kokoro", dir.join("kokoro/v.bin"), 4),
                ("Kokoro", dir.join("kokoro/v.bin.done"), 0),
                ("ComfyUI", dir.join("vae/a.bin"), 3)
            ]
        );
    }
}
