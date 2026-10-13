use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use eframe::egui::{self, RichText};

use super::{App, mmss};
use crate::config::Page;
use crate::flows::FlowId;
use crate::icons;
use crate::highlight;
use crate::jobs::{self, JobHandle, Launch, Level, Line, State};
use crate::sys;
use crate::ui::{self, AMBER, GREEN, RED};

impl App {
    pub(super) fn flow_page(&mut self, ui: &mut egui::Ui, id: FlowId) {
        let Some(root) = self.repo.as_ref().map(|r| r.paths.root.clone()) else { return };
        let (title, blurb) = {
            let f = self.cfg.forms.get(id);
            (f.title(), f.blurb())
        };
        ui.horizontal(|ui| {
            let c = ui.visuals().strong_text_color();
            ui.add(id.icon().image(26.0, c));
            ui.heading(title);
            if self.ready(id) {
                ui::status(ui, icons::CHECK_CIRCLE, GREEN, "ready");
            } else {
                ui::status(ui, icons::ALERT, AMBER, "needs setup");
            }
        });
        ui.label(RichText::new(blurb).weak());
        ui.add_space(4.0);
        self.presets_row(ui, id);
        ui.add_space(6.0);

        let (env, models) = self.missing(&self.needs(id));
        if !env.is_empty() || !models.is_empty() {
            let mut open = false;
            egui::Frame::group(ui.style()).stroke(egui::Stroke::new(1.0, AMBER)).inner_margin(10.0).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui::icon(ui, icons::WRENCH, AMBER);
                    let mut parts = Vec::new();
                    if !env.is_empty() {
                        parts.push(format!("{} setting(s) to fill in", env.len()));
                    }
                    if !models.is_empty() {
                        let r = self.repo.as_ref().expect("repo");
                        parts.push(format!("{} model file(s) to download ({})", models.len(), sys::human(r.models.missing_bytes(&models))));
                    }
                    ui.label(format!("Before the first run: {}.", parts.join(" and ")));
                    open = ui::button(ui, icons::WRENCH, "Set up now").clicked();
                });
            });
            if open {
                self.prompt = Some(id);
            }
            ui.add_space(6.0);
        }

        let job = self.jobs.get(&id).cloned();
        egui::Panel::bottom(egui::Id::new(("run", id))).resizable(true).default_size(280.0).min_size(120.0).show(ui, |ui| {
            ui.add_space(6.0);
            ui::step(ui, 2, "Run");
            self.run_bar(ui, id, job.as_ref());
            ui.add_space(6.0);
            match &job {
                Some(h) => log_view(ui, h),
                None => {
                    ui.label(RichText::new("Press Run to start. Progress and messages of the run appear here.").weak());
                }
            }
        });

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui::step(ui, 1, "Choose what to make");
            ui::card(ui, |ui| self.cfg.forms.get_mut(id).ui(ui, &root));
            ui.add_space(8.0);
        });
    }

    /// Two rows: what the button does (run now / add to the queue), then how this flow's latest run is doing.
    fn run_bar(&mut self, ui: &mut egui::Ui, id: FlowId, job: Option<&JobHandle>) {
        let args = self.cfg.forms.get(id).args();
        let queued = self.queue.iter().filter(|(q, _)| *q == id).count();
        let mut open_queue = false;
        ui.horizontal(|ui| {
            if self.running.is_some() {
                let tip = "only one flow runs at a time - this run waits with the settings as they are now";
                if ui::primary(ui, args.is_ok(), icons::PLAYLIST_PLUS, "Add to queue").on_hover_text(tip).clicked() {
                    self.run(id, true);
                }
            } else {
                let tip = "starts right away; press again to line up more runs";
                if ui::primary(ui, args.is_ok(), icons::PLAY, "Run").on_hover_text(tip).clicked() {
                    self.run(id, false);
                }
                let tip = "line it up without starting - collect several runs, then press Start on the Queue page";
                if ui::button_enabled(ui, args.is_ok(), icons::PLAYLIST_PLUS, "Add to queue").on_hover_text(tip).clicked() {
                    self.run(id, true);
                }
            }
            if let Err(e) = &args {
                ui::status(ui, icons::INFORMATION_OUTLINE, AMBER, e.as_str());
            }
            if queued > 0 {
                open_queue = ui.link(format!("{queued} of these waiting in the queue")).clicked();
            }
            if job.is_none()
                && let Some(r) = &self.repo
            {
                let out = self.cfg.forms.get(id).out_dir(&r.paths.root);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if out.exists() && ui::button(ui, icons::FOLDER_OPEN, "Open results").clicked() {
                        jobs::open_dir(&out);
                    }
                });
            }
        });
        if open_queue {
            self.select(Page::Queue);
        }
        let Some(h) = job else { return };
        let running = h.job.lock().unwrap().running();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(if running { "Running:" } else { "Last run:" }).weak());
            run_status(ui, h, self.cfg.step_times.get(&id), ui.available_width().min(540.0) - 270.0);
            if running && cancel_button(ui) {
                self.ask_cancel(id);
            }
            run_actions(ui, h);
        });
    }

    /// Queues the flow, or opens the setup dialog when env vars or models are missing.
    pub(super) fn run(&mut self, id: FlowId, hold: bool) {
        if !self.ready(id) {
            self.prompt = Some(id);
            return;
        }
        self.enqueue(id, hold);
    }

    /// Snapshots the form into a launch. When idle it starts at once (`hold`: waits in the queue,
    /// which goes on hold until Start); while a run is going it joins the end of the queue.
    pub(super) fn enqueue(&mut self, id: FlowId, hold: bool) {
        let Some(r) = &self.repo else { return };
        let flow = self.cfg.forms.get(id);
        let args = match flow.args() {
            Ok(a) => a,
            Err(e) => return self.notify(e, true),
        };
        let root = r.paths.root.clone();
        let needs = self.needs(id);
        let meta = self.run_meta(id, &needs.models);
        let launch = Launch {
            title: flow.title().to_owned(),
            python: r.paths.python(),
            script: r.paths.script(flow.script()),
            args,
            servers: needs.servers,
            console: flow.console(&root),
            out_dir: flow.out_dir(&root),
            meta,
            root,
        };
        if self.running.is_none() && !hold {
            // Run while idle: starts now, even ahead of a queue on hold
            self.start(id, launch);
        } else {
            self.held |= self.running.is_none();
            self.queue.push_back((id, launch));
            let when = if self.held { "press Start on the Queue page to run it" } else { "starts when its turn comes" };
            self.notify(format!("Added to the queue (position {}) - {when}", self.queue.len()), false);
        }
        self.save_config();
    }

    /// metadata.json content known before the run: flow, form settings, models and LLM setup.
    fn run_meta(&self, id: FlowId, model_ids: &[String]) -> serde_json::Value {
        let Some(r) = &self.repo else { return serde_json::Value::Null };
        let flow = self.cfg.forms.get(id);
        let models: Vec<serde_json::Value> = model_ids
            .iter()
            .map(|m| {
                if let Some(name) = m.strip_prefix("ollama:") {
                    serde_json::json!({ "id": m, "ollama": name })
                } else if let Some(c) = r.models.comfy_model(m) {
                    serde_json::json!({ "id": m, "file": format!("{}/{}", c.dir, c.file), "url": c.url })
                } else if let Some(l) = r.models.local_model(m) {
                    serde_json::json!({ "id": m, "file": format!("{}/{}", l.dir, l.file), "url": l.url })
                } else {
                    serde_json::json!({ "id": m })
                }
            })
            .collect();
        let llm = self.llm();
        let env = |k: &str| Some(r.env.get(k)).filter(|v| !v.trim().is_empty());
        let profile = self.profile();
        let text_model = env("OLLAMA_TEXT_MODEL").or_else(|| env(profile.base_key())).unwrap_or_else(|| profile.default_base().into());
        serde_json::json!({
            "flow": flow.title(),
            "script": flow.script(),
            "settings": self.cfg.forms.settings(id),
            "models": models,
            "llm": {
                "provider": llm.provider.env_value(),
                "openrouter_model": env("OPENROUTER_MODEL"),
                "opencode_model": env("OPENCODE_MODEL"),
                "ollama_vision_model": llm.ollama_model,
                "ollama_text_model": text_model,
                "profile": profile.env_value(),
            },
            "gui_version": env!("CARGO_PKG_VERSION"),
        })
    }

    /// Load / save / delete named settings of this flow's form.
    fn presets_row(&mut self, ui: &mut egui::Ui, id: FlowId) {
        let names: Vec<String> = self.cfg.presets.get(&id).map(|p| p.keys().cloned().collect()).unwrap_or_default();
        let (mut load, mut delete, mut save) = (None, None, false);
        ui.horizontal(|ui| {
            ui.add(icons::BOOKMARK_OUTLINE.image(16.0, ui.visuals().weak_text_color()));
            let shown = if names.is_empty() { "no presets yet".to_owned() } else { format!("Load a preset ({})", names.len()) };
            ui.add_enabled_ui(!names.is_empty(), |ui| {
                egui::ComboBox::from_id_salt(("presets", id)).selected_text(shown).width(200.0).show_ui(ui, |ui| {
                    for n in &names {
                        ui.horizontal(|ui| {
                            if ui::tool(ui, icons::DELETE_OUTLINE, "delete this preset").clicked() {
                                delete = Some(n.clone());
                            }
                            if ui.selectable_label(false, n).on_hover_text("fill the form with these settings").clicked() {
                                load = Some(n.clone());
                            }
                        });
                    }
                });
            });
            let name = self.preset_name.entry(id).or_default();
            let edit = ui.add(egui::TextEdit::singleline(name).hint_text("preset name").desired_width(150.0));
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let exists = names.iter().any(|n| n == name.trim());
            let label = if exists { "Overwrite" } else { "Save preset" };
            let tip = "saves the form as it is now under this name";
            save = (ui::button_enabled(ui, !name.trim().is_empty(), icons::CONTENT_SAVE_OUTLINE, label).on_hover_text(tip).clicked() || enter)
                && !name.trim().is_empty();
        });
        if let Some(n) = load
            && let Some(v) = self.cfg.presets.get(&id).and_then(|p| p.get(&n)).cloned()
        {
            match self.cfg.forms.set_settings(id, v) {
                Ok(()) => {
                    self.preset_name.insert(id, n.clone());
                    self.notify(format!("Loaded preset \"{n}\""), false);
                }
                Err(e) => self.notify(e, true),
            }
        }
        if let Some(n) = delete {
            if let Some(p) = self.cfg.presets.get_mut(&id) {
                p.remove(&n);
            }
            self.notify(format!("Deleted preset \"{n}\""), false);
        }
        if save {
            let name = self.preset_name.get(&id).map(|n| n.trim().to_owned()).unwrap_or_default();
            let settings = self.cfg.forms.settings(id);
            self.cfg.presets.entry(id).or_default().insert(name.clone(), settings);
            self.notify(format!("Saved preset \"{name}\""), false);
        }
    }

    /// Asked when Cancel is clicked, so a run isn't lost to a stray click.
    pub(super) fn confirm_cancel_dialog(&mut self, ctx: &egui::Context) {
        let Some((id, h)) = self.confirm_cancel.clone() else { return };
        let title = self.cfg.forms.get(id).title();
        let mut close = false;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("confirm_cancel")).show(ctx, |ui| {
            ui.set_width(320.0);
            ui.heading(format!("Cancel {title}?"));
            ui.label("Progress made so far is lost.");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::image_and_text(icons::STOP.image(16.0, RED), RichText::new("Cancel the run").color(RED))).clicked()
                {
                    cancel = true;
                }
                if ui.button("Keep running").clicked() {
                    close = true;
                }
            });
        });
        if modal.should_close() {
            close = true;
        }
        if cancel {
            h.cancel();
        }
        // also closes by itself when the run ends while asking
        if cancel || close || !h.job.lock().unwrap().running() {
            self.confirm_cancel = None;
        }
    }
}

