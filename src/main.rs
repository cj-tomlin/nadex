use crate::persistence::{ImageMeta, NadeType};
use eframe::{NativeOptions, egui};
use env_logger; // Import env_logger
use image;
use image::GenericImageView;
use log::{self, LevelFilter};

use std::path::PathBuf;

use std::time::Instant;
use std::sync::Arc;
use crate::services::persistence_service::PersistenceService;
use crate::services::thumbnail_service::ThumbnailService;

// persistence::copy_image_to_data is called via persistence::copy_image_to_data_threaded or directly in persistence module
use crate::app_state::AppState;
use crate::app_actions::AppAction;

mod app_logic;
mod persistence;
mod ui;
mod app_state;
mod app_actions;
mod services;

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .filter_module("nadex", LevelFilter::Debug) // Ensure nadex debug logs are shown
        .init();
    let mut options = NativeOptions::default();
    options.viewport.maximized = Some(true);
    eframe::run_native(
        "nadex",
        options,
        Box::new(|_cc| Box::new(NadexApp::default())),
    )
}

struct NadexApp {
    app_state: AppState,
    action_queue: Vec<AppAction>,
    // Potentially other fields that are NOT part of the shared AppState,
    // like UI-specific temporary state or handles not directly tied to core data.
    // For now, we assume all listed fields moved.
}

impl Default for NadexApp {
    fn default() -> Self {
        let mut app = Self {
            app_state: AppState::new(),
            action_queue: Vec::new(),
        };
        // filter_images_for_current_map needs to be called after AppState is initialized
        // and it will now operate on app.app_state fields.
        app.filter_images_for_current_map(); 
        app
    }
}

impl NadexApp {
    fn handle_top_bar_action(
        &mut self,
        ctx: &egui::Context,
        action: ui::top_bar_view::TopBarAction,
    ) {
        match action {
            ui::top_bar_view::TopBarAction::QueueAppAction(app_action) => {
                self.action_queue.push(app_action);
                // Repaint might be requested after all actions are processed in NadexApp::update
            }
            ui::top_bar_view::TopBarAction::ImageSizeChanged(size) => {
                self.app_state.grid_image_size = size;
                // Consider if a repaint is needed or if it's handled by egui's automatic detection
            }
            ui::top_bar_view::TopBarAction::NadeTypeFilterChanged(nade_type) => {
                self.app_state.selected_nade_type = nade_type;
                self.filter_images_for_current_map(); // Filter immediately on nade type change
                self.app_state.selected_image_for_detail = None; // Clear detail view
                self.app_state.detail_view_texture_handle = None;
                ctx.request_repaint(); // Request repaint as content changes
            }
            ui::top_bar_view::TopBarAction::UploadButtonPushed => {
                self.app_state.show_upload_modal = true;
            }
        }
    }

    fn handle_image_grid_action(
        &mut self,
        ctx: &egui::Context,
        action: ui::image_grid_view::ImageGridAction,
    ) {
        match action {
            ui::image_grid_view::ImageGridAction::ImageClicked(meta) => {
                // Toggle selection or select new
                if self
                    .app_state
                    .selected_image_for_detail
                    .as_ref()
                    .map_or(false, |selected| selected.filename == meta.filename)
                {
                    self.app_state.selected_image_for_detail = None;
                    self.app_state.detail_view_texture_handle = None;
                } else {
                    self.app_state.selected_image_for_detail = Some(meta.clone());
                    self.app_state.detail_view_texture_handle = None;
                    self.load_detail_image(ctx, &meta);
                }
            }
        }
    }

    fn handle_upload_modal_action(
        &mut self,
        ctx: &egui::Context,
        action: ui::upload_modal_view::UploadModalAction,
    ) {
        match action {
            ui::upload_modal_view::UploadModalAction::UploadConfirmed {
                file_path,
                nade_type,
                position,
                notes,
            } => {
                self.copy_image_to_data_threaded(ctx, file_path, nade_type, position, notes, Arc::clone(&self.app_state.persistence_service), Arc::clone(&self.app_state.thumbnail_service));
                self.app_state.show_upload_modal = false;
                self.app_state.upload_modal_file = None;
                self.app_state.upload_modal_nade_type = NadeType::Smoke; // Reset to default
                self.app_state.upload_modal_position = String::new();
                self.app_state.upload_modal_notes = String::new();
            }
            ui::upload_modal_view::UploadModalAction::Cancel => {
                self.app_state.show_upload_modal = false;
                self.app_state.upload_modal_file = None;
                self.app_state.upload_modal_nade_type = NadeType::Smoke; // Reset to default
                self.app_state.upload_modal_position = String::new();
                self.app_state.upload_modal_notes = String::new();
            }
        }
    }

