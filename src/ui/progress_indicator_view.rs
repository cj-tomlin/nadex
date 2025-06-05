use eframe::egui;

pub fn show(ctx: &egui::Context, is_visible: bool) {
    if is_visible {
        egui::Area::new("upload_progress_indicator".into())
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui_area| {
                egui::Frame::popup(&ctx.style()).show(ui_area, |ui| {
                    ui.set_min_width(200.0); // Set a minimum width for the popup
                    ui.set_max_width(200.0); // Constrain the maximum width as well
                    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                        ui.add_space(10.0);
                        ui.add(egui::Spinner::new());
                        ui.add_space(5.0);
                        ui.label("Processing, please wait..."); // Generic message
                        ui.add_space(10.0);
                    });
                });
            });
    }
}
