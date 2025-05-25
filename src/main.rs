use eframe::{egui, App, Frame, NativeOptions};
use rfd::FileDialog;
use image::GenericImageView;
use std::path::PathBuf;
use std::collections::{HashMap, VecDeque};
use egui::TextureHandle;
mod persistence;
mod thumbnail;
use persistence::{ImageManifest, save_manifest, load_manifest, copy_image_to_data};
use thumbnail::{generate_all_thumbnails, get_thumbnail, ALLOWED_THUMB_SIZES};
use dirs;

fn main() -> eframe::Result<()> {
    let mut options = NativeOptions::default();
    options.viewport.maximized = Some(true);
    eframe::run_native(
        "nadex",
        options,
        Box::new(|_cc| Ok(Box::new(NadexApp::default()))),
    )
}

struct NadexApp {
    // Current selected map
    current_map: String,
    // List of available maps
    maps: Vec<&'static str>,
    // Map of map name -> Vec of image file names (not full paths)
    manifest: ImageManifest,
    // For displaying error messages
    error_message: Option<String>,
    // Flag to trigger file dialog outside UI
    pending_file_dialog: bool,
    // App data dir
    data_dir: PathBuf,
    // User grid preferences
    grid_columns: usize,
    grid_image_size: f32,
    // Window state (future: persist)
    fullscreen: bool,
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
            current_map: "de_ancient".to_string(),
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
            pending_file_dialog: false,
            data_dir,
            grid_columns: 4,
            grid_image_size: 480.0, // Must be in ALLOWED_THUMB_SIZES
            fullscreen: true,
            thumb_texture_cache: HashMap::new(),
            thumb_cache_order: VecDeque::new(),
        }
    }
}

