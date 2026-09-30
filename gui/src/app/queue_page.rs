use eframe::egui::{self, RichText};

use super::{App, mmss};
use crate::config::Page;
use crate::icons;
use crate::ui::{self, AMBER};

impl App {
    /// Running flow + waiting runs (shown on the Start page).
    pub(super) fn queue_section(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add(icons::PLAY.image(20.0, ui::ACCENT));
            ui.label(RichText::new("Queue").strong().size(16.0));
        });
        ui.label(
            RichText::new(
                "One flow runs at a time so it gets the whole GPU. Everything else waits here and starts automatically, top to bottom.",
            )
            .weak(),
        );
        ui.add_space(8.0);

        ui::card(ui, |ui| {
            ui.label(RichText::new("Running now").strong());
            match self.running_job() {
                Some((id, h)) => {
                    let mut j = h.job.lock().unwrap();
                    let frac = j.progress.fraction();
                    let eta = j.eta();
                    let text = format!(
                        "{} · {}{}",
                        j.title,
                        mmss(j.elapsed()),
                        eta.map(|e| format!(" · about {} left", mmss(e))).unwrap_or_default()
                    );
                    drop(j);
                    let mut open = false;
                    ui.horizontal(|ui| {
                        let c = ui.visuals().text_color();
                        ui.add(id.icon().image(18.0, c));
                        ui.add(egui::ProgressBar::new(frac.unwrap_or(0.0)).desired_width(360.0).text(text).animate(frac.is_none()));
                        open = ui::button(ui, icons::OPEN_IN_NEW, "Show").clicked();
                        if ui::button(ui, icons::STOP, "Cancel").clicked() {
                            h.cancel();
                        }
                    });
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
                    if open {
                        self.select(Page::Flow(id));
                    }
                }
                None => {
                    ui.label(RichText::new("Nothing is running.").weak());
                }
            }
        });
        ui.add_space(8.0);

        ui::card(ui, |ui| {
            ui.label(RichText::new(format!("Waiting ({})", self.queue.len())).strong());
            if self.queue.is_empty() {
                ui.label(RichText::new("Press Run on a flow while another one runs to line it up here.").weak());
                return;
            }
            let (mut up, mut remove) = (None, None);
            for (i, (id, launch)) in self.queue.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("{}.", i + 1));
                    let c = ui.visuals().text_color();
                    ui.add(id.icon().image(18.0, c));
                    ui.label(&launch.title);
                    ui.label(RichText::new(launch.args.join(" ")).small().weak()).on_hover_text(launch.args.join("\n"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui::tool(ui, icons::DELETE_OUTLINE, "remove from the queue").clicked() {
                            remove = Some(i);
                        }
                        if i > 0 && ui::tool(ui, icons::ARROW_UP, "move up").clicked() {
                            up = Some(i);
                        }
                    });
                });
            }
            if let Some(i) = up {
                self.queue.swap(i, i - 1);
            }
            if let Some(i) = remove {
                self.queue.remove(i);
            }
            if ui.add(egui::Button::image_and_text(icons::DELETE_OUTLINE.image(16.0, AMBER), "Clear the queue")).clicked() {
                self.queue.clear();
            }
        });
    }
}
