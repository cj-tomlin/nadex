use crate::app_actions::AppAction; // Added AppAction for the queue
use crate::app_state::AppState;
use crate::persistence::ImageMeta;
use crate::persistence::NadeType;
use egui::{Align2, ComboBox, Context, Id, TextEdit, Vec2, Window};
use strum::IntoEnumIterator;

/// Data structure for the edit form.
#[derive(Clone, Debug)]
pub struct EditFormData {
    pub filename: String,
    pub nade_type: NadeType,
    pub position: String,
    pub notes: String,
}

impl EditFormData {
    pub fn from_meta(meta: &ImageMeta) -> Self {
        Self {
            filename: meta.filename.clone(),
            nade_type: meta.nade_type,
            position: meta.position.clone(),
            notes: meta.notes.clone(),
        }
    }
}

// EditModalAction enum removed as part of refactor

/// Renders the edit modal for an image.
///
/// Pushes `AppAction::EditModalSave` or `AppAction::EditModalCancel` to the action queue.
pub fn show_edit_modal(app_state: &mut AppState, ctx: &Context, action_queue: &mut Vec<AppAction>) {
    let mut open = app_state.editing_image_meta.is_some() && app_state.edit_form_data.is_some();
    let mut an_action_was_pushed_by_buttons = false;

    // Ensure edit_form_data exists. If not, and we were supposed to be editing, push Cancel action.
    // This modal should only be called if app_state.editing_image_meta is Some.
    // The presence of app_state.edit_form_data is critical.
    if !open {
        // if modal should not be open from the start due to state
        if app_state.editing_image_meta.is_some() && app_state.edit_form_data.is_none() {
            // This indicates an inconsistent state if we intended to edit but have no form data.
            action_queue.push(AppAction::EditModalCancel);
        }
        return; // Exit if no form data or not supposed to be editing.
    }

    // The following check was for the case where edit_form_data is None AFTER we decided to open.
    // This is now covered by the initial `open` and `return` logic.
    // if app_state.edit_form_data.is_none() {
    //     if app_state.editing_image_meta.is_some() {
    //         action_queue.push(AppAction::EditModalCancel);
    //     }
    //     return;
    // }

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
                        ui.add(
                            TextEdit::singleline(&mut form_data.position)
                                .hint_text("e.g., A Site, Mid Doors"),
                        );
                        ui.end_row();

                        ui.label("Notes:");
                        ui.add(
                            TextEdit::multiline(&mut form_data.notes)
                                .desired_rows(3)
                                .hint_text("Brief description or lineup"),
                        );
                        ui.end_row();
                    });

                ui.add_space(20.0);
                ui.separator();
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    if ui.button("Save").on_hover_text("Save changes").clicked() {
                        action_queue.push(AppAction::EditModalSave(form_data.clone()));
                        an_action_was_pushed_by_buttons = true;
                        // The modal will close because AppState will change via the action handler
                    }
                    if ui
                        .button("Cancel")
                        .on_hover_text("Discard changes")
                        .clicked()
                    {
                        action_queue.push(AppAction::EditModalCancel);
                        an_action_was_pushed_by_buttons = true;
                        // The modal will close because AppState will change via the action handler
                    }
                });
            } else {
                // This state should ideally not be reached due to the initial check.
                ui.label("Error: Form data became unavailable during rendering.");
                action_queue.push(AppAction::EditModalCancel); // Fallback
                an_action_was_pushed_by_buttons = true;
            }
        });

    // After the window is shown, `open` will be false if the user clicked the 'x' button.
    // If `open` is false at this point, and we haven't already pushed an action via buttons,
    // it means the user closed the modal through egui's native close controls.
    if !open && !an_action_was_pushed_by_buttons {
        action_queue.push(AppAction::EditModalCancel);
    }
    // The function no longer returns a value.
}
