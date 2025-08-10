use crate::app_actions::AppAction;
use crate::app_state::AppState;
use crate::persistence::NadeType;
use egui::Ui;

const SIZE_LABELS: [&str; 3] = ["Large", "Medium", "Small"];
const IMAGE_SIZES: [u32; 3] = [957, 637, 477]; // Direct size values for the grid

/// Renders the top bar UI elements (map selection, filters, upload button, etc.).
pub fn show_top_bar(
    app_state: &mut AppState, // Keep &mut for direct UI state like selected_nade_type, grid_image_size updates for immediate feedback
    ui: &mut Ui,
    action_queue: &mut Vec<AppAction>,
) {
    ui.horizontal(|ui_content| {
        // Map selection icon
        ui_content.label("Map:");
        let selected_map_text = app_state.current_map.clone();
        egui::ComboBox::new("map_selector_top_bar", "")
            .selected_text(&selected_map_text)
            .show_ui(ui_content, |ui_combo| {
                for map_name_str in &app_state.maps {
                    // For ComboBox, we need a mutable variable to bind to, even if we don't use its changed state directly for app_state.current_map.
                    // The actual change is driven by the action.
                    let mut current_selection_for_combo = app_state.current_map.clone();
                    if ui_combo
                        .selectable_value(
                            &mut current_selection_for_combo,
                            map_name_str.to_string(),
                            *map_name_str,
                        )
                        .changed()
                    {
                        // Only push action if the selection actually changed to the new map_name_str
                        if app_state.current_map != *map_name_str {
                            action_queue.push(AppAction::SelectMap(map_name_str.to_string()));
                        }
                    }
                }
            });

        // Image size icon
        ui_content.label("Image Size:");
        let current_thumb_idx = IMAGE_SIZES
            .iter()
            .position(|&s| s == app_state.grid_image_size as u32)
            .unwrap_or(0);
        // Create a temporary mutable variable for the ComboBox to bind to.
        let mut temp_selected_idx = current_thumb_idx;
        egui::ComboBox::new("thumb_size_select_top_bar", "")
            .selected_text(
                SIZE_LABELS
                    .get(current_thumb_idx)
                    .cloned()
                    .unwrap_or("Size"), // Fallback selected text
            )
            .show_ui(ui_content, |ui_combo| {
                for (i, &sz) in IMAGE_SIZES.iter().enumerate() {
                    if ui_combo
                        .selectable_value(
                            &mut temp_selected_idx,
                            i,
                            SIZE_LABELS.get(i).cloned().unwrap_or("Unknown"),
                        )
                        .clicked()
                        && app_state.grid_image_size != sz as f32
                    {
                        // The AppState will be updated by the action handler, but for immediate UI feedback in the combo box text,
                        // we can update it here. However, the canonical way is to let the action handler do it.
                        // For now, let's rely on the action handler to update app_state.grid_image_size.
                        action_queue.push(AppAction::SetGridImageSize(sz as f32));
                    }
                }
            });

        // Upload button logic
        if ui_content
            .button("Upload")
            .on_hover_text("Upload Screenshot")
            .clicked()
        {
            action_queue.push(AppAction::ShowUploadModal);
        }

        // Nade Type Filter Buttons
        ui_content.add_space(10.0);
        let original_item_spacing = ui_content.style_mut().spacing.item_spacing.x;
        ui_content.style_mut().spacing.item_spacing.x = 8.0_f32;

        let nade_types_options = [
            (None, "All"),
            (Some(NadeType::Smoke), "Smoke"),
            (Some(NadeType::Flash), "Flash"),
            (Some(NadeType::Molotov), "Molotov"),
            (Some(NadeType::Grenade), "Grenade"),
        ];

        let text_color_selected = ui_content.style().visuals.selection.stroke.color;
        let text_color_unselected = ui_content.style().visuals.widgets.inactive.text_color();

        for (filter_option, label_str) in nade_types_options {
            let is_selected = app_state.selected_nade_type == filter_option;

            let button_text = egui::RichText::new(label_str).color(if is_selected {
                text_color_selected
            } else {
                text_color_unselected
            });

            let mut button = egui::Button::new(button_text);

            if is_selected {
                button = button.fill(ui_content.style().visuals.selection.bg_fill);
            } else {
                button = button.fill(egui::Color32::TRANSPARENT);
            }

            if ui_content.add(button).clicked() && app_state.selected_nade_type != filter_option {
                // Similar to image size, the AppState will be updated by the action handler.
                action_queue.push(AppAction::SetNadeFilter(filter_option));
            }
        }
        ui_content.style_mut().spacing.item_spacing.x = original_item_spacing;

        // Reorder mode toggle button
        ui_content.add_space(15.0);
        let reorder_button_text = if app_state.reorder_mode {
            egui::RichText::new("🔄 Reorder Mode: ON").color(text_color_selected)
        } else {
            egui::RichText::new("🔄 Reorder Mode").color(text_color_unselected)
        };
        
        let mut reorder_button = egui::Button::new(reorder_button_text);
        if app_state.reorder_mode {
            reorder_button = reorder_button.fill(ui_content.style().visuals.selection.bg_fill);
        } else {
            reorder_button = reorder_button.fill(egui::Color32::TRANSPARENT);
        }
        
        if ui_content.add(reorder_button).clicked() {
            action_queue.push(AppAction::ToggleReorderMode);
        }

        // Add flexible space to push the share button to the right
        ui_content.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Share button logic (now on the right side)
            if ui
                .button("Share")
                .on_hover_text("Export or Import Nade Lineups")
                .clicked()
            {
                action_queue.push(AppAction::ShowSharingView);
            }
        });
    });
}
