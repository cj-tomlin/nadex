// src/services/persistence_service.rs
use std::fs; // For future use, e.g. create_dir_all
use std::io; // For io::Error
use std::path::Path;
use std::path::PathBuf;
// For Path type hint, though join works with PathBuf
use crate::persistence::ImageManifest; // To return ImageManifest
use chrono::Utc;
use serde_json; // For deserialization // For timestamp in copy_image_to_data
// crate::thumbnail is no longer needed for these, but might be for generate_all_thumbnails later
// For now, let's remove it and add back if necessary. We will need image ops though.
use crate::services::thumbnail_service::{ThumbnailService, ThumbnailServiceError}; // Added for thumbnail generation call and error type
use std::sync::{Arc, Mutex}; // Added for Arc and Mutex
// image::imageops::FilterType and image::ImageFormat are no longer needed here as thumbnail generation moved

#[derive(Debug)]
pub enum PersistenceServiceError {
    IoError(std::io::Error),
    ThumbnailGenerationFailed(ThumbnailServiceError),
    InvalidInput(String),
    SerializationError(String),
}

impl std::fmt::Display for PersistenceServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PersistenceServiceError::IoError(err) => write!(f, "Persistence IO error: {}", err),
            PersistenceServiceError::ThumbnailGenerationFailed(err) => {
                write!(f, "Thumbnail generation failed: {}", err)
            }
            PersistenceServiceError::InvalidInput(msg) => write!(f, "Invalid input: {}", msg),
            PersistenceServiceError::SerializationError(msg) => {
                write!(f, "Serialization error: {}", msg)
            }
        }
    }
}

impl std::error::Error for PersistenceServiceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PersistenceServiceError::IoError(err) => Some(err),
            PersistenceServiceError::ThumbnailGenerationFailed(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for PersistenceServiceError {
    fn from(err: std::io::Error) -> Self {
        PersistenceServiceError::IoError(err)
    }
}

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
                    eprintln!(
                        "Failed to parse manifest.json: {}. Returning default manifest.",
                        e
                    );
                    ImageManifest::default()
                }),
                Err(e) => {
                    eprintln!(
                        "Failed to read manifest.json: {}. Returning default manifest.",
                        e
                    );
                    ImageManifest::default()
                }
            }
        } else {
            ImageManifest::default()
        }
    }

    pub fn save_manifest(&self, manifest: &ImageManifest) -> Result<(), PersistenceServiceError> {
        let manifest_path = self.data_dir.join("manifest.json");
        let json = serde_json::to_string_pretty(manifest).map_err(|e| {
            PersistenceServiceError::SerializationError(format!(
                "Failed to serialize manifest: {}",
                e
            ))
        })?;
        Ok(fs::write(manifest_path, json)?)
    }

    fn ensure_map_dir(&self, map: &str) -> Result<PathBuf, PersistenceServiceError> {
        let map_dir = self.data_dir.join(map);
        fs::create_dir_all(&map_dir)?;
        Ok(map_dir)
    }

    pub fn copy_image_to_data(
        &self,
        src: &Path,
        map: &str,
        thumbnail_service: &Arc<Mutex<ThumbnailService>>,
    ) -> Result<(PathBuf, String), PersistenceServiceError> {
        let map_dir = self.ensure_map_dir(map)?;

        let original_filename = src.file_name().ok_or_else(|| {
            PersistenceServiceError::InvalidInput("Invalid source path".to_string())
        })?;
        let stem = Path::new(original_filename)
            .file_stem()
            .ok_or_else(|| {
                PersistenceServiceError::InvalidInput("Could not extract file stem".to_string())
            })?
            .to_string_lossy();
        let extension = Path::new(original_filename)
            .extension()
            .map_or_else(|| "", |ext| ext.to_str().unwrap_or(""));

        let timestamp = Utc::now().format("%Y%m%d%H%M%S%3f").to_string();
        let unique_filename_str = if extension.is_empty() {
            format!("{}_{}", stem, timestamp)
        } else {
            format!("{}_{}.{}", stem, timestamp, extension)
        };

        let dest_path = map_dir.join(&unique_filename_str);
        fs::copy(src, &dest_path)?;

        // After successfully copying the main image, generate thumbnails using ThumbnailService.
        thumbnail_service
            .lock()
            .map_err(|e| {
                PersistenceServiceError::InvalidInput(format!(
                    "Failed to lock thumbnail service: {}",
                    e
                ))
            })? // Or a specific LockError variant
            .generate_thumbnails_for_image(&self.data_dir, map, &dest_path)
            .map_err(PersistenceServiceError::ThumbnailGenerationFailed)?;

        Ok((dest_path, unique_filename_str))
    }

    pub fn delete_image_and_thumbnails(
        &self,
        map_name: &str,
        image_filename: &str,
        thumbnail_service: &mut ThumbnailService,
    ) -> Result<(), PersistenceServiceError> {
        let image_path_in_data_dir = self.data_dir.join(map_name).join(image_filename);

        // 1. Delegate thumbnail deletion (cache and disk) to ThumbnailService
        //    This needs the base data_dir, map_name, and the original image's filename.
        // Attempt to delete thumbnails from disk and clear from cache via ThumbnailService
        // Errors here are logged by ThumbnailService but do not stop main image deletion.
        thumbnail_service.clear_cached_thumbnails_for_image(
            image_filename,
            map_name,
            &self.data_dir,
        );

        // 2. Delete main image file
        //    If this fails, we return the error immediately.
        fs::remove_file(&image_path_in_data_dir).map_err(|e| {
            log::error!(
                "PersistenceService: Failed to delete main image file {}: {}",
                image_path_in_data_dir.display(),
                e
            );
            e
        })?;
        log::info!(
            "PersistenceService: Deleted main image file: {}",
            image_path_in_data_dir.display()
        );

        // Note: The logic to remove the .thumbnails directory if empty could be added here or in ThumbnailService.
        // For now, clear_cached_thumbnails_for_image in ThumbnailService handles individual file deletions.
        // If ThumbnailService becomes the sole manager of the .thumbnails dir, it might handle its creation/deletion too.

        Ok(())
    }
}