    fn handle_detail_modal_action(&mut self, action: ui::detail_view::DetailModalAction) {
        match action {
            ui::detail_view::DetailModalAction::Close => {
                self.app_state.selected_image_for_detail = None;
                self.app_state.detail_view_texture_handle = None;
                self.app_state.detail_view_error = None;
                self.app_state.editing_image_meta = None;
                self.app_state.edit_form_data = None;
            }
            ui::detail_view::DetailModalAction::RequestEdit(meta) => {
                // Set up for edit modal
                self.app_state.editing_image_meta = Some(meta.clone());
                self.app_state.edit_form_data = Some(ui::edit_view::EditFormData::from_meta(&meta));

                // Close detail view
                self.app_state.selected_image_for_detail = None;
                self.app_state.detail_view_texture_handle = None;
                self.app_state.detail_view_error = None;
            }
            ui::detail_view::DetailModalAction::RequestDelete(meta) => {
                // Set up for delete confirmation modal
                self.app_state.show_delete_confirmation = Some(meta);

                // Close detail view
                self.app_state.selected_image_for_detail = None;
                self.app_state.detail_view_texture_handle = None;
                self.app_state.detail_view_error = None;
            }
        }
    }

    fn handle_edit_modal_action(
        &mut self,
        ctx: &egui::Context,
        action: ui::edit_view::EditModalAction,
    ) {
        match action {
            ui::edit_view::EditModalAction::Save(updated_form_data) => {
                self.handle_save_image_edit(updated_form_data, ctx);
                // self.editing_image_meta and self.edit_form_data are reset within handle_save_image_edit
            }
            ui::edit_view::EditModalAction::Cancel => {
                self.app_state.editing_image_meta = None;
                self.app_state.edit_form_data = None;
                self.app_state.error_message = None; // Clear any potential error from a previous failed edit attempt
                ctx.request_repaint();
            }
        }
    }

    fn handle_delete_confirmation_action(
        &mut self,
        ctx: &egui::Context,
        action: ui::delete_confirmation_view::DeleteConfirmationAction,
        meta_to_delete: persistence::ImageMeta,
    ) {
        match action {
            ui::delete_confirmation_view::DeleteConfirmationAction::ConfirmDelete => {
                self.handle_confirm_image_delete(meta_to_delete, ctx);
                // State changes like show_delete_confirmation = None are handled within handle_confirm_image_delete
            }
            ui::delete_confirmation_view::DeleteConfirmationAction::Cancel => {
                self.app_state.show_delete_confirmation = None;
                ctx.request_repaint();
            }
        }
    }

    fn filter_images_for_current_map(&mut self) {
        self.app_state.filter_images_for_current_map();
    }

