//! Interface panels drawn over the scene.

use orion_core::sun::SurfacePoint;

use std::time::Duration;

use orion_core::catalogue::ObjectKind;

use crate::feed::{Origin, Refresh, Status};
use crate::renderer::GpuInfo;
use crate::tracked::Details;
use crate::tracked::Summary;

/// Everything the diagnostics panel reports for one frame.
pub struct Diagnostics {
    pub frame_ms: f32,
    pub subsolar: SurfacePoint,
    pub tracked: Summary,
}

pub fn draw(ctx: &egui::Context, gpu: &GpuInfo, diagnostics: &Diagnostics) {
    let Diagnostics {
        frame_ms,
        subsolar,
        tracked,
    } = *diagnostics;
    egui::Window::new("Diagnostics")
        .default_pos([12.0, 12.0])
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(format!("{} ({})", gpu.adapter, gpu.backend));
            if frame_ms > 0.0 {
                ui.label(format!(
                    "{frame_ms:.1} ms per frame ({:.0} fps)",
                    1000.0 / frame_ms
                ));
            }
            ui.label(format!("Sun overhead at {}", format_point(subsolar)));

            ui.separator();
            ui.label(format!(
                "{} Satellites, {} Rocket Bodies, {} Debris",
                tracked.satellites, tracked.rocket_bodies, tracked.debris
            ));
            if tracked.without_position > 0 {
                ui.label(format!("{} have no Position now", tracked.without_position));
            }
            if let Some(days) = tracked.median_element_age_days {
                ui.label(format!("Median Element Age is {days:.1} days"));
            }
            ui.label(format!(
                "Propagation takes {:.2} ms",
                tracked.propagation_ms
            ));
        });
}

/// What the user asked for in the information panel this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectionActions {
    pub clear: bool,
}

/// Draws the information panel for the selected Tracked Object.
pub fn draw_selected(
    ctx: &egui::Context,
    details: &Details,
    show_ground_track: &mut bool,
) -> SelectionActions {
    let mut actions = SelectionActions::default();
    egui::Window::new(&details.name)
        // The title changes with the selection; a fixed id keeps it one window.
        .id(egui::Id::unique("selected"))
        .anchor(egui::Align2::RIGHT_TOP, [-12.0, 12.0])
        .resizable(false)
        .show(ctx, |ui| {
            let row = |ui: &mut egui::Ui, label: &str, value: String| {
                ui.label(label);
                ui.label(value);
                ui.end_row();
            };

            egui::Grid::new("identity").num_columns(2).show(ui, |ui| {
                row(ui, "Catalogue number", details.catalogue_number.to_string());
                row(ui, "Designator", details.designator.clone());
                row(ui, "Kind", kind_name(details.kind).to_owned());
            });

            ui.separator();
            match details.now {
                Some(now) => {
                    egui::Grid::new("now").num_columns(2).show(ui, |ui| {
                        row(ui, "Latitude", format_latitude(now.over.latitude_deg));
                        row(ui, "Longitude", format_longitude(now.over.longitude_deg));
                        row(ui, "Altitude", format!("{:.0} km", now.over.altitude_km));
                        row(ui, "Speed", format!("{:.2} km/s", now.speed_km_s));
                    });
                }
                None => {
                    ui.label("No Position can be computed for this moment.");
                }
            }

            ui.separator();
            egui::Grid::new("orbit").num_columns(2).show(ui, |ui| {
                row(ui, "Period", format!("{:.1} min", details.period_minutes));
                row(
                    ui,
                    "Inclination",
                    format!("{:.1}°", details.inclination_deg),
                );
                row(
                    ui,
                    "Highest point",
                    format!("{:.0} km", details.apogee_altitude_km),
                );
                row(
                    ui,
                    "Lowest point",
                    format!("{:.0} km", details.perigee_altitude_km),
                );
                row(ui, "Eccentricity", format!("{:.4}", details.eccentricity));
            });

            ui.separator();
            egui::Grid::new("element set")
                .num_columns(2)
                .show(ui, |ui| {
                    let epoch = details.epoch;
                    row(
                        ui,
                        "Epoch",
                        format!(
                            "{:04}-{:02}-{:02} {:02}:{:02} UTC",
                            epoch.year, epoch.month, epoch.day, epoch.hour, epoch.minute
                        ),
                    );
                    row(
                        ui,
                        "Element Age",
                        format!("{:.1} days", details.element_age_days),
                    );
                });
            if details.stale {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 190, 90),
                    "This Element Set is old. The Position may be far off.",
                );
            }

            ui.separator();
            ui.checkbox(show_ground_track, "Show Ground Track");
            actions.clear = ui.button("Clear selection").clicked();
        });
    actions
}

