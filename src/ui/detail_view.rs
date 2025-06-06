// Placeholder for the image detail view UI logic.
// Code from main.rs related to the detail modal will be moved here.

use crate::app_actions::AppAction;
use crate::app_state::AppState; // Added AppState
use egui; // Added AppAction

/// Shows the image detail modal.
pub fn show_detail_modal(
    app_state: &mut AppState,
    ui: &mut egui::Ui,
    action_queue: &mut Vec<AppAction>,
) {
    // This modal should only be called if app_state.selected_image_for_detail is Some.
    // We'll unwrap it here, assuming the caller guarantees this.
    let selected_image_meta = match &app_state.selected_image_for_detail {
        Some(meta) => meta,
        None => return, // Should not happen if logic in main.rs is correct
    };
    let detail_view_texture_handle = &app_state.detail_view_texture_handle;
    let screen_rect = ui.ctx().screen_rect(); // Get screen_rect from ui context

    let default_modal_width = screen_rect.width() * 0.5;
    let default_modal_height = screen_rect.height() * 0.5;

    let mut modal_target_width = default_modal_width;
    let mut modal_target_height = default_modal_height;
    let mut image_display_max_size =
        egui::vec2(default_modal_width - 60.0, default_modal_height - 200.0);

    let controls_and_padding_height = 150.0;
    let horizontal_padding = 30.0;

    if let Some(texture) = detail_view_texture_handle {
        let image_native_size = texture.size_vec2();

        let max_img_display_width = screen_rect.width() * 0.75;
        let max_img_display_height = screen_rect.height() * 0.70;

        let mut scaled_img_size = image_native_size;
        let aspect_ratio = image_native_size.x / image_native_size.y;

        if scaled_img_size.x > max_img_display_width {
            scaled_img_size.x = max_img_display_width;
            scaled_img_size.y = scaled_img_size.x / aspect_ratio;
        }
        if scaled_img_size.y > max_img_display_height {
            scaled_img_size.y = max_img_display_height;
            scaled_img_size.x = scaled_img_size.y * aspect_ratio;
        }
        if scaled_img_size.x > max_img_display_width {
            // Re-check width
            scaled_img_size.x = max_img_display_width;
            scaled_img_size.y = scaled_img_size.x / aspect_ratio;
        }

        image_display_max_size = scaled_img_size;

        modal_target_width = image_display_max_size.x + horizontal_padding;
        modal_target_height = image_display_max_size.y + controls_and_padding_height;

        modal_target_width = modal_target_width.min(screen_rect.width());
        modal_target_height = modal_target_height.min(screen_rect.height());

        modal_target_width = modal_target_width.max(400.0);
        modal_target_height = modal_target_height.max(300.0);
    }

    let dim_painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new("image_detail_dim_layer"),
    ));
    dim_painter.rect_filled(screen_rect, 0.0, egui::Color32::from_black_alpha(180));

    let modal_area_id = egui::Id::new("image_detail_modal_area");

    egui::Area::new(modal_area_id)
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .interactable(true)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .inner_margin(egui::Margin::same(10.0))
                .show(ui, |ui| {
                    ui.set_max_width(modal_target_width);
                    ui.set_max_height(modal_target_height);
                    ui.set_min_width(modal_target_width.min(screen_rect.width() * 0.9));
                    ui.set_min_height(modal_target_height.min(screen_rect.height() * 0.9));

                    ui.with_layout(egui::Layout::top_down(egui::Align::RIGHT), |ui| {
                        if ui.button(" X ").on_hover_text("Close (Esc)").clicked()
                            || ui.ctx().input(|i| i.key_pressed(egui::Key::Escape))
                        {
                            // Close button & Esc key
                            action_queue.push(AppAction::DetailModalClose);
                        }
                    });

                    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| { // Center the main content column
                        // Inner layout to keep separator, image, and grid left-aligned with each other
                        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui_sub| {
                            ui_sub.separator();
                            if let Some(texture) = detail_view_texture_handle {
                                let img_widget = egui::Image::new(egui::load::SizedTexture::new(
                                    texture.id(),
                                    texture.size_vec2(),
                                ))
                                .max_size(image_display_max_size)
                                .maintain_aspect_ratio(true);
    
                                ui_sub.add(img_widget);
                            } else {
                                ui_sub.label("Loading image...");
                            }
    
                            ui_sub.add_space(5.0);
    
                            egui::Grid::new("detail_metadata_grid")
                                .num_columns(2)
                                .spacing([20.0, 3.0])
                                .show(ui_sub, |ui| { // Inner ui for grid
                                    ui.strong("Nade Type:");
                                    ui.label(format!("{:?}", selected_image_meta.nade_type));
                                    ui.end_row();
    
                                    ui.strong("Position:");
                                    ui.label(if selected_image_meta.position.is_empty() {
                                        "[No Position]"
                                    } else {
                                        &selected_image_meta.position
                                    });
                                    ui.end_row();
                                    ui.strong("Notes:");
                                    let notes_text = if selected_image_meta.notes.is_empty() {
                                        "[No Notes]"
                                    } else {
                                        &selected_image_meta.notes
                                    };
                                    ui.add(egui::Label::new(notes_text).wrap(true));
                                    ui.end_row();
                                });
                        }); // End of ui_sub (left_aligned_sub_block)

                        ui.add_space(8.0); // Reduced from 15.0

                        // Centered horizontal layout for buttons
                        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                            ui.horizontal(|ui| {
                                if ui
                                    .button("Edit")
                                    .on_hover_text("Edit image details")
                                    .clicked()
                                {
                                    action_queue.push(AppAction::DetailModalRequestEdit(
                                        selected_image_meta.clone(),
                                    ));
                                }
                                ui.add_space(10.0); // Space between buttons
                                if ui
                                    .button("Delete")
                                    .on_hover_text("Delete this image")
                                    .clicked()
                                {
                                    action_queue.push(AppAction::DetailModalRequestDelete(
                                        selected_image_meta.clone(),
                                    ));
                                }
                            });
                        });
                    });
                });
        });
}
