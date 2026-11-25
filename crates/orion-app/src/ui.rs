//! Interface panels drawn over the scene.

use crate::renderer::GpuInfo;

pub fn draw(ctx: &egui::Context, gpu: &GpuInfo, frame_ms: f32) {
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
        });
}
