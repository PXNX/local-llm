use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::servers::{self, Server};
use crate::sys;

const MAX_LINES: usize = 5000;
const REPAINT: Duration = Duration::from_millis(80);

/// One log line: plain text plus the colored ranges its ANSI escape codes asked for.
pub struct Line {
    pub time: String,
    pub text: String,
    pub err: bool,
    pub colors: Vec<(usize, usize, egui::Color32)>,
}

#[derive(Clone, PartialEq, Debug)]
pub enum State {
    Running,
    Finished(i32),
    Failed(String),
    Cancelled,
}

/// What the run is doing: "sticker 2/3" from the script's log, sampler steps from ComfyUI.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Progress {
    pub label: String,
    pub item: Option<(u32, u32)>,
    pub step: Option<(u32, u32)>,
}

impl Progress {
    pub fn fraction(&self) -> Option<f32> {
        let step = self.step.map(|(v, m)| v as f32 / m.max(1) as f32);
        match (self.item, step) {
            (Some((i, n)), s) => Some(((i.saturating_sub(1)) as f32 + s.unwrap_or(0.0)) / n.max(1) as f32),
            (None, s) => s,
        }
    }

    /// `[stylize] sticker 2/3 (seed 5)` -> label "stylize", item 2 of 3.
    fn parse(&mut self, line: &str) {
        let Some(rest) = line.strip_prefix('[') else { return };
        let Some((tag, text)) = rest.split_once(']') else { return };
        let item = text.split(|c: char| c.is_whitespace() || c == ',' || c == ':').rev().find_map(|w| {
            let (a, b) = w.split_once('/')?;
            let (a, b) = (a.parse::<u32>().ok()?, b.parse::<u32>().ok()?);
            (a >= 1 && a <= b && b <= 10_000).then_some((a, b))
        });
        if let Some(item) = item {
            self.label = tag.trim().to_owned();
            if self.item != Some(item) {
                self.step = None;
            }
            self.item = Some(item);
        }
    }
}

pub struct Job {
    pub title: String,
    pub progress: Progress,
    pub lines: VecDeque<Line>,
    pub state: State,
    pub started: Instant,
    pub ended: Option<Instant>,
    pub out_dir: PathBuf,
    pub console: bool,
    pub command: String,
    last_progress: bool,
    eta: Option<(Instant, Duration)>,
}

impl Job {
    fn push(&mut self, raw: String, err: bool, progress: bool) {
        let (text, colors) = parse_ansi(&raw);
        if !err {
            self.progress.parse(&text);
        }
        let time = sys::now_hms();
        if progress && self.last_progress && !self.lines.is_empty() {
            // tqdm-style "\r" progress: overwrite the previous progress line instead of scrolling
            let last = self.lines.back_mut().expect("non-empty");
            *last = Line { time, text, err, colors };
        } else {
            if self.lines.len() == MAX_LINES {
                self.lines.pop_front();
            }
            self.lines.push_back(Line { time, text, err, colors });
        }
        self.last_progress = progress;
    }

    pub fn running(&self) -> bool {
        self.state == State::Running
    }

    pub fn elapsed(&self) -> Duration {
        self.ended.unwrap_or_else(Instant::now) - self.started
    }

    /// Remaining time from progress so far; recomputed every 3 s so the number doesn't jitter.
    pub fn eta(&mut self) -> Option<Duration> {
        let now = Instant::now();
        if self.eta.is_none_or(|(at, _)| now - at > Duration::from_secs(3)) {
            let f = self.progress.fraction()?;
            if f < 0.01 {
                return None;
            }
            let elapsed = self.elapsed().as_secs_f32();
            self.eta = Some((now, Duration::from_secs_f32(elapsed * (1.0 - f) / f)));
        }
        let (at, eta) = self.eta?;
        Some(eta.saturating_sub(now - at))
    }
}

#[derive(Clone)]
pub struct Launch {
    pub title: String,
    pub root: PathBuf,
    pub python: PathBuf,
    pub script: PathBuf,
    pub args: Vec<String>,
    pub servers: Vec<Server>,
    pub console: bool,
    pub out_dir: PathBuf,
}

#[derive(Clone)]
pub struct JobHandle {
    pub job: Arc<Mutex<Job>>,
    cancel: Arc<AtomicBool>,
}

