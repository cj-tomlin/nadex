use crate::persistence::{ImageManifest, ImageMeta, NadeType, load_manifest, save_manifest};
use crate::thumbnail::generate_all_thumbnails;
use eframe::{NativeOptions, egui};
use env_logger; // Import env_logger
use image;
use image::GenericImageView;
use log::{self, LevelFilter};

use std::path::PathBuf;

use std::time::Instant;

// persistence::copy_image_to_data is called via persistence::copy_image_to_data_threaded or directly in persistence module
use crate::ui::image_grid_view::ThumbnailCache;

mod app_logic;
mod persistence;
mod thumbnail;
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
    // Filtering UI state
    selected_nade_type: Option<NadeType>,
    // Upload modal state
    show_upload_modal: bool,
    upload_modal_file: Option<PathBuf>,
    upload_modal_nade_type: NadeType,
    upload_modal_notes: String,
    upload_modal_position: String,
    uploads: Vec<app_logic::upload_processor::UploadTask>,
    current_map: String,
    current_map_images: Vec<ImageMeta>, // Added field

    // List of available maps
    maps: Vec<&'static str>,
    // Map of map name -> Vec of image file names (not full paths)
    image_manifest: ImageManifest,
    // For displaying error messages
    error_message: Option<String>,
    // App data dir
    data_dir: PathBuf,
    // User grid preferences
    grid_image_size: f32,
    // Window state (future: persist)
    thumbnail_cache: ThumbnailCache,
    selected_image_for_detail: Option<ImageMeta>,
    detail_view_texture_handle: Option<egui::TextureHandle>,
    editing_image_meta: Option<ImageMeta>,
    edit_form_data: Option<ui::edit_view::EditFormData>,
    show_delete_confirmation: Option<ImageMeta>,
    detail_view_error: Option<String>,
}

impl Default for NadexApp {
    fn default() -> Self {
        // Use C:/Users/<user>/AppData/Local/nadex
        let mut data_dir = dirs::data_local_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        data_dir.push("nadex");
        std::fs::create_dir_all(&data_dir).ok();
        let manifest = load_manifest(&data_dir);
        let mut app = Self {
            selected_nade_type: None,
            uploads: Vec::new(),
            current_map: "de_ancient".to_string(),
            current_map_images: Vec::new(), // Initialize new field
            show_upload_modal: false,
            upload_modal_file: None,
            upload_modal_nade_type: NadeType::Smoke,
            upload_modal_notes: String::new(),
            upload_modal_position: String::new(),
            maps: vec![
                "de_ancient",
                "de_anubis",
                "de_cache",
                "de_dust2",
                "de_inferno",
                "de_mirage",
                "de_nuke",
                "de_overpass",
                "de_train",
                "de_vertigo",
            ],
            image_manifest: manifest,
            error_message: None,
            data_dir,

            grid_image_size: 480.0,

            thumbnail_cache: ThumbnailCache::new(),
            selected_image_for_detail: None,
            detail_view_texture_handle: None,
            editing_image_meta: None,
            edit_form_data: None,
            show_delete_confirmation: None,
            detail_view_error: None,
        };
        app.filter_images_for_current_map(); // Call the new method
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
            ui::top_bar_view::TopBarAction::MapSelected(map_name) => {
                self.current_map = map_name;
                self.filter_images_for_current_map();
                self.selected_image_for_detail = None;
                self.detail_view_texture_handle = None;
                ctx.request_repaint();
            }
            ui::top_bar_view::TopBarAction::ImageSizeChanged(size) => {
                self.grid_image_size = size;
            }
            ui::top_bar_view::TopBarAction::NadeTypeFilterChanged(nade_type) => {
                self.selected_nade_type = nade_type;
            }
            ui::top_bar_view::TopBarAction::UploadButtonPushed => {
                self.show_upload_modal = true;
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
                    .selected_image_for_detail
                    .as_ref()
                    .map_or(false, |selected| selected.filename == meta.filename)
                {
                    self.selected_image_for_detail = None;
                    self.detail_view_texture_handle = None;
                } else {
                    self.selected_image_for_detail = Some(meta.clone());
                    self.detail_view_texture_handle = None;
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
                self.copy_image_to_data_threaded(ctx, file_path, nade_type, position, notes);
                self.show_upload_modal = false;
                self.upload_modal_file = None;
                self.upload_modal_nade_type = NadeType::Smoke; // Reset to default
                self.upload_modal_position = String::new();
                self.upload_modal_notes = String::new();
            }
            ui::upload_modal_view::UploadModalAction::Cancel => {
                self.show_upload_modal = false;
                self.upload_modal_file = None;
                self.upload_modal_nade_type = NadeType::Smoke; // Reset to default
                self.upload_modal_position = String::new();
                self.upload_modal_notes = String::new();
            }
        }
    }

