use std::sync::{Arc, Mutex};

use eframe::egui::{self, RichText};

use super::{App, or_default};
use crate::coding::{self, Editor, Profile};
use crate::config::Page;
use crate::icons;
use crate::ui::{self, AMBER, GREEN, RED};

/// What the Coding page knows about Ollama, filled in by background threads.
#[derive(Default)]
pub struct CodingState {
    /// Models Ollama has (None until listed).
    pub installed: Option<Vec<String>>,
    /// The profile being set up right now.
    pub busy: Option<Profile>,
    pub last: Option<Result<String, String>>,
}

pub type Shared = Arc<Mutex<CodingState>>;

impl App {
    pub(super) fn profile(&self) -> Profile {
        self.repo.as_ref().map_or(Profile::Speed, |r| Profile::from_env(&r.env.get("LLM_LOCAL_PROFILE")))
    }

    pub(super) fn set_profile(&mut self, p: Profile) {
        if p != self.profile() {
            self.save_env(&[("LLM_LOCAL_PROFILE", p.env_value().into())]);
        }
    }

    /// Speed / Quality segmented toggle (top bar and Coding page).
    pub(super) fn profile_toggle(&mut self, ui: &mut egui::Ui) {
        let current = self.profile();
        let mut pick = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for p in Profile::ALL {
                let icon = if p == Profile::Speed { icons::ROCKET_LAUNCH } else { icons::DIAMOND_STONE };
                let on = p == current;
                let color = if on { egui::Color32::WHITE } else { ui.visuals().text_color() };
                let b = egui::Button::image_and_text(icon.image(15.0, color), RichText::new(p.label()).color(color))
                    .fill(if on { ui::ACCENT } else { ui.visuals().widgets.inactive.weak_bg_fill })
                    .corner_radius(if p == Profile::Speed { egui::CornerRadius { nw: 6, sw: 6, ne: 0, se: 0 } } else { egui::CornerRadius { nw: 0, sw: 0, ne: 6, se: 6 } });
                let tip = format!("local LLM tuned for {}: {}", p.label().to_lowercase(), p.summary());
                if ui.add(b).on_hover_text(tip).clicked() {
                    pick = Some(p);
                }
            }
        });
        if let Some(p) = pick {
            self.set_profile(p);
        }
    }

    pub(super) fn coding_page(&mut self, ui: &mut egui::Ui) {
        if self.coding.lock().unwrap().installed.is_none() {
            self.list_ollama_models();
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.heading("Coding");
            ui.label(
                RichText::new(
                    "A local model for T3 Code, OpenCode and Pi, built from the models you already have - nothing is downloaded. \
                     The same toggle also decides which model the flows use for local text prompts.",
                )
                .weak(),
            );
            ui.add_space(10.0);
            ui::card(ui, |ui| self.profile_card(ui));
            ui.add_space(8.0);
            ui::card(ui, |ui| self.models_card(ui));
            ui.add_space(8.0);
            ui::card(ui, |ui| self.editors_card(ui));
        });
    }

    fn profile_card(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Tuned for").strong());
        self.profile_toggle(ui);
        ui.add_space(4.0);
        let p = self.profile();
        ui.label(p.summary());
        ui.label(
            RichText::new(format!(
                "Model {}, context {}k, thinking {}. Flows: local text prompts use {}.",
                p.variant(),
                p.num_ctx() / 1024,
                if p.thinking() { "on" } else { "off" },
                self.base(p),
            ))
            .small()
            .weak(),
        );
    }

    fn base(&self, p: Profile) -> String {
        self.repo.as_ref().map_or_else(|| p.default_base().to_owned(), |r| or_default(&r.env.get(p.base_key()), p.default_base()))
    }

    fn models_card(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Models").strong());
            if ui::tool(ui, icons::REFRESH, "list Ollama's models again").clicked() {
                self.list_ollama_models();
            }
        });
        let (installed, busy, last) = {
            let s = self.coding.lock().unwrap();
            (s.installed.clone(), s.busy, s.last.clone())
        };
        let Some(installed) = installed else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("asking Ollama ...");
            });
            return;
        };
        let bases: Vec<String> = installed.iter().filter(|m| !m.starts_with("local-coder:")).cloned().collect();
        let mut setup = None;
        egui::Grid::new("coding-models").num_columns(4).spacing([12.0, 8.0]).show(ui, |ui| {
            for p in Profile::ALL {
                ui.label(RichText::new(p.label()).strong());
                let mut base = self.base(p);
                let before = base.clone();
                egui::ComboBox::from_id_salt(("base", p.env_value())).selected_text(&base).width(260.0).show_ui(ui, |ui| {
                    for m in &bases {
                        ui.selectable_value(&mut base, m.clone(), m);
                    }
                });
                if base != before {
                    self.save_env(&[(p.base_key(), base.clone())]);
                }
                let have_base = installed.contains(&base);
                let built = installed.iter().any(|m| m == p.variant());
                if !have_base {
                    ui::status(ui, icons::ALERT, AMBER, "not downloaded");
                } else if built {
                    ui::status(ui, icons::CHECK_CIRCLE, GREEN, "set up");
                } else {
                    ui::status(ui, icons::CIRCLE_OUTLINE, AMBER, "not set up yet");
                }
                if busy == Some(p) {
                    ui.spinner();
                } else {
                    let label = if built { "Set up again" } else { "Set up" };
                    let tip = format!("builds {} from {base} and adds it to OpenCode and Pi", p.variant());
                    if ui::button_enabled(ui, have_base && busy.is_none(), icons::WRENCH, label).on_hover_text(tip).clicked() {
                        setup = Some((p, base));
                    }
                }
                ui.end_row();
            }
        });
        if bases.is_empty() {
            ui.label(RichText::new("Ollama has no models yet.").weak());
        }
        if ui.link("Download more coding models on the Models page").clicked() {
            self.select(Page::Models);
        }
        match &last {
            Some(Ok(m)) => {
                ui::status(ui, icons::CHECK_CIRCLE, GREEN, m.as_str());
            }
            Some(Err(e)) => {
                ui::status(ui, icons::ALERT_CIRCLE, RED, e.as_str());
            }
            None => {}
        }
        if let Some((p, base)) = setup {
            self.setup_profile(p, base);
        }
    }

    fn editors_card(&mut self, ui: &mut egui::Ui) {
        let p = self.profile();
        let root = self.repo.as_ref().map(|r| r.paths.root.clone());
        let ready = self.coding.lock().unwrap().installed.as_ref().is_some_and(|i| i.iter().any(|m| m == p.variant()));
        ui.label(RichText::new("Open").strong());
        ui.horizontal(|ui| {
            for e in Editor::ALL {
                let installed = e.exe().is_some();
                let tip = if installed { format!("loads {} into memory and opens {}", p.variant(), e.label()) } else { "not installed - run setup.bat".into() };
                if ui::button_enabled(ui, installed && ready, icons::OPEN_IN_NEW, e.label()).on_hover_text(tip).clicked()
                    && let Some(root) = &root
                {
                    coding::preload(p);
                    match e.command(p, root) {
                        Ok(mut cmd) => {
                            self.launch_once(e.label(), &mut cmd, e.label());
                            if e == Editor::T3Code {
                                self.notify(format!("T3 Code: pick provider OpenCode or Pi, then {}", p.variant()), false);
                            }
                        }
                        Err(err) => self.notify(err, true),
                    }
                }
            }
        });
        if !ready {
            ui.label(RichText::new(format!("Set up the {} model above first.", p.label().to_lowercase())).weak());
        }
    }

    pub(super) fn list_ollama_models(&self) {
        let state = self.coding.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let names = ureq::get("http://127.0.0.1:11434/api/tags")
                .call()
                .ok()
                .and_then(|r| r.into_body().read_json::<serde_json::Value>().ok())
                .map(|v| v["models"].as_array().into_iter().flatten().filter_map(|m| m["name"].as_str().map(str::to_owned)).collect())
                .unwrap_or_default();
            state.lock().unwrap().installed = Some(names);
            ctx.request_repaint();
        });
    }

    fn setup_profile(&mut self, p: Profile, base: String) {
        let state = self.coding.clone();
        let ctx = self.ctx.clone();
        state.lock().unwrap().busy = Some(p);
        std::thread::spawn(move || {
            let result = coding::apply(p, &base);
            {
                let mut s = state.lock().unwrap();
                // the variant shows up as "set up" without a manual refresh
                if result.is_ok()
                    && let Some(list) = &mut s.installed
                    && !list.iter().any(|m| m == p.variant())
                {
                    list.push(p.variant().to_owned());
                }
                s.busy = None;
                s.last = Some(result);
            }
            ctx.request_repaint();
        });
    }
}
