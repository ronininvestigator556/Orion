//! Interface panels drawn over the scene.

use orion_core::sun::SurfacePoint;

use std::time::Duration;

use orion_core::clock::{MAX_OFFSET_DAYS, SimulationClock};
use orion_core::layer::FeatureCard;
use orion_core::time::JulianDate;

use crate::basemap::BaseMapChoices;
use crate::camera::ViewFrame;
use crate::layer::Toggle;
use crate::reference::PlaceLabel;
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
        .default_pos([crate::sidebar::WIDTH + 12.0, 12.0])
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
        .default_pos([crate::sidebar::WIDTH + 200.0, 12.0])
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
            ui.label("Earth imagery: NASA Blue Marble and Black Marble, and a Landsat composite, streamed from NASA's Global Imagery Browse Services.");
            ui.label("Borders, roads and place names: Natural Earth, in the public domain.");
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
    pub base_map: BaseMapChoices,
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

            ui.separator();
            ui.label("Base Map");
            ui.checkbox(&mut choices.base_map.imagery, "Sharper imagery")
                .on_hover_text(
                    "Pictures of the ground that sharpen as you move closer, fetched from \
                     NASA's Global Imagery Browse Services. When off, nothing is fetched and \
                     the picture that comes with Orion is shown.",
                );
            ui.checkbox(&mut choices.base_map.borders, "Borders")
                .on_hover_text("Country borders and, closer in, state and province lines.");
            ui.checkbox(&mut choices.base_map.roads, "Major roads")
                .on_hover_text("Motorways and main roads, shown close to the ground.");
            ui.checkbox(&mut choices.base_map.names, "Place names")
                .on_hover_text("Countries and cities, more of them the closer you are.");
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
        // Clear of the Layers sidebar, which fills the left edge.
        .anchor(
            egui::Align2::LEFT_BOTTOM,
            [crate::sidebar::WIDTH + 12.0, -12.0],
        )
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

/// Draws the names of countries and cities over the Base Map, beneath every panel.
pub fn draw_place_names(ctx: &egui::Context, labels: &[PlaceLabel<'_>]) {
    use orion_core::reference::PlaceKind;

    let painter = ctx.layer_painter(egui::LayerId::background());
    let shadow = egui::Color32::from_black_alpha(200);
    let ink = egui::Color32::from_rgb(235, 235, 225);
    for label in labels {
        let (anchor, align, font) = match label.kind {
            PlaceKind::Country => (
                label.position,
                egui::Align2::CENTER_CENTER,
                egui::FontId::proportional(15.0),
            ),
            PlaceKind::Capital | PlaceKind::City => (
                label.position + egui::vec2(7.0, 0.0),
                egui::Align2::LEFT_CENTER,
                egui::FontId::proportional(12.0),
            ),
        };
        match label.kind {
            PlaceKind::Country => {}
            // A capital is a ring, any other city a dot.
            PlaceKind::Capital => {
                painter.circle_filled(label.position, 3.5, shadow);
                painter.circle_stroke(label.position, 3.0, egui::Stroke::new(1.5, ink));
            }
            PlaceKind::City => {
                painter.circle_filled(label.position, 3.0, shadow);
                painter.circle_filled(label.position, 2.0, ink);
            }
        }
        // A dark copy underneath keeps the name legible over bright ground.
        painter.text(
            anchor + egui::vec2(1.0, 1.0),
            align,
            label.name,
            font.clone(),
            shadow,
        );
        painter.text(anchor, align, label.name, font, ink);
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

/// Formats a length of time to the nearest sensible unit, such as `45 min`, `3.2 hours` or `5.1 days`.
pub fn format_duration(duration: Duration) -> String {
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
