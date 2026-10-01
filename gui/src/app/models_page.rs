use std::collections::BTreeSet;
use std::time::Duration;

use eframe::egui::{self, RichText};

use super::{App, status_text};
use crate::flows::FlowId;
use crate::icons;
use crate::models::{Dl, Models, Share, Status, Task, ollama_id};
use crate::paths;
use crate::servers::{self, Server};
use crate::sys;
use crate::ui::{self, AMBER, GREEN, RED};

impl App {
    pub(super) fn models_page(&mut self, ui: &mut egui::Ui) {
        let Some(models) = self.repo.as_ref().map(|r| r.models.clone()) else { return };
        ui.heading("Models");
        ui.label(RichText::new("Every model file in one place. Nothing is downloaded or stored twice: finished files are marked, files that already exist under another name - or as an Ollama model with the same bytes - are hard-linked instead of copied.").weak());
        ui.add_space(6.0);

        let dir = self.models_dir();
        ui::card(ui, |ui| {
            ui.horizontal(|ui| {
                let c = ui.visuals().text_color();
                ui::icon(ui, icons::HARDDISK, c);
                ui.label("Models folder:");
                if ui.link(dir.display().to_string()).on_hover_text("open the folder").clicked() {
                    sys::open(&dir);
                }
                if let Some(free) = sys::free_space(&dir) {
                    ui.label(RichText::new(format!("{} free", sys::human(free))).weak());
                }
            });
            ui.label(
                RichText::new("ComfyUI, Ollama, Hugging Face, torch, rembg and Kokoro all store their models here. Change it under Settings > Model storage.")
                    .small()
                    .weak(),
            );
        });
        ui.add_space(6.0);

        // everything the flows need with the current settings
        let needed: Vec<String> = FlowId::ALL.iter().flat_map(|id| self.needs(*id).models).collect::<BTreeSet<_>>().into_iter().collect();
        let missing: Vec<String> = needed.iter().filter(|m| !models.status(m).usable()).cloned().collect();
        let dup = models.duplicate_bytes();
        ui.horizontal(|ui| {
            let bytes = models.missing_bytes(&missing);
            if ui::button_enabled(
                ui,
                !missing.is_empty(),
                icons::DOWNLOAD,
                &format!("Download everything the flows need ({})", sys::human(bytes)),
            )
            .clicked()
            {
                for m in &missing {
                    models.enqueue(m, Task::Download);
                }
            }
            if dup > 0
                && ui::button(ui, icons::SHARE_VARIANT, &format!("Share duplicates with Ollama (frees {})", sys::human(dup))).clicked()
            {
                for c in models.comfy.iter() {
                    if matches!(models.share(&c.id), Some(Share::Duplicate { .. })) {
                        models.enqueue(&c.id, Task::Share);
                    }
                }
            }
            if ui::button(ui, icons::REFRESH, "Check again").clicked() {
                models.rescan();
            }
        });
        self.move_panel(ui, &models);
        ui.add_space(6.0);

        let active = models.busy();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let mut groups: Vec<&str> = Vec::new();
            for m in models.comfy.iter() {
                if !groups.contains(&m.group.as_str()) {
                    groups.push(&m.group);
                }
            }
            for group in groups {
                egui::CollapsingHeader::new(RichText::new(format!("ComfyUI · {group}")).strong())
                    .default_open(!group.starts_with("Extra"))
                    .show(ui, |ui| {
                        egui::Grid::new(("g", group)).num_columns(4).striped(true).spacing([12.0, 4.0]).min_col_width(70.0).show(
                            ui,
                            |ui| {
                                for m in models.comfy.iter().filter(|m| m.group == group) {
                                    ui.label(&m.file).on_hover_text(format!(
                                        "{}/{}\nused by: {}{}",
                                        m.dir,
                                        m.file,
                                        m.used_by,
                                        if m.note.is_empty() { String::new() } else { format!("\n{}", m.note) }
                                    ));
                                    ui.label(m.size.map(sys::human).unwrap_or_else(|| "?".into()));
                                    row_status(ui, &models, &m.id, &m.note);
                                    ui.end_row();
                                }
                            },
                        );
                    });
            }
            let mut ollama_groups: Vec<&str> = Vec::new();
            for m in models.ollama.iter() {
                if !ollama_groups.contains(&m.group.as_str()) {
                    ollama_groups.push(&m.group);
                }
            }
            for group in ollama_groups {
                egui::CollapsingHeader::new(RichText::new(format!("Ollama · {group}")).strong())
                    .default_open(!group.starts_with("Coding"))
                    .show(ui, |ui| {
                        egui::Grid::new(("o", group)).num_columns(4).striped(true).spacing([12.0, 4.0]).min_col_width(70.0).show(
                            ui,
                            |ui| {
                                for m in models.ollama.iter().filter(|m| m.group == group) {
                                    ui.label(&m.model).on_hover_text(format!("used by: {}", m.used_by));
                                    ui.label(m.size.map(|s| format!("~{}", sys::human(s))).unwrap_or_else(|| "?".into()));
                                    row_status(ui, &models, &ollama_id(&m.model), "");
                                    ui.end_row();
                                }
                            },
                        );
                    });
            }
            let mut local_groups: Vec<&str> = Vec::new();
            for m in models.local.iter() {
                if !local_groups.contains(&m.group.as_str()) {
                    local_groups.push(&m.group);
                }
            }
            for group in local_groups {
                egui::CollapsingHeader::new(RichText::new(group).strong()).default_open(true).show(ui, |ui| {
                    egui::Grid::new(("l", group)).num_columns(4).striped(true).spacing([12.0, 4.0]).min_col_width(70.0).show(ui, |ui| {
                        for m in models.local.iter().filter(|m| m.group == group) {
                            ui.label(&m.file).on_hover_text(format!("{}/{}\nused by: {}", m.dir, m.file, m.used_by));
                            ui.label(m.size.map(sys::human).unwrap_or_else(|| "?".into()));
                            row_status(ui, &models, &m.id, "");
                            ui.end_row();
                        }
                    });
                });
            }
        });
        if active {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
    }

    /// Offers to move files into the configured folders, and shows the move while it runs.
    fn move_panel(&mut self, ui: &mut egui::Ui, models: &Models) {
        if let Some(mv) = models.moving() {
            ui::card(ui, |ui| {
                if mv.finished {
                    let (icon, color, text) = if mv.errors.is_empty() {
                        (icons::CHECK_CIRCLE, GREEN, "Models moved.".to_owned())
                    } else {
                        (icons::ALERT, AMBER, format!("Moved, {} file(s) could not be moved:", mv.errors.len()))
                    };
                    ui::status(ui, icon, color, text);
                    for e in &mv.errors {
                        ui.label(RichText::new(e).small().color(RED));
                    }
                    if ui::button(ui, icons::CLOSE, "OK").clicked() {
                        models.clear_move();
                        if mv.errors.is_empty() {
                            self.cfg.ollama_prev = None;
                            self.cfg.models_prev = None;
                            self.configure_models();
                        }
                    }
                } else {
                    let frac = mv.done as f32 / mv.total.max(1) as f32;
                    ui.add(egui::ProgressBar::new(frac).text(format!(
                        "moving {} · {} / {}",
                        mv.file,
                        sys::human(mv.done),
                        sys::human(mv.total)
                    )));
                    ui.ctx().request_repaint_after(Duration::from_millis(250));
                }
            });
            return;
        }
        let plan = models.move_plan();
        if plan.is_empty() {
            return;
        }
        let bytes: u64 = plan.iter().map(|m| m.size).sum();
        let mut labels: Vec<(&str, u64)> = Vec::new();
        for m in &plan {
            match labels.iter_mut().find(|(l, _)| *l == m.label) {
                Some((_, b)) => *b += m.size,
                None => labels.push((m.label, m.size)),
            }
        }
        let ollama_dir = self.models_dir().join(paths::OLLAMA);
        let moves_ollama = plan.iter().any(|m| m.to.starts_with(&ollama_dir));
        ui::card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui::icon(ui, icons::FOLDER_MOVE, AMBER);
                let from: Vec<String> = labels.iter().map(|(l, b)| format!("{l} {}", sys::human(*b))).collect();
                ui.label(format!(
                    "{} file(s) ({}) are still outside the models folder: {}.",
                    plan.len(),
                    sys::human(bytes),
                    from.join(", ")
                ))
                .on_hover_text("Hugging Face and torch: only their model caches move (other programs download again if they need them)");
                let running = (moves_ollama && self.servers.up(Server::Ollama)) || self.servers.up(Server::Comfy);
                if running {
                    ui.label(RichText::new("Stop ComfyUI and Ollama first:").color(AMBER));
                    if self.servers.up(Server::Ollama) && ui::button(ui, icons::POWER, "Stop Ollama").clicked() {
                        servers::stop_ollama();
                    }
                }
                if ui::button_enabled(ui, !running, icons::FOLDER_MOVE, "Move them")
                    .on_hover_text("instant on the same drive, otherwise copied and then deleted")
                    .clicked()
                {
                    models.start_move(plan);
                }
            });
        });
    }
}

