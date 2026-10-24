//! A shared log of the command lines the GUI starts and what they print, shown on the Coding page.
//!
//! The llama.cpp router's stdout/stderr is read directly; Ollama's server writes its output to a log
//! file, which is followed here; the terminal editors (Pi, OpenCode) print in their own console, so
//! at least their command line is recorded.

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use eframe::egui;

use crate::jobs::{Line, level, parse_ansi};
use crate::sys;

/// Enough for a router start plus an Ollama log tail; older lines scroll away.
const MAX_LINES: usize = 2000;

pub type Shared = Arc<Mutex<Console>>;

/// Repaints the window when a background reader adds a line.
static CTX: OnceLock<egui::Context> = OnceLock::new();

pub fn init(ctx: egui::Context) {
    let _ = CTX.set(ctx);
}

fn repaint() {
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint();
    }
}

#[derive(Default)]
pub struct Console {
    pub lines: VecDeque<Line>,
}

impl Console {
    fn push(&mut self, raw: &str, err: bool) {
        let (text, colors) = parse_ansi(raw);
        if text.trim().is_empty() {
            return;
        }
        if self.lines.len() == MAX_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(Line { time: sys::now_hms(), level: level(&text, err), text, colors });
        repaint();
    }
}

/// Adds one line (already prefixed with its source, e.g. `[llama] ...`).
pub fn line(console: &Shared, text: impl Into<String>, err: bool) {
    console.lock().unwrap().push(&text.into(), err);
}

pub fn clear(console: &Shared) {
    console.lock().unwrap().lines.clear();
}

/// Records what is about to run, so failures still show the exact command line.
pub fn command(console: &Shared, prefix: &str, cmd: &Command) {
    let mut text = format!("[{prefix}] {}", cmd.get_program().to_string_lossy());
    for arg in cmd.get_args() {
        text.push(' ');
        text.push_str(&quote(arg));
    }
    line(console, text, false);
}

/// Reads the child's stdout and stderr into the console until it exits.
pub fn capture(console: &Shared, prefix: &'static str, child: &mut Child) {
    if let Some(out) = child.stdout.take() {
        reader(console.clone(), prefix, out, false);
    }
    if let Some(err) = child.stderr.take() {
        reader(console.clone(), prefix, err, true);
    }
}

fn reader(console: Shared, prefix: &'static str, stream: impl Read + Send + 'static, err: bool) {
    let _ = std::thread::Builder::new().name(format!("console-{prefix}")).spawn(move || {
        let mut buf = String::new();
        let mut r = BufReader::new(stream);
        loop {
            buf.clear();
            match r.read_line(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let text = buf.trim_end_matches(['\r', '\n']);
                    console.lock().unwrap().push(&format!("[{prefix}] {text}"), err);
                }
            }
        }
    });
}

/// Follows a log file (Ollama's `server.log`), starting with what it already contains.
pub fn tail(console: Shared, path: PathBuf, prefix: &'static str) {
    let _ = std::thread::Builder::new().name(format!("tail-{prefix}")).spawn(move || {
        let mut offset = 0u64;
        loop {
            if let Ok(meta) = std::fs::metadata(&path) {
                if meta.len() < offset {
                    offset = 0;
                }
                if meta.len() > offset
                    && let Ok(mut file) = std::fs::File::open(&path)
                {
                    let mut text = String::new();
                    if file.seek(SeekFrom::Start(offset)).is_ok() && file.read_to_string(&mut text).is_ok() {
                        offset = meta.len();
                        let mut c = console.lock().unwrap();
                        for l in text.lines() {
                            c.push(&format!("[{prefix}] {l}"), false);
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

fn quote(arg: &OsStr) -> String {
    let s = arg.to_string_lossy();
    if s.is_empty() {
        "\"\"".into()
    } else if s.contains(' ') {
        format!("\"{s}\"")
    } else {
        s.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::Level;

    #[test]
    fn keeps_the_last_lines_and_marks_errors() {
        let c: Shared = Default::default();
        line(&c, "[llama] loading model", false);
        line(&c, "[llama] boom", true);
        let mut c = c.lock().unwrap();
        assert_eq!(c.lines.len(), 2);
        assert_eq!(c.lines[0].text, "[llama] loading model");
        assert_eq!(c.lines[1].level, Level::Error);
        c.lines.clear();
        assert!(c.lines.is_empty());
    }

    #[test]
    fn command_line_is_quoted() {
        let mut cmd = Command::new("llama");
        cmd.args(["serve", "--models-dir", "C:\\my models"]);
        let c: Shared = Default::default();
        command(&c, "llama", &cmd);
        let text = c.lock().unwrap().lines[0].text.clone();
        assert_eq!(text, "[llama] llama serve --models-dir \"C:\\my models\"");
    }

    #[test]
    fn captures_child_output() {
        let c: Shared = Default::default();
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "echo hello"]);
        cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        capture(&c, "test", &mut child);
        child.wait().unwrap();
        for _ in 0..100 {
            if !c.lock().unwrap().lines.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let c = c.lock().unwrap();
        assert!(c.lines.iter().any(|l| l.text == "[test] hello"), "{:?}", c.lines.iter().map(|l| &l.text).collect::<Vec<_>>());
    }
}
