//! Transfers: resumable HTTP downloads, `ollama pull`, and a watchdog that restarts stalled attempts.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use crate::servers::{self, Server};

const STALL: Duration = Duration::from_secs(90);

pub enum Attempt {
    Retry(String),
    Fatal(String),
}

pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .user_agent("local-llm-gui")
        .build()
        .new_agent()
}

/// Runs `transfer` in its own thread and restarts it (resuming) when it stalls: ureq has no idle timeout.
/// `progress(done, total, bytes_per_s)` is called 4x per second.
pub fn supervise<F>(expected: Option<u64>, stop: &AtomicBool, progress: &dyn Fn(u64, Option<u64>, f64), transfer: F) -> Result<(), String>
where
    F: Fn(&AtomicU64, &AtomicU64, &AtomicBool) -> Result<(), Attempt> + Send + Sync + 'static,
{
    let transfer = Arc::new(transfer);
    let mut failures = 0;
    loop {
        let done = Arc::new(AtomicU64::new(0));
        let total = Arc::new(AtomicU64::new(expected.unwrap_or(0)));
        let attempt_stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        {
            let (t, done, total, attempt_stop) = (transfer.clone(), done.clone(), total.clone(), attempt_stop.clone());
            std::thread::spawn(move || {
                let _ = tx.send(t(&done, &total, &attempt_stop));
            });
        }
        let (mut last, mut last_change, mut rate, mut tick) = (0u64, Instant::now(), 0.0f64, Instant::now());
        let outcome = loop {
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(r) => break Some(r),
                Err(mpsc::RecvTimeoutError::Disconnected) => break Some(Err(Attempt::Fatal("transfer thread died".into()))),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if stop.load(Ordering::Relaxed) {
                attempt_stop.store(true, Ordering::Relaxed);
                return Err("cancelled".into());
            }
            let d = done.load(Ordering::Relaxed);
            let dt = tick.elapsed().as_secs_f64();
            tick = Instant::now();
            if d != last {
                let inst = d.saturating_sub(last) as f64 / dt.max(1e-3);
                rate = if rate == 0.0 { inst } else { rate * 0.8 + inst * 0.2 };
                last = d;
                last_change = Instant::now();
            } else if last_change.elapsed() > STALL {
                attempt_stop.store(true, Ordering::Relaxed);
                break None;
            }
            let t = total.load(Ordering::Relaxed);
            progress(d, (t > 0).then_some(t), rate);
        };
        match outcome {
            Some(Ok(())) => return Ok(()),
            Some(Err(Attempt::Fatal(e))) => return Err(e),
            Some(Err(Attempt::Retry(e))) if failures >= 8 => return Err(e),
            Some(Err(Attempt::Retry(_))) | None => {
                failures += 1;
                let wait = Duration::from_secs(2u64.pow(failures.min(5)));
                let t0 = Instant::now();
                while t0.elapsed() < wait {
                    if stop.load(Ordering::Relaxed) {
                        return Err("cancelled".into());
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        }
    }
}

pub fn http_get(
    url: &str,
    token: &str,
    part: &Path,
    expected: Option<u64>,
    done: &AtomicU64,
    total: &AtomicU64,
    stop: &AtomicBool,
) -> Result<(), Attempt> {
    let have = fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    if expected == Some(have) {
        done.store(have, Ordering::Relaxed);
        return Ok(());
    }
    let mut req = agent().get(url);
    if have > 0 {
        req = req.header("Range", format!("bytes={have}-"));
    }
    if !token.is_empty() {
        req = req.header("Authorization", format!("Bearer {token}"));
    }
    let resp = req.call().map_err(|e| Attempt::Retry(e.to_string()))?;
    let status = resp.status().as_u16();
    let len = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    let (mut file, start) = match status {
        206 => (OpenOptions::new().append(true).open(part).map_err(|e| Attempt::Fatal(e.to_string()))?, have),
        200 => (File::create(part).map_err(|e| Attempt::Fatal(e.to_string()))?, 0),
        416 if have > 0 => return Ok(()), // already complete
        401 | 403 => {
            return Err(Attempt::Fatal(format!("HTTP {status}: gated model - accept the license on Hugging Face and set HF_TOKEN")));
        }
        404 => return Err(Attempt::Fatal("HTTP 404: file not found at the URL in models.json".into())),
        s => return Err(Attempt::Retry(format!("HTTP {s}"))),
    };
    if let Some(len) = len {
        total.store(start + len, Ordering::Relaxed);
    }
    done.store(start, Ordering::Relaxed);
    let mut body = resp.into_body().into_reader();
    // own buffer instead of BufWriter: a stopped (stalled) attempt must drop its bytes, not flush them
    // into a file the next attempt is already appending to
    let mut buf = vec![0u8; 1 << 20];
    let mut pending: Vec<u8> = Vec::with_capacity(8 << 20);
    let mut got = start;
    loop {
        let n = body.read(&mut buf).map_err(|e| Attempt::Retry(e.to_string()))?;
        if stop.load(Ordering::Relaxed) {
            return Err(Attempt::Retry("stopped".into()));
        }
        if n == 0 {
            break;
        }
        pending.extend_from_slice(&buf[..n]);
        if pending.len() >= 8 << 20 {
            file.write_all(&pending).map_err(|e| Attempt::Fatal(format!("write: {e}")))?;
            pending.clear();
        }
        got += n as u64;
        done.store(got, Ordering::Relaxed);
    }
    file.write_all(&pending).map_err(|e| Attempt::Fatal(format!("write: {e}")))?;
    match expected {
        Some(e) if got != e => Err(Attempt::Retry(format!("got {got} of {e} bytes"))),
        _ => Ok(()),
    }
}

pub fn ollama_pull(name: &str, done: &AtomicU64, total: &AtomicU64, stop: &AtomicBool) -> Result<(), Attempt> {
    if !servers::is_up(Server::Ollama) {
        let never = AtomicBool::new(false);
        servers::ensure(Server::Ollama, Path::new("."), &never, &mut |_| {}).map_err(Attempt::Fatal)?;
    }
    let resp = agent()
        .post("http://127.0.0.1:11434/api/pull")
        .send_json(serde_json::json!({ "model": name, "stream": true }))
        .map_err(|e| Attempt::Retry(e.to_string()))?;
    if resp.status().as_u16() != 200 {
        return Err(Attempt::Fatal(format!("Ollama answered HTTP {}", resp.status().as_u16())));
    }
    // Ollama reports per layer; sum the layers for one overall progress bar
    let mut layers: HashMap<String, (u64, u64)> = HashMap::new();
    for line in BufReader::new(resp.into_body().into_reader()).lines() {
        let line = line.map_err(|e| Attempt::Retry(e.to_string()))?;
        if stop.load(Ordering::Relaxed) {
            return Err(Attempt::Retry("stopped".into()));
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
            return Err(Attempt::Fatal(format!("Ollama: {e}")));
        }
        if let (Some(digest), Some(t)) = (v.get("digest").and_then(|d| d.as_str()), v.get("total").and_then(|t| t.as_u64())) {
            let c = v.get("completed").and_then(|c| c.as_u64()).unwrap_or(0);
            layers.insert(digest.to_owned(), (c, t));
            done.store(layers.values().map(|l| l.0).sum(), Ordering::Relaxed);
            total.store(layers.values().map(|l| l.1).sum(), Ordering::Relaxed);
        }
        if v.get("status").and_then(|s| s.as_str()) == Some("success") {
            return Ok(());
        }
    }
    Err(Attempt::Retry("Ollama closed the connection".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Network: resumes a partial file with a Range request. `cargo test -- --ignored`
    #[test]
    #[ignore]
    fn http_resume() {
        let dir = std::env::temp_dir().join(format!("llm-gui-http-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let url = "https://huggingface.co/city96/FLUX.1-schnell-gguf/resolve/main/README.md";
        let full = dir.join("full");
        let (d, t, s) = (AtomicU64::new(0), AtomicU64::new(0), AtomicBool::new(false));
        assert!(http_get(url, "", &full, None, &d, &t, &s).is_ok());
        let whole = fs::read(&full).unwrap();
        let part = dir.join("part");
        fs::write(&part, &whole[..100]).unwrap();
        assert!(http_get(url, "", &part, Some(whole.len() as u64), &d, &t, &s).is_ok());
        assert_eq!(fs::read(&part).unwrap(), whole);
    }
}
