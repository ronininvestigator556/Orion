//! Interface panels drawn over the scene.

use orion_core::sun::SurfacePoint;

use crate::renderer::GpuInfo;

pub fn draw(ctx: &egui::Context, gpu: &GpuInfo, frame_ms: f32, subsolar: SurfacePoint) {
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
        });
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
