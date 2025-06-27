#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use crate::persistence::ImageMeta;
use eframe::{NativeOptions, egui};

use log::{self, LevelFilter};

// persistence::copy_image_to_data is called via persistence::copy_image_to_data_threaded or directly in persistence module
use crate::app_actions::AppAction;
use crate::app_state::AppState;
use crate::ui::sharing_view::SharingView;
use crate::ui::update_dialog::UpdateDialog;
use crate::ui::upload_modal_view::UploadModal;
use std::sync::Arc;

mod app_actions;
mod app_state;
pub mod common;
mod persistence;
mod services;
mod ui;

#[cfg(test)]
pub mod tests_common;

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .filter_module("nadex", LevelFilter::Debug) // Ensure nadex debug logs are shown
        .init();

    log::info!("Starting Nadex application");

    let mut options = NativeOptions::default();
    options.viewport.maximized = Some(true);

    log::info!("Initializing eframe");

    // Get version from Cargo.toml at compile time
    let app_version = env!("CARGO_PKG_VERSION");
    let app_title = format!("Nadex v{}", app_version);

    eframe::run_native(
        &app_title,
        options,
        Box::new(|_cc| {
            log::info!("Creating NadexApp instance");
            let app = NadexApp::default();
            log::info!("NadexApp instance created successfully");
            Ok(Box::new(app) as Box<dyn eframe::App>)
        }),
    )
}

struct NadexApp {
    app_state: AppState,
    action_queue: Vec<AppAction>,
    upload_modal: UploadModal,
    update_dialog: UpdateDialog,
    sharing_view: SharingView, // Added sharing view field
                               // Potentially other fields that are NOT part of the shared AppState,
                               // like UI-specific temporary state or handles not directly tied to core data.
                               // For now, we assume all listed fields moved.
}

impl Default for NadexApp {
    fn default() -> Self {
        let mut app = Self {
            app_state: AppState::new(),
            action_queue: Vec::new(),
            upload_modal: UploadModal::new(),
            update_dialog: UpdateDialog::default(),
            sharing_view: SharingView::new(),
        };

        // filter_images_for_current_map needs to be called after AppState is initialized
        // and it will now operate on app.app_state fields.
        app.filter_images_for_current_map();

        // Start automatic update check on startup
        log::info!("Checking for updates on startup...");
        use std::sync::mpsc;
        let (tx, rx) = mpsc::channel();

        // Start update check in background thread
        let ctx_handle = eframe::egui::Context::default();
        std::thread::spawn(move || {
            let status = crate::services::updater::update_to_latest();
            tx.send(status).unwrap_or_else(|e| {
                log::error!("Failed to send update status: {}", e);
            });
            ctx_handle.request_repaint();
        });

        // Store the update check receiver for processing in the first update cycle
        app.update_dialog.startup_check_receiver = Some(rx);
        app
    }
}

impl NadexApp {
    fn filter_images_for_current_map(&mut self) {
        self.app_state.filter_images_for_current_map();
    }

