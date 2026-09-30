use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::sys;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Server {
    Ollama,
    Comfy,
}

impl Server {
    pub fn name(self) -> &'static str {
        match self {
            Server::Ollama => "Ollama",
            Server::Comfy => "ComfyUI",
        }
    }

    pub fn port(self) -> u16 {
        match self {
            Server::Ollama => 11434,
            Server::Comfy => 8188,
        }
    }

    /// ComfyUI loads its nodes first, which takes about a minute on a cold start.
    fn start_timeout(self) -> Duration {
        match self {
            Server::Ollama => Duration::from_secs(60),
            Server::Comfy => Duration::from_secs(300),
        }
    }
}

pub fn is_up(server: Server) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], server.port()));
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

/// Up/down state of both servers, refreshed by a background thread so the UI never blocks on a socket.
#[derive(Clone)]
pub struct Servers {
    ollama: Arc<AtomicBool>,
    comfy: Arc<AtomicBool>,
}

impl Servers {
    pub fn spawn(ctx: egui::Context) -> Self {
        let s = Self { ollama: Arc::default(), comfy: Arc::default() };
        let t = s.clone();
        std::thread::Builder::new()
            .name("server-poll".into())
            .spawn(move || {
                loop {
                    let mut changed = false;
                    for (server, flag) in [(Server::Ollama, &t.ollama), (Server::Comfy, &t.comfy)] {
                        let up = is_up(server);
                        changed |= flag.swap(up, Ordering::Relaxed) != up;
                    }
                    if changed {
                        ctx.request_repaint();
                    }
                    std::thread::sleep(Duration::from_secs(2));
                }
            })
            .expect("spawn server poller");
        s
    }

    pub fn up(&self, server: Server) -> bool {
        match server {
            Server::Ollama => self.ollama.load(Ordering::Relaxed),
            Server::Comfy => self.comfy.load(Ordering::Relaxed),
        }
    }
}

/// Models folder Ollama is started with (None = its default / the user's OLLAMA_MODELS).
static OLLAMA_MODELS: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn set_ollama_models(dir: Option<PathBuf>) {
    *OLLAMA_MODELS.lock().unwrap() = dir;
}

/// Low-VRAM defaults (as in setup.bat) so Ollama and ComfyUI fit next to each other on a small GPU;
/// values the user set in the environment win.
const OLLAMA_ENV: &[(&str, &str)] =
    &[("OLLAMA_FLASH_ATTENTION", "1"), ("OLLAMA_KV_CACHE_TYPE", "q8_0"), ("OLLAMA_MAX_LOADED_MODELS", "1"), ("OLLAMA_NUM_PARALLEL", "1")];

/// Launches the server the same way the .bat flows do. Returns immediately.
pub fn start(server: Server, root: &Path) -> Result<(), String> {
    match server {
        Server::Ollama => {
            let app = ollama_dir().join("ollama app.exe");
            let mut cmd = if app.is_file() {
                Command::new(app)
            } else {
                let mut c = Command::new("ollama");
                c.arg("serve");
                c
            };
            for (k, v) in OLLAMA_ENV {
                if std::env::var_os(k).is_none() {
                    cmd.env(k, v);
                }
            }
            if let Some(dir) = OLLAMA_MODELS.lock().unwrap().clone() {
                cmd.env("OLLAMA_MODELS", dir);
            }
            sys::hide_window(&mut cmd).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            cmd.spawn().map(drop).map_err(|e| format!("could not start Ollama ({e}) - is it installed? (setup.bat)"))
        }
        Server::Comfy => {
            let bat = root.join("start-comfyui.bat");
            if !bat.is_file() {
                return Err(format!("{} not found", bat.display()));
            }
            // own minimized console window, like the .bat flows: ComfyUI keeps running when the GUI closes
            let mut cmd = Command::new("cmd");
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.raw_arg(format!("/C start \"ComfyUI\" /min /normal \"{}\"", bat.display()));
            }
            sys::hide_window(&mut cmd).current_dir(root);
            cmd.spawn().map(drop).map_err(|e| format!("could not start ComfyUI: {e}"))
        }
    }
}

/// Starts the server if needed and waits until it accepts connections.
pub fn ensure(server: Server, root: &Path, cancel: &AtomicBool, log: &mut dyn FnMut(String)) -> Result<(), String> {
    if is_up(server) {
        return Ok(());
    }
    log(format!("[gui  ] starting {} ...", server.name()));
    start(server, root)?;
    let t0 = Instant::now();
    while t0.elapsed() < server.start_timeout() {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        std::thread::sleep(Duration::from_millis(500));
        if is_up(server) {
            log(format!("[gui  ] {} is up after {} s", server.name(), t0.elapsed().as_secs()));
            return Ok(());
        }
    }
    Err(format!("{} did not start within {} s", server.name(), server.start_timeout().as_secs()))
}

/// Stops Ollama (tray app and server), e.g. before moving its models.
pub fn stop_ollama() {
    for exe in ["ollama app.exe", "ollama.exe"] {
        let mut cmd = Command::new("taskkill");
        cmd.args(["/IM", exe, "/F"]).stdout(Stdio::null()).stderr(Stdio::null());
        let _ = sys::hide_window(&mut cmd).status();
    }
}

pub fn ollama_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default().join("Programs").join("Ollama")
}

/// OLLAMA_MODELS from the environment, else Ollama's default folder.
pub fn ollama_default_dir() -> PathBuf {
    std::env::var_os("OLLAMA_MODELS")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default().join(".ollama").join("models"))
}
