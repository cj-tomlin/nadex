use eframe::egui;
use crate::persistence::ImageMeta;
use crate::app_state::AppState; // Assuming NadexApp is pub(crate) or pub

#[derive(Debug, Clone, PartialEq)]
pub enum DeleteConfirmationAction {
    ConfirmDelete,
    Cancel,
}

pub fn show_delete_confirmation_modal(
    _app: &AppState, // May need &mut if modal has its own transient state to manage within app struct
    ctx: &egui::Context,
    image_to_delete: &ImageMeta,
) -> Option<DeleteConfirmationAction> {
    let mut action: Option<DeleteConfirmationAction> = None;

    egui::Window::new("Confirm Delete")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(format!("Are you sure you want to delete '{}'?", image_to_delete.filename));
            ui.label("This action cannot be undone.");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Delete").clicked() {
                    action = Some(DeleteConfirmationAction::ConfirmDelete);
                }
                if ui.button("Cancel").clicked() {
                    action = Some(DeleteConfirmationAction::Cancel);
                }
            });
        });

    action
}
