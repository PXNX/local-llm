use std::path::Path;

use eframe::egui::{self, RichText};

use crate::icons;
use crate::ui;

/// Two-column form: label left, control right.
pub fn form(ui: &mut egui::Ui, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id).num_columns(2).spacing([14.0, 8.0]).min_col_width(130.0).show(ui, add);
}

/// Field label; a help icon shows the explanation on hover.
pub fn label(ui: &mut egui::Ui, text: &str, tip: &str) {
    ui.horizontal(|ui| {
        ui.label(text);
        if !tip.is_empty() {
            let weak = ui.visuals().weak_text_color();
            ui::icon(ui, icons::HELP_CIRCLE_OUTLINE, weak).on_hover_text(tip);
        }
    });
}

pub fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).small().weak());
}

fn pick_file(filter: (&str, &[&str]), start: &str) -> Option<String> {
    let mut d = rfd::FileDialog::new();
    if !filter.1.is_empty() {
        d = d.add_filter(filter.0, filter.1);
    }
    if let Some(dir) = Path::new(start).parent().filter(|p| p.is_dir()) {
        d = d.set_directory(dir);
    }
    d.pick_file().map(|p| p.display().to_string())
}

pub fn file(ui: &mut egui::Ui, value: &mut String, filter: (&str, &[&str]), hint_text: &str) {
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(value).hint_text(hint_text).desired_width(ui.available_width() - 100.0));
        if ui::button(ui, icons::FILE_SEARCH_OUTLINE, "Browse").clicked()
            && let Some(p) = pick_file(filter, value)
        {
            *value = p;
        }
    });
}

pub fn folder(ui: &mut egui::Ui, value: &mut String, hint_text: &str) {
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(value).hint_text(hint_text).desired_width(ui.available_width() - 100.0));
        if ui::button(ui, icons::FOLDER_SEARCH_OUTLINE, "Browse").clicked()
            && let Some(p) = rfd::FileDialog::new().pick_folder()
        {
            *value = p.display().to_string();
        }
    });
}

/// Editable list of strings (repeatable CLI option), optional file/folder pickers.
pub fn list(ui: &mut egui::Ui, id: &str, values: &mut Vec<String>, hint_text: &str, files: Option<(&str, &[&str])>, folders: bool) {
    ui.vertical(|ui| {
        let mut remove = None;
        for (i, v) in values.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.push_id((id, i), |ui| {
                    ui.add(egui::TextEdit::singleline(v).hint_text(hint_text).desired_width(ui.available_width() - 34.0));
                    if ui::tool(ui, icons::CLOSE, "remove").clicked() {
                        remove = Some(i);
                    }
                });
            });
        }
        if let Some(i) = remove {
            values.remove(i);
        }
        ui.horizontal(|ui| {
            if ui::button(ui, icons::PLUS, "Add").clicked() {
                values.push(String::new());
            }
            if let Some(filter) = files
                && ui::button(ui, icons::FILE_MULTIPLE, "Add files").clicked()
            {
                let mut d = rfd::FileDialog::new();
                if !filter.1.is_empty() {
                    d = d.add_filter(filter.0, filter.1);
                }
                for p in d.pick_files().unwrap_or_default() {
                    values.push(p.display().to_string());
                }
            }
            if folders
                && ui::button(ui, icons::FOLDER_PLUS, "Add folder").clicked()
                && let Some(p) = rfd::FileDialog::new().pick_folder()
            {
                values.push(p.display().to_string());
            }
        });
    });
}

pub fn choice(ui: &mut egui::Ui, id: &str, value: &mut String, options: &[(&str, &str)]) {
    let current = options.iter().find(|o| o.0 == value).map_or(value.as_str(), |o| o.1).to_owned();
    egui::ComboBox::from_id_salt(id).selected_text(current).width(260.0).show_ui(ui, |ui| {
        for (key, text) in options {
            ui.selectable_value(value, key.to_string(), *text);
        }
    });
}

/// Seed: empty = random each run.
pub fn seed(ui: &mut egui::Ui, value: &mut Option<i64>) {
    ui.horizontal(|ui| {
        let mut fixed = value.is_some();
        if ui.checkbox(&mut fixed, "fixed").changed() {
            *value = fixed.then_some(42);
        }
        match value {
            Some(v) => {
                ui.add(egui::DragValue::new(v).range(0..=i64::from(i32::MAX)));
            }
            None => hint(ui, "random each run"),
        }
    });
}
