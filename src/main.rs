use crate::persistence::{ImageManifest, ImageMeta, NadeType, load_manifest, save_manifest};
use crate::thumbnail::generate_all_thumbnails;
use eframe::{NativeOptions, egui};
use image;
use image::GenericImageView;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use crate::persistence::copy_image_to_data;

mod persistence;
mod thumbnail;
mod ui;

fn main() -> eframe::Result<()> {
    let mut options = NativeOptions::default();
    options.viewport.maximized = Some(true);
    eframe::run_native(
        "nadex",
        options,
        Box::new(|_cc| Ok(Box::new(NadexApp::default()))),
    )
}

struct UploadTask {
    map: String,
    rx: Receiver<Result<(), String>>,
    status: UploadStatus,
    finished_time: Option<Instant>,
}

#[derive(PartialEq)]
enum UploadStatus {
    InProgress,
    Success,
    Error,
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
    uploads: Vec<UploadTask>,
    current_map: String,

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
    thumb_texture_cache: HashMap<(String, u32), egui::TextureHandle>,
    thumb_cache_order: VecDeque<(String, u32)>,
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
        Self {
            selected_nade_type: None,
            uploads: Vec::new(),
            current_map: "de_ancient".to_string(),
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

            thumb_texture_cache: HashMap::new(),
            thumb_cache_order: VecDeque::new(),
            selected_image_for_detail: None,
            detail_view_texture_handle: None,
            editing_image_meta: None,
            edit_form_data: None,
            show_delete_confirmation: None,
            detail_view_error: None,
        }
    }
}

impl NadexApp {
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
        path: PathBuf,
        nade_type: NadeType,
        position: String,
        notes: String,
    ) {
        let map_name_for_thread = self.current_map.clone();
        let map_name_for_task = self.current_map.clone(); // For UploadTask
        let data_dir_clone = self.data_dir.clone();
        let ctx_clone = ctx.clone(); // For repaint requests
        let (tx, rx) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let result = image::open(&path)
                .map_err(|e| format!("Failed to open image: {}", e))
                .and_then(|img| {
                    let dims = img.dimensions();
                    if dims == (1920, 1440) {
                        copy_image_to_data(&path, &data_dir_clone, &map_name_for_thread)
                            .map_err(|e| format!("Failed to copy image: {}", e))
                            .map(|dest_path| {
                                (
                                    dest_path,
                                    map_name_for_thread,
                                    data_dir_clone,
                                    nade_type,
                                    position,
                                    notes,
                                )
                            })
                    } else {
                        Err(format!(
                            "Invalid image dimensions: {:?}. Expected 1920x1440.",
                            dims
                        ))
                    }
                })
                .and_then(|(dest_path, map_name, data_dir, n_type, pos, nts)| {
                    generate_all_thumbnails(
                        &dest_path,
                        &data_dir.join(&map_name).join(".thumbnails"),
                    );

                    let mut manifest = persistence::load_manifest(&data_dir);

                    let new_image_meta = ImageMeta {
                        filename: dest_path.file_name().unwrap().to_str().unwrap().to_string(),
                        map: map_name.clone(), // <--- Add this line, using the map_name from the tuple
                        nade_type: n_type,
                        position: pos, // Ensure this is correctly assigned
                        notes: nts,    // Ensure this is correctly assigned
                    };

                    manifest
                        .images
                        .entry(map_name.clone())
                        .or_default()
                        .push(new_image_meta);
                    // Sort images by filename after adding, if desired (optional)
                    // manifest.images.get_mut(&map_name).unwrap().sort_by(|a, b| a.filename.cmp(&b.filename));

                    save_manifest(&manifest, &data_dir)
                        .map_err(|e| format!("Failed to save manifest: {}", e))
                });

            if let Err(e) = tx.send(result.map_err(|e_str| e_str)) {
                // Ensure error is String
                eprintln!("Failed to send upload result: {}", e);
            }
            ctx_clone.request_repaint();
        });

        self.uploads.push(UploadTask {
            map: map_name_for_task,
            rx,
            status: UploadStatus::InProgress,
            finished_time: None,
        });
    }
}

