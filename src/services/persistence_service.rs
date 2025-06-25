// src/services/persistence_service.rs
use std::fs; // For future use, e.g. create_dir_all
use std::io; // For io::Error
use std::path::Path;
use std::path::PathBuf;
// For Path type hint, though join works with PathBuf
use crate::persistence::{ImageManifest, ImageMeta, MapMeta, NadeType}; // To return ImageManifest and use its components
use chrono::Utc;
use serde_json;
use std::time::SystemTime; // For deserialization // For timestamp in copy_image_to_data
// crate::thumbnail is no longer needed for these, but might be for generate_all_thumbnails later
// For now, let's remove it and add back if necessary. We will need image ops though.
use crate::services::thumbnail_service::{ThumbnailServiceError, ThumbnailServiceTrait}; // Added for thumbnail generation call and error type
use std::sync::{Arc, Mutex}; // Added for Arc and Mutex
// image::imageops::FilterType and image::ImageFormat are no longer needed here as thumbnail generation moved

#[derive(Debug)]
pub enum PersistenceServiceError {
    IoError(std::io::Error),
    InvalidInput(String),
    SerializationError(String),
    ThumbnailGenerationFailed(ThumbnailServiceError),
    ThumbnailDeletionFailed(ThumbnailServiceError), // New variant, expects ThumbnailServiceError
}

impl std::fmt::Display for PersistenceServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PersistenceServiceError::IoError(err) => write!(f, "Persistence IO error: {}", err),
            PersistenceServiceError::InvalidInput(msg) => write!(f, "Invalid input: {}", msg),
            PersistenceServiceError::SerializationError(msg) => {
                write!(f, "Serialization error: {}", msg)
            }
            PersistenceServiceError::ThumbnailGenerationFailed(err) => {
                write!(f, "Thumbnail generation failed: {}", err)
            }
            PersistenceServiceError::ThumbnailDeletionFailed(err) => {
                write!(f, "Thumbnail deletion failed: {}", err)
            }
        }
    }
}

