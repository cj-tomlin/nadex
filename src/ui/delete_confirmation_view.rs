use crate::app_actions::AppAction;
use crate::app_state::AppState;
use crate::persistence::ImageMeta;
use eframe::egui; // Added for action queue

// DeleteConfirmationAction enum removed

pub fn show_delete_confirmation_modal(
    app_state: &mut AppState, // Changed to &mut AppState, though not strictly needed for current logic, good for consistency
    ctx: &egui::Context,
    image_to_delete: &ImageMeta,
    action_queue: &mut Vec<AppAction>,
) {
    let mut open = app_state.show_delete_confirmation.is_some(); // Control window visibility based on app_state
    let mut button_action_taken = false;

    egui::Window::new("Confirm Delete")
        .open(&mut open) // Allow egui to close the window (e.g. via 'x' or escape)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(format!(
                "Are you sure you want to delete '{}'?",
                image_to_delete.filename
            ));
            ui.label("This action cannot be undone.");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Delete").clicked() {
                    action_queue.push(AppAction::DeleteConfirm);
                    button_action_taken = true;
                }
                if ui.button("Cancel").clicked() {
                    action_queue.push(AppAction::DeleteCancel);
                    button_action_taken = true;
                }
            });
        });

    // If the window was closed by egui (e.g., 'x' button) and no button action was taken,
    // it implies a cancel action.
    if !open && !button_action_taken {
        action_queue.push(AppAction::DeleteCancel);
    }
    // Function no longer returns a value
}
