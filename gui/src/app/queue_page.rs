use eframe::egui::{self, RichText};

use super::App;
use super::flow_page::{cancel_button, run_actions, run_status};
use crate::config::Page;
use crate::flows::FlowId;
use crate::icons;
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
            ui.horizontal(|ui| {
                flow_link(ui, id, &h.job.lock().unwrap().title, open);
                run_status(ui, &h, ui.available_width() - 300.0);
                if cancel_button(ui) {
                    self.ask_cancel(id);
                }
                run_actions(ui, &h);
            });
        });
    }

    fn waiting_card(&mut self, ui: &mut egui::Ui, open: &mut Option<FlowId>) {
        ui::card(ui, |ui| {
            ui.label(RichText::new(format!("Waiting ({})", self.queue.len())).strong());
            if self.queue.is_empty() {
                ui.label(RichText::new("Press Add to queue on any flow - also the running one - to line up more runs here.").weak());
                return;
            }
            let (mut up, mut down, mut remove) = (None, None, None);
            let last = self.queue.len() - 1;
            for (i, (id, launch)) in self.queue.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("{}.", i + 1));
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
                        let args = launch.args.join(" ");
                        ui.add(egui::Label::new(RichText::new(&args).small().weak()).truncate()).on_hover_text(launch.args.join("\n"));
                    });
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
                ui.horizontal(|ui| {
                    flow_link(ui, *id, &h.job.lock().unwrap().title, open);
                    run_status(ui, h, 0.0);
                    run_actions(ui, h);
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
                if let Some((_, h)) = self.running_job() {
                    let title = h.job.lock().unwrap().title.clone();
                    ui.label(RichText::new(format!("Running: {title}")).strong());
                    run_status(ui, h, 260.0);
                }
                if !self.queue.is_empty() {
                    ui.label(format!("{} waiting", self.queue.len()));
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

/// Flow icon + title, click opens the flow's page.
fn flow_link(ui: &mut egui::Ui, id: FlowId, title: &str, open: &mut Option<FlowId>) {
    let c = ui.visuals().text_color();
    ui.add(id.icon().image(18.0, c));
    if ui.link(title).on_hover_text("open the flow").clicked() {
        *open = Some(id);
    }
}
