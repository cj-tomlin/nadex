use crate::persistence::NadeType;
use eframe::egui;
use rfd::FileDialog;
use std::path::PathBuf;
use strum::IntoEnumIterator;

// This struct will hold the local state for the upload modal.
#[derive(Debug, Default)] // Added Default for easier initialization if needed elsewhere
pub struct UploadModal {
    pub file_path: Option<PathBuf>,
    pub nade_type: NadeType,
    pub position: String,
    pub notes: String,
    // Consider adding `pub error_message: Option<String>;` if modal needs to show specific errors
}

impl UploadModal {
    pub fn new() -> Self {
        Self {
            file_path: None,
            nade_type: NadeType::Smoke, // Default nade type
            position: String::new(),
            notes: String::new(),
        }
    }
}

use crate::app_actions::AppAction; // Added
use crate::app_state::AppState; // Added

impl UploadModal {
    #[allow(clippy::too_many_lines)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        app_state: &mut AppState,
        action_queue: &mut Vec<AppAction>,
    ) {
        if !app_state.show_upload_modal {
            return;
        }

        let mut reset_and_close = false;

        egui::Window::new("Upload Image")
            .open(&mut app_state.show_upload_modal) // Directly use AppState's flag
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.vertical_centered_justified(|ui| {
                    // File Picker
                    ui.horizontal(|ui_h| {
                        ui_h.label("File:");
                        let file_display_text = self
                            .file_path
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
                                self.file_path = Some(path);
                            }
                        }
                    });

                    // Nade Type
                    ui.horizontal(|ui_h| {
                        ui_h.label("Nade Type:");
                        egui::ComboBox::from_label("")
                            .selected_text(format!("{:?}", self.nade_type))
                            .show_ui(ui_h, |ui_combo| {
                                for nade_type_iter in NadeType::iter() {
                                    ui_combo.selectable_value(
                                        &mut self.nade_type,
                                        nade_type_iter,
                                        format!("{:?}", nade_type_iter),
                                    );
                                }
                            });
                    });

                    // Position
                    ui.horizontal(|ui_h| {
                        ui_h.label("Position:");
                        ui_h.text_edit_singleline(&mut self.position);
                    });

                    // Notes
                    ui.horizontal(|ui_h| {
                        ui_h.label("Notes:");
                        ui_h.text_edit_multiline(&mut self.notes);
                    });

                    ui.add_space(10.0);

                    // Confirm / Cancel Buttons
                    ui.horizontal(|ui_h| {
                        if ui_h.button("Cancel").clicked() {
                            reset_and_close = true;
                        }
                        // Enable confirm only if a file is selected
                        let confirm_enabled = self.file_path.is_some();
                        ui_h.add_enabled_ui(confirm_enabled, |ui_enabled_h| {
                            if ui_enabled_h.button("Confirm Upload").clicked() {
                                if let Some(file_path) = self.file_path.clone() {
                                    // Shadow to ensure it's Some
                                    action_queue.push(AppAction::SetProcessingUpload(true));
                                    action_queue.push(AppAction::SubmitUpload {
                                        file_path,
                                        map_name: app_state.current_map.clone(), // Get map_name from AppState
                                        nade_type: self.nade_type,               // NadeType is Copy
                                        position: self.position.clone(),
                                        notes: self.notes.clone(),
                                    });
                                    reset_and_close = true;
                                }
                            }
                        });
                    });
                });
            });

        // If the window was closed by the 'x' button, app_state.show_upload_modal will be false.
        // Or if Cancel/Confirm was clicked, reset_and_close is true.
        if reset_and_close || !app_state.show_upload_modal {
            app_state.show_upload_modal = false; // Ensure it's marked as closed
            *self = UploadModal::new(); // Reset the modal's internal state
        }
    }
}