    fn handle_detail_modal_action(&mut self, action: ui::detail_view::DetailModalAction) {
        match action {
            ui::detail_view::DetailModalAction::Close => {
                self.selected_image_for_detail = None;
                self.detail_view_texture_handle = None;
                self.detail_view_error = None;
                self.editing_image_meta = None;
                self.edit_form_data = None;
            }
            ui::detail_view::DetailModalAction::RequestEdit(meta) => {
                // Set up for edit modal
                self.editing_image_meta = Some(meta.clone());
                self.edit_form_data = Some(ui::edit_view::EditFormData::from_meta(&meta));

                // Close detail view
                self.selected_image_for_detail = None;
                self.detail_view_texture_handle = None;
                self.detail_view_error = None;
            }
            ui::detail_view::DetailModalAction::RequestDelete(meta) => {
                // Set up for delete confirmation modal
                self.show_delete_confirmation = Some(meta);

                // Close detail view
                self.selected_image_for_detail = None;
                self.detail_view_texture_handle = None;
                self.detail_view_error = None;
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
                self.editing_image_meta = None;
                self.edit_form_data = None;
                self.error_message = None; // Clear any potential error from a previous failed edit attempt
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
                self.show_delete_confirmation = None;
                ctx.request_repaint();
            }
        }
    }

    fn filter_images_for_current_map(&mut self) {
        self.current_map_images = self
            .image_manifest
            .images
            .get(&self.current_map)
            .map_or_else(Vec::new, |images_for_map| {
                let mut sorted_images = images_for_map.clone();
                sorted_images.sort_by(|a, b| a.filename.cmp(&b.filename));
                sorted_images
            });
    }

    fn load_detail_image(&mut self, ctx: &egui::Context, image_meta: &ImageMeta) {
        let full_image_path = self
            .data_dir
            .join(&self.current_map)
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
                self.detail_view_texture_handle = Some(handle);
            }
            Err(e) => {
                eprintln!(
                    "Failed to load detail image '{}': {}",
                    image_meta.filename, e
                );
                self.detail_view_error = Some(format!("Error loading image: {}", e));
                self.selected_image_for_detail = None;
                self.detail_view_texture_handle = None;
            }
        }
    }

    fn copy_image_to_data_threaded(
        &mut self,
        ctx: &egui::Context,
        path: PathBuf, // Original path of the image to upload
        nade_type: NadeType,
        position: String,
        notes: String,
    ) {
        let map_name_clone = self.current_map.clone();
        let data_dir_clone = self.data_dir.clone();
        let ctx_clone = ctx.clone();
        let path_clone = path.clone(); // Clone path for the thread

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
                let (new_image_path_in_data, unique_filename_str) = 
                    persistence::copy_image_to_data(&path_clone, &data_dir_clone, &map_name_clone)
                        .map_err(|e| {
                            format!(
                                "Failed to copy image '{}' to data directory: {}",
                                path_clone.display(),
                                e
                            )
                        })?;

                // 3. Generate thumbnails for this newly copied unique file
                let thumb_dir = data_dir_clone.join(&map_name_clone).join(".thumbnails");
                generate_all_thumbnails(&new_image_path_in_data, &thumb_dir);

                // 4. Construct ImageMeta with the unique filename
                Ok(ImageMeta {
                    filename: unique_filename_str, // This is the unique, timestamped filename
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
        self.uploads.push(app_logic::upload_processor::UploadTask {
            map: self.current_map.clone(), // Map context for the upload
            rx, // Receiver for the result
            status: app_logic::upload_processor::UploadStatus::InProgress,
            finished_time: None,
            start_time: Instant::now(),
        });
    }

    fn handle_confirm_image_delete(&mut self, meta_to_delete: ImageMeta, ctx: &egui::Context) {
        let filename_to_delete = meta_to_delete.filename.clone();
        let map_name_of_deleted = meta_to_delete.map.clone();

        let mut image_path_in_data_dir = self.data_dir.clone();
        image_path_in_data_dir.push(&map_name_of_deleted);
        image_path_in_data_dir.push(&filename_to_delete);

        if let Err(e) = std::fs::remove_file(&image_path_in_data_dir) {
            log::error!(
                "Failed to delete image file {}: {}",
                image_path_in_data_dir.display(),
                e
            );
            self.error_message = Some(format!("Failed to delete image file: {}", e));
        }

        let thumb_base_dir = self.data_dir.join(&meta_to_delete.map).join(".thumbnails");
        for &size in thumbnail::ALLOWED_THUMB_SIZES.iter() {
            let thumb_path_to_delete = thumbnail::thumbnail_path(
                &image_path_in_data_dir, // This should be the original image path in data_dir
                &thumb_base_dir,
                size,
            );
            if let Err(e) = std::fs::remove_file(&thumb_path_to_delete) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    log::error!(
                        "Failed to delete thumbnail file {}: {}",
                        thumb_path_to_delete.display(),
                        e
                    );
                }
            }
        }

        log::debug!("[Delete Flow] Meta to delete: {:?}", meta_to_delete);
        if let Some(images_in_map_before_retain) =
            self.image_manifest.images.get(&map_name_of_deleted)
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

        if let Some(images_for_map) = self.image_manifest.images.get_mut(&map_name_of_deleted) {
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

        // Clear thumbnail cache for the deleted image
        self.thumbnail_cache.remove_image_thumbnails(
            &filename_to_delete,
            &map_name_of_deleted,
            &self.data_dir,
        );
        log::debug!(
            "Attempted to remove thumbnails for '{}' from map '{}' from the new cache.",
            filename_to_delete,
            map_name_of_deleted
        );

        if let Err(e) = save_manifest(&self.image_manifest, &self.data_dir) {
            log::error!("Error saving manifest after delete: {}", e);
            self.error_message = Some(format!("Failed to save changes after delete: {}", e));
        } else {
            log::info!(
                "Manifest saved successfully after deleting '{}'.",
                filename_to_delete
            );
            self.error_message = None; // Clear previous error on successful save
        }
        self.selected_image_for_detail = None;
        self.detail_view_texture_handle = None;
        self.show_delete_confirmation = None;
        self.filter_images_for_current_map();
        ctx.request_repaint();
    }

    fn handle_save_image_edit(
        &mut self,
        form_data_to_save: ui::edit_view::EditFormData,
        ctx: &egui::Context,
    ) {
        if let Some(image_to_update) = self
            .image_manifest
            .images
            .values_mut()
            .flatten()
            .find(|img| img.filename == form_data_to_save.filename)
        {
            image_to_update.nade_type = form_data_to_save.nade_type;
            image_to_update.position = form_data_to_save.position.clone();
            image_to_update.notes = form_data_to_save.notes.clone();

            if let Err(e) = save_manifest(&self.image_manifest, &self.data_dir) {
                log::error!("Error saving manifest after edit: {}", e);
                self.error_message = Some(format!("Failed to save changes: {}", e));
            } else {
                log::info!(
                    "Manifest saved successfully after editing '{}'.",
                    form_data_to_save.filename
                );
                self.error_message = None; // Clear error on successful save
            }
            self.editing_image_meta = None;
            self.edit_form_data = None;
            self.filter_images_for_current_map(); // Refresh the view
            ctx.request_repaint();
        } else {
            log::error!(
                "Error: Could not find image to update after edit: {}",
                form_data_to_save.filename
            );
            self.error_message = Some(format!(
                "Failed to find image {} to update.",
                form_data_to_save.filename
            ));
        }
    }
}

