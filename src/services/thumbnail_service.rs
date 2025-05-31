// src/services/thumbnail_service.rs

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::fmt;
use std::fs;

use egui; // For Ui, TextureHandle, ColorImage, TextureOptions
use image;
use image::imageops::FilterType;
use image::ImageFormat;
use log;

const MAX_THUMB_CACHE_SIZE: usize = 18;

/// Allowed thumbnail widths (pixels).
pub(crate) const ALLOWED_THUMB_SIZES: [u32; 3] = [960, 720, 480];

/// Constructs the canonical path for a thumbnail file.
/// img_path: Full path to the original image file (e.g., within the map's data directory)
/// thumb_dir: Full path to the .thumbnails directory for that map
/// size: The target width of the thumbnail
pub(crate) fn module_construct_thumbnail_path(img_path: &Path, thumb_dir: &Path, size: u32) -> PathBuf {
    let stem = img_path.file_stem().unwrap_or_default().to_string_lossy();
    // Ensure consistent naming, e.g. always webp
    thumb_dir.join(format!("{}_{}.webp", stem, size))
}

// --- ThumbnailCache struct and impl --- 
pub struct ThumbnailCache {
    textures: HashMap<String, (egui::TextureHandle, (u32, u32))>, // Key: thumb_path_str
    order: VecDeque<String>,                                      // For LRU: stores thumb_path_str
}

impl ThumbnailCache {
    pub fn new() -> Self {
        Self {
            textures: HashMap::new(),
            order: VecDeque::with_capacity(MAX_THUMB_CACHE_SIZE),
        }
    }

    fn prune(&mut self) {
        while self.order.len() > MAX_THUMB_CACHE_SIZE {
            if let Some(oldest_key) = self.order.pop_back() {
                if self.textures.remove(&oldest_key).is_some() {
                    // log::debug!("Cache PRUNED: {}", oldest_key);
                }
            } else {
                break;
            }
        }
    }

    pub fn get_or_load(
        &mut self,
        ui: &egui::Ui,
        image_file_path: &PathBuf,    // Full path to original image in its map folder
        thumb_storage_dir: &PathBuf, // Path to .thumbnails directory for the map
        target_size: u32,
    ) -> Option<&(egui::TextureHandle, (u32, u32))> {
        let thumb_path = module_construct_thumbnail_path(image_file_path, thumb_storage_dir, target_size);
        
        if !thumb_path.exists() {
            // log::warn!("Thumbnail not found on disk for {:?} (size {}). Generation might be needed.", image_file_path, target_size);
            // In the future, this could trigger on-demand generation if desired.
            return None;
        }
        
        let thumb_path_str = thumb_path.to_string_lossy().into_owned();

        if self.textures.contains_key(&thumb_path_str) {
            if let Some(index) = self.order.iter().position(|x| x == &thumb_path_str) {
                let key = self.order.remove(index).unwrap();
                self.order.push_front(key);
            } else {
                // Should not happen if contains_key is true, but as a fallback:
                self.order.push_front(thumb_path_str.clone());
            }
            return self.textures.get(&thumb_path_str);
        }

        match image::open(&thumb_path) {
            Ok(img) => {
                let image_width = img.width();
                let image_height = img.height();
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [image_width as usize, image_height as usize],
                    img.to_rgba8().as_flat_samples().as_slice(),
                );

                let texture_name = thumb_path_str.clone();
                let texture_handle =
                    ui.ctx()
                        .load_texture(texture_name, color_image, egui::TextureOptions::LINEAR);

                self.textures.insert(
                    thumb_path_str.clone(),
                    (texture_handle, (image_width, image_height)),
                );
                self.order.push_front(thumb_path_str.clone());
                self.prune();
                self.textures.get(&thumb_path_str)
            }
            Err(_e) => {
                log::error!("Failed to open thumbnail image {}: {}", thumb_path_str, _e);
                None
            }
        }
    }

    pub fn remove_image_thumbnails(
        &mut self,
        image_filename: &str,    // Original image filename (e.g., "myimage_timestamp.jpg")
        image_map_name: &str,    // Map name (e.g., "de_dust2")
        data_dir: &PathBuf,      // Root data directory
    ) {
        let map_data_dir = data_dir.join(image_map_name);
        let original_image_path_in_data = map_data_dir.join(image_filename); // Path to the main image
        let thumb_storage_dir = map_data_dir.join(".thumbnails");

        for &size in ALLOWED_THUMB_SIZES.iter() {
            let expected_thumb_path = module_construct_thumbnail_path(
                &original_image_path_in_data, 
                &thumb_storage_dir,
                size
            );
            let expected_thumb_path_str = expected_thumb_path.to_string_lossy().into_owned();

            if self.textures.remove(&expected_thumb_path_str).is_some() {
                self.order.retain(|k| k != &expected_thumb_path_str);
            }
        }
    }
}