impl eframe::App for NadexApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        // Process finished uploads
        self.uploads.retain_mut(|upload| {
            match upload.rx.try_recv() {
                Ok(Ok(())) => {
                    upload.status = UploadStatus::Success;
                    upload.finished_time = Some(now);
                    false // Remove from active uploads, but keep for notification
                }
                Ok(Err(e)) => {
                    eprintln!("Upload error: {}", e);
                    self.error_message = Some(e);
                    // Keep for a bit to show error, or handle differently
                    upload.status = UploadStatus::Error;
                    upload.finished_time = Some(now); // Mark to remove after showing error
                    false
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => true, // Keep, still in progress
                Err(std::sync::mpsc::TryRecvError::Disconnected) => false, // Channel closed, remove
            }
        });

        // Remove old success/error notifications
        self.uploads.retain(|upload| {
            if let Some(finished) = upload.finished_time {
                now.duration_since(finished) < Duration::from_secs(5) // Keep for 5 seconds
            } else {
                true // Still in progress or not yet finished
            }
        });

        // Top Bar (already refactored)
        egui::TopBottomPanel::top("top_panel").show(ctx, |top_ui| {
            if let Some(action) = ui::top_bar_view::show_top_bar(self, top_ui) {
                match action {
                    ui::top_bar_view::TopBarAction::MapSelected(map_name) => {
                        self.current_map = map_name;
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
        });

        // Main Central Panel
        egui::CentralPanel::default()
            .frame(
                egui::Frame::central_panel(&*ctx.style()).inner_margin(egui::Margin {
                    left: 0_i8,
                    right: 0_i8,
                    top: 0_i8, // No top margin for central panel after top bar
                    bottom: 0_i8,
                }),
            )
            .show(ctx, |panel_ui| {
                // Show upload overlay if any upload is in progress
                let num_uploads_in_progress = self
                    .uploads
                    .iter()
                    .filter(|u| u.status == UploadStatus::InProgress)
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
                    match grid_action {
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
                                self.detail_view_texture_handle = None; // Reset texture handle
                                self.load_detail_image(ctx, &meta);
                            }
                        }
                    }
                }
            });

        // --- Upload Modal (Refactored) ---
        if self.show_upload_modal {
            if let Some(action) = ui::upload_modal_view::show_upload_modal(self, ctx) {
                match action {
                    ui::upload_modal_view::UploadModalAction::UploadConfirmed {
                        file_path,
                        nade_type,
                        position,
                        notes,
                    } => {
                        self.copy_image_to_data_threaded(
                            ctx, file_path, nade_type, position, notes,
                        );
                        self.show_upload_modal = false;
                        // Reset app.upload_modal_ fields, as their state is primarily managed by the modal view now
                        self.upload_modal_file = None;
                        self.upload_modal_nade_type = NadeType::Smoke; // Default
                        self.upload_modal_position = String::new();
                        self.upload_modal_notes = String::new();
                    }
                    ui::upload_modal_view::UploadModalAction::Cancel => {
                        self.show_upload_modal = false;
                        self.upload_modal_file = None;
                        self.upload_modal_nade_type = NadeType::Smoke; // Default
                        self.upload_modal_position = String::new();
                        self.upload_modal_notes = String::new();
                    }
                }
            }
        }

        // Show floating success/error notifications for uploads
        for upload_task in &self.uploads {
            // Iterate over potentially completed tasks
            if let Some(finished_time) = upload_task.finished_time {
                let elapsed = now.duration_since(finished_time);
                if elapsed < Duration::from_secs(5) {
                    // Show for 5 seconds
                    let alpha = 1.0_f32 - (elapsed.as_secs_f32() / 5.0_f32);
                    let (text_color, bg_color, message) = match upload_task.status {
                        UploadStatus::Success => (
                            egui::Color32::WHITE,
                            egui::Color32::from_rgba_unmultiplied(0, 200, 0, (alpha * 192.0) as u8),
                            format!("Upload to '{}' successful!", upload_task.map),
                        ),
                        UploadStatus::Error => (
                            egui::Color32::WHITE,
                            egui::Color32::from_rgba_unmultiplied(200, 0, 0, (alpha * 192.0) as u8),
                            format!("Upload to '{}' failed.", upload_task.map), // Generic message, specific error in self.error_message
                        ),
                        UploadStatus::InProgress => continue, // Should not happen if finished_time is Some
                    };

                    let notification_frame = egui::Frame::new()
                        .fill(bg_color)
                        .corner_radius(egui::CornerRadius::same(8_u8))
                        .inner_margin(egui::Margin::same(12_i8));

                    egui::Area::new(format!("upload_notification_{}", upload_task.map).into())
                        .anchor(egui::Align2::RIGHT_TOP, [-24.0_f32, 24.0_f32])
                        .show(ctx, |ui| {
                            notification_frame.show(ui, |ui| {
                                ui.label(egui::RichText::new(message).color(text_color));
                            });
                        });
                }
            }
        }

        // --- Image Detail View Modal (Refactored) ---
        if let Some(selected_meta_clone) = self.selected_image_for_detail.clone() {
            let mut view_state = ui::detail_view::DetailModalViewState {
                ctx,
                screen_rect: ctx.screen_rect(),
                selected_image_meta: &selected_meta_clone, // Use the clone
                detail_view_texture_handle: &self.detail_view_texture_handle,
            };

            if let Some(action) = ui::detail_view::show_detail_modal(&mut view_state) {
                match action {
                    ui::detail_view::DetailModalAction::Close => {
                        self.selected_image_for_detail = None;
                        self.detail_view_texture_handle = None;
                    }
                    ui::detail_view::DetailModalAction::RequestEdit(meta_to_edit) => {
                        self.editing_image_meta = Some(meta_to_edit);
                        self.selected_image_for_detail = None; // Close detail view
                        self.detail_view_texture_handle = None;
                    }
                    ui::detail_view::DetailModalAction::RequestDelete(filename_to_delete) => {
                        self.show_delete_confirmation = Some(filename_to_delete);
                    }
                }
            }
        } // End of refactored detail modal call

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
                match action {
                    ui::edit_view::EditModalAction::Save(form_data_to_save) => {
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
                                eprintln!("Error saving manifest: {}", e);
                                self.error_message = Some(format!("Failed to save changes: {}", e));
                            } else {
                                self.error_message = None;
                            }
                            // editing_image_meta and edit_form_data are cleared by show_edit_modal's internal logic
                        } else {
                            eprintln!(
                                "Error: Could not find image to update after edit: {}",
                                form_data_to_save.filename
                            );
                            self.error_message = Some(format!(
                                "Failed to find image {} to update.",
                                form_data_to_save.filename
                            ));
                        }
                    }
                    ui::edit_view::EditModalAction::Cancel => {
                        // editing_image_meta and edit_form_data are cleared by show_edit_modal's internal logic
                        self.error_message = None;
                    }
                }
            }
        }

        // --- Delete Confirmation Modal (Refactored) ---
        if let Some(meta_to_delete) = self.show_delete_confirmation.clone() {
            // Clone meta to avoid borrow issues
            // The call to the new modal function:
            if let Some(action) = ui::show_delete_confirmation_modal(self, ctx, &meta_to_delete) {
                match action {
                    ui::DeleteConfirmationAction::ConfirmDelete => {
                        let filename_to_delete = meta_to_delete.filename.clone();
                        // Assuming ImageMeta has a 'map' field that stores the map name.
                        // If not, you'll need to ensure you can get the map name for the image.
                        // For this example, let's assume meta_to_delete.map is available.
                        let map_name_of_deleted = meta_to_delete.map.clone();

                        // Construct the full path to the image in the data directory
                        let mut image_path_in_data_dir = self.data_dir.clone();
                        image_path_in_data_dir.push(&map_name_of_deleted);
                        image_path_in_data_dir.push(&filename_to_delete);

                        // Delete the main image file
                        if let Err(e) = std::fs::remove_file(&image_path_in_data_dir) {
                            eprintln!(
                                "Failed to delete image file {}: {}",
                                image_path_in_data_dir.display(),
                                e
                            );
                            self.error_message =
                                Some(format!("Failed to delete image file: {}", e));
                        }

                        // Delete associated thumbnails
                        // thumbnail::thumbnail_path expects the *original* image path, not the one in data_dir,
                        // if thumbnails are generated based on original paths or if the path structure differs.
                        // For now, assuming image_path_in_data_dir is what's needed for context,
                        // but this might need adjustment based on how thumbnail_path works.
                        // Let's assume thumbnail_path can correctly derive paths using image_path_in_data_dir
                        // and thumb_base_dir.
                        let thumb_base_dir = self.data_dir.join(".thumbnails");
                        for &size in thumbnail::ALLOWED_THUMB_SIZES.iter() {
                            // We need a consistent way to get the path that was used to *create* the thumbnail
                            // or a way for thumbnail_path to work with the data_dir path.
                            // For now, I'll use image_path_in_data_dir, but this is a potential point of failure
                            // if thumbnail_path expects something else (e.g. an absolute path from original import).
                            // This was likely correct in your original code if it used full_image_path which was derived similarly.
                            let thumb_path_to_delete = thumbnail::thumbnail_path(
                                &image_path_in_data_dir, // This was full_image_path in your original code
                                &thumb_base_dir,
                                size,
                            );
                            if let Err(e) = std::fs::remove_file(&thumb_path_to_delete) {
                                if e.kind() != std::io::ErrorKind::NotFound {
                                    eprintln!(
                                        "Failed to delete thumbnail file {}: {}",
                                        thumb_path_to_delete.display(),
                                        e
                                    );
                                }
                            }
                        }

                        // Remove from manifest
                        // Ensure you're removing from the correct map's list of images.
                        // self.current_map might not be the map of the image being deleted if the user changed maps
                        // after initiating the delete. It's safer to use map_name_of_deleted.
                        if let Some(images_for_map) =
                            self.image_manifest.images.get_mut(&map_name_of_deleted)
                        {
                            images_for_map.retain(|meta| meta.filename != filename_to_delete);
                        }

                        if let Err(e) = save_manifest(&self.image_manifest, &self.data_dir) {
                            eprintln!("Error saving manifest after delete: {}", e);
                            self.error_message =
                                Some(format!("Failed to save changes after delete: {}", e));
                        } else {
                            self.error_message = None;
                        }
                        self.selected_image_for_detail = None; // Close detail view if open
                        self.detail_view_texture_handle = None;
                        self.show_delete_confirmation = None; // Close confirmation modal
                    }
                    ui::DeleteConfirmationAction::Cancel => {
                        self.show_delete_confirmation = None;
                    }
                }
            }
        }
    }
}