    fn load_detail_image(&mut self, ctx: &egui::Context, image_meta: &ImageMeta) {
        // First try to load from the full-size WebP in the thumbnails directory
        let map_dir = self.app_state.data_dir.join(&self.app_state.current_map);
        // With WebP-only storage, image files are already in WebP format in the main directory
        let image_path_to_load = map_dir.join(&image_meta.filename);
        match image::open(&image_path_to_load) {
            Ok(img) => {
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [img.width() as usize, img.height() as usize],
                    img.to_rgba8().as_flat_samples().as_slice(),
                );
                let texture_name = format!(
                    "detail_{}_{:?}",
                    image_meta.filename,
                    std::time::SystemTime::now()
                );
                let handle =
                    ctx.load_texture(texture_name, color_image, egui::TextureOptions::default());
                self.app_state.detail_view_texture_handle = Some(handle);
                self.app_state.detail_view_error = None;
            }
            Err(e) => {
                eprintln!(
                    "Failed to load detail image '{}': {}",
                    image_meta.filename, e
                );
                self.app_state.detail_view_error = Some(format!("Error loading image: {}", e));
                self.app_state.selected_image_for_detail = None;
                self.app_state.detail_view_texture_handle = None;
            }
        } // Closes match
    } // Closes load_detail_image

    fn handle_confirm_image_delete(&mut self, meta_to_delete: ImageMeta, ctx: &egui::Context) {
        let filename_to_delete = meta_to_delete.filename.clone(); // Keep for logging and cache clearing

        // Call ImageService to handle deletion (file removal, manifest update, manifest save)
        match self
            .app_state
            .image_service
            .delete_image(&meta_to_delete, &mut self.app_state.image_manifest)
        {
            Ok(_) => {
                log::info!(
                    "Image '{}' deleted successfully via ImageService.",
                    filename_to_delete
                );
                self.app_state.error_message = None; // Clear previous error on successful deletion
            }
            Err(e) => {
                log::error!(
                    "ImageService failed to delete image '{}': {}",
                    filename_to_delete,
                    e
                );
                self.app_state.error_message = Some(format!("Failed to delete image: {}", e));
                // Note: Even if ImageService fails, we proceed to UI cleanup.
                // Depending on the error, some parts (like file deletion) might have partially succeeded.
            }
        }

        // The on-disk thumbnails are deleted by ImageService -> PersistenceService -> ThumbnailService.
        // UI elements (grid, detail view) will refresh based on the updated image_manifest / current_map_images.
        // The detail_view_texture_handle is cleared separately.
        // No explicit, separate UI-wide thumbnail cache object needs manual clearing here beyond what egui manages for displayed textures.
        log::debug!(
            "Image deletion process completed for: '{}'. UI will refresh.",
            filename_to_delete
        );

        // UI state updates
        self.app_state.selected_image_for_detail = None;
        self.app_state.detail_view_texture_handle = None;
        self.app_state.show_delete_confirmation = None;
        self.filter_images_for_current_map(); // Refresh the grid view
        ctx.request_repaint();
    }

    fn handle_save_image_edit(
        &mut self,
        form_data_to_save: ui::edit_view::EditFormData,
        ctx: &egui::Context,
    ) {
        // Get the original metadata, which includes the map name
        if let Some(original_meta) = self.app_state.editing_image_meta.clone() {
            match self.app_state.image_service.update_image_metadata(
                &mut self.app_state.image_manifest,
                &original_meta, // Pass the original meta to locate the image in the correct map
                &form_data_to_save,
            ) {
                Ok(_) => {
                    log::info!(
                        "Image metadata updated and manifest saved successfully via ImageService for '{}'.",
                        form_data_to_save.filename
                    );
                    self.app_state.error_message = None;
                }
                Err(e) => {
                    log::error!(
                        "ImageService failed to update image metadata or save manifest: {}",
                        e
                    );
                    self.app_state.error_message = Some(format!("Failed to save changes: {}", e));
                }
            }

            // Common cleanup regardless of success or failure of the service call or save
            self.app_state.editing_image_meta = None;
            self.app_state.edit_form_data = None;
            self.filter_images_for_current_map(); // Refresh the view
            ctx.request_repaint();
        } else {
            log::error!(
                "Critical Error: editing_image_meta was None when trying to save edit for {}. This should not happen.",
                form_data_to_save.filename
            );
            self.app_state.error_message =
                Some("Internal error: No image was being edited. Please try again.".to_string());
            // Also clear edit state here to prevent further issues
            self.app_state.editing_image_meta = None;
            self.app_state.edit_form_data = None;
        }
    }
}

