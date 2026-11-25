//! Interface panels drawn over the scene.

use orion_core::sun::SurfacePoint;

use std::time::Duration;

use orion_core::catalogue::ObjectKind;
use orion_core::clock::{MAX_OFFSET_DAYS, SimulationClock};
use orion_core::time::JulianDate;

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

/// What the Simulation Clock panel remembers between frames.
#[derive(Clone, Debug, Default)]
pub struct ClockPanel {
    /// What has been typed into the "jump to" box.
    jump_text: String,
    /// True if the last thing entered there could not be read as a time.
    jump_rejected: bool,
    show_local_time: bool,
}

/// What the user asked the Simulation Clock to do this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClockAction {
    SetPaused(bool),
    Faster,
    Slower,
    /// Move by this many days, positive for later.
    JumpBy(f64),
    JumpTo(JulianDate),
    ReturnToLive,
}

/// Draws the Simulation Clock panel.
pub fn draw_clock(
    ctx: &egui::Context,
    clock: &SimulationClock,
    real_now: JulianDate,
    panel: &mut ClockPanel,
) -> Option<ClockAction> {
    let mut action = None;
    egui::Window::new("Simulation Clock")
        .default_pos([12.0, 340.0])
        .resizable(false)
        .show(ctx, |ui| {
            let moment = clock.moment();
            ui.heading(if panel.show_local_time {
                format_local(moment)
            } else {
                format_utc(moment)
            });
            ui.label(clock_status(clock, real_now));

            // Scrub anywhere within the allowed window around real time.
            let mut offset_days = clock.offset_days(real_now);
            let slider = egui::Slider::new(&mut offset_days, -MAX_OFFSET_DAYS..=MAX_OFFSET_DAYS)
                .fixed_decimals(2)
                .suffix(" days");
            if ui.add(slider).changed() {
                action = Some(ClockAction::JumpTo(JulianDate(real_now.0 + offset_days)));
            }

            ui.horizontal(|ui| {
                if ui.button("-1 day").clicked() {
                    action = Some(ClockAction::JumpBy(-1.0));
                }
                if ui.button("-1 hour").clicked() {
                    action = Some(ClockAction::JumpBy(-1.0 / 24.0));
                }
                let pause_label = if clock.is_paused() { "Play" } else { "Pause" };
                if ui.button(pause_label).clicked() {
                    action = Some(ClockAction::SetPaused(!clock.is_paused()));
                }
                if ui.button("+1 hour").clicked() {
                    action = Some(ClockAction::JumpBy(1.0 / 24.0));
                }
                if ui.button("+1 day").clicked() {
                    action = Some(ClockAction::JumpBy(1.0));
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Slower").clicked() {
                    action = Some(ClockAction::Slower);
                }
                if ui.button("Faster").clicked() {
                    action = Some(ClockAction::Faster);
                }
                if ui
                    .add_enabled(!clock.is_live(), egui::Button::new("Back to now"))
                    .clicked()
                {
                    action = Some(ClockAction::ReturnToLive);
                }
            });

            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Jump to (UTC)");
                let field = egui::TextEdit::singleline(&mut panel.jump_text)
                    .hint_text("YYYY-MM-DD HH:MM")
                    .desired_width(130.0);
                let entered = ui.add(field).lost_focus()
                    && ui.input(|input| input.key_pressed(egui::Key::Enter));
                if ui.button("Go").clicked() || entered {
                    match JulianDate::parse_utc(&panel.jump_text) {
                        Some(moment) => {
                            panel.jump_rejected = false;
                            action = Some(ClockAction::JumpTo(moment));
                        }
                        None => panel.jump_rejected = true,
                    }
                }
            });
            if panel.jump_rejected {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 190, 90),
                    "Write the time as YYYY-MM-DD HH:MM, in UTC.",
                );
            }
            ui.label(format!(
                "The clock stays within {MAX_OFFSET_DAYS:.0} days of now."
            ));
            ui.checkbox(&mut panel.show_local_time, "Show local time");
        });
    action
}

