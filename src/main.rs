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

#[derive(PartialEq)] // Add this derive
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
    upload_modal_notes: String,    // How to throw
    upload_modal_position: String, // Position label (e.g., "A Main Smoke")
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
                    ui.label("Position (e.g. A Main Smoke):");
                    ui.text_edit_singleline(&mut self.upload_modal_position);
                    ui.separator();
                    ui.label("Notes:");
                    ui.text_edit_singleline(&mut self.upload_modal_notes);
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.show_upload_modal = false;
                            self.upload_modal_file = None;
                            self.upload_modal_notes.clear();
                            self.upload_modal_position.clear();
                        }
                        let can_upload = self.upload_modal_file.is_some();
                        if ui
                            .add_enabled(can_upload, egui::Button::new("Upload"))
                            .clicked()
                        {
                            // Only upload if required fields are filled
                            if let Some(path) = self.upload_modal_file.as_ref() {
                                let path = path.clone();
                                let map_name_for_thread = self.current_map.clone(); // Clone for the thread
                                let map_name_for_task = self.current_map.clone(); // Clone for the UploadTask
                                let data_dir_clone = self.data_dir.clone();
                                let nade_type_for_upload = self.upload_modal_nade_type.clone();
                                let notes_for_upload = self.upload_modal_notes.clone();
                                let position_for_upload = self.upload_modal_position.clone();
                                let ctx_clone = ctx.clone();
                                let (tx, rx) = std::sync::mpsc::channel();
                                std::thread::spawn(move || {
                                    let result = image::open(&path).and_then(|img| {
                                        let dims = img.dimensions();
                                        if dims == (1920, 1440) {
                                            copy_image_to_data(
                                                &path,
                                                &data_dir_clone,
                                                &map_name_for_thread,
                                            )
                                            .map_err(|io_err| {
                                                image::ImageError::IoError(std::io::Error::new(
                                                    std::io::ErrorKind::Other,
                                                    io_err.to_string(),
                                                ))
                                            })
                                            .map(
                                                |dest_path| {
                                                    generate_all_thumbnails(
                                                        &dest_path,
                                                        &data_dir_clone
                                                            .join(&map_name_for_thread)
                                                            .join(".thumbnails"),
                                                    );

                                                    let mut manifest =
                                                        load_manifest(&data_dir_clone);
                                                    let entry = manifest
                                                        .images
                                                        .entry(map_name_for_thread.clone())
                                                        .or_default();
                                                    let filename = dest_path
                                                        .file_name()
                                                        .unwrap()
                                                        .to_string_lossy()
                                                        .to_string();

                                                    if !entry
                                                        .iter()
                                                        .any(|meta| meta.filename == filename)
                                                    {
                                                        entry.push(ImageMeta {
                                                            filename: filename.clone(),
                                                            nade_type: nade_type_for_upload,
                                                            notes: notes_for_upload,
                                                            position: position_for_upload,
                                                        });
                                                        let _ = save_manifest(
                                                            &manifest,
                                                            &data_dir_clone,
                                                        );
                                                    }
                                                },
                                            )
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
                                self.upload_modal_position.clear();
                                self.upload_modal_nade_type = NadeType::Smoke;
                            }
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
                    self.show_upload_modal = true;
                    self.upload_modal_file = None;
                    self.upload_modal_notes.clear();
                    self.upload_modal_position.clear();
                    self.upload_modal_nade_type = NadeType::Smoke;
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
            .filter(|t| t.status == UploadStatus::InProgress)
            .count();

        // This CentralPanel seems to be part of view_menu_bar or a similar top-level structure.
        // Reverting its frame to NONE as the padding is for the main content grid/tabs.
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE) // Reverted to original Frame::NONE
            .show(ctx, |_ui| {
                // Assuming _ui was the original variable name here
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
        egui::CentralPanel::default()
            // Apply padding to the main content panel's frame
            .frame(egui::Frame::default().inner_margin(egui::Margin {
                left: 8i8,
                right: 0i8,
                top: 0i8,
                bottom: 0i8,
            }))
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

                ui.add_space(4.0); // Add some space above the nade type selectors

                // Filtering UI - Adjusted to blend with menu UI
                ui.horizontal(|ui_buttons| {
                    // The CentralPanel already provides left padding of 8. No need for extra ui.add_space here.
                    let original_item_spacing = ui_buttons.style_mut().spacing.item_spacing.x;
                    ui_buttons.style_mut().spacing.item_spacing.x = 8.0; // More standard spacing for buttons

                    let nade_types = [
                        (None, "All"),
                        (Some(NadeType::Smoke), "Smoke"),
                        (Some(NadeType::Flash), "Flash"),
                        (Some(NadeType::Molotov), "Molotov"),
                        (Some(NadeType::Grenade), "Grenade"),
                    ];

                    let text_color_selected = ui_buttons.style().visuals.selection.stroke.color;
                    let text_color_unselected =
                        ui_buttons.style().visuals.widgets.inactive.text_color();

                    for (filter_option, label_str) in nade_types {
                        let is_selected = self.selected_nade_type == filter_option;

                        let button_text = egui::RichText::new(label_str).color(if is_selected {
                            text_color_selected
                        } else {
                            text_color_unselected
                        });

                        let mut button = egui::Button::new(button_text);

                        if is_selected {
                            button = button.fill(ui_buttons.style().visuals.selection.bg_fill);
                        } else {
                            // For unselected buttons to blend, make them transparent or use a very subtle fill
                            button = button.fill(egui::Color32::TRANSPARENT);
                        }

                        if ui_buttons.add(button).clicked() {
                            self.selected_nade_type = filter_option;
                        }
                    }
                    ui_buttons.style_mut().spacing.item_spacing.x = original_item_spacing; // Restore original spacing
                });

                ui.add_space(8.0); // Add some space below the nade type selectors, before the grid

                // Display image grid for self.current_map
                let filenames = self
                    .manifest
                    .images
                    .get(&self.current_map)
                    .cloned()
                    .unwrap_or_default();
                let filtered_filenames: Vec<_> = match self.selected_nade_type {
                    None => filenames.clone(),
                    Some(ref filter_type) => filenames
                        .iter()
                        .filter(|meta| meta.nade_type == *filter_type)
                        .cloned()
                        .collect(),
                };
                if !filtered_filenames.is_empty() {
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
                    let spacing = 12.0; // Increased spacing for less cramped look
                    let min_padding = 8.0; // Padding around each image cell
                    let display_width = self.grid_image_size;
                    // Responsive: ensure at least 1 column, but maximize columns for window size
                    let num_columns = ((grid_rect.width() + spacing)
                        / (display_width + spacing + 2.0 * min_padding))
                        .floor()
                        .max(1.0) as usize;
                    egui::ScrollArea::vertical().show_viewport(ui, |ui, viewport| {
                        let grid = egui::Grid::new("image_grid").spacing([spacing, spacing]);
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
                            for (i, meta) in filtered_filenames
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
                                                // Display the image
                                                let image_widget = egui::Image::new(
                                                    egui::load::SizedTexture::new(
                                                        texture.id(),
                                                        texture.size_vec2(),
                                                    ),
                                                )
                                                .corner_radius(egui::CornerRadius::same(4));

                                                let image_response = ui.add_sized(
                                                    [display_width, display_height],
                                                    image_widget.sense(egui::Sense::click()),
                                                );

                                                // Persistent overlay for nade info
                                                let image_rect = image_response.rect;
                                                let painter = ui.painter_at(image_rect); // Draw within the image bounds

                                                let bar_height = 24.0; // Height of the top and bottom bars
                                                let icon_radius = (bar_height * 0.7) / 2.0; // Radius of the nade type icon
                                                let text_padding = 5.0; // Padding for elements within bars
                                                let font_size = bar_height * 0.65;
                                                let bar_color =
                                                    egui::Color32::from_rgba_unmultiplied(
                                                        20, 20, 20, 160,
                                                    );

                                                // --- Top Bar (Nade Type Icon + Position Label) ---
                                                let top_bar_y_start = image_rect.min.y;
                                                let top_bar_rect = egui::Rect::from_x_y_ranges(
                                                    image_rect.x_range(),
                                                    egui::Rangef::new(
                                                        top_bar_y_start,
                                                        top_bar_y_start + bar_height,
                                                    ),
                                                );
                                                painter.rect_filled(
                                                    top_bar_rect,
                                                    egui::CornerRadius {
                                                        nw: 4,
                                                        ne: 4,
                                                        sw: 0,
                                                        se: 0,
                                                    },
                                                    bar_color,
                                                );

                                                // Nade Type Icon (placeholder circle)
                                                let icon_center_y =
                                                    top_bar_rect.min.y + bar_height / 2.0;
                                                let icon_center_x =
                                                    top_bar_rect.min.x + text_padding + icon_radius;
                                                let icon_color = match meta.nade_type {
                                                    NadeType::Smoke => egui::Color32::DARK_GRAY,
                                                    NadeType::Flash => egui::Color32::WHITE,
                                                    NadeType::Molotov => {
                                                        egui::Color32::from_rgb(255, 69, 0)
                                                    } // OrangeRed
                                                    NadeType::Grenade => {
                                                        egui::Color32::from_rgb(34, 139, 34)
                                                    } // ForestGreen
                                                };
                                                painter.circle_filled(
                                                    egui::pos2(icon_center_x, icon_center_y),
                                                    icon_radius,
                                                    icon_color,
                                                );
                                                painter.circle_stroke(
                                                    egui::pos2(icon_center_x, icon_center_y),
                                                    icon_radius,
                                                    egui::Stroke::new(1.0, egui::Color32::BLACK),
                                                );

                                                // Position Label - Greedy Centering
                                                let position_text_str = if meta.position.is_empty()
                                                {
                                                    "[No Position]".to_string()
                                                } else {
                                                    meta.position.clone()
                                                };
                                                let text_color = egui::Color32::WHITE;
                                                let font_id = egui::FontId::proportional(font_size);

                                                // Layout the text to get its size
                                                let text_galley = painter.layout_no_wrap(
                                                    position_text_str,
                                                    font_id,
                                                    text_color,
                                                );

                                                // Calculate icon's right boundary (where text should not start before)
                                                let icon_right_boundary =
                                                    icon_center_x + icon_radius + text_padding;

                                                // Calculate ideal X for the text to be centered in the entire top_bar_rect
                                                let ideal_text_x = top_bar_rect.center().x
                                                    - text_galley.size().x / 2.0;

                                                // Ensure text starts after the icon, and also not before the bar's left edge + padding (if icon wasn't there)
                                                let actual_text_x =
                                                    ideal_text_x.max(icon_right_boundary);

                                                // Ensure text doesn't overflow past the right edge of the bar (minus padding)
                                                let max_text_x = top_bar_rect.max.x
                                                    - text_padding
                                                    - text_galley.size().x;
                                                let actual_text_x = actual_text_x.min(max_text_x);

                                                // Calculate Y for vertical centering within the top_bar_rect
                                                let actual_text_y = top_bar_rect.center().y
                                                    - text_galley.size().y / 2.0;

                                                // Draw the galley
                                                painter.galley(
                                                    egui::pos2(actual_text_x, actual_text_y),
                                                    text_galley,
                                                    text_color,
                                                );

                                                // --- Bottom Bar (Notes) ---
                                                let notes_text = if meta.notes.is_empty() {
                                                    "[No Notes]".to_string()
                                                } else {
                                                    meta.notes.clone()
                                                };
                                                let font_size = bar_height * 0.65;
                                                let text_color = egui::Color32::WHITE;
                                                let font_id = egui::FontId::proportional(font_size);
                                                let bar_color =
                                                    egui::Color32::from_rgba_unmultiplied(
                                                        20, 20, 20, 160,
                                                    );

                                                let bottom_bar_y_start =
                                                    image_rect.max.y - bar_height;
                                                let bottom_bar_rect = egui::Rect::from_x_y_ranges(
                                                    image_rect.x_range(),
                                                    egui::Rangef::new(
                                                        bottom_bar_y_start,
                                                        image_rect.max.y,
                                                    ),
                                                );
                                                painter.rect_filled(
                                                    bottom_bar_rect,
                                                    egui::CornerRadius {
                                                        nw: 0,
                                                        ne: 0,
                                                        sw: 4,
                                                        se: 4,
                                                    },
                                                    bar_color,
                                                );

                                                painter.text(
                                                    bottom_bar_rect.center(),
                                                    egui::Align2::CENTER_CENTER,
                                                    notes_text,
                                                    font_id,
                                                    text_color,
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
                                            0.0,
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
                if filtered_filenames.is_empty() {
                    ui.label("[No images uploaded for this filter]");
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
    }
}