fn kind_name(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Satellite => "Satellite",
        ObjectKind::RocketBody => "Rocket Body",
        ObjectKind::Debris => "Debris",
    }
}

/// Draws a name next to each given point on screen, in logical points.
pub fn draw_labels(ctx: &egui::Context, labels: &[(egui::Pos2, String)]) {
    let painter = ctx.layer_painter(egui::LayerId::background());
    let font = egui::FontId::proportional(13.0);
    for (position, text) in labels {
        let anchor = *position + egui::vec2(12.0, -8.0);
        // A dark copy underneath keeps the name legible over bright ground.
        painter.text(
            anchor + egui::vec2(1.0, 1.0),
            egui::Align2::LEFT_BOTTOM,
            text,
            font.clone(),
            egui::Color32::BLACK,
        );
        painter.text(
            anchor,
            egui::Align2::LEFT_BOTTOM,
            text,
            font.clone(),
            egui::Color32::WHITE,
        );
    }
}

/// Draws the panel about where the Element Sets came from. Returns true if the user
/// asked for a refresh.
pub fn draw_element_sets(ctx: &egui::Context, status: &Status) -> bool {
    let mut refresh_requested = false;
    egui::Window::new("Element Sets")
        .default_pos([12.0, 190.0])
        .resizable(false)
        .show(ctx, |ui| {
            let age = format_duration(status.age);
            ui.label(match status.origin {
                Origin::Snapshot => format!("Bundled Snapshot, fetched {age} ago"),
                Origin::Fetched => format!("From {}, fetched {age} ago", status.source),
            });

            let button = egui::Button::new(format!("Refresh from {}", status.source));
            match &status.refresh {
                Refresh::Ready => refresh_requested = ui.add(button).clicked(),
                Refresh::InProgress => {
                    ui.add_enabled(false, button);
                    ui.label("Fetching…");
                }
                Refresh::Locked(remaining) => {
                    ui.add_enabled(false, button);
                    ui.label(format!(
                        "Available again in {}",
                        format_duration(*remaining)
                    ));
                }
                Refresh::Failed(reason) => {
                    ui.add_enabled(false, button);
                    ui.label(format!("Refresh failed: {reason}"));
                    ui.label("No more attempts until Orion is restarted.");
                }
                Refresh::Unavailable => {
                    ui.add_enabled(false, button);
                    ui.label("No cache folder is available, so nothing is fetched.");
                }
            }
        });
    refresh_requested
}

/// Formats a length of time to the nearest sensible unit, such as `45 min`, `3.2 hours` or `5.1 days`.
fn format_duration(duration: Duration) -> String {
    let minutes = duration.as_secs_f64() / 60.0;
    if minutes < 60.0 {
        format!("{} min", minutes.round())
    } else if minutes < 48.0 * 60.0 {
        format!("{:.1} hours", minutes / 60.0)
    } else {
        format!("{:.1} days", minutes / (24.0 * 60.0))
    }
}

/// Formats a place as, for example, `20.7° S, 94.1° E`.
fn format_point(point: SurfacePoint) -> String {
    format!(
        "{}, {}",
        format_latitude(point.latitude_deg),
        format_longitude(point.longitude_deg)
    )
}

fn format_latitude(degrees: f64) -> String {
    let hemisphere = if degrees >= 0.0 { 'N' } else { 'S' };
    format!("{:.1}° {hemisphere}", degrees.abs())
}

fn format_longitude(degrees: f64) -> String {
    let hemisphere = if degrees >= 0.0 { 'E' } else { 'W' };
    format!("{:.1}° {hemisphere}", degrees.abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_use_the_nearest_sensible_unit() {
        assert_eq!(format_duration(Duration::from_secs(20)), "0 min");
        assert_eq!(format_duration(Duration::from_secs(45 * 60)), "45 min");
        assert_eq!(format_duration(Duration::from_secs(192 * 60)), "3.2 hours");
        assert_eq!(
            format_duration(Duration::from_secs(5 * 86_400 + 8_640)),
            "5.1 days"
        );
    }

    #[test]
    fn points_are_labelled_by_hemisphere() {
        let point = SurfacePoint {
            latitude_deg: -20.74,
            longitude_deg: 94.06,
        };
        assert_eq!(format_point(point), "20.7° S, 94.1° E");

        let point = SurfacePoint {
            latitude_deg: 51.5,
            longitude_deg: -0.13,
        };
        assert_eq!(format_point(point), "51.5° N, 0.1° W");
    }
}