/// Describes what the Simulation Clock is doing, such as `Running at 60×, 2.0 hours ahead of now`.
fn clock_status(clock: &SimulationClock, real_now: JulianDate) -> String {
    if clock.is_live() {
        return "Live".to_owned();
    }
    let rate = clock.rate();
    let motion = if clock.is_paused() {
        "Paused".to_owned()
    } else if rate == 1.0 {
        "Running at normal speed".to_owned()
    } else if rate == -1.0 {
        "Running backwards at normal speed".to_owned()
    } else if rate < 0.0 {
        format!("Running backwards at {:.0}×", -rate)
    } else {
        format!("Running at {rate:.0}×")
    };

    let offset_seconds = clock.offset_days(real_now) * 86_400.0;
    let offset = if offset_seconds.abs() < 30.0 {
        "at the present".to_owned()
    } else {
        format!(
            "{} {} now",
            format_duration(Duration::from_secs_f64(offset_seconds.abs())),
            if offset_seconds > 0.0 {
                "ahead of"
            } else {
                "behind"
            }
        )
    };
    format!("{motion}, {offset}")
}

fn format_utc(moment: JulianDate) -> String {
    let time = moment.to_utc();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        time.year,
        time.month,
        time.day,
        time.hour,
        time.minute,
        time.second.floor()
    )
}

/// Formats a moment in the computer's time zone, with its offset from UTC.
fn format_local(moment: JulianDate) -> String {
    let unix_seconds = (moment.0 - JulianDate::UNIX_EPOCH.0) * 86_400.0;
    match chrono::DateTime::from_timestamp(unix_seconds.floor() as i64, 0) {
        Some(utc) => utc
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S %:z")
            .to_string(),
        None => format_utc(moment),
    }
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

            egui::Grid::new("identity")
                .num_columns(2)
                // Owner and launch site names can be long; wrap them instead of
                // stretching the panel across the globe.
                .max_col_width(240.0)
                .show(ui, |ui| {
                    row(ui, "Catalogue number", details.catalogue_number.to_string());
                    row(ui, "Designator", details.designator.clone());
                    row(ui, "Kind", kind_name(details.kind).to_owned());
                    match &details.record {
                        Some(record) => {
                            row(ui, "Owner", record.owner.clone());
                            row(ui, "Launched", record.launch_date.clone());
                            row(ui, "Launch site", record.launch_site.clone());
                        }
                        None => row(ui, "Owner and launch", "Not yet on record".to_owned()),
                    }
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
    fn clock_status_says_what_the_clock_is_doing() {
        let real_now = JulianDate::from_utc(2026, 11, 25, 6, 0, 0.0);
        let mut clock = SimulationClock::new(real_now);
        assert_eq!(clock_status(&clock, real_now), "Live");

        clock.set_paused(true);
        assert_eq!(clock_status(&clock, real_now), "Paused, at the present");

        clock.set_paused(false);
        clock.jump_by(2.0 / 24.0, real_now);
        assert_eq!(
            clock_status(&clock, real_now),
            "Running at normal speed, 2.0 hours ahead of now"
        );

        clock.faster();
        clock.faster();
        clock.jump_by(-3.0, real_now);
        assert_eq!(
            clock_status(&clock, real_now),
            "Running at 60×, 2.9 days behind now"
        );

        for _ in 0..4 {
            clock.slower();
        }
        assert_eq!(
            clock_status(&clock, real_now),
            "Running backwards at 10×, 2.9 days behind now"
        );
    }

    #[test]
    fn moments_are_shown_in_utc_to_the_second() {
        let moment = JulianDate::from_utc(2026, 11, 25, 6, 21, 7.6);
        assert_eq!(format_utc(moment), "2026-11-25 06:21:07 UTC");
        // The local form shows the same instant with an offset, whatever the zone.
        let local = format_local(moment);
        assert!(local.starts_with("2026-11-2"), "{local}");
        assert_eq!(local.len(), "2026-11-25 06:21:07 +00:00".len(), "{local}");
    }

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
