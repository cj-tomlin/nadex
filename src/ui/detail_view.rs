// Placeholder for the image detail view UI logic.
// Code from main.rs related to the detail modal will be moved here.

use crate::persistence::ImageMeta;
use egui;

// Example of how we might structure this (this will be refined):

/// Represents actions that can be taken from the detail modal.
#[derive(Debug, Clone)]
pub enum DetailModalAction {
    Close,
    RequestEdit(ImageMeta),
    RequestDelete(ImageMeta),
}

/// State specifically needed for rendering the detail modal.
/// This might be passed from NadexApp or constructed as needed.
pub struct DetailModalViewState<'a> {
    pub ctx: &'a egui::Context,
    pub screen_rect: egui::Rect,
    pub selected_image_meta: &'a ImageMeta, // Assuming it's always Some when this is called
    pub detail_view_texture_handle: &'a Option<egui::TextureHandle>,
    // Potentially other fields like current_map, data_dir if needed for path construction
    // or if actions here directly interact with filesystem/manifest.
}

/// Shows the image detail modal.
/// Returns an Option<DetailModalAction> indicating if the user took an action.
pub fn show_detail_modal(state: &mut DetailModalViewState) -> Option<DetailModalAction> {
    // All the UI rendering logic for the detail modal will go here.
    // For now, let's just put a placeholder and a close button.

    let mut action: Option<DetailModalAction> = None;

    let default_modal_width = state.screen_rect.width() * 0.5;
    let default_modal_height = state.screen_rect.height() * 0.5;

    let mut modal_target_width = default_modal_width;
    let mut modal_target_height = default_modal_height;
    let mut image_display_max_size =
        egui::vec2(default_modal_width - 60.0, default_modal_height - 200.0);

    let controls_and_padding_height = 200.0;
    let horizontal_padding = 60.0;

    if let Some(texture) = state.detail_view_texture_handle {
        let image_native_size = texture.size_vec2();

        let max_img_display_width = state.screen_rect.width() * 0.75;
        let max_img_display_height = state.screen_rect.height() * 0.70;

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

        modal_target_width = modal_target_width.min(state.screen_rect.width() * 0.95);
        modal_target_height = modal_target_height.min(state.screen_rect.height() * 0.95);

        modal_target_width = modal_target_width.max(400.0);
        modal_target_height = modal_target_height.max(300.0);
    }

    let dim_painter = state.ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new("image_detail_dim_layer"),
    ));
    dim_painter.rect_filled(state.screen_rect, 0.0, egui::Color32::from_black_alpha(180));

    let modal_area_id = egui::Id::new("image_detail_modal_area");

    egui::Area::new(modal_area_id)
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .interactable(true)
        .show(state.ctx, |ui| {
            egui::Frame::popup(ui.style())
                .inner_margin(egui::Margin::same(15.0))
                .show(ui, |ui| {
                    ui.set_max_width(modal_target_width);
                    ui.set_max_height(modal_target_height);
                    ui.set_min_width(modal_target_width.min(state.screen_rect.width() * 0.9));
                    ui.set_min_height(modal_target_height.min(state.screen_rect.height() * 0.9));

                    ui.with_layout(egui::Layout::top_down(egui::Align::RIGHT), |ui| {
                        if ui.button(" X ").on_hover_text("Close (Esc)").clicked()
                            || state.ctx.input(|i| i.key_pressed(egui::Key::Escape))
                        {
                            action = Some(DetailModalAction::Close);
                        }
                    });
                    ui.add_space(5.0);

                    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                        ui.add_space(5.0);
                        ui.heading(format!("Nade: {}", state.selected_image_meta.filename));
                        ui.separator();
                        ui.add_space(10.0);

                        if let Some(texture) = state.detail_view_texture_handle {
                            let img_widget = egui::Image::new(egui::load::SizedTexture::new(
                                texture.id(),
                                texture.size_vec2(),
                            ))
                            .max_size(image_display_max_size)
                            .maintain_aspect_ratio(true);

                            let available_width_for_centering = ui.available_width();
                            let image_width =
                                image_display_max_size.x.min(available_width_for_centering);
                            let margin = (available_width_for_centering - image_width) / 2.0;
                            if margin > 0.0 {
                                ui.add_space(margin); // This might not work as expected in a top_down layout for horizontal centering.
                                // Consider ui.horizontal centered layout for the image if needed.
                            }
                            ui.add(img_widget);
                        } else {
                            ui.label("Loading image...");
                        }

                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(10.0);

                        egui::Grid::new("detail_metadata_grid")
                            .num_columns(2)
                            .spacing([20.0, 5.0])
                            .show(ui, |ui| {
                                ui.strong("Nade Type:");
                                ui.label(format!("{:?}", state.selected_image_meta.nade_type));
                                ui.end_row();

                                ui.strong("Position:");
                                ui.label(if state.selected_image_meta.position.is_empty() {
                                    "[No Position]"
                                } else {
                                    &state.selected_image_meta.position
                                });
                                ui.end_row();

                                ui.strong("Notes:");
                                ui.push_id("notes_scroll_area_detail_view", |ui| {
                                    egui::ScrollArea::vertical()
                                        .auto_shrink([false, true])
                                        .max_height(60.0)
                                        .show(ui, |ui| {
                                            let notes_text =
                                                if state.selected_image_meta.notes.is_empty() {
                                                    "[No Notes]"
                                                } else {
                                                    &state.selected_image_meta.notes
                                                };
                                            ui.add(egui::Label::new(notes_text).wrap(true));
                                        });
                                });
                                ui.end_row();
                            });

                        ui.add_space(15.0);

                        ui.horizontal(|ui| {
                            if ui
                                .button("Edit")
                                .on_hover_text("Edit image details")
                                .clicked()
                            {
                                action = Some(DetailModalAction::RequestEdit(
                                    state.selected_image_meta.clone(),
                                ));
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .button("Delete")
                                        .on_hover_text("Delete this image")
                                        .clicked()
                                    {
                                        action = Some(DetailModalAction::RequestDelete(
                                            state.selected_image_meta.clone(),
                                        ));
                                    }
                                },
                            );
                        });
                    });
                });
        });
    action
}