pub(super) fn cancel_button(ui: &mut egui::Ui) -> bool {
    ui.add(egui::Button::image_and_text(icons::STOP.image(16.0, RED), RichText::new("Cancel").color(RED))).clicked()
}

/// Progress bar while running, else how the run ended.
pub(super) fn run_status(ui: &mut egui::Ui, h: &JobHandle, history: Option<&HashMap<String, f32>>, width: f32) {
    let mut j = h.job.lock().unwrap();
    let time = mmss(j.elapsed());
    match j.state.clone() {
        State::Running => {
            let frac = j.progress.fraction();
            let eta = j.eta(history);
            let what = match (j.progress.item, j.progress.step) {
                (Some((i, n)), Some((s, m))) => format!("{} {i}/{n} · step {s}/{m}", j.progress.label),
                (Some((i, n)), None) => format!("{} {i}/{n}", j.progress.label),
                (None, Some((s, m))) => format!("step {s}/{m}"),
                (None, None) if j.console => "in the console window".into(),
                (None, None) => j.current_step.clone().unwrap_or_else(|| "working".into()),
            };
            let text = format!("{what} · {time}{}", eta.map(|e| format!(" · about {} left", mmss(e))).unwrap_or_default());
            ui::progress(ui, frac, width.max(160.0), &text, ui::ACCENT);
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }
        State::Finished(0) => {
            ui::status(ui, icons::CHECK_CIRCLE, GREEN, format!("done in {time}"));
        }
        State::Finished(code) => {
            ui::status(ui, icons::ALERT_CIRCLE, RED, format!("failed (exit code {code}) after {time}"))
                .on_hover_text("the flow's log says why");
        }
        State::Failed(e) => {
            ui::status(ui, icons::ALERT_CIRCLE, RED, e);
        }
        State::Cancelled => {
            ui::status(ui, icons::STOP, AMBER, format!("cancelled after {time}"));
        }
    }
}

