// src/services/persistence_service.rs
use std::path::PathBuf;
use std::fs; // For future use, e.g. create_dir_all
use std::io; // For io::Error
use std::path::Path;
 // For Path type hint, though join works with PathBuf
use crate::persistence::ImageManifest; // To return ImageManifest
use serde_json; // For deserialization
use chrono::Utc; // For timestamp in copy_image_to_data
// crate::thumbnail is no longer needed for these, but might be for generate_all_thumbnails later
// For now, let's remove it and add back if necessary. We will need image ops though.
use image::imageops::FilterType; // For thumbnail generation
use image::ImageFormat; // For thumbnail generation

/// Allowed thumbnail widths (pixels). Should be divisors of 1920 for best quality.
pub const ALLOWED_THUMB_SIZES: [u32; 3] = [960, 720, 480]; // Moved from thumbnail.rs

#[derive(Debug)]
pub struct PersistenceService {
    data_dir: PathBuf,
}

impl PersistenceService {
    pub fn new(data_dir: PathBuf) -> io::Result<Self> {
        // Ensure the data directory exists
        if !data_dir.exists() {
            fs::create_dir_all(&data_dir)?;
        }
        Ok(Self { data_dir })
    }

    // Methods for load_manifest, save_manifest, copy_image_to_storage, etc., will be added here.
    pub fn load_manifest(&self) -> ImageManifest {
        let manifest_path = self.data_dir.join("manifest.json");
        if manifest_path.exists() {
            match fs::read_to_string(&manifest_path) {
                Ok(json) => serde_json::from_str(&json).unwrap_or_else(|e| {
                    eprintln!("Failed to parse manifest.json: {}. Returning default manifest.", e);
                    ImageManifest::default()
                }),
                Err(e) => {
                    eprintln!("Failed to read manifest.json: {}. Returning default manifest.", e);
                    ImageManifest::default()
                }
            }
        } else {
            ImageManifest::default()
        }
    }