impl eframe::App for NadexApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // --- Process AppActions ---

        // Check for results from background upload threads
        let mut received_actions_from_thread = false; // Initialize flag
        while let Ok(action_from_thread) = self.app_state.upload_result_receiver.try_recv() {
            log::info!(
                "Received action from background thread: {:?}",
                action_from_thread
            );
            self.action_queue.push(action_from_thread);
            received_actions_from_thread = true; // Set flag
        }

        if received_actions_from_thread {
            ctx.request_repaint(); // Request repaint if actions were received
        }

        // Process results from thumbnail loading thread incrementally
        let mut new_thumbnails_loaded_this_frame = false;
        const MAX_THUMB_RESULTS_PER_FRAME: usize = 5; // Process up to 5 thumbnails per frame

        let mut results_batch = Vec::new();
        for _ in 0..MAX_THUMB_RESULTS_PER_FRAME {
            match self.app_state.thumbnail_result_receiver.try_recv() {
                Ok(result) => results_batch.push(result),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // Channel is empty, stop collecting for this frame
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    log::error!("Thumbnail result channel has disconnected.");
                    // TODO: Consider if the worker thread needs to be respawned or app needs to handle this state
                    break;
                }
            }
        }

        if !results_batch.is_empty() {
            // Lock the service once to process the current batch
            match self.app_state.thumbnail_service.lock() {
                Ok(mut service) => {
                    if service.process_loaded_thumbnails(ctx, results_batch) {
                        new_thumbnails_loaded_this_frame = true;
                    }
                }
                Err(poisoned_error) => {
                    log::error!(
                        "ThumbnailService mutex is poisoned: {}. Unable to process thumbnails.",
                        poisoned_error
                    );
                    // Handle poisoned mutex, e.g., by trying to reinitialize or shut down gracefully.
                }
            }
        }

        if new_thumbnails_loaded_this_frame {
            ctx.request_repaint(); // Request repaint if new thumbnails were loaded this frame
        }

        let actions_to_process = self.action_queue.drain(..).collect::<Vec<_>>();
        if !actions_to_process.is_empty() {
            for action in actions_to_process {
                match action {
                    AppAction::SelectMap(map_name) => {
                        self.app_state.current_map = map_name;
                        self.filter_images_for_current_map();
                        self.app_state.selected_image_for_detail = None;
                        self.app_state.detail_view_texture_handle = None;
                        // It's good practice to request repaint if state that affects UI changes.
                        ctx.request_repaint();
                    }
                    AppAction::SubmitUpload {
                        file_path,
                        map_name,
                        nade_type,
                        position,
                        notes,
                    } => {
                        log::info!(
                            "Offloading SubmitUpload for map: {}, file: {:?}",
                            map_name,
                            file_path
                        );

                        let image_service_clone = Arc::clone(&self.app_state.image_service);
                        let sender_clone = self.app_state.upload_result_sender.clone();
                        // Clone other data needed by the thread
                        let file_path_clone = file_path.clone();
                        let map_name_clone = map_name.clone();
                        // nade_type, position, notes are Copy or easily clonable

                        let initial_manifest_clone = self.app_state.image_manifest.clone();
                        std::thread::spawn(move || {
                            log::info!(
                                "Background thread: Delegating to ImageService.orchestrate_full_upload_process for file: {:?}",
                                file_path_clone
                            );
                            image_service_clone.orchestrate_full_upload_process(
                                file_path_clone,
                                map_name_clone,
                                nade_type,
                                position,
                                notes,
                                initial_manifest_clone, // Use the manifest cloned outside the thread
                                sender_clone,
                            );
                            // ImageService::orchestrate_full_upload_process will send
                            // AppAction::UploadSucceededBackgroundTask or AppAction::UploadFailed
                            // via the sender_clone. No need to send actions from this thread.
                        });

                        // The UI thread continues, spinner is managed by SetProcessingUpload actions.
                        ctx.request_repaint();
                    }
                    AppAction::SetProcessingUpload(is_processing) => {
                        log::info!("Setting is_processing_upload to: {}", is_processing);
                        self.app_state.is_processing_upload = is_processing;
                        ctx.request_repaint();
                    }
                    AppAction::UploadSucceededBackgroundTask {
                        new_image_meta,
                        map_name,
                    } => {
                        log::info!(
                            "Processing UploadSucceededBackgroundTask for image: {:?}, map: {}",
                            new_image_meta.filename,
                            map_name
                        );
                        // Update in-memory manifest
                        let is_current_map = map_name == self.app_state.current_map; // Compare before map_name is moved

                        self.app_state
                            .image_manifest
                            .images
                            .entry(map_name) // map_name is moved here
                            .or_default()
                            .push(new_image_meta.clone()); // Clone new_image_meta for the manifest, original is still available

                        // We no longer need to create a duplicate WebP in .thumbnails directory
                        // The main WebP file is already created in the map directory during upload
                        // and the display logic uses that file directly
                        log::info!(
                            "Using WebP file from main directory: {}",
                            new_image_meta.filename
                        );

                        // If the uploaded image is for the currently viewed map, update current_map_images directly.
                        if is_current_map {
                            self.app_state.current_map_images.push(new_image_meta); // Original new_image_meta is moved here
                            self.app_state
                                .current_map_images
                                .sort_by(|a, b| a.filename.cmp(&b.filename));
                        } else {
                            // If the upload was for a different map, filter_images_for_current_map will handle it
                            // when that map is next selected. The original new_image_meta is dropped here if not used,
                            // which is fine as it's already cloned into the manifest.
                        }
                        self.app_state.error_message = None; // Clear any previous error, UI part was fine.

                        // Manifest saving is now orchestrated by ImageService after this action is sent.
                        // NadexApp only needs to update its in-memory state here.
                        // A repaint is good to reflect the new image in the grid immediately.
                        ctx.request_repaint();
                    }
                    AppAction::UploadFailed { error_message } => {
                        log::error!("Processing UploadFailed action: {:?}", error_message);
                        self.app_state.error_message = error_message;
                        self.action_queue
                            .push(AppAction::SetProcessingUpload(false)); // Hide spinner
                        ctx.request_repaint();
                    }
                    AppAction::ManifestSaveCompleted {
                        success,
                        error_message,
                    } => {
                        log::info!(
                            "Processing ManifestSaveCompleted: success={}, error_message={:?}",
                            success,
                            error_message
                        );
                        self.app_state.is_processing_upload = false; // Hide spinner
                        if !success {
                            self.app_state.error_message = error_message.or_else(|| {
                                Some("Failed to save manifest after upload.".to_string())
                            });
                        } else {
                            self.app_state.error_message = None; // Clear any previous error
                        }
                        ctx.request_repaint();
                    }
                    AppAction::SetGridImageSize(size) => {
                        self.app_state.grid_image_size = size;
                        // TODO: Consider if thumbnail cache needs pruning/clearing here or if it's handled elsewhere.
                        // For now, a simple repaint should reflect the size change in how items are laid out or scaled.
                        ctx.request_repaint();
                    }
                    AppAction::ShowUploadModal => {
                        self.app_state.show_upload_modal = true;
                        // Repaint is likely handled by the modal itself when it shows.
                    }
                    AppAction::SetNadeFilter(nade_type) => {
                        self.app_state.selected_nade_type = nade_type;
                        self.filter_images_for_current_map();
                        self.app_state.selected_image_for_detail = None; // Clear detail view if filter changes
                        self.app_state.detail_view_texture_handle = None;
                        ctx.request_repaint();
                    }
                    AppAction::ImageGridImageClicked(meta) => {
                        if self
                            .app_state
                            .selected_image_for_detail
                            .as_ref()
                            .is_some_and(|selected| selected.filename == meta.filename)
                        {
                            self.app_state.selected_image_for_detail = None;
                            self.app_state.detail_view_texture_handle = None;
                        } else {
                            self.app_state.selected_image_for_detail = Some(meta.clone());
                            self.app_state.detail_view_texture_handle = None; // Texture will be loaded
                            self.load_detail_image(ctx, &meta);
                        }
                        ctx.request_repaint();
                    }
                    AppAction::DetailModalClose => {
                        self.app_state.selected_image_for_detail = None;
                        self.app_state.detail_view_texture_handle = None;
                        self.app_state.detail_view_error = None;
                        self.app_state.editing_image_meta = None;
                        self.app_state.edit_form_data = None;
                        ctx.request_repaint();
                    }
                    AppAction::DetailModalRequestEdit(meta) => {
                        self.app_state.editing_image_meta = Some(meta.clone());
                        self.app_state.edit_form_data =
                            Some(ui::edit_view::EditFormData::from_meta(&meta));
                        self.app_state.selected_image_for_detail = None;
                        self.app_state.detail_view_texture_handle = None;
                        self.app_state.detail_view_error = None;
                        ctx.request_repaint();
                    }
                    AppAction::DetailModalRequestDelete(meta) => {
                        self.app_state.show_delete_confirmation = Some(meta);
                        self.app_state.selected_image_for_detail = None;
                        self.app_state.detail_view_texture_handle = None;
                        self.app_state.detail_view_error = None;
                        ctx.request_repaint();
                    }
                    AppAction::EditModalSave(form_data) => {
                        self.handle_save_image_edit(form_data, ctx);
                        // Note: handle_save_image_edit should handle repainting and clearing editing_image_meta/edit_form_data on success/failure.
                    }
                    AppAction::EditModalCancel => {
                        self.app_state.editing_image_meta = None;
                        self.app_state.edit_form_data = None;
                        self.app_state.error_message = None; // Clear any potential error from a previous failed edit attempt
                        ctx.request_repaint();
                    }
                    AppAction::DeleteConfirm => {
                        if let Some(meta_to_delete) =
                            self.app_state.show_delete_confirmation.clone()
                        {
                            // It's important that handle_confirm_image_delete also sets show_delete_confirmation to None.
                            // Or, we do it here explicitly before/after the call if it doesn't.
                            // Based on old comment, handle_confirm_image_delete handles this.
                            self.handle_confirm_image_delete(meta_to_delete, ctx);
                        } else {
                            // This case should ideally not happen if DeleteConfirm is only sent when modal is shown.
                            log::warn!(
                                "DeleteConfirm action received but no image was marked for deletion."
                            );
                        }
                    }
                    AppAction::DeleteCancel => {
                        self.app_state.show_delete_confirmation = None;
                        ctx.request_repaint();
                    }
                    AppAction::ShowSharingView => {
                        self.app_state.show_sharing_view = true;
                        ctx.request_repaint();
                    }
                } // End match action
            } // End for loop
        } // End if !actions_to_process.is_empty()

        // Check for completed thumbnail jobs
        while let Ok(result) = self.app_state.thumbnail_result_receiver.try_recv() {
            if let Some(err_msg) = &result.error {
                log::error!(
                    "Thumbnail generation failed for key '{}': {}",
                    result.thumb_path_key,
                    err_msg
                );
            } else if let Some(color_image) = result.color_image {
                if let Some(dimensions) = result.dimensions {
                    log::debug!(
                        "Received thumbnail for key '{}', w: {}, h: {}",
                        result.thumb_path_key,
                        dimensions.0,
                        dimensions.1
                    );
                    // Lock the thumbnail_service and process the completed job
                    // Ensure thumbnail_service is Arc<Mutex<ConcreteThumbnailService>> or similar
                    let mut thumbnail_service = self.app_state.thumbnail_service.lock().unwrap();
                    thumbnail_service.process_completed_job(
                        result.thumb_path_key,
                        color_image,
                        dimensions,
                        ctx, // Pass the egui::Context
                    );
                } else {
                    log::error!(
                        "Thumbnail generation succeeded for key '{}' but dimensions are missing.",
                        result.thumb_path_key
                    );
                }
            } else {
                log::warn!(
                    "Received thumbnail result for key '{}' with no image and no error.",
                    result.thumb_path_key
                );
            }
        }

        // Top Bar
        egui::TopBottomPanel::top("top_panel").show(ctx, |top_ui| {
            top_ui.horizontal(|ui| {
                ui::top_bar_view::show_top_bar(&mut self.app_state, ui, &mut self.action_queue);
            });
        });

        // Main Central Panel
        egui::CentralPanel::default()
            .frame(egui::Frame::default().inner_margin(egui::Margin {
                left: 0,
                right: 0,
                top: 0,
                bottom: 0,
            }))
            .show(ctx, |panel_ui| {
                // Show error message if any
                if let Some(ref msg) = self.app_state.error_message {
                    panel_ui.colored_label(egui::Color32::RED, msg);
                }

                panel_ui.add_space(4.0);

                // Call the refactored image grid view function
                ui::image_grid_view::show_image_grid(
                    &mut self.app_state,
                    panel_ui,
                    &mut self.action_queue,
                );

                // --- Image Detail View Modal ---
                if self.app_state.selected_image_for_detail.is_some() {
                    // panel_ui is in scope here
                    ui::detail_view::show_detail_modal(
                        &mut self.app_state,
                        panel_ui,
                        &mut self.action_queue,
                    );
                }
            });

        // --- Upload Progress Indicator ---
        ui::progress_indicator_view::show(ctx, self.app_state.is_processing_upload);

        // --- Upload Modal ---
        if self.app_state.show_upload_modal {
            self.upload_modal
                .show(ctx, &mut self.app_state, &mut self.action_queue);
        }

        // --- Edit Image Modal (Refactored) ---
        if let Some(current_editing_meta) = &self.app_state.editing_image_meta.clone() {
            if self.app_state.edit_form_data.is_none()
                || self
                    .app_state
                    .edit_form_data
                    .as_ref()
                    .is_none_or(|data| data.filename != current_editing_meta.filename)
            {
                self.app_state.edit_form_data = Some(ui::edit_view::EditFormData {
                    filename: current_editing_meta.filename.clone(),
                    nade_type: current_editing_meta.nade_type,
                    position: current_editing_meta.position.clone(),
                    notes: current_editing_meta.notes.clone(),
                });
            }

            ui::edit_view::show_edit_modal(&mut self.app_state, ctx, &mut self.action_queue);
        }

        // --- Delete Confirmation Modal (Refactored) ---
        if let Some(meta_to_delete) = self.app_state.show_delete_confirmation.clone() {
            ui::delete_confirmation_view::show_delete_confirmation_modal(
                &mut self.app_state,
                ctx,
                &meta_to_delete,
                &mut self.action_queue,
            );
        }

        // --- Update Dialog ---
        self.update_dialog.show(ctx);

        // --- Sharing View Modal ---
        if self.app_state.show_sharing_view {
            egui::Window::new("Share Nade Lineups")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    self.sharing_view.show(ui, &mut self.app_state);

                    ui.add_space(10.0);
                    if ui.button("Close").clicked() {
                        self.app_state.show_sharing_view = false;
                    }
                });
        }
    }
}