impl fmt::Debug for ThumbnailCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ThumbnailCache")
            .field("textures_count", &self.textures.len())
            .field("order_count", &self.order.len())
            .finish()
    }
}

// --- ThumbnailService struct and impl ---
#[derive(Debug)]
pub struct ThumbnailService {
    cache: ThumbnailCache,
}

impl ThumbnailService {
    pub fn new() -> Self {
        Self {
            cache: ThumbnailCache::new(),
        }
    }

    // Delegating methods for cache access
    pub fn get_or_load_thumbnail_texture(
        &mut self,
        ui: &egui::Ui,
        image_file_path: &PathBuf,    
        thumb_storage_dir: &PathBuf, 
        target_size: u32,
    ) -> Option<&(egui::TextureHandle, (u32, u32))> {
        self.cache.get_or_load(ui, image_file_path, thumb_storage_dir, target_size)
    }

    pub fn clear_cached_thumbnails_for_image(
        &mut self,
        image_filename: &str,
        image_map_name: &str,
        data_dir: &PathBuf,
    ) {
        self.cache.remove_image_thumbnails(image_filename, image_map_name, data_dir);
    }

    // Thumbnail generation method
    pub fn generate_thumbnails_for_image(
        &self, // Doesn't modify self.cache, so &self is fine. Could be static if no self fields were needed.
        data_dir: &Path, 
        map_name: &str, 
        original_image_path_in_data: &Path
    ) -> Result<(), String> {
        let thumb_dir = data_dir.join(map_name).join(".thumbnails");
        if let Err(e) = fs::create_dir_all(&thumb_dir) {
            let err_msg = format!("Failed to create thumbnail directory {}: {}", thumb_dir.display(), e);
            log::error!("ThumbnailService: {}", err_msg);
            return Err(err_msg);
        }

        match image::open(original_image_path_in_data) {
            Ok(img) => {
                for &size in ALLOWED_THUMB_SIZES.iter() {
                    let thumb_path = module_construct_thumbnail_path(original_image_path_in_data, &thumb_dir, size);
                    if !thumb_path.exists() { // Avoid re-generating if it somehow exists
                        let width = size;
                        let height = (size as f32 * img.height() as f32 / img.width() as f32).round() as u32;
                        let height = if height == 0 { (size as f32 * 3.0/4.0).round() as u32 } else { height }; // Fallback

                        let resized = img.resize(width, height, FilterType::Lanczos3);
                        
                        match fs::File::create(&thumb_path) {
                            Ok(mut file) => {
                                if let Err(e) = resized.write_to(&mut file, ImageFormat::WebP) {
                                    log::error!(
                                        "ThumbnailService: Failed to write WebP thumbnail {}: {}",
                                        thumb_path.display(),
                                        e
                                    );
                                    // Continue to try other sizes
                                }
                            }
                            Err(e) => {
                                log::error!(
                                    "ThumbnailService: Failed to create WebP thumbnail file {}: {}",
                                    thumb_path.display(),
                                    e
                                );
                                // Continue to try other sizes
                            }
                        }
                    }
                }
                Ok(())
            }
            Err(e) => {
                let err_msg = format!(
                    "ThumbnailService: Failed to open image {} for thumbnail generation: {}",
                    original_image_path_in_data.display(),
                    e
                );
                log::error!("{}", err_msg);
                Err(err_msg)
            }
        }
    }
}