fn row_status(ui: &mut egui::Ui, models: &Models, id: &str, note: &str) {
    match models.download(id) {
        Some(Dl::Queued) => {
            ui::status(ui, icons::TIMER_SAND, egui::Color32::GRAY, "waiting");
            if ui::tool(ui, icons::CLOSE, "cancel").clicked() {
                models.cancel(id);
            }
        }
        Some(Dl::Running { done, total, rate, verb }) => {
            let text = match total {
                Some(t) if rate > 0.0 => {
                    let eta = (t.saturating_sub(done) as f64 / rate) as u64;
                    format!(
                        "{verb} {} / {} · {}/s · {}:{:02} left",
                        sys::human(done),
                        sys::human(t),
                        sys::human(rate as u64),
                        eta / 60,
                        eta % 60
                    )
                }
                Some(t) => format!("{verb} {} / {}", sys::human(done), sys::human(t)),
                None => format!("{verb} {}", sys::human(done)),
            };
            let frac = total.map_or(0.0, |t| done as f32 / t.max(1) as f32);
            ui.add(egui::ProgressBar::new(frac).desired_width(320.0).text(text));
            if ui::tool(ui, icons::CLOSE, "cancel (resumes later)").clicked() {
                models.cancel(id);
            }
        }
        Some(Dl::Failed(e)) => {
            ui::status(ui, icons::ALERT_CIRCLE, RED, "failed").on_hover_text(e.as_str());
            ui.horizontal(|ui| {
                if ui::button(ui, icons::REFRESH, "Retry").clicked() {
                    models.dismiss(id);
                    models.enqueue(id, Task::Download);
                }
                if ui::tool(ui, icons::CLOSE, "dismiss").clicked() {
                    models.dismiss(id);
                }
                ui.label(RichText::new(e).small().color(RED));
            });
        }
        None => {
            let status = models.status(id);
            let (icon, text, color) = status_text(&status);
            ui.horizontal(|ui| {
                ui::status(ui, icon, color, text);
                match models.share(id) {
                    Some(Share::Shared(with)) => {
                        ui::icon(ui, icons::LINK, GREEN).on_hover_text(format!("one copy on disk, shared with {with}"));
                    }
                    Some(Share::Duplicate { with, .. }) => {
                        ui::icon(ui, icons::CONTENT_COPY, AMBER)
                            .on_hover_text(format!("the same bytes are stored a second time by {with}"));
                    }
                    None => {}
                }
            });
            ui.horizontal(|ui| match &status {
                Status::Present(p) | Status::SizeDiffers { path: p, .. } => {
                    if ui::tool(ui, icons::FOLDER_OPEN, "show in Explorer").clicked() {
                        crate::jobs::open_dir(p);
                    }
                    if matches!(models.share(id), Some(Share::Duplicate { .. }))
                        && ui::button(ui, icons::SHARE_VARIANT, "Share")
                            .on_hover_text("verify the bytes, then keep one copy for both tools")
                            .clicked()
                    {
                        models.enqueue(id, Task::Share);
                    }
                    if let Status::SizeDiffers { .. } = status {
                        if ui::button(ui, icons::DOWNLOAD, "Resume").on_hover_text("continue it as an unfinished download").clicked() {
                            models.enqueue(id, Task::Download);
                        }
                        if ui::button(ui, icons::SWAP_HORIZONTAL, "Replace").on_hover_text("download the listed file over it").clicked() {
                            models.enqueue(id, Task::Replace);
                        }
                    }
                }
                Status::Linkable(_) => {
                    if ui::button(ui, icons::LINK_VARIANT, "Link").clicked() {
                        models.enqueue(id, Task::Download);
                    }
                }
                Status::Missing { partial } => {
                    if ui::button(ui, icons::DOWNLOAD, if *partial > 0 { "Resume" } else { "Download" }).clicked() {
                        models.enqueue(id, Task::Download);
                    }
                }
                Status::Manual => {
                    ui.label(RichText::new(note).small().color(AMBER));
                }
                Status::Unknown => {}
            });
        }
    }
}