impl std::error::Error for PersistenceServiceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PersistenceServiceError::IoError(err) => Some(err),
            PersistenceServiceError::ThumbnailGenerationFailed(err) => Some(err),
            PersistenceServiceError::ThumbnailDeletionFailed(err) => Some(err),
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
        nade_type: NadeType,
        notes: String,
        position: String,
        thumbnail_service: &Arc<Mutex<dyn ThumbnailServiceTrait>>,
    ) -> Result<(PathBuf, String), PersistenceServiceError> {
        if map.trim().is_empty() {
            return Err(PersistenceServiceError::InvalidInput(
                "Map name cannot be empty.".to_string(),
            ));
        }
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

        // After successfully copying the main image, convert it to full-size WebP.
        let thumb_storage_dir = map_dir.join(".thumbnails");

        // Ensure the thumbnails directory exists
        if !thumb_storage_dir.exists() {
            match fs::create_dir_all(&thumb_storage_dir) {
                Ok(_) => log::info!(
                    "Created thumbnails directory: {}",
                    thumb_storage_dir.display()
                ),
                Err(e) => {
                    log::error!(
                        "Failed to create thumbnails directory {}: {}",
                        thumb_storage_dir.display(),
                        e
                    );
                    return Err(PersistenceServiceError::IoError(e));
                }
            }
        }

        // Acquire lock on thumbnail service with better error handling
        let thumbnail_service_locked = match thumbnail_service.lock() {
            Ok(service) => service,
            Err(e) => {
                log::error!("Failed to acquire lock on thumbnail service: {}", e);
                // Clean up the copied image since we can't proceed
                if let Err(remove_err) = fs::remove_file(&dest_path) {
                    log::error!(
                        "Additionally, failed to cleanup main image file {} after lock error: {}",
                        dest_path.display(),
                        remove_err
                    );
                }
                return Err(PersistenceServiceError::InvalidInput(format!(
                    "Internal error: Failed to access thumbnail service: {}",
                    e
                )));
            }
        };

        // Generate a single full-size WebP version of the image instead of multiple thumbnails
        match thumbnail_service_locked.convert_to_full_webp(&dest_path, &thumb_storage_dir) {
            Ok(webp_path) => {
                log::info!(
                    "WebP image successfully generated at: {}",
                    webp_path.display()
                );
            }
            Err(e) => {
                log::error!("Failed to convert image to WebP: {:?}", e);
                // Attempt to clean up the copied main image file
                if let Err(remove_err) = fs::remove_file(&dest_path) {
                    log::error!(
                        "Additionally, failed to cleanup main image file {} after WebP conversion error: {}",
                        dest_path.display(),
                        remove_err
                    );
                }
                // Propagate the WebP conversion error, wrapped in PersistenceServiceError
                return Err(PersistenceServiceError::ThumbnailGenerationFailed(e));
            }
        }

        // Successfully copied image and generated thumbnails, now update manifest
        let mut manifest = self.load_manifest();

        let image_meta = ImageMeta {
            filename: unique_filename_str.clone(), // unique_filename_str is String
            map: map.to_string(),                  // map is &str from function arguments
            nade_type,                             // Use passed-in nade_type
            notes,                                 // Use passed-in notes
            position,                              // Use passed-in position
        };

        manifest
            .images
            .entry(map.to_string())
            .or_default()
            .push(image_meta);

        // Update map metadata, specifically last_accessed time
        manifest
            .maps
            .entry(map.to_string())
            .or_insert_with(|| MapMeta {
                last_accessed: SystemTime::now(),
            })
            .last_accessed = SystemTime::now();

        self.save_manifest(&manifest)?;

        Ok((dest_path, unique_filename_str))
    }

    pub fn delete_image_and_thumbnails(
        &self,
        map_name: &str,
        image_filename: &str,
        thumbnail_service: &Arc<Mutex<dyn ThumbnailServiceTrait>>,
    ) -> Result<(), PersistenceServiceError> {
        let image_path_in_data_dir = self.data_dir.join(map_name).join(image_filename);

        // 1. Delegate thumbnail deletion (cache and disk) to ThumbnailService
        //    This needs the base data_dir, map_name, and the original image's filename.
        // Attempt to delete thumbnails from disk and clear from cache via ThumbnailService
        // Errors here are logged by ThumbnailService but do not stop main image deletion.
        thumbnail_service
            .lock()
            .unwrap()
            .remove_thumbnails_for_image(image_filename, map_name, &self.data_dir)
            .map_err(|io_err| {
                // io_err is std::io::Error
                log::error!(
                    "PersistenceService: Failed to remove thumbnails for image {} in map {}: {}",
                    image_filename,
                    map_name,
                    &io_err // Log by reference as io_err will be moved
                );
                PersistenceServiceError::ThumbnailDeletionFailed(io_err) // Use the correct variant
            })?;

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

        // Now, update the manifest to remove the image entry
        let mut manifest = self.load_manifest();
        let mut map_became_empty = false;

        if let Some(images_in_map) = manifest.images.get_mut(map_name) {
            let original_len = images_in_map.len();
            images_in_map.retain(|img| img.filename != image_filename);
            if images_in_map.is_empty() && original_len > 0 {
                map_became_empty = true;
            }
        }

        // If the map's image list is now empty, remove the map itself from both images and maps collections.
        if map_became_empty {
            manifest.images.remove(map_name);
            manifest.maps.remove(map_name); // Also remove associated map metadata
            log::info!(
                "PersistenceService: Map '{}' became empty and was removed from manifest after deleting image '{}'.",
                map_name,
                image_filename
            );
        }

        self.save_manifest(&manifest)?;
        log::info!(
            "PersistenceService: Updated manifest after deleting image '{}' from map '{}'.",
            image_filename,
            map_name
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*; // Make parent module's items available
    use std::fs::{self, File};
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use tempfile::NamedTempFile; // NamedTempFile for dummy files

    use crate::persistence::{ImageManifest, ImageMeta, MapMeta, NadeType};
    use crate::services::thumbnail_service::{
        SerializableImageError, SerializableIoError, ThumbnailServiceError, ThumbnailServiceTrait,
    };
    // Import the new common setup utilities and MockThumbnailService from tests_common
    use crate::tests_common::setup_persistence_test_env;

    fn create_dummy_source_file() -> io::Result<NamedTempFile> {
        let mut file = NamedTempFile::new()?;
        writeln!(file, "dummy image content")?;
        file.flush()?;
        Ok(file)
    }

    #[test]
    fn test_ensure_map_dir_creates_new_dir() {
        let env = setup_persistence_test_env();
        let service = env.persistence_service;
        let map_name = "test_map_new";
        let map_path_result = service.ensure_map_dir(map_name);
        assert!(map_path_result.is_ok(), "ensure_map_dir should return Ok");
        let map_path = map_path_result.unwrap();
        assert!(
            map_path.exists(),
            "Map directory should exist at {:?}",
            map_path
        );
        assert!(map_path.is_dir(), "{:?} should be a directory", map_path);
        assert_eq!(
            map_path,
            env.data_dir_path.join(map_name),
            "Returned path should match expected path"
        );
    }

    #[test]
    fn test_ensure_map_dir_existing_dir() {
        let env = setup_persistence_test_env();
        let service = env.persistence_service;
        let map_name = "test_map_existing";
        let expected_map_path = env.data_dir_path.join(map_name);
        fs::create_dir_all(&expected_map_path).unwrap_or_else(|e| {
            panic!(
                "Failed to pre-create map dir {:?}: {}",
                expected_map_path, e
            )
        });
        let map_path_result = service.ensure_map_dir(map_name);
        assert!(
            map_path_result.is_ok(),
            "ensure_map_dir should return Ok for existing dir"
        );
        let map_path = map_path_result.unwrap();
        assert!(
            map_path.exists(),
            "Existing map directory should still exist at {:?}",
            map_path
        );
        assert!(
            map_path.is_dir(),
            "{:?} should still be a directory",
            map_path
        );
        assert_eq!(
            map_path, expected_map_path,
            "Returned path for existing dir should match expected"
        );
    }

    #[test]
    fn test_load_manifest_new_service_returns_default() {
        let env = setup_persistence_test_env();
        let service = env.persistence_service;
        let manifest = service.load_manifest();
        assert_eq!(
            manifest,
            ImageManifest::default(),
            "Should return default manifest when none exists"
        );
    }

    #[test]
    fn test_load_manifest_reads_valid_manifest() {
        let env = setup_persistence_test_env();
        let service = env.persistence_service;
        let manifest_path = env.data_dir_path.join("manifest.json");
        let mut expected_manifest = ImageManifest::default();
        let test_map_name = "Test Map".to_string();
        let test_map_meta = MapMeta {
            last_accessed: std::time::SystemTime::now(),
        };
        expected_manifest
            .maps
            .insert(test_map_name.clone(), test_map_meta.clone());
        let test_image = ImageMeta {
            filename: "test_image.jpg".to_string(),
            map: test_map_name.clone(),
            nade_type: NadeType::default(),
            notes: "Test notes".to_string(),
            position: "A Site".to_string(),
        };
        expected_manifest
            .images
            .insert(test_map_name.clone(), vec![test_image.clone()]);
        let json_data = serde_json::to_string_pretty(&expected_manifest)
            .expect("Failed to serialize test manifest");
        fs::write(&manifest_path, json_data).expect("Failed to write test manifest.json");
        let loaded_manifest = service.load_manifest();
        assert_eq!(
            loaded_manifest, expected_manifest,
            "Loaded manifest should match the written one"
        );
    }

    #[test]
    fn test_load_manifest_corrupt_json_returns_default() {
        let env = setup_persistence_test_env();
        let service = env.persistence_service;
        let manifest_path = env.data_dir_path.join("manifest.json");
        fs::write(&manifest_path, "{corrupt_json_data:}")
            .expect("Failed to write corrupt manifest.json");
        let manifest = service.load_manifest();
        assert_eq!(
            manifest,
            ImageManifest::default(),
            "Should return default manifest for corrupt JSON"
        );
    }

    #[test]
    fn test_save_manifest_creates_file_and_content_matches() {
        let env = setup_persistence_test_env();
        let service = env.persistence_service;
        let manifest_path = env.data_dir_path.join("manifest.json");
        let mut manifest_to_save = ImageManifest::default();
        let test_map_name = "Saved Map".to_string();
        let test_map_meta = MapMeta {
            last_accessed: std::time::SystemTime::now(),
        };
        manifest_to_save
            .maps
            .insert(test_map_name.clone(), test_map_meta.clone());
        let test_image = ImageMeta {
            filename: "saved_image.jpg".to_string(),
            map: test_map_name.clone(),
            nade_type: NadeType::default(),
            notes: "Saved notes".to_string(),
            position: "B Site".to_string(),
        };
        manifest_to_save
            .images
            .insert(test_map_name.clone(), vec![test_image.clone()]);
        let save_result = service.save_manifest(&manifest_to_save);
        assert!(
            save_result.is_ok(),
            "save_manifest should return Ok. Error: {:?}",
            save_result.err()
        );
        assert!(
            manifest_path.exists(),
            "manifest.json should be created by save_manifest at {:?}",
            manifest_path
        );
        let loaded_manifest_after_save = service.load_manifest();
        assert_eq!(
            loaded_manifest_after_save, manifest_to_save,
            "Loaded manifest should match the saved one"
        );
    }

    #[test]
    fn test_copy_image_to_data_success() {
        let env = setup_persistence_test_env();
        let service = env.persistence_service;
        let thumbnail_service_arc = env.mock_thumbnail_service;

        let source_file_temp =
            create_dummy_source_file().expect("Failed to create dummy source file");
        let source_file_path = source_file_temp.path();
        let map_name = "test_map_copy_success";
        {
            let mock_ts = thumbnail_service_arc.lock().unwrap();
            *mock_ts.generate_should_fail.lock().unwrap() = false;
        }
        let result = service.copy_image_to_data(
            source_file_path,
            map_name,
            NadeType::default(),
            String::new(),
            String::new(),
            &(thumbnail_service_arc.clone() as Arc<Mutex<dyn ThumbnailServiceTrait>>),
        );
        assert!(result.is_ok(), "copy_image_to_data failed: {:?}", result);
        let (dest_path, unique_filename) = result.unwrap();
        assert!(
            dest_path.exists(),
            "Destination file should exist at {:?}",
            dest_path
        );
        let expected_dest_path = env.data_dir_path.join(map_name).join(&unique_filename);
        assert_eq!(dest_path, expected_dest_path, "Destination path mismatch");
        let manifest = service.load_manifest();
        assert!(
            manifest.images.get(map_name).is_some(),
            "Map should exist in manifest"
        );
        assert_eq!(
            manifest.images.get(map_name).unwrap().len(),
            1,
            "Should be one image in manifest for map"
        );
        assert_eq!(
            manifest.images.get(map_name).unwrap()[0].filename,
            unique_filename,
            "Filename in manifest mismatch"
        );
        assert!(
            manifest.maps.get(map_name).is_some(),
            "Map metadata should exist in manifest"
        );
    }

    #[test]
    fn test_copy_image_to_data_thumbnail_generation_fails() {
        let env = setup_persistence_test_env();
        let service = env.persistence_service;
        let thumbnail_service_arc = env.mock_thumbnail_service;

        let source_file_temp =
            create_dummy_source_file().expect("Failed to create dummy source file");
        let source_file_path = source_file_temp.path();
        let map_name = "test_map_thumb_fail";
        {
            let mock_ts = thumbnail_service_arc.lock().unwrap();
            *mock_ts.generate_should_fail.lock().unwrap() = true;
            let specific_error = ThumbnailServiceError::ImageSave(
                PathBuf::from("dummy_path_save_fail.webp"),
                SerializableImageError::from(&image::ImageError::Encoding(
                    image::error::EncodingError::new(
                        image::error::ImageFormatHint::Exact(image::ImageFormat::WebP),
                        "Mock thumbnail save error from tests_common".to_string(),
                    ),
                )),
            );
            *mock_ts.generation_error_type.lock().unwrap() = Some(specific_error);
        }
        let result = service.copy_image_to_data(
            source_file_path,
            map_name,
            NadeType::default(),
            String::new(),
            String::new(),
            &(thumbnail_service_arc.clone() as Arc<Mutex<dyn ThumbnailServiceTrait>>),
        );
        assert!(result.is_err(), "Expected copy_image_to_data to fail");
        match result.err().unwrap() {
            PersistenceServiceError::ThumbnailGenerationFailed(ts_error) => match ts_error {
                ThumbnailServiceError::ImageSave(path, img_err) => {
                    assert_eq!(path, PathBuf::from("dummy_path_save_fail.webp"));
                    assert!(
                        img_err
                            .to_string()
                            .contains("Mock thumbnail save error from tests_common")
                    );
                }
                _ => panic!("Unexpected ThumbnailServiceError variant: {:?}", ts_error),
            },
            other_err => panic!("Expected ThumbnailGenerationFailed, got {:?}", other_err),
        }
        let map_dir = env.data_dir_path.join(map_name);
        assert!(
            !map_dir.exists() || fs::read_dir(&map_dir).unwrap().next().is_none(),
            "Map directory {:?} should be empty or not exist if copy failed early",
            map_dir
        );
        let manifest = service.load_manifest();
        assert!(
            manifest.images.get(map_name).is_none()
                || manifest.images.get(map_name).unwrap().is_empty(),
            "No image should be added to manifest if copy failed"
        );
    }

    #[test]
    fn test_copy_image_to_data_source_not_found() {
        let env = setup_persistence_test_env();
        let service = env.persistence_service;
        let thumbnail_service_arc = env.mock_thumbnail_service;

        let non_existent_path = PathBuf::from("path/to/non_existent_image.jpg");
        let map_name = "test_map_src_not_found";
        let result = service.copy_image_to_data(
            &non_existent_path,
            map_name,
            NadeType::default(),
            String::new(),
            String::new(),
            &(thumbnail_service_arc.clone() as Arc<Mutex<dyn ThumbnailServiceTrait>>),
        );
        assert!(
            result.is_err(),
            "Expected copy to fail for non-existent source"
        );
        match result.err().unwrap() {
            PersistenceServiceError::IoError(io_err) => {
                assert_eq!(io_err.kind(), std::io::ErrorKind::NotFound);
            }
            other_err => panic!("Expected IoError(NotFound), got {:?}", other_err),
        }
    }

    #[test]
    fn test_copy_image_to_data_invalid_map_name_empty() {
        let env = setup_persistence_test_env();
        let service = env.persistence_service;
        let thumbnail_service_arc = env.mock_thumbnail_service;

        let source_file_temp =
            create_dummy_source_file().expect("Failed to create dummy source file");
        let source_file_path = source_file_temp.path();
        let map_name_empty = ""; // Invalid map name
        let result = service.copy_image_to_data(
            source_file_path,
            map_name_empty,
            NadeType::default(),
            String::new(),
            String::new(),
            &(thumbnail_service_arc.clone() as Arc<Mutex<dyn ThumbnailServiceTrait>>),
        );
        assert!(result.is_err(), "Expected copy to fail for empty map name");
        match result.err().unwrap() {
            PersistenceServiceError::InvalidInput(msg) => {
                assert!(msg.contains("Map name cannot be empty"));
            }
            other_err => panic!("Expected InvalidInput, got {:?}", other_err),
        }
    }

    #[test]
    fn test_copy_image_to_data_map_path_is_file() {
        let env = setup_persistence_test_env();
        let service = env.persistence_service;
        let thumbnail_service_arc = env.mock_thumbnail_service;

        let map_name = "map_is_file";
        let map_path_as_file = env.data_dir_path.join(map_name);
        // Create a file where a directory is expected
        File::create(&map_path_as_file).expect("Failed to create dummy file for map path");
        let source_file_temp =
            create_dummy_source_file().expect("Failed to create dummy source file");
        let source_file_path = source_file_temp.path();
        let result = service.copy_image_to_data(
            source_file_path,
            map_name,
            NadeType::default(),
            String::new(),
            String::new(),
            &(thumbnail_service_arc.clone() as Arc<Mutex<dyn ThumbnailServiceTrait>>),
        );
        assert!(
            result.is_err(),
            "Expected copy to fail when map path is a file"
        );
        match result.err().unwrap() {
            PersistenceServiceError::IoError(io_err) => {
                // The exact error might vary by OS (e.g., NotADirectory on Unix, some other error on Windows)
                // Checking that it's an IO error is usually sufficient here.
                // For more robustness, one might check for specific kinds if consistent across platforms.
                log::debug!(
                    "Got expected IO error when map path is a file: {:?}",
                    io_err
                );
            }
            other_err => panic!(
                "Expected IoError when map path is a file, got {:?}",
                other_err
            ),
        }
    }

    #[test]
    fn test_delete_image_and_thumbnails_success() {
        let env = setup_persistence_test_env();
        let service = env.persistence_service;
        let thumbnail_service_arc = env.mock_thumbnail_service;

        let source_file_temp =
            create_dummy_source_file().expect("Failed to create dummy source file for delete test");
        let source_file_path = source_file_temp.path();
        let map_name = "test_map_delete_success";
        {
            let mock_ts = thumbnail_service_arc.lock().unwrap();
            *mock_ts.generate_should_fail.lock().unwrap() = false;
            *mock_ts.remove_should_fail.lock().unwrap() = false;
        }
        let (copied_image_path, unique_filename) = service
            .copy_image_to_data(
                source_file_path,
                map_name,
                NadeType::default(),
                String::new(),
                String::new(),
                &(thumbnail_service_arc.clone() as Arc<Mutex<dyn ThumbnailServiceTrait>>),
            )
            .expect("Setup: copy_image_to_data failed for delete test");
        assert!(
            copied_image_path.exists(),
            "Setup: Copied image should exist"
        );
        let manifest_before_delete = service.load_manifest();
        assert_eq!(
            manifest_before_delete
                .images
                .get(map_name)
                .map_or(0, |v| v.len()),
            1,
            "Image should be in manifest before delete"
        );
        let delete_result = service.delete_image_and_thumbnails(
            map_name,
            &unique_filename,
            &(thumbnail_service_arc.clone() as Arc<Mutex<dyn ThumbnailServiceTrait>>),
        );
        assert!(
            delete_result.is_ok(),
            "delete_image_and_thumbnails failed: {:?}",
            delete_result
        );
        assert!(
            !copied_image_path.exists(),
            "Copied image should be deleted"
        );
        let data_map_path = env.data_dir_path.join(map_name);
        let thumb_dir = data_map_path.join(".thumbnails");

        // For full-size WebP, the naming format is just the file stem with .webp extension
        let expected_thumb_path = thumb_dir.join(format!(
            "{}.webp",
            Path::new(&unique_filename)
                .file_stem()
                .unwrap()
                .to_str()
                .unwrap()
        ));
        assert!(
            !expected_thumb_path.exists(),
            "Expected thumbnail {:?} to be deleted",
            expected_thumb_path
        );

        let manifest_after_delete = service.load_manifest();
        assert!(
            manifest_after_delete.images.get(map_name).is_none()
                || manifest_after_delete
                    .images
                    .get(map_name)
                    .unwrap()
                    .is_empty(),
            "Image should be removed from manifest after delete, or map entry removed"
        );

        // If the map became empty, the map entry itself should be removed from the maps collection too
        if manifest_after_delete.images.get(map_name).is_none() {
            assert!(
                manifest_after_delete.maps.get(map_name).is_none(),
                "Map metadata should be removed if map becomes empty"
            );
        }
    }

    #[test]
    fn test_delete_image_and_thumbnails_image_not_found() {
        let env = setup_persistence_test_env();
        let service = env.persistence_service;
        let thumbnail_service_arc = env.mock_thumbnail_service;
        let map_name = "test_map_delete_not_found";
        // Setup: copy an image first to ensure the map exists and manifest might have it.
        let source_file_temp =
            create_dummy_source_file().expect("Failed to create dummy source file for setup");
        let source_file_path = source_file_temp.path();
        let (_copied_image_path, _unique_filename) = service
            .copy_image_to_data(
                source_file_path,
                map_name,
                NadeType::default(),
                String::new(),
                String::new(),
                &(thumbnail_service_arc.clone() as Arc<Mutex<dyn ThumbnailServiceTrait>>),
            )
            .expect("Setup: copy_image_to_data failed for delete test");
        let non_existent_filename = "non_existent_image.jpg";
        let result = service.delete_image_and_thumbnails(
            map_name,
            non_existent_filename,
            &(thumbnail_service_arc.clone() as Arc<Mutex<dyn ThumbnailServiceTrait>>),
        );
        assert!(
            result.is_err(),
            "Expected delete to fail for non-existent image"
        );
        match result.err().unwrap() {
            PersistenceServiceError::IoError(io_err) => {
                assert_eq!(io_err.kind(), std::io::ErrorKind::NotFound);
            }
            other_err => panic!("Expected IoError(NotFound), got {:?}", other_err),
        }
    }

    #[test]
    fn test_delete_image_and_thumbnails_fails_if_thumbnail_service_fails() {
        let env = setup_persistence_test_env();
        let service = env.persistence_service;
        let thumbnail_service_arc = env.mock_thumbnail_service;

        let source_file_temp =
            create_dummy_source_file().expect("Failed to create dummy source file for delete test");
        let source_file_path = source_file_temp.path();
        let map_name = "test_map_delete_thumb_fail";
        {
            let mock_ts = thumbnail_service_arc.lock().unwrap();
            *mock_ts.generate_should_fail.lock().unwrap() = false;
        }
        let (copied_image_path, unique_filename) = service
            .copy_image_to_data(
                source_file_path,
                map_name,
                NadeType::default(),
                String::new(),
                String::new(),
                &(thumbnail_service_arc.clone() as Arc<Mutex<dyn ThumbnailServiceTrait>>),
            )
            .expect("Setup: copy_image_to_data failed for delete test");
        assert!(
            copied_image_path.exists(),
            "Setup: Copied image should exist"
        );
        {
            let mock_ts = thumbnail_service_arc.lock().unwrap();
            *mock_ts.remove_should_fail.lock().unwrap() = true;
            let error_to_set = ThumbnailServiceError::FileRemoval(
                PathBuf::from("mock_path_for_remove_error.webp"), // Dummy path for the error
                SerializableIoError {
                    kind: std::io::ErrorKind::PermissionDenied,
                    message: "Mock remove: Permission denied".to_string(),
                },
            );
            *mock_ts.removal_error_type.lock().unwrap() = Some(error_to_set);
        }
        let delete_result = service.delete_image_and_thumbnails(
            map_name,
            &unique_filename,
            &(thumbnail_service_arc.clone() as Arc<Mutex<dyn ThumbnailServiceTrait>>),
        );
        assert!(
            delete_result.is_err(),
            "delete_image_and_thumbnails should fail when thumbnail service fails. Result: {:?}",
            delete_result
        );
        let actual_err = delete_result.err().unwrap(); // Extract error once
        match actual_err {
            PersistenceServiceError::ThumbnailDeletionFailed(ref ts_error) => {
                // Changed io_err to ts_error for clarity
                match ts_error {
                    // ts_error is &ThumbnailServiceError
                    ThumbnailServiceError::FileRemoval(_path, serializable_err) => {
                        assert_eq!(
                            serializable_err.kind,
                            std::io::ErrorKind::PermissionDenied,
                            "Unexpected IO error kind in SerializableIoError"
                        );
                        assert!(
                            serializable_err
                                .message
                                .contains("Mock remove: Permission denied"),
                            "Error message mismatch. Got: {}",
                            serializable_err.message
                        );
                    }
                    other_ts_err => panic!(
                        "Expected ThumbnailServiceError::FileRemoval, got {:?}",
                        other_ts_err
                    ),
                }
            }
            ref other_err => panic!(
                "Expected PersistenceServiceError::ThumbnailDeletionFailed, but got {:?}. Full error: {:?}",
                other_err, actual_err
            ),
        }
        assert!(
            copied_image_path.exists(),
            "Main image should NOT be deleted if thumbnail service fails. Path: {:?}. Delete result: {:?}",
            copied_image_path,
            actual_err
        );
    }
}