/// Right-aligned "Open results" and "copy command line" of a run.
pub(super) fn run_actions(ui: &mut egui::Ui, h: &JobHandle) {
    let (out, command) = {
        let j = h.job.lock().unwrap();
        (j.out_dir.clone(), j.command.clone())
    };
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui::button(ui, icons::FOLDER_OPEN, "Open results").clicked() {
            jobs::open_dir(&out);
        }
        if ui::tool(ui, icons::CONTENT_COPY, &format!("copy the command line:\n{command}")).clicked() {
            ui.ctx().copy_text(command);
        }
    });
}

/// Virtualized: only the visible rows are laid out, so long logs stay cheap.
pub(super) fn log_view(ui: &mut egui::Ui, h: &JobHandle) {
    let j = h.job.lock().unwrap();
    let row = ui.text_style_height(&egui::TextStyle::Monospace) + 2.0;
    egui::Frame::NONE.fill(ui.visuals().extreme_bg_color).inner_margin(6.0).corner_radius(4.0).show(ui, |ui| {
        egui::ScrollArea::both().auto_shrink([false, false]).stick_to_bottom(true).show_rows(ui, row, j.lines.len(), |ui, range| {
            for line in j.lines.range(range) {
                log_line(ui, line);
            }
        });
    });
}