    fn load_detail_image(&mut self, ctx: &egui::Context, image_meta: &ImageMeta) {
        let full_image_path = self
            .app_state
            .data_dir
            .join(&self.app_state.current_map)
            .join(&image_meta.filename);
        match image::open(&full_image_path) {
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

    fn copy_image_to_data_threaded(
        &mut self,
        ctx: &egui::Context,
        path: PathBuf, // Original path of the image to upload
        nade_type: NadeType,
        position: String,
        notes: String,
        persistence_service: Arc<PersistenceService>,
        thumbnail_service: Arc<std::sync::Mutex<ThumbnailService>>,
    ) {
        let map_name_clone = self.app_state.current_map.clone();
        // data_dir_clone is no longer needed here as PersistenceService has its own data_dir.
        let ctx_clone = ctx.clone();
        let path_clone = path.clone(); // Clone path for the thread

        // Clone Arcs for the thread
        let ps_clone = Arc::clone(&persistence_service);
        let ts_clone = Arc::clone(&thumbnail_service);

        let (tx, rx) = std::sync::mpsc::channel::<Result<ImageMeta, String>>();

        std::thread::spawn(move || {
            let result: Result<ImageMeta, String> = (|| { // IIFE for ? operator usage
                // 1. Validate the image (open and check dimensions)
                let img = image::open(&path_clone).map_err(|e| {
                    format!(
                        "Failed to open image '{}': {}",
                        path_clone.display(),
                        e
                    )
                })?;
                let dims = img.dimensions();
                if dims != (1920, 1440) {
                    Err(format!(
                        "Invalid image dimensions for '{}': {:?}. Expected 1920x1440.",
                        path_clone.display(),
                        dims
                    ))?
                }

                // 2. Copy the image to the data directory (gets unique filename)
                //    Now calls the instance method on PersistenceService and passes ThumbnailService.
                let result = ps_clone.copy_image_to_data(
                    &path_clone,
                    &map_name_clone,
                    &ts_clone, // Pass the ThumbnailService Arc
                );

                let (_new_image_path_in_data, unique_filename_str) = match result {
                    Ok((path, filename)) => (path, filename),
                    Err(e) => return Err(format!("Failed to copy image '{}' to data directory: {}", path_clone.display(), e)),
                };

                // 3. Thumbnail generation is now handled by PersistenceService::copy_image_to_data via ThumbnailService

                // 4. Construct ImageMeta with the unique filename
                Ok(ImageMeta {
                    filename: unique_filename_str.to_string(), // This is the unique, timestamped filename
                    map: map_name_clone,          // The map it belongs to
                    nade_type,                   // NadeType (Smoke, Flash, etc.)
                    notes,                       // User-provided notes
                    position,                    // User-provided position identifier
                })
            })(); // End of IIFE

            if let Err(e) = tx.send(result) {
                log::error!(
                    "Failed to send upload result for '{}': {}",
                    path_clone.display(),
                    e
                );
            }
            ctx_clone.request_repaint(); // Request repaint from the worker thread
        });

        // Add the task to the uploads queue for main thread processing
        self.app_state.uploads.push(app_logic::upload_processor::UploadTask {
            map: self.app_state.current_map.clone(), // Map context for the upload
            rx, // Receiver for the result
            status: app_logic::upload_processor::UploadStatus::InProgress,
            finished_time: None,
            start_time: Instant::now(),
        });
    }

    fn handle_confirm_image_delete(&mut self, meta_to_delete: ImageMeta, ctx: &egui::Context) {
        let filename_to_delete = meta_to_delete.filename.clone();
        let map_name_of_deleted = meta_to_delete.map.clone();

        // Use PersistenceService to delete the image and its thumbnails
        if let Err(e) = self.app_state.persistence_service.delete_image_and_thumbnails(
            &map_name_of_deleted,
            &filename_to_delete,
            &mut *self.app_state.thumbnail_service.lock().unwrap(), // Pass mutable ref from locked Mutex
        ) {
            log::error!(
                "Error during deletion via PersistenceService for image '{}' in map '{}': {}",
                filename_to_delete,
                map_name_of_deleted,
                e
            );
            self.app_state.error_message = Some(format!("Failed to delete image files: {}", e));
        }

        log::debug!("[Delete Flow] Meta to delete: {:?}", meta_to_delete);
        if let Some(images_in_map_before_retain) =
            self.app_state.image_manifest.images.get(&map_name_of_deleted)
        {
            log::debug!(
                "[Delete Flow] Images in map '{}' before retain:",
                map_name_of_deleted
            );
            for (index, existing_meta) in images_in_map_before_retain.iter().enumerate() {
                let is_equal = existing_meta == &meta_to_delete;
                log::debug!(
                    "  [{}]: {:?} (Is equal to meta_to_delete: {})",
                    index,
                    existing_meta,
                    is_equal
                );
            }
        }

        if let Some(images_for_map) = self.app_state.image_manifest.images.get_mut(&map_name_of_deleted) {
            images_for_map.retain(|meta| meta != &meta_to_delete);
            log::debug!(
                "[Delete Flow] Images in map '{}' after retain: {:?}",
                map_name_of_deleted,
                images_for_map
            );
        } else {
            log::warn!(
                "[Delete Flow] No images found for map '{}' during retain operation.",
                map_name_of_deleted
            );
        }

        // Thumbnail cache and file clearing is now handled by persistence_service.delete_image_and_thumbnails
        // via the ThumbnailService.

        if let Err(e) = self.app_state.persistence_service.save_manifest(&self.app_state.image_manifest) {
            log::error!("Error saving manifest after delete: {}", e);
            self.app_state.error_message = Some(format!("Failed to save changes after delete: {}", e));
        } else {
            log::info!(
                "Manifest saved successfully after deleting '{}'.",
                filename_to_delete
            );
            self.app_state.error_message = None; // Clear previous error on successful save
        }
        self.app_state.selected_image_for_detail = None;
        self.app_state.detail_view_texture_handle = None;
        self.app_state.show_delete_confirmation = None;
        self.filter_images_for_current_map();
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
                // Manifest in memory is updated, now save it to disk
                if let Err(e) = self.app_state.persistence_service.save_manifest(&self.app_state.image_manifest) {
                    log::error!("Error saving manifest after edit: {}", e);
                    self.app_state.error_message = Some(format!("Failed to save changes: {}", e));
                } else {
                    log::info!(
                        "Manifest saved successfully after editing '{}'.",
                        form_data_to_save.filename
                    );
                    self.app_state.error_message = None; // Clear error on successful save
                }
            }
            Err(e) => {
                log::error!("Error updating image metadata: {}", e);
                self.app_state.error_message = Some(format!("Failed to update image: {}", e));
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
        self.app_state.error_message = Some(format!(
            "Internal error: No image was being edited. Please try again."
        ));
        // Also clear edit state here to prevent further issues
        self.app_state.editing_image_meta = None;
        self.app_state.edit_form_data = None;
    }
}
} // Added to close impl NadexApp


impl eframe::App for NadexApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // --- Process AppActions ---
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
                    // Add other AppAction variants here as they are defined
                }
            }
        }

        app_logic::upload_processor::process_upload_tasks(&mut self.app_state, ctx);

        // Top Bar
        egui::TopBottomPanel::top("top_panel").show(ctx, |top_ui| {
            if let Some(action) = ui::top_bar_view::show_top_bar(&mut self.app_state, top_ui) {
                self.handle_top_bar_action(ctx, action);
            }
        });

        // Main Central Panel
        egui::CentralPanel::default()
            .frame(egui::Frame::default().inner_margin(egui::Margin {
                left: 0.0,
                right: 0.0,
                top: 0.0,
                bottom: 0.0,
            }))
            .show(ctx, |panel_ui| {
                // Show upload overlay if any upload is in progress
                let num_uploads_in_progress = self
                    .app_state.uploads
                    .iter()
                    .filter(|u| u.status == app_logic::upload_processor::UploadStatus::InProgress)
                    .count();
                if num_uploads_in_progress > 0 {
                    egui::Window::new("Uploading...")
                        .anchor(egui::Align2::CENTER_CENTER, [0.0_f32, 0.0_f32])
                        .collapsible(false)
                        .resizable(false)
                        .title_bar(false)
                        .show(ctx, |ui| {
                            ui.label("Uploading and processing image...");
                            ui.add(egui::Spinner::default());
                        });
                }

                // Show error message if any
                if let Some(ref msg) = self.app_state.error_message {
                    panel_ui.colored_label(egui::Color32::RED, msg);
                }

                panel_ui.add_space(4.0);

                // Call the new image grid view function
                if let Some(grid_action) = ui::image_grid_view::show_image_grid(&mut self.app_state, panel_ui) {
                    self.handle_image_grid_action(ctx, grid_action);
                }
            });

        // --- Upload Modal (Refactored) ---
        if self.app_state.show_upload_modal {
            if let Some(action) = ui::upload_modal_view::show_upload_modal(&mut self.app_state, ctx) {
                self.handle_upload_modal_action(ctx, action);
            }
        }

        // --- Image Detail View Modal ---
        if let Some(selected_meta_clone) = self.app_state.selected_image_for_detail.clone() {
            // Construct the view state required by ui::detail_view::show_detail_modal
            let mut view_state = ui::detail_view::DetailModalViewState {
                ctx, // Pass the context
                screen_rect: ctx.screen_rect(),
                selected_image_meta: &selected_meta_clone, // Pass the cloned meta
                detail_view_texture_handle: &self.app_state.detail_view_texture_handle, // Pass ref to Option<TextureHandle>
                                                                              // error_message and is_editing are not part of the detail_view.rs's DetailModalViewState
                                                                              // Those will be handled by NadexApp based on the action or other state
            };

            // The show_detail_modal function in detail_view.rs now takes &mut DetailModalViewState
            // and NadexApp itself is no longer passed directly to it.
            // Instead, NadexApp fields are accessed via the DetailModalViewState or handled by NadexApp after an action.
            if let Some(action) = ui::detail_view::show_detail_modal(&mut view_state) {
                self.handle_detail_modal_action(action);
            }
        }

        // --- Edit Image Modal (Refactored) ---
        if let Some(current_editing_meta) = &self.app_state.editing_image_meta.clone() {
            if self.app_state.edit_form_data.is_none()
                || self.app_state.edit_form_data.as_ref().map_or(true, |data| {
                    &data.filename != &current_editing_meta.filename
                })
            {
                self.app_state.edit_form_data = Some(ui::edit_view::EditFormData {
                    filename: current_editing_meta.filename.clone(),
                    nade_type: current_editing_meta.nade_type,
                    position: current_editing_meta.position.clone(),
                    notes: current_editing_meta.notes.clone(),
                });
            }

            if let Some(action) = ui::edit_view::show_edit_modal(&mut self.app_state, ctx) {
                self.handle_edit_modal_action(ctx, action);
            }
        }

        // --- Delete Confirmation Modal (Refactored) ---
        if let Some(meta_to_delete) = self.app_state.show_delete_confirmation.clone() {
            if let Some(action) = ui::delete_confirmation_view::show_delete_confirmation_modal(
                &mut self.app_state,
                ctx,
                &meta_to_delete,
            ) {
                self.handle_delete_confirmation_action(ctx, action, meta_to_delete);
            }
        }
    }
}
