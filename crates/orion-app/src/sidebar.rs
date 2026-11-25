//! The Layers sidebar: every Layer under its Theme, and one search across them.

use std::time::SystemTime;

use orion_core::layer::Theme;
use orion_core::time::JulianDate;

use crate::feed::Refresh;
use crate::layer::{FeatureRef, Layers};
use crate::ui::format_duration;

/// Width of the sidebar in logical points.
pub const WIDTH: f32 = 300.0;
/// The most search results listed for one Layer.
const MAX_RESULTS: usize = 50;
const WARNING: egui::Color32 = egui::Color32::from_rgb(255, 190, 90);

/// What the sidebar remembers between frames.
#[derive(Clone, Debug, Default)]
pub struct Sidebar {
    /// What has been typed into the search box.
    pub search: String,
    /// What has been typed into the box for the path of a file to open.
    pub import_path: String,
    /// The path the user asked to open this frame.
    pub open_requested: Option<String>,
    /// The Imported Layer the user asked to remove this frame, by its position.
    pub remove_requested: Option<usize>,
    /// How the last attempt to open a file went, and whether it failed.
    pub import_result: Option<(String, bool)>,
}

/// Draws the sidebar, which changes the Layers in place: switching them on and off,
/// filtering them and asking for refreshes. Returns a search result the user clicked.
pub fn draw(
    ui: &mut egui::Ui,
    layers: &mut Layers,
    sidebar: &mut Sidebar,
    at: JulianDate,
    real_now: JulianDate,
    now: SystemTime,
) -> Option<FeatureRef> {
    let mut picked = None;
    egui::Panel::left("layers")
        .resizable(false)
        .exact_size(WIDTH)
        .show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let field = egui::TextEdit::singleline(&mut sidebar.search)
                    .hint_text("Search Layers that are on")
                    .desired_width(WIDTH - 90.0);
                ui.add(field);
                if ui
                    .add_enabled(!sidebar.search.is_empty(), egui::Button::new("Clear"))
                    .clicked()
                {
                    sidebar.search.clear();
                }
            });
            // The search narrows every Layer, so what is listed is what is drawn.
            for index in 0..layers.len() {
                if let Some(layer) = layers.get_mut(index) {
                    layer.set_search(&sidebar.search);
                }
            }

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if !sidebar.search.trim().is_empty() {
                        picked = draw_results(ui, layers);
                        ui.separator();
                    }
                    for theme in Theme::ALL {
                        let members: Vec<usize> = (0..layers.len())
                            .filter(|index| {
                                layers
                                    .get(*index)
                                    .is_some_and(|layer| layer.theme() == theme)
                            })
                            .collect();
                        // Imported is always listed: it is where files are opened.
                        if members.is_empty() && theme != Theme::Imported {
                            continue;
                        }
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(theme.name()).strong());
                        for index in members {
                            let covered = layers.covers(index, at, real_now);
                            if draw_layer(ui, layers, index, covered, now) {
                                sidebar.remove_requested = Some(index);
                            }
                        }
                        if theme == Theme::Imported {
                            draw_open_file(ui, sidebar);
                        }
                    }
                });
        });
    picked
}

/// Draws where a file of the user's own places is opened.
fn draw_open_file(ui: &mut egui::Ui, sidebar: &mut Sidebar) {
    ui.weak("Drop a GeoJSON, KML or CSV file on the window, or give its path:");
    ui.horizontal(|ui| {
        let field = egui::TextEdit::singleline(&mut sidebar.import_path)
            .hint_text("Path to a file")
            .desired_width(WIDTH - 90.0);
        let entered =
            ui.add(field).lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let clicked = ui
            .add_enabled(
                !sidebar.import_path.trim().is_empty(),
                egui::Button::new("Open"),
            )
            .clicked();
        if (entered || clicked) && !sidebar.import_path.trim().is_empty() {
            sidebar.open_requested = Some(sidebar.import_path.trim().to_owned());
        }
    });
    match &sidebar.import_result {
        Some((message, true)) => {
            ui.colored_label(WARNING, message);
        }
        Some((message, false)) => {
            ui.weak(message);
        }
        None => {}
    }
    ui.weak(
        "A file is read where it is and sent nowhere. A CSV needs latitude and longitude columns.",
    );
}

