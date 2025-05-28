use egui::{Context, Window, ComboBox, TextEdit, Id, Align2, Vec2};
use crate::persistence::NadeType;
use crate::NadexApp;
use strum::IntoEnumIterator;

/// Data structure for the edit form.
#[derive(Clone, Debug)]
pub struct EditFormData {
    pub filename: String,
    pub nade_type: NadeType,
    pub position: String,
    pub notes: String,
}

/// Actions that can be taken from the edit modal.
pub enum EditModalAction {
    Save(EditFormData),
    Cancel,
}

/// Renders the edit modal for an image.
///
/// Returns an `Option<EditModalAction>`: 
/// - `Some(EditModalAction::Save(data))` if the user clicks "Save".
/// - `Some(EditModalAction::Cancel)` if the user clicks "Cancel".
/// - `None` if the modal is still open or not interacted with in a way that closes it.
pub fn show_edit_modal(
    app_state: &mut NadexApp,
    ctx: &Context,
) -> Option<EditModalAction> {
    let mut action: Option<EditModalAction> = None;
    let mut open = true; // Controls the modal's visibility

    // Ensure edit_form_data exists. If not, and we were supposed to be editing, trigger Cancel.
    // Otherwise, if we weren't editing, no action (None) is fine.
    if app_state.edit_form_data.is_none() {
        return if app_state.editing_image_meta.is_some() {
            app_state.editing_image_meta = None; // Clear editing state if form data is missing unexpectedly
            Some(EditModalAction::Cancel)
        } else {
            None // Not supposed to be editing, so no modal action
        };
    }

    // Clone filename for the window ID and title to avoid borrow checker issues with app_state.edit_form_data inside the closure.
    let filename_for_title = app_state.edit_form_data.as_ref().unwrap().filename.clone();
    let window_id = Id::new("edit_image_modal_window").with(&filename_for_title);

    Window::new(format!("Edit: {}", filename_for_title))
        .id(window_id)
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            // Re-borrow app_state.edit_form_data mutably here, as it's confirmed to be Some.
            if let Some(form_data) = &mut app_state.edit_form_data {
                ui.set_min_width(300.0);
                ui.set_max_width(400.0); 

                // Window title is set above, no need for another heading here.
                ui.separator();
                ui.add_space(10.0);

                egui::Grid::new("edit_form_grid")
                    .num_columns(2)
                    .spacing([10.0, 10.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("Nade Type:");
                        ComboBox::new("nade_type_combo_edit", "") // Unique ID source, empty label
                            .selected_text(format!("{:?}", form_data.nade_type))
                            .show_ui(ui, |ui| {
                                for n_type in NadeType::iter() {
                                    ui.selectable_value(
                                        &mut form_data.nade_type,
                                        n_type,
                                        format!("{:?}", n_type),
                                    );
                                }
                            });
                        ui.end_row();

                        ui.label("Position:");
                        ui.add(TextEdit::singleline(&mut form_data.position).hint_text("e.g., A Site, Mid Doors"));
                        ui.end_row();

                        ui.label("Notes:");
                        ui.add(TextEdit::multiline(&mut form_data.notes).desired_rows(3).hint_text("Brief description or lineup"));
                        ui.end_row();
                    });

                ui.add_space(20.0);
                ui.separator();
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    if ui.button("Save").on_hover_text("Save changes").clicked() {
                        action = Some(EditModalAction::Save(form_data.clone()));
                    }
                    if ui.button("Cancel").on_hover_text("Discard changes").clicked() {
                        action = Some(EditModalAction::Cancel);
                    }
                });
            } else {
                // This state should ideally not be reached due to the initial check.
                ui.label("Error: Form data became unavailable during rendering.");
                action = Some(EditModalAction::Cancel); // Fallback
            }
        });

    // Handle modal closure (either by 'x' button or by Save/Cancel actions)
    if !open || (action.is_some() && (matches!(action, Some(EditModalAction::Save(_))) || matches!(action, Some(EditModalAction::Cancel)))) {
        if action.is_none() { // If closed by 'x', it's a Cancel action
            action = Some(EditModalAction::Cancel);
        }
        // Clear editing state from NadexApp as the modal interaction is complete.
        app_state.editing_image_meta = None;
        app_state.edit_form_data = None; 
    }
    
    action
}