/// One log line: ANSI colors, errors in red, warnings in amber, http(s) links and existing file paths clickable.
fn log_line(ui: &mut egui::Ui, line: &Line) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add(egui::Label::new(RichText::new(format!("{} ", line.time)).monospace().weak()));
        let base = match line.level {
            Level::Error => Some(RED),
            Level::Warn => Some(AMBER),
            Level::Info => None,
        };
        let dark = ui.visuals().dark_mode;
        for (text, color, kind, target) in segments(line) {
            let rich = RichText::new(text).monospace();
            // the script's own ANSI colors first, then the tag, then the line's level, then the tinting
            let tint = kind.map(|k| k.color(dark));
            let color = match kind {
                Some(highlight::Kind::Tag(_)) => color.or(tint),
                _ => color.or(base).or(tint),
            };
            let rich = match color {
                Some(c) => rich.color(c),
                None => rich,
            };
            match target {
                Some(Target::Url(url)) => {
                    ui.add(egui::Hyperlink::from_label_and_url(rich.underline(), url));
                }
                Some(Target::Path(p)) => {
                    if ui.add(egui::Label::new(rich.underline()).sense(egui::Sense::click()).extend()).on_hover_text("open").clicked() {
                        jobs::open_dir(Path::new(&p));
                    }
                }
                None => {
                    ui.add(egui::Label::new(rich).extend());
                }
            }
        }
    });
}

