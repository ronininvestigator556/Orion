//! Interface panels drawn over the scene.

use orion_core::sun::SurfacePoint;

use std::time::Duration;

use crate::feed::{Origin, Refresh, Status};
use crate::renderer::GpuInfo;
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
    let north_south = if point.latitude_deg >= 0.0 { 'N' } else { 'S' };
    let east_west = if point.longitude_deg >= 0.0 { 'E' } else { 'W' };
    format!(
        "{:.1}° {north_south}, {:.1}° {east_west}",
        point.latitude_deg.abs(),
        point.longitude_deg.abs()
    )
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
