//! Light syntax coloring for log lines and command-line arguments: `[tag]` prefixes get a stable
//! color of their own, `--flags` and their values contrast, numbers, quotes and "done" words are
//! tinted. Byte ranges only; the caller decides what wins over ANSI colors and error/warning lines.

use eframe::egui::{self, Color32};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    /// `[tag]` at the start of a script's log line, colored by its name.
    Tag(u32),
    Flag,
    Value,
    /// The interpreter / script part of a command line.
    Program,
    Number,
    Quote,
    Good,
}

impl Kind {
    pub fn color(self, dark: bool) -> Color32 {
        let rgb = |d: (u8, u8, u8), l: (u8, u8, u8)| {
            let (r, g, b) = if dark { d } else { l };
            Color32::from_rgb(r, g, b)
        };
        match self {
            Kind::Tag(hash) => {
                let hue = (hash % 360) as f32 / 360.0;
                egui::ecolor::Hsva::new(hue, if dark { 0.5 } else { 0.75 }, if dark { 1.0 } else { 0.62 }, 1.0).into()
            }
            Kind::Flag => rgb((120, 170, 255), (30, 90, 200)),
            Kind::Value => rgb((235, 200, 130), (150, 95, 10)),
            Kind::Program => rgb((140, 140, 150), (120, 120, 130)),
            Kind::Number => rgb((120, 215, 215), (0, 120, 130)),
            Kind::Quote => rgb((170, 220, 140), (50, 130, 40)),
            Kind::Good => rgb((110, 210, 130), (20, 140, 60)),
        }
    }
}

/// FNV-1a: the same tag always gets the same color, across runs.
fn hash(s: &str) -> u32 {
    s.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193))
}

/// Ranges to color in a log line.
pub fn log(text: &str) -> Vec<(usize, usize, Kind)> {
    let mut out = Vec::new();
    let mut body = 0;
    if text.starts_with('[')
        && let Some(end) = text.find(']')
    {
        let tag = text[1..end].trim();
        out.push((0, end + 1, Kind::Tag(hash(tag))));
        body = end + 1;
        // the GUI logs the command line it runs as "[input] <python> -s <script> <args>"
        if tag == "input" {
            out.extend(args_in(text, body));
            return out;
        }
    }
    words(text, body, &mut out);
    out
}

/// Numbers (with units/fractions/percent), "quoted text" and success words.
fn words(text: &str, from: usize, out: &mut Vec<(usize, usize, Kind)>) {
    let bytes = text.as_bytes();
    let mut i = from;
    while i < text.len() {
        let c = bytes[i];
        let prev_word = i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
        if c == b'"' {
            if let Some(len) = text[i + 1..].find('"') {
                out.push((i, i + len + 2, Kind::Quote));
                i += len + 2;
                continue;
            }
        } else if c.is_ascii_digit() && !prev_word {
            let mut j = i;
            while j < text.len() && (bytes[j].is_ascii_digit() || (matches!(bytes[j], b'.' | b'/' | b':') && j + 1 < text.len() && bytes[j + 1].is_ascii_digit())) {
                j += 1;
            }
            if j < text.len() && bytes[j] == b'%' {
                j += 1;
            }
            // a number glued to letters ("x264", "3d") is part of a word, not a value
            if j >= text.len() || !bytes[j].is_ascii_alphabetic() || matches!(text[j..].split(|c: char| !c.is_ascii_alphabetic()).next(), Some("s" | "ms" | "x" | "px" | "fps" | "MB" | "GB" | "KB" | "min")) {
                out.push((i, j, Kind::Number));
            }
            i = j.max(i + 1);
            continue;
        } else if c.is_ascii_alphabetic() && !prev_word {
            let len = text[i..].find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(text.len() - i);
            if matches!(&text[i..i + len], "done" | "Done" | "DONE" | "saved" | "Saved" | "finished" | "ready" | "ok" | "OK" | "success") {
                out.push((i, i + len, Kind::Good));
            }
            i += len.max(1);
            continue;
        }
        i += text[i..].chars().next().map_or(1, char::len_utf8);
    }
}

