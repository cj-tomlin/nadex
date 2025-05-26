use eframe::{App, Frame, NativeOptions, egui};
use egui::TextureHandle;
use image::GenericImageView;
use rfd::FileDialog;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
mod persistence;
mod thumbnail;
use persistence::{
    ImageManifest, ImageMeta, NadeType, copy_image_to_data, load_manifest, save_manifest,
};
use thumbnail::{ALLOWED_THUMB_SIZES, generate_all_thumbnails, get_thumbnail};

fn main() -> eframe::Result<()> {
    let mut options = NativeOptions::default();
    options.viewport.maximized = Some(true);
    eframe::run_native(
        "nadex",
        options,
        Box::new(|_cc| Ok(Box::new(NadexApp::default()))),
    )
}

use std::sync::mpsc::Receiver;

use std::time::{Duration, Instant};

struct UploadTask {
    map: String,
    rx: Receiver<Result<(), String>>,
    status: UploadStatus,
    finished_time: Option<Instant>,
}

enum UploadStatus {
    InProgress,
    Success,
    Error,
}

struct NadexApp {
    // Upload modal state
    show_upload_modal: bool,
    upload_modal_file: Option<PathBuf>,
    upload_modal_nade_type: NadeType,
    upload_modal_notes: String,
    uploads: Vec<UploadTask>,
    current_map: String,

    // List of available maps
    maps: Vec<&'static str>,
    // Map of map name -> Vec of image file names (not full paths)
    manifest: ImageManifest,
    // For displaying error messages
    error_message: Option<String>,
    // App data dir
    data_dir: PathBuf,
    // User grid preferences
    grid_image_size: f32,
    // Window state (future: persist)
    thumb_texture_cache: HashMap<(String, u32), TextureHandle>,
    thumb_cache_order: VecDeque<(String, u32)>, // for LRU eviction
}