impl JobHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub fn start(ctx: &egui::Context, l: Launch) -> JobHandle {
    let command =
        format!("{} -s {} {}", l.python.display(), l.script.display(), l.args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" "));
    let job = Arc::new(Mutex::new(Job {
        title: l.title.clone(),
        progress: Progress::default(),
        lines: VecDeque::new(),
        state: State::Running,
        started: Instant::now(),
        ended: None,
        out_dir: l.out_dir.clone(),
        console: l.console,
        command,
        last_progress: false,
        eta: None,
    }));
    let handle = JobHandle { job, cancel: Arc::default() };
    let h = handle.clone();
    let ctx = ctx.clone();
    std::thread::Builder::new()
        .name("job".into())
        .spawn(move || {
            let state = run(&ctx, &h, l);
            let mut j = h.job.lock().unwrap();
            if let State::Failed(e) = &state {
                j.push(format!("[gui  ] {e}"), true, false);
            }
            j.state = state;
            j.ended = Some(Instant::now());
            drop(j);
            ctx.request_repaint();
        })
        .expect("spawn job thread");
    handle
}

fn run(ctx: &egui::Context, h: &JobHandle, l: Launch) -> State {
    let log = |text: String| {
        h.job.lock().unwrap().push(text, false, false);
        ctx.request_repaint_after(REPAINT);
    };
    let mut log_mut = log;
    log_mut(format!("[input] {}", h.job.lock().unwrap().command));
    for server in &l.servers {
        if let Err(e) = servers::ensure(*server, &l.root, &h.cancel, &mut log_mut) {
            return if h.cancel.load(Ordering::Relaxed) { State::Cancelled } else { State::Failed(e) };
        }
    }
    if !l.python.is_file() {
        return State::Failed(format!("{} not found - run setup.bat first (Settings > Run setup.bat)", l.python.display()));
    }

    let mut cmd = Command::new(&l.python);
    cmd.arg("-s").current_dir(&l.root).env("PYTHONUTF8", "1").env("PYTHONIOENCODING", "utf-8").env("PYTHONUNBUFFERED", "1");
    let child = if l.console {
        cmd.arg(l.root.join("common").join("console_run.py")).arg(&l.script).args(&l.args);
        sys::new_console(&mut cmd);
        log_mut("[gui  ] running in its own console window (type the Telegram login there if asked)".into());
        cmd.spawn()
    } else {
        cmd.arg("-u").arg(&l.script).args(&l.args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        sys::hide_window(&mut cmd);
        cmd.spawn()
    };
    let mut child: Child = match child {
        Ok(c) => c,
        Err(e) => return State::Failed(format!("could not start python: {e}")),
    };
    let ended = Arc::new(AtomicBool::new(false));
    if l.servers.contains(&Server::Comfy) {
        let (job, ctx, ended) = (h.job.clone(), ctx.clone(), ended.clone());
        std::thread::spawn(move || comfy_progress(&job, &ctx, &ended));
    }
    let readers: Vec<_> = [
        child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>),
        child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .enumerate()
    .filter_map(|(i, r)| r.map(|r| (i == 1, r)))
    .map(|(err, r)| {
        let (job, ctx) = (h.job.clone(), ctx.clone());
        std::thread::spawn(move || pump(r, err, &job, &ctx))
    })
    .collect();

    let state = loop {
        if h.cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            break State::Cancelled;
        }
        match child.try_wait() {
            Ok(Some(status)) => break State::Finished(status.code().unwrap_or(-1)),
            Ok(None) => std::thread::sleep(Duration::from_millis(150)),
            Err(e) => break State::Failed(e.to_string()),
        }
    };
    ended.store(true, Ordering::Relaxed);
    for r in readers {
        let _ = r.join();
    }
    state
}

