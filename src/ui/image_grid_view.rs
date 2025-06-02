use crate::app_state::AppState;
use egui::{Rounding, Sense, Ui, Vec2};
use crate::persistence::{ImageMeta, NadeType};

use crate::app_actions::AppAction; // Added import

/// Renders the main image grid.
#[allow(clippy::too_many_lines)] // This function is inherently long due to UI logic
pub fn show_image_grid(app: &mut AppState, ui: &mut Ui, action_queue: &mut Vec<AppAction>) {

    // Display image grid for app.current_map
    let data_dir_clone = app.data_dir.clone();

    // Use pre-filtered and sorted images from app.current_map_images
    // The nade_type filter is applied on top of this.
    let filtered_images: Vec<&ImageMeta> = app
        .current_map_images
        .iter()
        .filter(|meta| {
            app.selected_nade_type.is_none()
                || app.selected_nade_type == Some(meta.nade_type.clone())
        })
        .collect();

    let _available_width = ui.available_width();

    // Determine number of columns to fit the window
    let grid_rect = ui.max_rect();
    let spacing = 12.0_f32;
    let min_padding = 8.0_f32;
    let display_width_config = app.grid_image_size;
    let num_columns = ((grid_rect.width() + spacing)
        / (display_width_config + spacing + 2.0_f32 * min_padding))
        .floor()
        .max(1.0_f32) as usize;

    egui::ScrollArea::vertical().show_viewport(ui, |ui, viewport| {
        let grid = egui::Grid::new("image_grid_internal").spacing([spacing, spacing]);
        let display_height_config = display_width_config * 3.0_f32 / 4.0_f32;
        let row_height = display_height_config + spacing;
        let offset_y = viewport.min.y;
        let first_visible_row = (offset_y / row_height).floor().max(0.0) as usize;
        let last_visible_row = ((offset_y + viewport.height()) / row_height).ceil() as usize + 1;

        grid.show(ui, |ui| {
            for (i, meta) in filtered_images.iter().enumerate() {
                let current_meta_ref: &ImageMeta = *meta;

                let img_path_check = data_dir_clone
                    .join(&current_meta_ref.map)
                    .join(&current_meta_ref.filename);
                if !img_path_check.exists() {
                    if i % num_columns != 0 && i != 0 { // Ensure not first item of a new row already
                        // This logic might be tricky, if an item is missing, how to fill grid?
                        // For now, just allocate space if it's supposed to be there.
                    }
                    let display_width = app.grid_image_size;
                    let display_height = app.grid_image_size * 3.0_f32 / 4.0_f32;
                    ui.allocate_space(egui::Vec2::new(display_width, display_height));
                    if (i + 1) % num_columns == 0 || i == filtered_images.len() - 1 {
                        ui.end_row();
                    }
                    continue;
                }

                let this_row = i / num_columns;
                if this_row < first_visible_row || this_row > last_visible_row {
                    let display_width = app.grid_image_size;
                    let display_height = app.grid_image_size * 3.0_f32 / 4.0_f32;
                    let (rect_alloc, _) = ui.allocate_exact_size(
                        Vec2::new(display_width, display_height),
                        Sense::hover(),
                    );
                    ui.painter().rect_filled(
                        rect_alloc,
                        Rounding::default(),
                        egui::Color32::from_gray(30), // Darker placeholder
                    );
                } else {
                    let img_path = data_dir_clone
                        .join(&current_meta_ref.map)
                        .join(&current_meta_ref.filename);
                    let thumb_dir = data_dir_clone
                        .join(&current_meta_ref.map)
                        .join(".thumbnails");
                    let target_display_size = app.grid_image_size as u32;
                    let mut loaded_thumbnail = false;

                    if let Some((texture_handle, (img_w, img_h))) = app.thumbnail_service.lock().unwrap().get_or_load_thumbnail_texture(
                        ui,
                        &img_path,
                        &thumb_dir,
                        target_display_size,
                    ) {
                        let img_w_f32 = *img_w as f32;
                        let img_h_f32 = *img_h as f32;
                        let aspect_ratio = if img_h_f32 > 0.001 {
                            img_w_f32 / img_h_f32
                        } else {
                            4.0 / 3.0
                        }; // Default aspect ratio if height is zero or too small

                        let display_width = app.grid_image_size;
                        let display_height = display_width / aspect_ratio;

                        let image_widget = egui::Image::new(egui::load::SizedTexture::new(
                            texture_handle.id(),
                            // Use actual image dimensions for SizedTexture for correct internal scaling by egui::Image
                            Vec2::new(img_w_f32, img_h_f32),
                        ))
                        .rounding(Rounding::same(4.0))
                        .sense(egui::Sense::click());

                        // ui.add_sized will scale the SizedTexture to fit [display_width, display_height] while maintaining aspect ratio.
                        let image_response =
                            ui.add_sized([display_width, display_height], image_widget);

                        if image_response.clicked() {
                            action_queue.push(AppAction::ImageGridImageClicked(
                                current_meta_ref.clone(),
                            ));
                        }

                        // Persistent overlay for nade info
                        let image_rect = image_response.rect;
                        let painter = ui.painter_at(image_rect);
                        let bar_height = 24.0_f32;
                        let icon_radius = (bar_height * 0.7_f32) / 2.0_f32;
                        let text_padding = 5.0_f32;
                        let font_size_overlay = bar_height * 0.65_f32;
                        let bar_color = egui::Color32::from_rgba_unmultiplied(20, 20, 20, 160);

                        // Top Bar (Nade Type Icon + Position Label)
                        let top_bar_y_start = image_rect.min.y;
                        let top_bar_rect = egui::Rect::from_x_y_ranges(
                            image_rect.x_range(),
                            egui::Rangef::new(top_bar_y_start, top_bar_y_start + bar_height),
                        );
                        painter.rect_filled(top_bar_rect, Rounding::same(4.0), bar_color);

                        let icon_center_y = top_bar_rect.min.y + bar_height / 2.0_f32;
                        let icon_center_x = top_bar_rect.min.x + text_padding + icon_radius;
                        let icon_color = match current_meta_ref.nade_type {
                            NadeType::Smoke => egui::Color32::DARK_GRAY,
                            NadeType::Flash => egui::Color32::WHITE,
                            NadeType::Molotov => egui::Color32::from_rgb(255, 69, 0),
                            NadeType::Grenade => egui::Color32::from_rgb(34, 139, 34),
                        };
                        painter.circle_filled(
                            egui::pos2(icon_center_x, icon_center_y),
                            icon_radius,
                            icon_color,
                        );
                        painter.circle_stroke(
                            egui::pos2(icon_center_x, icon_center_y),
                            icon_radius,
                            egui::Stroke::new(1.0_f32, egui::Color32::BLACK),
                        );

                        let position_text_str = if current_meta_ref.position.is_empty() {
                            "[No Position]".to_string()
                        } else {
                            current_meta_ref.position.clone()
                        };
                        let text_color = egui::Color32::WHITE;
                        let font_id_overlay = egui::FontId::proportional(font_size_overlay);
                        let text_galley = painter.layout_no_wrap(
                            position_text_str,
                            font_id_overlay.clone(),
                            text_color,
                        );
                        let icon_right_boundary = icon_center_x + icon_radius + text_padding;
                        let ideal_text_x = top_bar_rect.center().x - text_galley.size().x / 2.0_f32;
                        let actual_text_x = ideal_text_x.max(icon_right_boundary);
                        let max_text_x = top_bar_rect.max.x - text_padding - text_galley.size().x;
                        let final_text_x = actual_text_x.min(max_text_x);
                        let actual_text_y =
                            top_bar_rect.center().y - text_galley.size().y / 2.0_f32;
                        painter.galley(
                            egui::pos2(final_text_x, actual_text_y),
                            text_galley,
                            text_color,
                        );

                        // Bottom Bar (Notes)
                        let notes_text = if current_meta_ref.notes.is_empty() {
                            "[No Notes]".to_string()
                        } else {
                            current_meta_ref.notes.clone()
                        };
                        let bottom_bar_y_start = image_rect.max.y - bar_height;
                        let bottom_bar_rect = egui::Rect::from_x_y_ranges(
                            image_rect.x_range(),
                            egui::Rangef::new(bottom_bar_y_start, image_rect.max.y),
                        );
                        painter.rect_filled(bottom_bar_rect, Rounding::same(4.0), bar_color);
                        painter.text(
                            bottom_bar_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            notes_text,
                            font_id_overlay,
                            text_color,
                        );
                        loaded_thumbnail = true;
                    }
                    if !loaded_thumbnail {
                        let display_width = app.grid_image_size;
                        let display_height = app.grid_image_size * 3.0_f32 / 4.0_f32;
                        let (rect_alloc, _) = ui.allocate_exact_size(
                            Vec2::new(display_width, display_height),
                            Sense::hover(),
                        );
                        ui.painter().rect_filled(
                            rect_alloc,
                            Rounding::default(),
                            egui::Color32::from_gray(30), // Darker placeholder
                        );
                    }
                }
                if (i + 1) % num_columns == 0 {
                    ui.end_row();
                }
            }
        });
    });

    if filtered_images.is_empty() {
        ui.label("[No images uploaded for this filter]");
    }

}