/// A shell-like command line from byte `from` on: program/script dim, flags and values contrasting.
fn args_in(text: &str, from: usize) -> Vec<(usize, usize, Kind)> {
    let mut out = Vec::new();
    let mut seen_script = false;
    let mut after_flag = false;
    for (a, b) in tokens(text, from) {
        let tok = &text[a..b];
        let kind = if !seen_script {
            seen_script = tok.trim_matches('"').ends_with(".py");
            Kind::Program
        } else if tok.starts_with("--") {
            after_flag = true;
            Kind::Flag
        } else if after_flag {
            after_flag = false;
            Kind::Value
        } else {
            Kind::Quote // positional argument (input file, topic ...)
        };
        out.push((a, b, kind));
    }
    out
}

/// Whitespace-separated tokens, a double-quoted run counting as one.
fn tokens(text: &str, from: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    let mut quoted = false;
    for (i, c) in text[from..].char_indices().map(|(i, c)| (i + from, c)) {
        if c == '"' {
            quoted = !quoted;
        }
        match (c.is_whitespace() && !quoted, start) {
            (true, Some(s)) => {
                out.push((s, i));
                start = None;
            }
            (false, None) => start = Some(i),
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, text.len()));
    }
    out
}

/// Arguments of a run colored (flags, values, positional inputs), wrapped at `width` into at most `rows`
/// lines, the rest cut off with "…".
pub fn args_job(args: &[String], width: f32, rows: usize, dark: bool, font: egui::FontId) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let mut after_flag = false;
    for (i, a) in args.iter().enumerate() {
        let kind = if a.starts_with("--") {
            after_flag = true;
            Kind::Flag
        } else if std::mem::take(&mut after_flag) {
            Kind::Value
        } else {
            Kind::Quote
        };
        let shown = if a.contains(' ') { format!("\"{a}\"") } else { a.clone() };
        let sep = if i == 0 { "" } else { " " };
        job.append(sep, 0.0, egui::TextFormat::simple(font.clone(), Color32::GRAY));
        job.append(&shown, 0.0, egui::TextFormat::simple(font.clone(), kind.color(dark)));
    }
    job.wrap = egui::text::TextWrapping { max_width: width, max_rows: rows, break_anywhere: true, overflow_character: Some('…') };
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(&str, Kind)> {
        log(text).into_iter().map(|(a, b, k)| (&text[a..b], k)).collect()
    }

    #[test]
    fn tags_numbers_quotes() {
        let k = kinds(r#"[render] frame 12/240 at 30 fps, "Trump" done"#);
        assert!(matches!(k[0], ("[render]", Kind::Tag(_))));
        assert!(k.contains(&("12/240", Kind::Number)));
        assert!(k.contains(&("30", Kind::Number)));
        assert!(k.contains(&("\"Trump\"", Kind::Quote)));
        assert!(k.contains(&("done", Kind::Good)));
        assert!(!kinds("x264 v3d").iter().any(|(_, k)| *k == Kind::Number));
    }

    #[test]
    fn same_tag_same_color() {
        assert_eq!(kinds("[cast ] a")[0].1, kinds("[cast] b")[0].1);
    }

    #[test]
    fn input_line_is_a_command() {
        let k = kinds(r#"[input] C:\py\python.exe -s C:\r\comedy\make_comedy.py --topic "a b" --preview photo.jpg"#);
        assert!(k.contains(&(r"C:\r\comedy\make_comedy.py", Kind::Program)));
        assert!(k.contains(&("--topic", Kind::Flag)));
        assert!(k.contains(&("\"a b\"", Kind::Value)));
        assert!(k.contains(&("--preview", Kind::Flag)));
    }

    #[test]
    fn multibyte_text_is_safe() {
        for t in ["[chan ] 🍌 Memes 4 видео \"ü\"", "ü1", "1ü", "\"🍌", "[x"] {
            for (a, b, _) in log(t) {
                assert!(t.is_char_boundary(a) && t.is_char_boundary(b), "{t}");
            }
        }
    }
}