/// Sampler steps from ComfyUI's websocket (it broadcasts progress of prompts sent without a client id).
fn comfy_progress(job: &Mutex<Job>, ctx: &egui::Context, ended: &AtomicBool) {
    let Ok(stream) = std::net::TcpStream::connect(("127.0.0.1", Server::Comfy.port())) else { return };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let Ok((mut ws, _)) = tungstenite::client("ws://127.0.0.1:8188/ws?clientId=local-llm-gui", stream) else { return };
    while !ended.load(Ordering::Relaxed) {
        match ws.read() {
            Ok(tungstenite::Message::Text(t)) => {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(t.as_str()) else { continue };
                let step = match v["type"].as_str() {
                    Some("progress") => v["data"]["value"].as_u64().zip(v["data"]["max"].as_u64()).map(|(a, b)| (a as u32, b as u32)),
                    Some("executing") if v["data"]["node"].is_null() => None,
                    _ => continue,
                };
                job.lock().unwrap().progress.step = step;
                ctx.request_repaint_after(REPAINT);
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => break,
        }
    }
    let _ = ws.close(None);
}

/// Splits a child's output into lines; a bare "\r" marks a progress line that overwrites the previous one.
fn pump(mut r: Box<dyn Read + Send>, err: bool, job: &Mutex<Job>, ctx: &egui::Context) {
    let mut buf = [0u8; 8192];
    let mut cur: Vec<u8> = Vec::new();
    let (mut cr, mut progress) = (false, false);
    let commit = |cur: &mut Vec<u8>, progress: bool| {
        let text = String::from_utf8_lossy(cur).into_owned();
        cur.clear();
        job.lock().unwrap().push(text, err, progress);
    };
    while let Ok(n) = r.read(&mut buf) {
        if n == 0 {
            break;
        }
        for &b in &buf[..n] {
            if cr {
                cr = false;
                if b == b'\n' {
                    commit(&mut cur, progress);
                    progress = false;
                    continue;
                }
                if !cur.is_empty() {
                    commit(&mut cur, progress);
                }
                progress = true;
            }
            match b {
                b'\r' => cr = true,
                b'\n' => {
                    commit(&mut cur, progress);
                    progress = false;
                }
                _ => cur.push(b),
            }
        }
        ctx.request_repaint_after(REPAINT);
    }
    if !cur.is_empty() {
        commit(&mut cur, progress);
    }
    ctx.request_repaint();
}

const ANSI: [egui::Color32; 8] = [
    egui::Color32::from_rgb(110, 110, 110),
    egui::Color32::from_rgb(230, 90, 80),
    egui::Color32::from_rgb(90, 190, 110),
    egui::Color32::from_rgb(230, 190, 70),
    egui::Color32::from_rgb(90, 150, 240),
    egui::Color32::from_rgb(200, 110, 220),
    egui::Color32::from_rgb(80, 200, 210),
    egui::Color32::from_rgb(220, 220, 220),
];

/// Removes ANSI escape sequences, keeping SGR foreground colors (30-37, 90-97) as colored ranges.
fn parse_ansi(s: &str) -> (String, Vec<(usize, usize, egui::Color32)>) {
    if !s.contains('\u{1b}') {
        return (s.to_owned(), Vec::new());
    }
    let mut out = String::with_capacity(s.len());
    let mut colors = Vec::new();
    let mut current: Option<(usize, egui::Color32)> = None;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        if chars.peek() != Some(&'[') {
            continue;
        }
        chars.next();
        let mut params = String::new();
        let mut last = ' ';
        for c in chars.by_ref() {
            if ('@'..='~').contains(&c) {
                last = c;
                break;
            }
            params.push(c);
        }
        if last != 'm' {
            continue;
        }
        for code in params.split(';').map(|p| p.parse::<u32>().unwrap_or(0)) {
            let color = match code {
                30..=37 => Some(Some(ANSI[(code - 30) as usize])),
                90..=97 => Some(Some(ANSI[(code - 90) as usize])),
                0 | 39 => Some(None),
                _ => None,
            };
            if let Some(color) = color {
                if let Some((from, c)) = current.take()
                    && from < out.len()
                {
                    colors.push((from, out.len(), c));
                }
                current = color.map(|c| (out.len(), c));
            }
        }
    }
    if let Some((from, c)) = current
        && from < out.len()
    {
        colors.push((from, out.len(), c));
    }
    (out, colors)
}

fn quote(a: &str) -> String {
    if a.is_empty() || a.contains([' ', '"']) { format!("\"{}\"", a.replace('"', "\\\"")) } else { a.to_owned() }
}