/// Lists what the search found on each Layer that is on.
fn draw_results(ui: &mut egui::Ui, layers: &Layers) -> Option<FeatureRef> {
    let mut picked = None;
    let mut anything = false;
    for index in 0..layers.len() {
        let Some(layer) = layers.get(index).filter(|layer| layer.is_on()) else {
            continue;
        };
        let (shown, _) = layer.counts();
        if shown == 0 {
            continue;
        }
        anything = true;
        ui.label(format!("{}: {shown} found", layer.name()));
        let found = layer.found(MAX_RESULTS);
        for (feature, line) in &found {
            if ui.selectable_label(false, line).clicked() {
                picked = Some(FeatureRef {
                    layer: index,
                    index: *feature,
                });
            }
        }
        if shown > found.len() {
            ui.weak(format!("and {} more", shown - found.len()));
        }
    }
    if !anything {
        ui.label("Nothing found.");
    }
    picked
}

/// Draws one Layer's entry: its switch, colour and count, then, while it is on, its
/// status, refresh control and filter. Returns true if the user asked for the Layer,
/// one they imported, to be removed.
fn draw_layer(
    ui: &mut egui::Ui,
    layers: &mut Layers,
    index: usize,
    covered: bool,
    now: SystemTime,
) -> bool {
    let mut remove = false;
    let Some(layer) = layers.get_mut(index) else {
        return false;
    };
    ui.horizontal(|ui| {
        let mut on = layer.is_on();
        if ui.checkbox(&mut on, "").changed() {
            layer.set_on(on);
        }
        let [r, g, b, a] = layer.swatch().map(|channel| f32::from(channel) / 255.0);
        let (swatch, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(swatch, 2.0, egui::Rgba::from_rgba_unmultiplied(r, g, b, a));
        ui.label(layer.name());
        if layer.has_features() {
            ui.weak(format_counts(layer.counts()));
        }
        if layer.is_imported() {
            remove = ui
                .small_button("Remove")
                .on_hover_text("Takes the Layer off the globe. The file itself is not touched.")
                .clicked();
        }
    });
    if !layer.is_on() {
        return remove;
    }

    ui.indent(("layer", index), |ui| {
        if !covered {
            ui.colored_label(
                WARNING,
                "Cannot show the moment the Simulation Clock is at.",
            );
        }
        let status = layer.status(now);
        ui.weak(&status.text);
        if let Some(refresh) = &status.refresh {
            let button = egui::Button::new("Refresh");
            match refresh {
                Refresh::Ready => {
                    if ui.add(button).clicked() {
                        layer.request_refresh(now);
                    }
                }
                Refresh::InProgress => {
                    ui.add_enabled(false, button);
                    ui.weak("Fetching…");
                }
                Refresh::Locked(remaining) => {
                    ui.add_enabled(false, button);
                    ui.weak(format!(
                        "Available again in {}",
                        format_duration(*remaining)
                    ));
                }
                Refresh::Failed(reason) => {
                    ui.add_enabled(false, button);
                    ui.colored_label(WARNING, format!("Refresh failed: {reason}"));
                    ui.weak("No more attempts until Orion is restarted.");
                }
                Refresh::Unavailable => {
                    ui.add_enabled(false, button);
                    ui.weak("No cache folder is available, so nothing is fetched.");
                }
            }
        }
        if layer.has_features() && !layer.is_imported() {
            ui.collapsing("Filter", |ui| layer.filter_ui(ui));
        }
    });
    remove
}

/// Formats how many Features a Layer shows, such as `409 of 29389` or just `29389`.
fn format_counts((shown, total): (usize, usize)) -> String {
    if shown == total {
        total.to_string()
    } else {
        format!("{shown} of {total}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_mention_the_total_only_when_something_is_hidden() {
        assert_eq!(format_counts((29_389, 29_389)), "29389");
        assert_eq!(format_counts((409, 29_389)), "409 of 29389");
        assert_eq!(format_counts((0, 12)), "0 of 12");
    }
}
