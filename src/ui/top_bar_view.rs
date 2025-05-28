use egui::Ui;
use crate::NadexApp;
use crate::persistence::NadeType;
use crate::thumbnail::ALLOWED_THUMB_SIZES;

/// Actions that can be triggered from the top bar.
#[derive(Debug)]
pub enum TopBarAction {
    MapSelected(String),
    NadeTypeFilterChanged(Option<NadeType>),
    ImageSizeChanged(f32),
    UploadButtonPushed,
}

/// Renders the top bar UI elements (map selection, filters, upload button, etc.).
/// 
/// Returns `Option<TopBarAction>` if an action was taken by the user.
pub fn show_top_bar(
    app_state: &mut NadexApp,
    ui: &mut Ui,
) -> Option<TopBarAction> {
    let mut action: Option<TopBarAction> = None;

    ui.horizontal(|ui_content| {
        // Map selection icon
        ui_content.label("Map:");
        let selected_map_text = app_state.current_map.clone();
        egui::ComboBox::new("map_selector_top_bar", "")
            .selected_text(&selected_map_text)
            .show_ui(ui_content, |ui_combo| {
                for map_name_str in &app_state.maps {
                    let mut map_selectable = app_state.current_map.clone();
                    if ui_combo
                        .selectable_value(&mut map_selectable, map_name_str.to_string(), *map_name_str)
                        .changed()
                    {
                        app_state.current_map = map_selectable.clone();
                        action = Some(TopBarAction::MapSelected(map_selectable));
                    }
                }
            });

        // Image size icon
        ui_content.label("Image Size:");
        let current_thumb_idx = ALLOWED_THUMB_SIZES
            .iter()
            .position(|&s| s == app_state.grid_image_size as u32)
            .unwrap_or(0);
        egui::ComboBox::new("thumb_size_select_top_bar", "")
            .selected_text(format!(
                "{} px",
                ALLOWED_THUMB_SIZES
                    .get(current_thumb_idx)
                    .cloned()
                    .unwrap_or(app_state.grid_image_size as u32)
            ))
            .show_ui(ui_content, |ui_combo| {
                for (i, &sz) in ALLOWED_THUMB_SIZES.iter().enumerate() {
                    let mut temp_idx = current_thumb_idx;
                    if ui_combo
                        .selectable_value(&mut temp_idx, i, format!("{} px", sz))
                        .clicked()
                    {
                         if app_state.grid_image_size != sz as f32 {
                            app_state.grid_image_size = sz as f32;
                            action = Some(TopBarAction::ImageSizeChanged(app_state.grid_image_size));
                         }
                    }
                }
            });

        // Upload button logic
        if ui_content
            .button("Upload")
            .on_hover_text("Upload Screenshot")
            .clicked()
        {
            action = Some(TopBarAction::UploadButtonPushed);
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
        let text_color_unselected =
            ui_content.style().visuals.widgets.inactive.text_color();

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

            if ui_content.add(button).clicked() {
                if app_state.selected_nade_type != filter_option {
                    app_state.selected_nade_type = filter_option;
                    action = Some(TopBarAction::NadeTypeFilterChanged(filter_option));
                }
            }
        }
        ui_content.style_mut().spacing.item_spacing.x = original_item_spacing;
    });

    action
}
