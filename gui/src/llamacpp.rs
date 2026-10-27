//! The llama.cpp router: starts `llama serve` hidden in the background, loads one GGUF model and
//! exposes it on an OpenAI-compatible endpoint that Pi (and so T3 Code) can use.
//!
//! The router's own API: POST /models/load starts a model, POST /models/unload frees it again and
//! GET /v1/models lists what is loaded.

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Value, json};

use crate::sys;

/// .env keys of the router's endpoint, model and (optional) llama.cpp executable.
pub const URL_KEY: &str = "LLAMA_CPP_URL";
pub const MODEL_KEY: &str = "LLAMA_CPP_MODEL";
pub const EXE_KEY: &str = "LLAMA_CPP_EXE";
pub const CTX_KEY: &str = "LLAMA_CPP_CTX";

pub const DEFAULT_URL: &str = "http://127.0.0.1:8080";
pub const DEFAULT_MODEL: &str = "nvidia/NVIDIA-Nemotron-3.5-Bolt-E4B-4GB:Q4_K_M";

/// The provider key in Pi's and OpenCode's config files.
pub const PROVIDER: &str = "llama-cpp";
/// Default context the router loads the model with (and Pi's model entry says): 57344 of the
/// Nemotron's 65536 default. `LLAMA_CPP_CTX` raises it; the KV cache grows with it.
pub const CONTEXT: u32 = 57_344;
pub const MAX_TOKENS: u32 = 8_192;
/// The choices offered in the GUI (any other value can be typed into .env).
pub const CONTEXT_CHOICES: [u32; 7] = [16_384, 32_768, CONTEXT, 65_536, 131_072, 204_800, 262_144];
const MIN_CONTEXT: u32 = 4_096;

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Router {
    pub running: bool,
    /// Every model the router knows, loaded or not.
    pub models: Vec<Model>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Model {
    pub id: String,
    pub loaded: bool,
    pub loading: bool,
    pub failed: bool,
    pub exit_code: Option<i32>,
}

impl Router {
    fn get(&self, model: &str) -> Option<&Model> {
        self.models.iter().find(|m| m.id == model)
    }

    pub fn loaded(&self, model: &str) -> bool {
        self.get(model).is_some_and(|m| m.loaded)
    }

    pub fn loading(&self, model: &str) -> bool {
        self.get(model).is_some_and(|m| m.loading)
    }

    pub fn failed(&self, model: &str) -> Option<i32> {
        self.get(model).filter(|m| m.failed).map(|m| m.exit_code.unwrap_or_default())
    }

    pub fn loaded_names(&self) -> Vec<String> {
        self.models.iter().filter(|m| m.loaded).map(|m| m.id.clone()).collect()
    }
}

/// The endpoint with the entered value, falling back to the default when empty.
pub fn base_url(value: &str) -> String {
    let v = value.trim().trim_end_matches('/');
    if v.is_empty() { DEFAULT_URL.to_owned() } else { v.to_owned() }
}

pub fn model_id(value: &str) -> String {
    let v = value.trim();
    if v.is_empty() { DEFAULT_MODEL.to_owned() } else { v.to_owned() }
}

/// The context size from .env: plain tokens ("131072") or in k ("128k"); empty or too small falls back
/// to the default.
pub fn context(value: &str) -> u32 {
    let v = value.trim().to_ascii_lowercase();
    let tokens = match v.strip_suffix('k') {
        Some(k) => k.trim().parse::<u32>().ok().and_then(|k| k.checked_mul(1024)),
        None => v.parse::<u32>().ok(),
    };
    tokens.filter(|&t| t >= MIN_CONTEXT).unwrap_or(CONTEXT)
}

/// "57344" -> "56k", for labels.
pub fn context_label(tokens: u32) -> String {
    if tokens.is_multiple_of(1024) { format!("{}k", tokens / 1024) } else { tokens.to_string() }
}

/// The router's state: TCP first (a closed port is just "not running"), then `GET /v1/models`.
pub fn status(base: &str) -> Result<Router, String> {
    let addr = address(base)?;
    if TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_err() {
        return Ok(Router::default());
    }
    let body = agent(Duration::from_secs(3))
        .get(format!("{base}/v1/models"))
        .call()
        .map_err(|e| format!("llama.cpp did not answer on {base}: {e}"))?
        .into_body()
        .read_json::<Value>()
        .map_err(|e| format!("llama.cpp sent no model list: {e}"))?;
    Ok(Router { running: true, models: parse_models(&body) })
}

fn parse_models(body: &Value) -> Vec<Model> {
    body["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let state = m["status"]["value"].as_str().unwrap_or_default();
            Some(Model {
                id: m["id"].as_str()?.to_owned(),
                loaded: state == "loaded",
                loading: state == "loading",
                failed: m["status"]["failed"].as_bool().unwrap_or(false),
                exit_code: m["status"]["exit_code"].as_i64().map(|c| c as i32),
            })
        })
        .collect()
}