    pub fn save_manifest(&self, manifest: &ImageManifest) -> io::Result<()> {
        let manifest_path = self.data_dir.join("manifest.json");
        let json = serde_json::to_string_pretty(manifest).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("Failed to serialize manifest: {}", e))
        })?;
        fs::write(manifest_path, json)
    }

    // Made static temporarily to be callable from threads without passing self
    fn ensure_map_dir(data_dir: &Path, map: &str) -> io::Result<PathBuf> {
        let map_dir = data_dir.join(map);
        fs::create_dir_all(&map_dir)?;
        Ok(map_dir)
    }

    // Made static temporarily to be callable from threads without passing self
    pub fn copy_image_to_data(data_dir: &Path, src: &Path, map: &str) -> io::Result<(PathBuf, String)> {
        let map_dir = Self::ensure_map_dir(data_dir, map)?;
        
        let original_filename = src.file_name().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid source path"))?;
        let stem = Path::new(original_filename).file_stem().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "Could not extract file stem"))?.to_string_lossy();
        let extension = Path::new(original_filename).extension().map_or_else(|| "", |ext| ext.to_str().unwrap_or(""));

        let timestamp = Utc::now().format("%Y%m%d%H%M%S%3f").to_string(); // YYYYMMDDHHMMSSmmm (milliseconds)
        let unique_filename_str = if extension.is_empty() {
            format!("{}_{}", stem, timestamp)
        } else {
            format!("{}_{}.{}", stem, timestamp, extension)
        };

        let dest_path = map_dir.join(&unique_filename_str);
        fs::copy(src, &dest_path)?;

        // After successfully copying the main image, generate thumbnails.
        if let Err(thumb_err) = Self::_generate_all_thumbnails_for_image(data_dir, map, &dest_path) {
            log::warn!(
                "PersistenceService: Main image {} copied successfully to {}, but thumbnail generation failed: {}",
                src.display(),
                dest_path.display(),
                thumb_err
            );
            // We don't return an error here, as the main file is copied.
            // The caller can decide how to handle partial success if needed.
        }

        Ok((dest_path, unique_filename_str))
    }

    pub fn delete_image_and_thumbnails(&self, map_name: &str, filename: &str) -> io::Result<()> {
        let image_path_in_data_dir = self.data_dir.join(map_name).join(filename);

        // Delete main image file
        // If this fails, we return the error immediately.
        fs::remove_file(&image_path_in_data_dir).map_err(|e| {
            log::error!(
                "PersistenceService: Failed to delete main image file {}: {}",
                image_path_in_data_dir.display(),
                e
            );
            e
        })?;
        log::info!("PersistenceService: Deleted main image file: {}", image_path_in_data_dir.display());

        // Delete thumbnails
        // Errors in deleting thumbnails will be logged but won't cause the overall operation to fail,
        // as the main image deletion is the primary concern.
        let thumb_base_dir = self.data_dir.join(map_name).join(".thumbnails");
        
        // We need the original image path to correctly determine thumbnail names via thumbnail_path
        // The `image_path_in_data_dir` is exactly this path.
        for &size in ALLOWED_THUMB_SIZES.iter() { // Use module-level const directly
            // thumbnail::thumbnail_path expects the full path to the original image in the data directory
            // and the base directory where thumbnails are stored.
            let thumb_path_to_delete = Self::thumbnail_path( // Use Self::thumbnail_path
                &image_path_in_data_dir, // Path to the original image (which we just deleted or confirmed deletion of)
                &thumb_base_dir,         // Base .thumbnails directory for the map
                size,
            );

            match fs::remove_file(&thumb_path_to_delete) {
                Ok(_) => log::info!("PersistenceService: Deleted thumbnail: {}", thumb_path_to_delete.display()),
                Err(e) => {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        log::warn!(
                            "PersistenceService: Failed to delete thumbnail file {}: {}",
                            thumb_path_to_delete.display(),
                            e
                        );
                    }
                    // Otherwise, if NotFound, it's fine, maybe it was never created or already cleaned up.
                }
            }
        }
        Ok(())
    }

    /// Returns the path for a thumbnail of a given image at a given size (moved from thumbnail.rs)
    /// img_path: Full path to the original image file (e.g., within the map's data directory)
    /// thumb_dir: Full path to the .thumbnails directory for that map
    /// size: The target width of the thumbnail
    pub(crate) fn thumbnail_path(img_path: &Path, thumb_dir: &Path, size: u32) -> PathBuf {
        let stem = img_path.file_stem().unwrap_or_default().to_string_lossy();
        // Ensure consistent naming, e.g. always webp
        thumb_dir.join(format!("{}_{}.webp", stem, size))
    }

    fn _generate_all_thumbnails_for_image(
        data_dir: &Path,
        map_name: &str,
        original_image_path_in_data: &Path, // Full path to the already copied unique image
    ) -> Result<(), String> {
        let thumb_dir = data_dir.join(map_name).join(".thumbnails");
        if let Err(e) = fs::create_dir_all(&thumb_dir) {
            let err_msg = format!("Failed to create thumbnail directory {}: {}", thumb_dir.display(), e);
            log::error!("PersistenceService: {}", err_msg);
            return Err(err_msg);
        }

        match image::open(original_image_path_in_data) {
            Ok(img) => {
                for &size in ALLOWED_THUMB_SIZES.iter() {
                    let thumb_path = Self::thumbnail_path(original_image_path_in_data, &thumb_dir, size);
                    if !thumb_path.exists() { // Avoid re-generating if it somehow exists
                        let width = size;
                        // Maintain aspect ratio, assuming common 4:3 or 16:9, 
                        // but for simplicity, let's use a fixed 4:3 aspect for thumbnails like before.
                        // A more robust solution would get original aspect ratio.
                        let height = (size as f32 * img.height() as f32 / img.width() as f32).round() as u32;
                        let height = if height == 0 { (size as f32 * 3.0/4.0).round() as u32 } else { height }; // Fallback if calc is zero

                        let resized = img.resize(width, height, FilterType::Lanczos3);
                        
                        match fs::File::create(&thumb_path) {
                            Ok(mut file) => {
                                if let Err(e) = resized.write_to(&mut file, ImageFormat::WebP) {
                                    log::error!(
                                        "PersistenceService: Failed to write WebP thumbnail {}: {}",
                                        thumb_path.display(),
                                        e
                                    );
                                    // Continue to try other sizes
                                }
                            }
                            Err(e) => {
                                log::error!(
                                    "PersistenceService: Failed to create WebP thumbnail file {}: {}",
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
                    "PersistenceService: Failed to open image {} for thumbnail generation: {}",
                    original_image_path_in_data.display(),
                    e
                );
                log::error!("{}", err_msg);
                Err(err_msg)
            }
        }
    }

    /// Checks if a thumbnail file exists on disk and returns its path.
    /// original_image_path_in_data: Full path to the original image file in the data directory.
    /// thumb_dir: Full path to the .thumbnails directory for the map.
    /// size: The target width of the thumbnail.
    pub(crate) fn get_thumbnail_disk_path_if_exists(
        original_image_path_in_data: &Path, 
        thumb_dir: &Path, 
        size: u32
    ) -> Option<PathBuf> {
        let thumb_path = Self::thumbnail_path(original_image_path_in_data, thumb_dir, size);
        if thumb_path.exists() {
            Some(thumb_path)
        } else {
            None
        }
    }
}
