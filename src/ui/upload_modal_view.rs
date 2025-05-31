use crate::app_state::AppState;
use crate::persistence::NadeType;
use eframe::egui;
use rfd::FileDialog;
use std::path::PathBuf;
use strum::IntoEnumIterator;

#[derive(Debug)]
pub enum UploadModalAction {
    UploadConfirmed {
        file_path: PathBuf,
        nade_type: NadeType,
        position: String,
        notes: String,
    },
    Cancel,
}

#[allow(clippy::too_many_lines)]
pub fn show_upload_modal(
    app: &mut AppState,
    ctx: &egui::Context,
) -> Option<UploadModalAction> {
    let mut action: Option<UploadModalAction> = None;
    let mut open = true; // To control window visibility from within

    // Retain values for the modal fields from app_state
    // These will be updated by the UI elements
    let mut modal_file_path = app.upload_modal_file.clone();
    let mut modal_nade_type = app.upload_modal_nade_type.clone();
    let mut modal_position = app.upload_modal_position.clone();
    let mut modal_notes = app.upload_modal_notes.clone();

    egui::Window::new("Upload Image")
        .open(&mut open) // Allows closing the window via 'x'
        .collapsible(false)
        .resizable(true)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.vertical_centered_justified(|ui| {
                // File Picker
                ui.horizontal(|ui_h| {
                    ui_h.label("File:");
                    let file_display_text = modal_file_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .and_then(|os| os.to_str())
                        .unwrap_or("No file selected");
                    ui_h.label(file_display_text);
                    if ui_h.button("Browse...").clicked() {
                        if let Some(path) = FileDialog::new()
                            .add_filter("Image", &["png", "jpg", "jpeg", "gif"])
                            .pick_file()
                        {
                            modal_file_path = Some(path);
                        }
                    }
                });

                // Nade Type
                ui.horizontal(|ui_h| {
                    ui_h.label("Nade Type:");
                    egui::ComboBox::from_label("")
                        .selected_text(format!("{:?}", modal_nade_type))
                        .show_ui(ui_h, |ui_combo| {
                            for nade_type_iter in NadeType::iter() {
                                ui_combo.selectable_value(
                                    &mut modal_nade_type,
                                    nade_type_iter,
                                    format!("{:?}", nade_type_iter),
                                );
                            }
                        });
                });

                // Position
                ui.horizontal(|ui_h| {
                    ui_h.label("Position:");
                    ui_h.text_edit_singleline(&mut modal_position);
                });

                // Notes
                ui.horizontal(|ui_h| {
                    ui_h.label("Notes:");
                    ui_h.text_edit_multiline(&mut modal_notes);
                });

                ui.add_space(10.0);

                // Confirm / Cancel Buttons
                ui.horizontal(|ui_h| {
                    if ui_h.button("Cancel").clicked() {
                        action = Some(UploadModalAction::Cancel);
                    }
                    // Enable confirm only if a file is selected
                    let confirm_enabled = modal_file_path.is_some();
                    ui_h.add_enabled_ui(confirm_enabled, |ui_enabled_h|{
                        if ui_enabled_h.button("Confirm Upload").clicked() {
                            if let Some(file_path) = modal_file_path.clone() { // Shadow to ensure it's Some
                                action = Some(UploadModalAction::UploadConfirmed {
                                    file_path,
                                    nade_type: modal_nade_type.clone(),
                                    position: modal_position.clone(),
                                    notes: modal_notes.clone(),
                                });
                            }
                        }
                    });
                });
            });
        });

    // Update app_state with the potentially changed modal field values
    // This ensures if the modal is re-opened without confirming/cancelling (e.g. focus lost),
    // the entered data is preserved.
    app.upload_modal_file = modal_file_path;
    app.upload_modal_nade_type = modal_nade_type;
    app.upload_modal_position = modal_position;
    app.upload_modal_notes = modal_notes;

    if !open { // If window was closed via 'x'
        action = Some(UploadModalAction::Cancel);
    }

    action
}
