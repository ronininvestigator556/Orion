//! Interface panels drawn over the scene.

use orion_core::sun::SurfacePoint;

use std::time::Duration;

use orion_core::clock::{MAX_OFFSET_DAYS, SimulationClock};
use orion_core::filter::{Filter, Group};
use orion_core::layer::FeatureCard;
use orion_core::time::JulianDate;

use crate::camera::ViewFrame;
use crate::feed::{Origin, Refresh, Status};
use crate::layer::Toggle;
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
        .default_pos([770.0, 12.0])
        .default_open(false)
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
            ui.label(format!("{} pass the filter", tracked.shown));
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

/// Draws the About panel: what Orion is built from and who to thank for it.
pub fn draw_about(ctx: &egui::Context) {
    egui::Window::new("About")
        .default_pos([770.0, 50.0])
        .default_open(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.set_max_width(320.0);
            ui.label(format!("Orion {}", env!("CARGO_PKG_VERSION")));
            ui.label("Open source under the MIT or Apache 2.0 licence, at your option.");

            ui.separator();
            ui.label("Positions are computed from published orbital data. They are estimates, not live measurements.");

            ui.separator();
            ui.label("Orbital data and catalogue: CelesTrak, from the catalogue published by the U.S. Space Force at Space-Track.org.");
            ui.label("Earth imagery: NASA Blue Marble Next Generation and Black Marble 2016.");
            ui.label("Stars: the Yale Bright Star Catalogue, 5th revised edition.");

            ui.separator();
            ui.label("The licences of the open-source libraries Orion is built from are in THIRD-PARTY-LICENSES.html, which comes with each download. On macOS it is inside the app, under Contents/Resources.");
        });
}

/// How the camera behaves, as the View panel shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewChoices {
    pub frame: ViewFrame,
    pub following: bool,
}

/// Draws the View panel, which edits `choices` in place. `can_follow` says whether a
/// Tracked Object is selected for Follow Mode to follow.
pub fn draw_view(ctx: &egui::Context, choices: &mut ViewChoices, can_follow: bool) {
    egui::Window::new("View")
        .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -12.0])
        .resizable(false)
        .show(ctx, |ui| {
            ui.radio_value(
                &mut choices.frame,
                ViewFrame::EarthFixed,
                "Earth-Fixed View",
            )
            .on_hover_text("Earth stays still and objects move over it.");
            ui.radio_value(&mut choices.frame, ViewFrame::StarFixed, "Star-Fixed View")
                .on_hover_text("The stars stay still and Earth turns beneath the orbits.");
            ui.separator();
            ui.add_enabled(
                can_follow,
                egui::Checkbox::new(&mut choices.following, "Follow Mode"),
            )
            .on_hover_text("Keeps the selected object centred as it moves.")
            .on_disabled_hover_text("Select an object to follow it.");
        });
}

/// The most search results the Find panel lists at once.
pub const MAX_RESULTS: usize = 100;

/// What the filter currently lets through, for the Find panel.
pub struct Found {
    pub shown: usize,
    pub total: usize,
    /// The first [`MAX_RESULTS`] matches in name order: each object's index and a
    /// line of text naming it.
    pub results: Vec<(usize, String)>,
}