impl eframe::App for NadexApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        app_logic::upload_processor::process_upload_tasks(self, ctx);

        // Top Bar (already refactored)
        egui::TopBottomPanel::top("top_panel").show(ctx, |top_ui| {
            if let Some(action) = ui::top_bar_view::show_top_bar(self, top_ui) {
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
                    .uploads
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
                if let Some(ref msg) = self.error_message {
                    panel_ui.colored_label(egui::Color32::RED, msg);
                }

                panel_ui.add_space(4.0);

                // Call the new image grid view function
                if let Some(grid_action) = ui::image_grid_view::show_image_grid(self, panel_ui) {
                    self.handle_image_grid_action(ctx, grid_action);
                }
            });

        // --- Upload Modal (Refactored) ---
        if self.show_upload_modal {
            if let Some(action) = ui::upload_modal_view::show_upload_modal(self, ctx) {
                self.handle_upload_modal_action(ctx, action);
            }
        }

        // --- Image Detail View Modal ---
        if let Some(selected_meta_clone) = self.selected_image_for_detail.clone() {
            // Construct the view state required by ui::detail_view::show_detail_modal
            let mut view_state = ui::detail_view::DetailModalViewState {
                ctx, // Pass the context
                screen_rect: ctx.screen_rect(),
                selected_image_meta: &selected_meta_clone, // Pass the cloned meta
                detail_view_texture_handle: &self.detail_view_texture_handle, // Pass ref to Option<TextureHandle>
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
        if let Some(current_editing_meta) = &self.editing_image_meta.clone() {
            if self.edit_form_data.is_none()
                || self.edit_form_data.as_ref().map_or(true, |data| {
                    &data.filename != &current_editing_meta.filename
                })
            {
                self.edit_form_data = Some(ui::edit_view::EditFormData {
                    filename: current_editing_meta.filename.clone(),
                    nade_type: current_editing_meta.nade_type,
                    position: current_editing_meta.position.clone(),
                    notes: current_editing_meta.notes.clone(),
                });
            }

            if let Some(action) = ui::edit_view::show_edit_modal(self, ctx) {
                self.handle_edit_modal_action(ctx, action);
            }
        }

        // --- Delete Confirmation Modal (Refactored) ---
        if let Some(meta_to_delete) = self.show_delete_confirmation.clone() {
            if let Some(action) = ui::delete_confirmation_view::show_delete_confirmation_modal(
                self,
                ctx,
                &meta_to_delete,
            ) {
                self.handle_delete_confirmation_action(ctx, action, meta_to_delete);
            }
        }
    }
}