impl Default for NadexApp {
    fn default() -> Self {
        // Use C:/Users/<user>/AppData/Local/nadex
        let mut data_dir = dirs::data_local_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        data_dir.push("nadex");
        std::fs::create_dir_all(&data_dir).ok();
        let manifest = load_manifest(&data_dir);
        Self {
            uploads: Vec::new(),
            current_map: "de_ancient".to_string(),
            show_upload_modal: false,
            upload_modal_file: None,
            upload_modal_nade_type: NadeType::Smoke,
            upload_modal_notes: String::new(),
            maps: vec![
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
            manifest,
            error_message: None,
            data_dir,

            grid_image_size: 480.0, // Must be in ALLOWED_THUMB_SIZES

            thumb_texture_cache: HashMap::new(),
            thumb_cache_order: VecDeque::new(),
        }
    }
}

impl App for NadexApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        // Upload modal logic
        if self.show_upload_modal {
            egui::Window::new("Upload Image")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Select image file (1920x1440)");
                    if ui.button("Choose File").clicked() {
                        if let Some(path) = FileDialog::new()
                            .add_filter("Image", &["png", "jpg", "jpeg", "bmp"])
                            .pick_file()
                        {
                            self.upload_modal_file = Some(path);
                        }
                    }
                    if let Some(path) = &self.upload_modal_file {
                        ui.label(format!("Selected: {}", path.display()));
                    }
                    ui.separator();
                    ui.label("Nade Type:");
                    egui::ComboBox::from_label("")
                        .selected_text(format!("{:?}", self.upload_modal_nade_type))
                        .show_ui(ui, |ui| {
                            for variant in [
                                NadeType::Smoke,
                                NadeType::Flash,
                                NadeType::Molotov,
                                NadeType::Grenade,
                            ] {
                                ui.selectable_value(
                                    &mut self.upload_modal_nade_type,
                                    variant.clone(),
                                    format!("{:?}", &variant),
                                );
                            }
                        });
                    ui.separator();
                    ui.label("Notes:");
                    ui.text_edit_singleline(&mut self.upload_modal_notes);
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.show_upload_modal = false;
                            self.upload_modal_file = None;
                            self.upload_modal_notes.clear();
                        }
                    });
                });
        }

        // Controls panel
        egui::TopBottomPanel::top("controls_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Map selection icon
                ui.label("Map:");
                egui::ComboBox::new("map_selector", "")
                    .selected_text(&self.current_map)
                    .show_ui(ui, |ui| {
                        for map_name_str in &self.maps {
                            if ui
                                .selectable_value(
                                    &mut self.current_map,
                                    map_name_str.to_string(),
                                    *map_name_str,
                                )
                                .clicked()
                            {
                                // Map changed, could trigger reload or other actions if needed
                            }
                        }
                    });
                // Image size icon
                ui.label("Image Size:");
                // Find current index for grid_image_size in ALLOWED_THUMB_SIZES
                let mut current_thumb_idx = ALLOWED_THUMB_SIZES
                    .iter()
                    .position(|&s| s == self.grid_image_size as u32)
                    .unwrap_or(0); // Default to 0; ALLOWED_THUMB_SIZES should not be empty
                egui::ComboBox::new("thumb_size_select", "")
                    .selected_text(format!(
                        "{} px",
                        ALLOWED_THUMB_SIZES
                            .get(current_thumb_idx)
                            .cloned()
                            .unwrap_or(self.grid_image_size as u32)
                    ))
                    .show_ui(ui, |ui| {
                        for (i, &sz) in ALLOWED_THUMB_SIZES.iter().enumerate() {
                            if ui
                                .selectable_value(&mut current_thumb_idx, i, format!("{} px", sz))
                                .clicked()
                            {
                                self.grid_image_size = sz as f32;
                            }
                        }
                    });

                // Upload button logic
                if ui
                    .button("Upload")
                    .on_hover_text("Upload Screenshot")
                    .clicked()
                {
                    // Only upload if modal is open and required fields are filled
                    if self.show_upload_modal && self.upload_modal_file.is_some() {
                        let path = self.upload_modal_file.as_ref().unwrap().clone();
                        let map_name_for_thread = self.current_map.clone(); // Clone for the thread
                        let map_name_for_task = self.current_map.clone(); // Clone for the UploadTask
                        let data_dir_clone = self.data_dir.clone();
                        let nade_type_for_upload = self.upload_modal_nade_type.clone();
                        let notes_for_upload = self.upload_modal_notes.clone();
                        let ctx_clone = ctx.clone();
                        let (tx, rx) = std::sync::mpsc::channel();

                        std::thread::spawn(move || {
                            let result = image::open(&path).and_then(|img| {
                                let dims = img.dimensions();
                                if dims == (1920, 1440) {
                                    copy_image_to_data(&path, &data_dir_clone, &map_name_for_thread)
                                        .map_err(|io_err| {
                                            image::ImageError::IoError(std::io::Error::new(
                                                std::io::ErrorKind::Other,
                                                io_err.to_string(),
                                            ))
                                        })
                                        .map(|dest_path| {
                                            generate_all_thumbnails(
                                                &dest_path,
                                                &data_dir_clone
                                                    .join(&map_name_for_thread)
                                                    .join(".thumbnails"),
                                            );

                                            let mut manifest = load_manifest(&data_dir_clone);
                                            let entry = manifest
                                                .images
                                                .entry(map_name_for_thread.clone())
                                                .or_default();
                                            let filename = dest_path
                                                .file_name()
                                                .unwrap()
                                                .to_string_lossy()
                                                .to_string();

                                            if !entry.iter().any(|meta| meta.filename == filename) {
                                                entry.push(ImageMeta {
                                                    filename: filename.clone(),
                                                    nade_type: nade_type_for_upload,
                                                    notes: notes_for_upload,
                                                });
                                                let _ = save_manifest(&manifest, &data_dir_clone);
                                            }
                                        })
                                } else {
                                    Err(image::ImageError::IoError(std::io::Error::new(
                                        std::io::ErrorKind::Other,
                                        "Image must be 1920x1440",
                                    )))
                                }
                            });

                            let send_result = match result {
                                Ok(_) => tx.send(Ok(())),
                                Err(e) => tx.send(Err(e.to_string())),
                            };
                            let _ = send_result;
                            ctx_clone.request_repaint();
                        });

                        self.uploads.push(UploadTask {
                            map: map_name_for_task, // Use map_name_for_task
                            rx,
                            status: UploadStatus::InProgress,
                            finished_time: None,
                        });

                        // Close modal and clear fields after initiating upload
                        self.show_upload_modal = false;
                        self.upload_modal_file = None;
                        self.upload_modal_notes.clear();
                    } else {
                        // Open the modal for user to fill fields if not already properly set up
                        self.show_upload_modal = true;
                        self.upload_modal_file = None;
                        self.upload_modal_notes.clear();
                        self.upload_modal_nade_type = NadeType::Smoke;
                    }
                }
            });
        });
        // Poll all uploads and update their status
        for upload in &mut self.uploads {
            if let UploadStatus::InProgress = upload.status {
                match upload.rx.try_recv() {
                    Ok(Ok(())) => {
                        upload.status = UploadStatus::Success;
                        upload.finished_time = Some(Instant::now());
                        self.manifest = load_manifest(&self.data_dir);
                    }
                    Ok(Err(_e)) => {
                        upload.status = UploadStatus::Error;
                        upload.finished_time = Some(Instant::now());
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        upload.status = UploadStatus::Error;
                        upload.finished_time = Some(Instant::now());
                    }
                }
            }
        }
        // Remove finished uploads after 3 seconds
        let now = Instant::now();
        self.uploads.retain(|u| match u.status {
            UploadStatus::InProgress => true,
            _ => u
                .finished_time
                .is_none_or(|t| now.duration_since(t) < Duration::from_secs(3)), // Clippy: use is_none_or
        });
        // Show upload progress indicator only in CentralPanel to avoid duplicate widget IDs
        let num_uploads = self
            .uploads
            .iter()
            .filter(|u| matches!(u.status, UploadStatus::InProgress))
            .count();

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |_ui| {
                if num_uploads > 0 {
                    egui::Window::new("UploadingOverlay")
                        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                        .collapsible(false)
                        .resizable(false)
                        .title_bar(false)
                        .show(ctx, |ui| {
                            ui.horizontal(|ui| {
                                ui.add(egui::Spinner::default());
                                ui.label(format!(
                                    "{} upload{} in progress",
                                    num_uploads,
                                    if num_uploads == 1 { "" } else { "s" }
                                ));
                            });
                        });
                }
            });
        // Show floating success notification if any
        for upload in &self.uploads {
            if let UploadStatus::Success = upload.status {
                if let Some(finished) = upload.finished_time {
                    let elapsed = now.duration_since(finished);
                    if elapsed < Duration::from_secs(3) {
                        let alpha = 1.0 - (elapsed.as_secs_f32() / 3.0);
                        let color =
                            egui::Color32::from_rgba_unmultiplied(0, 200, 0, (alpha * 192.0) as u8);
                        let notification = egui::Frame::new()
                            .fill(color)
                            .corner_radius(egui::CornerRadius::same(8))
                            .inner_margin(egui::Margin::same(12));
                        egui::Area::new(format!("upload_success_{}", upload.map).into())
                            .anchor(egui::Align2::RIGHT_TOP, [-24.0, 24.0])
                            .show(ctx, |ui| {
                                notification.show(ui, |ui| {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "Upload to '{}' successful!",
                                            upload.map
                                        ))
                                        .color(egui::Color32::WHITE),
                                    );
                                });
                            });
                    }
                }
            }
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                // Show upload overlay if any upload is in progress
                if num_uploads > 0 {
                    egui::Window::new("Uploading...")
                        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
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
                    ui.colored_label(egui::Color32::RED, msg);
                }

                // Display image grid for self.current_map
                let filenames = self
                    .manifest
                    .images
                    .get(&self.current_map)
                    .cloned()
                    .unwrap_or_default();
                if !filenames.is_empty() {
                    // Filter out missing images and clean manifest
                    let map = &self.current_map;
                    let mut removed = false;
                    let mut to_remove = vec![];
                    for meta in &filenames {
                        let img_path = self.data_dir.join(map).join(&meta.filename);
                        if !img_path.exists() {
                            to_remove.push(meta.filename.clone());
                            removed = true;
                        }
                    }
                    if removed {
                        if let Some(entry) = self.manifest.images.get_mut(map) {
                            entry.retain(|meta| {
                                !to_remove.iter().any(|filename| &meta.filename == filename)
                            });
                            let _ = save_manifest(&self.manifest, &self.data_dir);
                        }
                    }
                    // Determine number of columns to fit the window
                    let grid_rect = ui.max_rect();
                    let spacing = 12.0;
                    let display_width = self.grid_image_size;
                    let num_columns = ((grid_rect.width() + spacing) / (display_width + spacing))
                        .floor()
                        .max(1.0) as usize;
                    egui::ScrollArea::vertical().show_viewport(ui, |ui, viewport| {
                        let grid = egui::Grid::new("image_grid");
                        // Estimate visible rows based on scroll offset and viewport height
                        let display_width = self.grid_image_size;
                        let display_height = display_width * 3.0 / 4.0;
                        let spacing = 12.0;
                        let row_height = display_height + spacing;
                        // Use the same num_columns for visible row math

                        let offset_y = viewport.min.y;
                        let viewport_height = viewport.height();
                        let first_visible_row = (offset_y / row_height).floor() as usize;
                        let last_visible_row =
                            ((offset_y + viewport_height) / row_height).ceil() as usize;
                        let mut row = 0;
                        grid.show(ui, |ui| {
                            for (i, meta) in filenames
                                .iter()
                                .filter(|meta| {
                                    let img_path = self.data_dir.join(map).join(&meta.filename);
                                    img_path.exists()
                                })
                                .enumerate()
                            {
                                let this_row = i / num_columns;
                                if this_row < first_visible_row || this_row > last_visible_row {
                                    // Not visible, show placeholder
                                    let display_width = self.grid_image_size;
                                    let display_height = self.grid_image_size * 3.0 / 4.0;
                                    let rect = ui.allocate_space(egui::Vec2::new(
                                        display_width,
                                        display_height,
                                    ));
                                    ui.painter().rect_filled(
                                        rect.1,
                                        4.0,
                                        egui::Color32::from_gray(80),
                                    );
                                } else {
                                    let img_path =
                                        self.data_dir.join(&self.current_map).join(&meta.filename);
                                    let thumb_dir =
                                        self.data_dir.join(&self.current_map).join(".thumbnails");
                                    // Find the closest allowed thumbnail size
                                    let requested_size = self.grid_image_size as u32;
                                    let &closest_size = ALLOWED_THUMB_SIZES
                                        .iter()
                                        .min_by_key(|&&s| (s as i32 - requested_size as i32).abs())
                                        .unwrap_or(&480);
                                    let cache_key = (meta.filename.clone(), closest_size);
                                    let mut loaded = false;
                                    if let Some(thumb_path) =
                                        get_thumbnail(&img_path, &thumb_dir, closest_size)
                                    {
                                        if let Ok(img) = image::open(&thumb_path) {
                                            let color_image =
                                                egui::ColorImage::from_rgba_unmultiplied(
                                                    [img.width() as usize, img.height() as usize],
                                                    img.to_rgba8().as_flat_samples().as_slice(),
                                                );
                                            // LRU cache eviction: remove oldest if over 256
                                            if !self.thumb_texture_cache.contains_key(&cache_key) {
                                                if self.thumb_texture_cache.len() >= 256 {
                                                    if let Some(oldest) =
                                                        self.thumb_cache_order.pop_front()
                                                    {
                                                        self.thumb_texture_cache.remove(&oldest);
                                                    }
                                                }
                                                let texture = ui.ctx().load_texture(
                                                    format!(
                                                        "thumb_{}_{}",
                                                        meta.filename, closest_size
                                                    ),
                                                    color_image,
                                                    egui::TextureOptions::default(),
                                                );
                                                self.thumb_texture_cache
                                                    .insert(cache_key.clone(), texture);
                                                self.thumb_cache_order.push_back(cache_key.clone());
                                            }
                                            if let Some(texture) =
                                                self.thumb_texture_cache.get(&cache_key)
                                            {
                                                let display_width = self.grid_image_size;
                                                let display_height =
                                                    self.grid_image_size * 3.0 / 4.0;
                                                ui.add(
                                                    egui::Image::new(texture).fit_to_exact_size(
                                                        egui::Vec2::new(
                                                            display_width,
                                                            display_height,
                                                        ),
                                                    ),
                                                );
                                                loaded = true;
                                            }
                                        }
                                    }
                                    if !loaded {
                                        let display_width = self.grid_image_size;
                                        let display_height = self.grid_image_size * 3.0 / 4.0;
                                        let rect = ui.allocate_space(egui::Vec2::new(
                                            display_width,
                                            display_height,
                                        ));
                                        ui.painter().rect_filled(
                                            rect.1,
                                            4.0,
                                            egui::Color32::from_gray(80),
                                        );
                                    }
                                }
                                if (i + 1) % num_columns == 0 {
                                    ui.end_row();
                                    row += 1;
                                }
                            }
                        });
                    });
                }
                if filenames.is_empty() {
                    ui.label("[No images uploaded for this map]");
                }
            });
    }
}