/// Draws the Find panel, which edits `filter` in place. Returns the index of a search
/// result the user clicked.
pub fn draw_find(ctx: &egui::Context, filter: &mut Filter, found: &Found) -> Option<usize> {
    let mut picked = None;
    egui::Window::new("Find")
        .default_pos([12.0, 12.0])
        .resizable(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let field = egui::TextEdit::singleline(&mut filter.text)
                    .hint_text("Name or catalogue number")
                    .desired_width(190.0);
                ui.add(field);
                if ui
                    .add_enabled(!filter.text.is_empty(), egui::Button::new("Clear"))
                    .clicked()
                {
                    filter.text.clear();
                }
            });

            ui.horizontal(|ui| {
                ui.checkbox(&mut filter.satellites, "Satellites");
                ui.checkbox(&mut filter.rocket_bodies, "Rocket Bodies");
                ui.checkbox(&mut filter.debris, "Debris");
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut filter.low, "Low");
                ui.checkbox(&mut filter.medium, "Medium");
                ui.checkbox(&mut filter.geostationary, "Geostationary");
            });
            ui.checkbox(&mut filter.highly_elliptical, "Highly elliptical");

            egui::ComboBox::from_label("Group")
                .selected_text(filter.group.map_or("Any", group_name))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut filter.group, None, "Any");
                    for group in Group::ALL {
                        ui.selectable_value(&mut filter.group, Some(group), group_name(group));
                    }
                });

            ui.separator();
            let narrowed = *filter != Filter::default();
            ui.horizontal(|ui| {
                ui.label(format!("{} of {} shown", found.shown, found.total));
                if ui
                    .add_enabled(narrowed, egui::Button::new("Show everything"))
                    .clicked()
                {
                    *filter = Filter::default();
                }
            });

            // With nothing narrowed down, a list of every object would be no help.
            if narrowed {
                egui::ScrollArea::vertical()
                    .max_height(180.0)
                    .show(ui, |ui| {
                        for (index, line) in &found.results {
                            if ui.selectable_label(false, line).clicked() {
                                picked = Some(*index);
                            }
                        }
                        if found.shown > found.results.len() {
                            ui.label(format!("and {} more", found.shown - found.results.len()));
                        }
                        if found.shown == 0 {
                            ui.label("Nothing matches.");
                        }
                    });
            }
        });
    picked
}

fn group_name(group: Group) -> &'static str {
    match group {
        Group::Weather => "Weather",
        Group::SpaceStations => "Space stations",
        Group::Navigation => "Navigation",
        Group::Starlink => "Starlink",
        Group::OneWeb => "OneWeb",
        Group::Kuiper => "Kuiper",
        Group::Qianfan => "Qianfan",
        Group::Guowang => "Guowang",
    }
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
        .anchor(egui::Align2::LEFT_BOTTOM, [12.0, -12.0])
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

/// Draws the information panel for the selected Feature: its card, then any switches
/// its Layer offers, which are edited in place.
pub fn draw_selected(
    ctx: &egui::Context,
    card: &FeatureCard,
    toggles: &mut [Toggle],
) -> SelectionActions {
    let mut actions = SelectionActions::default();
    egui::Window::new(&card.title)
        // The title changes with the selection; a fixed id keeps it one window.
        .id(egui::Id::unique("selected"))
        .anchor(egui::Align2::RIGHT_TOP, [-12.0, 12.0])
        .resizable(false)
        .show(ctx, |ui| {
            for (number, section) in card.sections.iter().enumerate() {
                if number > 0 {
                    ui.separator();
                }
                egui::Grid::new(("card section", number))
                    .num_columns(2)
                    // Some values are long; wrap them instead of stretching the
                    // panel across the globe.
                    .max_col_width(240.0)
                    .show(ui, |ui| {
                        for row in section {
                            ui.label(&row.label);
                            ui.label(&row.value);
                            ui.end_row();
                        }
                    });
            }
            for notice in &card.notices {
                ui.colored_label(egui::Color32::from_rgb(255, 190, 90), notice);
            }

            ui.separator();
            for toggle in toggles.iter_mut() {
                ui.checkbox(&mut toggle.on, toggle.label);
            }
            actions.clear = ui.button("Clear selection").clicked();
        });
    actions
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
        .default_pos([364.0, 12.0])
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

pub fn format_latitude(degrees: f64) -> String {
    let hemisphere = if degrees >= 0.0 { 'N' } else { 'S' };
    format!("{:.1}° {hemisphere}", degrees.abs())
}

pub fn format_longitude(degrees: f64) -> String {
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