enum Target {
    Url(String),
    Path(String),
}

/// Splits a line at color changes, highlighted words and links / Windows paths.
/// A piece of a log line: text, ANSI color, highlight and link target.
type Segment<'a> = (&'a str, Option<egui::Color32>, Option<highlight::Kind>, Option<Target>);

fn segments(line: &Line) -> Vec<Segment<'_>> {
    let text = line.text.as_str();
    // cut points: color range borders and link borders
    let mut links: Vec<(usize, usize, Target)> = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let start = if rest.starts_with("https://") || rest.starts_with("http://") {
            Some(false)
        } else if rest.len() > 3
            && rest.as_bytes()[0].is_ascii_alphabetic()
            // bytes, not &rest[1..3]: that str slice panics when it ends inside a multi-byte char ("mehr 🍌")
            && &rest.as_bytes()[1..3] == b":\\"
            && (i == 0 || text.as_bytes()[i - 1] == b' ')
        {
            Some(true)
        } else {
            None
        };
        if let Some(is_path) = start {
            let len = rest.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | ']' | '>' | ',')).unwrap_or(rest.len());
            let s = rest[..len].trim_end_matches(['.', ':', ';']);
            let target = if is_path { Target::Path(s.to_owned()) } else { Target::Url(s.to_owned()) };
            links.push((i, i + s.len(), target));
            i += s.len().max(1);
        } else {
            i += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    let mut cuts: Vec<usize> = vec![0, text.len()];
    cuts.extend(line.colors.iter().flat_map(|(a, b, _)| [*a, *b]));
    cuts.extend(links.iter().flat_map(|(a, b, _)| [*a, *b]));
    // links keep their own look, in one piece
    let marks: Vec<_> = highlight::log(text).into_iter().filter(|(a, b, _)| !links.iter().any(|(s, e, _)| a < e && s < b)).collect();
    cuts.extend(marks.iter().flat_map(|(a, b, _)| [*a, *b]));
    cuts.sort_unstable();
    cuts.dedup();
    let mut out = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a >= b || !text.is_char_boundary(a) || !text.is_char_boundary(b) {
            continue;
        }
        let color = line.colors.iter().find(|(s, e, _)| *s <= a && b <= *e).map(|(_, _, c)| *c);
        let target = links.iter().find(|(s, e, _)| *s <= a && b <= *e).map(|(_, _, t)| match t {
            Target::Url(u) => Target::Url(u.clone()),
            Target::Path(p) => Target::Path(p.clone()),
        });
        let kind = marks.iter().find(|(s, e, _)| *s <= a && b <= *e).map(|(_, _, k)| *k);
        out.push((&text[a..b], color, kind, target));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_and_paths_are_found() {
        let line = Line {
            time: "00:00:00".into(),
            text: r"[dl   ] https://t.me/budyarchive/123 -> C:\out\a.mp4 done".into(),
            level: Level::Info,
            colors: Vec::new(),
        };
        let segs = segments(&line);
        let urls: Vec<&str> = segs.iter().filter(|(_, _, _, t)| matches!(t, Some(Target::Url(_)))).map(|(s, _, _, _)| *s).collect();
        let paths: Vec<&str> = segs.iter().filter(|(_, _, _, t)| matches!(t, Some(Target::Path(_)))).map(|(s, _, _, _)| *s).collect();
        assert_eq!(urls, ["https://t.me/budyarchive/123"]);
        assert_eq!(paths, [r"C:\out\a.mp4"]);
        assert_eq!(segs.iter().map(|(s, _, _, _)| *s).collect::<String>(), line.text);
    }

    #[test]
    fn multibyte_chars_do_not_panic() {
        for text in ["[chan ] 🍌 Memes und mehr 🍌 #Россия: 4 video(s)", "Abonniere @NYX_Memes für mehr!", "a ü", "x 😂 C:\\out"]
        {
            let line = Line { time: "00:00:00".into(), text: text.into(), level: Level::Info, colors: Vec::new() };
            assert_eq!(segments(&line).iter().map(|(s, _, _, _)| *s).collect::<String>(), text);
        }
    }
}