/// Starts `llama serve` without a console window, loading models with `ctx` tokens of context (a
/// command-line `--ctx-size` also wins over a models preset). A router that is already up is left alone,
/// so a changed context needs a stop first.
pub fn start(base: &str, exe: &str, ctx: u32, console: &crate::console::Shared) -> Result<(), String> {
    if status(base)?.running {
        return Ok(());
    }
    let addr = address(base)?;
    let mut cmd = Command::new(if exe.trim().is_empty() { "llama" } else { exe.trim() });
    cmd.args(["serve", "--host", &addr.ip().to_string(), "--port", &addr.port().to_string(), "--models-max", "1"]);
    cmd.args(["--ctx-size", &ctx.to_string()]);
    // Downloads (--hf-repo) go to HF_HOME, i.e. into the one models folder.
    for (key, value) in crate::paths::current_env() {
        cmd.env(key, value);
    }
    crate::console::command(console, "llama", &cmd);
    sys::hide_window(&mut cmd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| {
        format!("could not start llama.cpp ({e}) - install it (winget install ggml.llamacpp) or set {EXE_KEY}")
    })?;
    crate::console::capture(console, "llama", &mut child);
    *CHILD.lock().unwrap() = Some(child);
    Ok(())
}

/// Waits until the router accepts requests. The start-up takes a few seconds on a cold start.
pub fn wait_until_up(base: &str, timeout: Duration) -> Result<(), String> {
    let t0 = std::time::Instant::now();
    while t0.elapsed() < timeout {
        if status(base)?.running {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    Err(format!("llama.cpp did not start on {base} within {} s", timeout.as_secs()))
}

/// Asks the router to load the model. It answers at once and loads in the background, so callers
/// wait for [`wait_until_loaded`].
pub fn load(base: &str, model: &str) -> Result<(), String> {
    agent(Duration::from_secs(30))
        .post(format!("{base}/models/load"))
        .send_json(json!({ "model": model }))
        .map_err(|e| format!("llama.cpp could not load {model}: {e}"))?;
    Ok(())
}

/// Starts the model unless it is already loaded or loading, then waits until it is ready.
pub fn ensure_loaded(base: &str, model: &str, timeout: Duration) -> Result<(), String> {
    let router = status(base)?;
    if router.loaded(model) {
        return Ok(());
    }
    if !router.loading(model) {
        load(base, model)?;
    }
    wait_until_loaded(base, model, timeout)
}

pub fn wait_until_loaded(base: &str, model: &str, timeout: Duration) -> Result<(), String> {
    let t0 = std::time::Instant::now();
    while t0.elapsed() < timeout {
        let router = status(base)?;
        if router.loaded(model) {
            return Ok(());
        }
        if let Some(code) = router.failed(model) {
            return Err(format!("llama.cpp could not load {model} (exit code {code}) - see the router log"));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(format!("{model} was not loaded within {} s", timeout.as_secs()))
}

/// Unloads every loaded model, so the VRAM is free again.
pub fn unload_all(base: &str) -> Result<String, String> {
    let loaded = status(base)?.loaded_names();
    if loaded.is_empty() {
        return Ok("no llama.cpp model is loaded".into());
    }
    let agent = agent(Duration::from_secs(120));
    for model in &loaded {
        agent
            .post(format!("{base}/models/unload"))
            .send_json(json!({ "model": model }))
            .map_err(|e| format!("llama.cpp could not unload {model}: {e}"))?;
    }
    Ok(format!("unloaded {}", loaded.join(", ")))
}

/// Unloads the models and stops the router: the child we started, or else the process listening on
/// the port (a router started outside the GUI). A router that isn't running is not an error.
pub fn stop(base: &str) -> Result<String, String> {
    let unloaded = unload_all(base).err();
    let mut stopped = false;
    if let Some(mut child) = CHILD.lock().unwrap().take()
        && matches!(child.try_wait(), Ok(None))
    {
        // /T also takes down a model subprocess that an unload could not stop
        let mut cmd = Command::new("taskkill");
        cmd.args(["/PID", &child.id().to_string(), "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null());
        if !sys::hide_window(&mut cmd).status().is_ok_and(|s| s.success()) {
            let _ = child.kill();
        }
        let _ = child.wait();
        stopped = true;
    }
    if !stopped && status(base)?.running {
        let pid = pid_on_port(address(base)?.port());
        if let Some(pid) = pid {
            let mut cmd = Command::new("taskkill");
            cmd.args(["/PID", &pid.to_string(), "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null());
            stopped = sys::hide_window(&mut cmd).status().is_ok();
        }
    }
    match (stopped, unloaded) {
        (false, _) => Ok("llama.cpp is not running".into()),
        (true, None) => Ok("llama.cpp stopped".into()),
        (true, Some(e)) => Ok(format!("llama.cpp stopped ({e})")),
    }
}

/// The PID of the process LISTENING on the local port, from netstat.
fn pid_on_port(port: u16) -> Option<u32> {
    let mut cmd = Command::new("netstat");
    cmd.args(["-ano", "-p", "TCP"]).stdout(Stdio::piped()).stderr(Stdio::null()).stdin(Stdio::null());
    let out = sys::hide_window(&mut cmd).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().find_map(|line| {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 5 && cols[1].ends_with(&format!(":{port}")) && cols[3] == "LISTENING" {
            cols[4].parse().ok()
        } else {
            None
        }
    })
}

/// Splits an http URL into the socket to connect to. Only local http endpoints are supported.
fn address(base: &str) -> Result<SocketAddr, String> {
    let rest = base
        .strip_prefix("http://")
        .ok_or_else(|| format!("{base} is not an http://host:port URL"))?
        .split('/')
        .next()
        .unwrap_or_default();
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().map_err(|_| format!("{base} has no valid port"))?),
        None => (rest, 80),
    };
    // prefer IPv4: `llama serve --host 127.0.0.1` binds it, while "localhost" may resolve to ::1
    let addrs: Vec<SocketAddr> =
        (host, port).to_socket_addrs().map_err(|e| format!("could not resolve {host}: {e}"))?.collect();
    addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or_else(|| addrs.first())
        .copied()
        .ok_or_else(|| format!("could not resolve {host}"))
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(timeout)).build().new_agent()
}

/// The router this GUI started, so Stop only kills that one when it can.
static CHILD: Mutex<Option<Child>> = Mutex::new(None);

/// A readable name from a Hugging Face repo id: "org/Name:Q4_K_M" -> "Name (Q4_K_M)".
pub fn display_name(model: &str) -> String {
    let (path, quant) = model.split_once(':').unwrap_or((model, ""));
    let name = path.rsplit('/').next().unwrap_or(path);
    if quant.is_empty() { name.to_owned() } else { format!("{name} ({quant})") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_urls() {
        assert_eq!(base_url("  "), DEFAULT_URL);
        assert_eq!(base_url("http://localhost:8080/"), "http://localhost:8080");
        assert_eq!(model_id(""), DEFAULT_MODEL);
        assert_eq!(context(""), CONTEXT);
        assert_eq!(context("131072"), 131_072);
        assert_eq!(context(" 128K "), 131_072);
        assert_eq!(context("200k"), 204_800);
        assert_eq!(context("12"), CONTEXT, "too small to be a context");
        assert_eq!(context("lots"), CONTEXT);
        assert_eq!(context_label(57_344), "56k");
        assert_eq!(context_label(5000), "5000");
        assert!(CONTEXT_CHOICES.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(address("http://127.0.0.1:8080").unwrap().port(), 8080);
        assert!(address("https://example.com").is_err());
        assert!(address("http://127.0.0.1:notaport").is_err());
    }

    #[test]
    fn model_rows_parse() {
        let body: Value = serde_json::from_str(
            r#"{"data":[
                {"id":"a:b","status":{"value":"loaded"}},
                {"id":"c:d","status":{"value":"unloaded"}},
                {"id":"e:f","status":{"value":"loading"}},
                {"id":"g:h","status":{"value":"unloaded","failed":true,"exit_code":3}}
            ]}"#,
        )
        .unwrap();
        let r = Router { running: true, models: parse_models(&body) };
        assert!(r.loaded("a:b"));
        assert!(!r.loaded("c:d"));
        assert!(r.loading("e:f"));
        assert_eq!(r.failed("g:h"), Some(3));
        assert_eq!(r.failed("a:b"), None);
        assert_eq!(r.loaded_names(), ["a:b"]);
        assert_eq!(display_name(DEFAULT_MODEL), "NVIDIA-Nemotron-3.5-Bolt-E4B-4GB (Q4_K_M)");
        assert_eq!(display_name("plain"), "plain");
    }
}