pub fn open_dir(p: &Path) {
    let dir = if p.is_dir() { p } else { p.parent().unwrap_or(p) };
    sys::open(dir);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(bytes: &[u8]) -> Vec<String> {
        let job = Mutex::new(Job {
            title: String::new(),
            progress: Progress::default(),
            lines: VecDeque::new(),
            state: State::Running,
            started: Instant::now(),
            ended: None,
            out_dir: PathBuf::new(),
            console: false,
            command: String::new(),
            last_progress: false,
            eta: None,
        });
        pump(Box::new(std::io::Cursor::new(bytes.to_vec())), false, &job, &egui::Context::default());
        job.into_inner().unwrap().lines.into_iter().map(|l| l.text).collect()
    }

    #[test]
    fn crlf_lines() {
        assert_eq!(run(b"a\r\nb\r\n"), ["a", "b"]);
    }

    #[test]
    fn progress_overwrites_itself_but_not_normal_lines() {
        assert_eq!(run(b"start\n\r 10%\r 50%\r100%\ndone\n"), ["start", "100%", "done"]);
    }

    #[test]
    fn ansi_is_stripped() {
        assert_eq!(run(b"\x1b[32mgreen\x1b[0m\n"), ["green"]);
    }

    #[test]
    fn progress_from_log_lines() {
        let mut p = Progress::default();
        p.parse("[stylize] sticker 2/3 (seed 5, engine flux1): waving");
        assert_eq!((p.label.as_str(), p.item), ("stylize", Some((2, 3))));
        p.step = Some((10, 20));
        assert_eq!(p.fraction(), Some(0.5));
        p.parse("[rate ] 7/25 channel funny 3.0");
        assert_eq!(p.item, Some((7, 25)));
        assert_eq!(p.step, None, "a new item resets the step");
        p.parse("[done   ] C:/out/a_1/2.png");
        assert_eq!(p.item, Some((7, 25)), "paths are no progress");
        p.parse("no tag 1/2");
        assert_eq!(p.item, Some((7, 25)));
    }

    #[test]
    fn ansi_colors_are_kept() {
        let (text, colors) = parse_ansi("a \u{1b}[1;31mred\u{1b}[0m b \u{1b}[92mok");
        assert_eq!(text, "a red b ok");
        assert_eq!(colors, vec![(2, 5, ANSI[1]), (8, 10, ANSI[2])]);
    }

    #[test]
    fn log_is_capped() {
        let text: String = (0..MAX_LINES + 10).map(|i| format!("{i}\n")).collect();
        let lines = run(text.as_bytes());
        assert_eq!(lines.len(), MAX_LINES);
        assert_eq!(lines[0], "10");
    }
}

#[cfg(test)]
mod run_tests {
    use super::*;

    /// Runs the real vectorize flow through the job runner. `cargo test -- --ignored`
    #[test]
    #[ignore]
    fn vectorize_end_to_end() {
        let root = crate::paths::find_repo(None).expect("run inside the local-llm checkout");
        let paths = crate::paths::Paths { root: root.clone() };
        let out = std::env::temp_dir().join("llm-gui-test.svg");
        let _ = std::fs::remove_file(&out);
        let h = start(
            &egui::Context::default(),
            Launch {
                title: String::new(),
                python: paths.python(),
                script: paths.script("vectorize/trace_svg.py"),
                args: vec![root.join("stickers/test_input.jpg").display().to_string(), "--out".into(), out.display().to_string()],
                servers: Vec::new(),
                console: false,
                out_dir: out.clone(),
                root,
            },
        );
        let t0 = Instant::now();
        while h.job.lock().unwrap().running() {
            assert!(t0.elapsed() < Duration::from_secs(120), "timeout");
            std::thread::sleep(Duration::from_millis(100));
        }
        let j = h.job.lock().unwrap();
        let log: Vec<&str> = j.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(j.state, State::Finished(0), "{log:#?}");
        assert!(out.is_file() && std::fs::metadata(&out).unwrap().len() > 100, "{log:#?}");
    }

    #[test]
    #[ignore]
    fn cancel_kills_the_process() {
        let root = crate::paths::find_repo(None).expect("run inside the local-llm checkout");
        let paths = crate::paths::Paths { root: root.clone() };
        let script = std::env::temp_dir().join("llm-gui-sleep.py");
        std::fs::write(&script, "import time\nprint('sleeping', flush=True)\ntime.sleep(60)\n").unwrap();
        let h = start(
            &egui::Context::default(),
            Launch {
                title: String::new(),
                python: paths.python(),
                script,
                args: Vec::new(),
                servers: Vec::new(),
                console: false,
                out_dir: PathBuf::new(),
                root,
            },
        );
        std::thread::sleep(Duration::from_secs(2));
        h.cancel();
        let t0 = Instant::now();
        while h.job.lock().unwrap().running() {
            assert!(t0.elapsed() < Duration::from_secs(5), "cancel did not stop the job");
            std::thread::sleep(Duration::from_millis(50));
        }
        let j = h.job.lock().unwrap();
        assert_eq!(j.state, State::Cancelled);
        assert_eq!(j.lines.front().map(|l| l.text.as_str()), Some("sleeping"));
    }
}