impl App for NadexApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        // Handle file dialog outside of egui UI
        if self.pending_file_dialog {
            self.pending_file_dialog = false;
            if let Some(path) = FileDialog::new()
                .add_filter("Image", &["png", "jpg", "jpeg", "bmp"])
                .pick_file()
            {
                match image::open(&path) {
                    Ok(img) => {
                        let dims = img.dimensions();
                        if dims == (1920, 1440) {
                            // Copy image to data dir
                            match copy_image_to_data(&path, &self.data_dir, &self.current_map) {
                                Ok(dest_path) => {
                                    let filename = dest_path.file_name().unwrap().to_string_lossy().to_string();
                                    // Generate all thumbnails for this image
                                    let thumb_dir = self.data_dir.join(&self.current_map).join(".thumbnails");
                                    generate_all_thumbnails(&dest_path, &thumb_dir);
                                    let entry = self.manifest.images.entry(self.current_map.clone()).or_default();
                                    if !entry.contains(&filename) {
                                        entry.push(filename);
                                        let _ = save_manifest(&self.manifest, &self.data_dir);
                                    }
                                    self.error_message = None;

                                }
                                Err(e) => {
                                    self.error_message = Some(format!("Failed to copy image: {}", e));
                                }
                            }
                        } else {
                            self.error_message = Some(format!(
                                "Image must be 1920x1440 pixels (got {}x{})",
                                dims.0, dims.1
                            ));
                        }
                    }
                    Err(e) => {
                        self.error_message = Some(format!("Failed to open image: {}", e));
                    }
                }
            }
        }
        egui::TopBottomPanel::top("map_selector_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Map:");
                egui::ComboBox::new("map_selector", "")
                    .selected_text(&self.current_map)
                    .show_ui(ui, |ui| {
                        for map in &self.maps {
                            if ui.selectable_value(&mut self.current_map, map.to_string(), *map).clicked() {
                                // Map changed

                            }
                        }
                    });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Lineup Screenshots");

            // Upload button
            if ui.button("Upload Screenshot").clicked() {
                self.pending_file_dialog = true;
            }

            // Grid controls
            ui.horizontal(|ui| {
                ui.label("Image size:");
                let mut idx = ALLOWED_THUMB_SIZES.iter().position(|&s| s == self.grid_image_size as u32).unwrap_or(0);
                egui::ComboBox::from_id_source("thumb_size_select")
                    .selected_text(format!("{} px", ALLOWED_THUMB_SIZES[idx]))
                    .show_ui(ui, |ui| {
                        for (i, &sz) in ALLOWED_THUMB_SIZES.iter().enumerate() {
                            if ui.selectable_value(&mut idx, i, format!("{} px", sz)).clicked() {
                                self.grid_image_size = sz as f32;
                            }
                        }
                    });
            });

            // Show error message if any
            if let Some(ref msg) = self.error_message {
                ui.colored_label(egui::Color32::RED, msg);
            }

            // Display image grid for self.current_map
            let filenames = self.manifest.images.get(&self.current_map).cloned().unwrap_or_default();
            if !filenames.is_empty() {
                // Filter out missing images and clean manifest
                let map = &self.current_map;
                let mut removed = false;
                let mut to_remove = vec![];
                for filename in &filenames {
                    let img_path = self.data_dir.join(map).join(filename);
                    if !img_path.exists() {
                        to_remove.push(filename.clone());
                        removed = true;
                    }
                }
                if removed {
                    if let Some(entry) = self.manifest.images.get_mut(map) {
                        entry.retain(|f| !to_remove.contains(f));
                        let _ = save_manifest(&self.manifest, &self.data_dir);
                    }
                }
                // Determine number of columns to fit the window
                let available_width = ui.available_width();
                let spacing = 12.0;
                let img_w = self.grid_image_size;
                let _num_columns = ((available_width + spacing) / (img_w + spacing)).floor().max(1.0) as usize;
                let _row = 0;
                egui::ScrollArea::vertical().show_viewport(ui, |ui, viewport| {
                    let grid = egui::Grid::new("image_grid");
                    // Estimate visible rows based on scroll offset and viewport height
                    let img_h = self.grid_image_size;
                    let spacing = 12.0;
                    let row_height = img_h + spacing;
                    let num_columns = ((ui.available_width() + spacing) / (self.grid_image_size + spacing)).floor().max(1.0) as usize;
                    let total_images = filenames.iter().filter(|f| {
                        let img_path = self.data_dir.join(map).join(f);
                        img_path.exists()
                    }).count();
                    let total_rows = (total_images + num_columns - 1) / num_columns;
                    let offset_y = viewport.min.y;
                    let viewport_height = viewport.height();
                    let first_visible_row = (offset_y / row_height).floor() as usize;
                    let last_visible_row = ((offset_y + viewport_height) / row_height).ceil() as usize;
                    let mut row = 0;
                    grid.show(ui, |ui| {
                        for (i, filename) in filenames.iter().filter(|f| {
                            let img_path = self.data_dir.join(map).join(f);
                            img_path.exists()
                        }).enumerate() {
                            let this_row = i / num_columns;
                            if this_row < first_visible_row || this_row > last_visible_row {
                                // Not visible, show placeholder
                                let (w, h) = (self.grid_image_size, self.grid_image_size);
                                let rect = ui.allocate_space(egui::Vec2::new(w, h));
                                ui.painter().rect_filled(rect.1, 4.0, egui::Color32::from_gray(80));
                            } else {
                                let img_path = self.data_dir.join(&self.current_map).join(filename);
                                let thumb_dir = self.data_dir.join(&self.current_map).join(".thumbnails");
                                // Find the closest allowed thumbnail size
                                let requested_size = self.grid_image_size as u32;
                                let &closest_size = ALLOWED_THUMB_SIZES.iter().min_by_key(|&&s| (s as i32 - requested_size as i32).abs()).unwrap_or(&480);
                                let cache_key = (filename.clone(), closest_size);
                                let mut loaded = false;
                                if let Some(thumb_path) = get_thumbnail(&img_path, &thumb_dir, closest_size) {
                                    if let Ok(img) = image::open(&thumb_path) {
                                        let color_image = egui::ColorImage::from_rgba_unmultiplied([
                                            img.width() as usize,
                                            img.height() as usize,
                                        ],
                                        img.to_rgba8().as_flat_samples().as_slice());
                                        // LRU cache eviction: remove oldest if over 256
                                        if !self.thumb_texture_cache.contains_key(&cache_key) {
                                            if self.thumb_texture_cache.len() >= 256 {
                                                if let Some(oldest) = self.thumb_cache_order.pop_front() {
                                                    self.thumb_texture_cache.remove(&oldest);
                                                }
                                            }
                                            let texture = ui.ctx().load_texture(
                                                format!("thumb_{}_{}", filename, closest_size),
                                                color_image,
                                                egui::TextureOptions::default(),
                                            );
                                            self.thumb_texture_cache.insert(cache_key.clone(), texture);
                                            self.thumb_cache_order.push_back(cache_key.clone());
                                        }
                                        if let Some(texture) = self.thumb_texture_cache.get(&cache_key) {
                                            ui.add(egui::Image::new(texture).fit_to_exact_size(egui::Vec2::new(self.grid_image_size, self.grid_image_size)));
                                            loaded = true;
                                        }
                                    }
                                }
                                if !loaded {
                                    let (w, h) = (self.grid_image_size, self.grid_image_size);
                                    let rect = ui.allocate_space(egui::Vec2::new(w, h));
                                    ui.painter().rect_filled(rect.1, 4.0, egui::Color32::from_gray(80));
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
