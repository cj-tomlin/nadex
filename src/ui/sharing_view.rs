// src/ui/sharing_view.rs
use crate::app_state::AppState;
use eframe::egui;
use rfd::FileDialog;
use std::thread;

pub struct SharingView {
    export_in_progress: bool,
    import_in_progress: bool,
    last_status_message: Option<String>,
    last_status_is_error: bool,
}

impl Default for SharingView {
    fn default() -> Self {
        Self {
            export_in_progress: false,
            import_in_progress: false,
            last_status_message: None,
            last_status_is_error: false,
        }
    }
}

impl SharingView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn show(&mut self, ui: &mut egui::Ui, app_state: &mut AppState) {
        ui.vertical(|ui| {
            ui.heading("Share Your Nade Lineups");
            ui.add_space(10.0);

            // Export section
            ui.group(|ui| {
                ui.set_min_width(540.0);
                ui.vertical(|ui| {
                    ui.heading("Export Nade Library");
                    ui.label("Create a file with all your nade lineups to share with friends.");
                    ui.add_space(5.0);

                    if ui
                        .add_enabled(
                            !self.export_in_progress,
                            egui::Button::new("Export Library"),
                        )
                        .clicked()
                    {
                        self.handle_export(app_state);
                    }
                });
            });

            ui.add_space(15.0);

            // Import section
            ui.group(|ui| {
                ui.set_min_width(540.0);
                ui.vertical(|ui| {
                    ui.heading("Import Nade Library");
                    ui.label("Import nade lineups shared by your friends.");
                    ui.add_space(5.0);

                    if ui
                        .add_enabled(
                            !self.import_in_progress,
                            egui::Button::new("Import Library"),
                        )
                        .clicked()
                    {
                        self.handle_import(app_state);
                    }
                });
            });

            // Status message
            if let Some(message) = &self.last_status_message {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if self.last_status_is_error {
                        ui.colored_label(egui::Color32::RED, "❌");
                    } else {
                        ui.colored_label(egui::Color32::GREEN, "✓");
                    }
                    ui.label(message);
                });
            }
        });
    }

    fn handle_export(&mut self, app_state: &AppState) {
        self.export_in_progress = true;
        self.last_status_message = None;

        // Ask the user where to save the file
        if let Some(path) = FileDialog::new()
            .add_filter("Nade Lineup Package", &["nadex"])
            .set_file_name("my_nade_lineups.nadex")
            .save_file()
        {
            // Create an instance of the export service
            let export_service = crate::services::export_service::ExportService::new(
                app_state.persistence_service.clone(),
            );

            // Perform the export
            match export_service.export_library(&path, &app_state.data_dir) {
                Ok(_) => {
                    self.last_status_message = Some(format!(
                        "Successfully exported nade library to {}",
                        path.display()
                    ));
                    self.last_status_is_error = false;
                }
                Err(err) => {
                    self.last_status_message = Some(format!("Export failed: {}", err));
                    self.last_status_is_error = true;
                }
            }
        }

        self.export_in_progress = false;
    }

    fn handle_import(&mut self, app_state: &mut AppState) {
        self.import_in_progress = true;
        self.last_status_message = None;

        // Set app_state flag to show progress indicator
        app_state.is_processing_upload = true;

        // Ask the user to select a file to import
        if let Some(path) = FileDialog::new()
            .add_filter("Nade Lineup Package", &["nadex"])
            .pick_file()
        {
            // Clone what we need for the background thread
            let path_clone = path.clone();
            let persistence_service_clone = app_state.persistence_service.clone();

            // Create a thread to handle the import operation
            let handle = thread::spawn(move || {
                // Create an instance of the export service
                let export_service =
                    crate::services::export_service::ExportService::new(persistence_service_clone);

                // Perform the import
                export_service.import_library(&path_clone)
            });

            // Wait for the import to complete
            match handle.join() {
                Ok(import_result) => {
                    match import_result {
                        Ok(updated_manifest) => {
                            // Update the app state with the new manifest
                            app_state.image_manifest = updated_manifest;
                            app_state.filter_images_for_current_map();

                            self.last_status_message = Some(format!(
                                "Successfully imported nade library from {}",
                                path.display()
                            ));
                            self.last_status_is_error = false;
                        }
                        Err(err) => {
                            self.last_status_message = Some(format!("Import failed: {}", err));
                            self.last_status_is_error = true;
                        }
                    }
                }
                Err(_) => {
                    self.last_status_message =
                        Some("Import failed: Background process error".to_string());
                    self.last_status_is_error = true;
                }
            }
        }

        // Hide the progress indicator
        app_state.is_processing_upload = false;
        self.import_in_progress = false;
    }
}
