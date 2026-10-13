use std::sync::Arc;

use eframe::egui::{self, RichText};

use super::App;
use super::flow_page::{cancel_button, log_view, run_actions, run_status};
use crate::config::Page;
use crate::flows::FlowId;
use crate::highlight;
use crate::icons;
use crate::jobs::JobHandle;
use crate::ui::{self, AMBER};

impl App {
    /// Queue page: the running run, the waiting ones and what ran this session.
    pub(super) fn queue_page(&mut self, ui: &mut egui::Ui) {
        let mut open = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.heading("Queue");
            ui.label(
                RichText::new(
                    "One flow runs at a time so it gets the whole GPU. Everything else waits here and starts automatically, top to bottom. \
                     Each waiting run keeps the settings it was added with.",
                )
                .weak(),
            );
            ui.add_space(10.0);
            self.running_card(ui, &mut open);
            ui.add_space(8.0);
            self.waiting_card(ui, &mut open);
            ui.add_space(8.0);
            self.finished_card(ui, &mut open);
        });
        if let Some(id) = open {
            self.select(Page::Flow(id));
        }
    }

    fn running_card(&mut self, ui: &mut egui::Ui, open: &mut Option<FlowId>) {
        ui::card(ui, |ui| {
            ui.label(RichText::new("Running now").strong());
            let Some((id, h)) = self.running_job().map(|(id, h)| (id, h.clone())) else {
                ui.label(RichText::new("Nothing is running.").weak());
                return;
            };
            entry(ui, |ui| {
                ui.horizontal(|ui| {
                    self.job_link(ui, id, &h, open);
                    run_status(ui, &h, self.cfg.step_times.get(&id), ui.available_width() - 300.0);
                    if cancel_button(ui) {
                        self.ask_cancel(id);
                    }
                    run_actions(ui, &h);
                });
                args_line(ui, &h.job.lock().unwrap().args);
                self.job_log(ui, &h);
            });
        });
    }

    /// Flow icon + title of a started run: the title shows/hides its log, the icon opens the flow.
    fn job_link(&mut self, ui: &mut egui::Ui, id: FlowId, h: &JobHandle, open: &mut Option<FlowId>) {
        let key = Arc::as_ptr(&h.job) as usize;
        let shown = self.queue_open == Some(key);
        let c = ui.visuals().text_color();
        if ui.add(egui::Button::image(id.icon().image(18.0, c)).frame(false)).on_hover_text("open the flow").clicked() {
            *open = Some(id);
        }
        let title = h.job.lock().unwrap().title.clone();
        let arrow = if shown { "▾" } else { "▸" };
        if ui.link(format!("{arrow} {title}")).on_hover_text(if shown { "hide the log" } else { "show the log" }).clicked() {
            self.queue_open = if shown { None } else { Some(key) };
        }
    }

    /// The run's log under its row, when opened with `job_link`.
    fn job_log(&self, ui: &mut egui::Ui, h: &JobHandle) {
        let key = Arc::as_ptr(&h.job) as usize;
        if self.queue_open != Some(key) {
            return;
        }
        ui.push_id(key, |ui| {
            ui.allocate_ui(egui::vec2(ui.available_width(), 280.0), |ui| log_view(ui, h));
        });
        ui.add_space(6.0);
    }

    fn waiting_card(&mut self, ui: &mut egui::Ui, open: &mut Option<FlowId>) {
        ui::card(ui, |ui| {
            ui.label(RichText::new(format!("Waiting ({})", self.queue.len())).strong());
            if self.queue.is_empty() {
                ui.label(RichText::new("Press Add to queue on any flow - also the running one - to line up more runs here.").weak());
                return;
            }
            ui.horizontal(|ui| {
                if self.held {
                    let tip = "runs the waiting runs top to bottom, one after the other";
                    if ui::primary(ui, true, icons::PLAY, "Start").on_hover_text(tip).clicked() {
                        self.held = false;
                    }
                    let when = if self.running.is_some() { "after the current run" } else { "until you press Start" };
                    ui::status(ui, icons::PAUSE, AMBER, format!("On hold - nothing more starts {when}"));
                } else if self.running.is_some() {
                    let tip = "the current run finishes; the waiting ones stay until you press Start";
                    if ui::button(ui, icons::PAUSE, "Hold after this run").on_hover_text(tip).clicked() {
                        self.held = true;
                    }
                }
            });
            ui.add_space(4.0);
            let (mut up, mut down, mut remove) = (None, None, None);
            let last = self.queue.len() - 1;
            for (i, (id, launch)) in self.queue.iter().enumerate() {
                entry(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{}.", i + 1)).weak());
                        flow_link(ui, *id, &launch.title, open);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui::tool(ui, icons::DELETE_OUTLINE, "remove from the queue").clicked() {
                                remove = Some(i);
                            }
                            if i < last && ui::tool(ui, icons::ARROW_DOWN, "move down").clicked() {
                                down = Some(i);
                            }
                            if i > 0 && ui::tool(ui, icons::ARROW_UP, "move up").clicked() {
                                up = Some(i);
                            }
                        });
                    });
                    args_line(ui, &launch.args);
                });
            }
            if let Some(i) = up {
                self.queue.swap(i, i - 1);
            }
            if let Some(i) = down {
                self.queue.swap(i, i + 1);
            }
            if let Some(i) = remove {
                self.queue.remove(i);
            }
            ui.add_space(4.0);
            if ui.add(egui::Button::image_and_text(icons::DELETE_OUTLINE.image(16.0, AMBER), "Clear the queue")).clicked() {
                self.queue.clear();
            }
        });
    }

    fn finished_card(&mut self, ui: &mut egui::Ui, open: &mut Option<FlowId>) {
        let done: Vec<_> = self.recent.iter().filter(|(_, h)| !h.job.lock().unwrap().running()).cloned().collect();
        ui::card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(icons::HISTORY.image(16.0, ui.visuals().text_color()));
                ui.label(RichText::new("Finished").strong());
            });
            if done.is_empty() {
                ui.label(RichText::new("Runs that ended since the app was started show up here.").weak());
                return;
            }
            for (id, h) in &done {
                entry(ui, |ui| {
                    ui.horizontal(|ui| {
                        self.job_link(ui, *id, h, open);
                        run_status(ui, h, self.cfg.step_times.get(id), 0.0);
                        run_actions(ui, h);
                    });
                    args_line(ui, &h.job.lock().unwrap().args);
                    self.job_log(ui, h);
                });
            }
            ui.add_space(4.0);
            if ui::button(ui, icons::DELETE_OUTLINE, "Clear the list").clicked() {
                self.recent.retain(|(_, h)| h.job.lock().unwrap().running());
            }
        });
    }

    /// Short queue line for the Start page; the details are on the Queue page.
    pub(super) fn queue_summary(&mut self, ui: &mut egui::Ui) {
        if self.running.is_none() && self.queue.is_empty() {
            return;
        }
        let mut go = false;
        ui::card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(icons::PLAYLIST_PLAY.image(20.0, ui::ACCENT));
                if let Some((id, h)) = self.running_job() {
                    let title = h.job.lock().unwrap().title.clone();
                    ui.label(RichText::new(format!("Running: {title}")).strong());
                    run_status(ui, h, self.cfg.step_times.get(&id), 260.0);
                }
                if !self.queue.is_empty() {
                    ui.label(format!("{} waiting{}", self.queue.len(), if self.held { " (on hold)" } else { "" }));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    go = ui::button(ui, icons::OPEN_IN_NEW, "Open queue").clicked();
                });
            });
        });
        ui.add_space(10.0);
        if go {
            self.select(Page::Queue);
        }
    }
}

/// One queue entry: a softly filled box, full width.
fn entry(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new().fill(ui.visuals().faint_bg_color).corner_radius(6.0).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
    ui.add_space(4.0);
}

/// The run's settings as colored arguments, wrapped to at most two lines (all of them on hover).
fn args_line(ui: &mut egui::Ui, args: &[String]) {
    if args.is_empty() {
        return;
    }
    let job = highlight::args_job(args, ui.available_width(), 2, ui.visuals().dark_mode, egui::FontId::monospace(11.5));
    ui.add(egui::Label::new(job)).on_hover_text(args.join("\n"));
}

/// Flow icon + title, click opens the flow's page.
fn flow_link(ui: &mut egui::Ui, id: FlowId, title: &str, open: &mut Option<FlowId>) {
    let c = ui.visuals().text_color();
    ui.add(id.icon().image(18.0, c));
    if ui.link(title).on_hover_text("open the flow").clicked() {
        *open = Some(id);
    }
}
