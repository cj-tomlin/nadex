// This file will contain shared test utilities, starting with MockThumbnailService.

use crate::services::thumbnail_service::{
    SerializableImageError, SerializableIoError, ThumbnailServiceError, ThumbnailServiceTrait,
};
// Removed ALLOWED_THUMB_SIZES, ThumbnailLoadJob, ThumbnailLoadResult, ImageMeta, AppState, NadexPath, egui as they are not directly used by this refined mock's trait implementation
// If specific error generation needs ALLOWED_THUMB_SIZES, it can be re-added.
use crate::services::image_service::ImageService;
use crate::services::persistence_service::PersistenceService;
use image::{ImageBuffer, ImageFormat, Rgba};
use std::fmt; // For Debug trait
use std::fs; // For dummy file operations
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use tempfile::{TempDir, tempdir}; // For create_dummy_image_file

pub struct MockThumbnailService {
    pub generate_should_fail: Mutex<bool>,
    pub remove_should_fail: Mutex<bool>,
    pub created_thumbnail_paths: Mutex<Vec<PathBuf>>,
    // Specific error to return for generation, if any. Copied from local mock's style.
    pub generation_error_type: Mutex<Option<ThumbnailServiceError>>,
    // Specific error to return for removal, if any. Copied from local mock's style.
    pub removal_error_type: Mutex<Option<ThumbnailServiceError>>,
}

// Manual Debug impl because Mutex fields don't auto-derive Debug well for assert messages.
impl fmt::Debug for MockThumbnailService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MockThumbnailService")
            .field(
                "generate_should_fail",
                &self.generate_should_fail.lock().unwrap(),
            )
            .field(
                "remove_should_fail",
                &self.remove_should_fail.lock().unwrap(),
            )
            .field(
                "created_thumbnail_paths",
                &self.created_thumbnail_paths.lock().unwrap(),
            )
            .finish()
    }
}

impl MockThumbnailService {
    pub fn new(generate_should_fail: bool, remove_should_fail: bool) -> Self {
        Self {
            generate_should_fail: Mutex::new(generate_should_fail),
            remove_should_fail: Mutex::new(remove_should_fail),
            created_thumbnail_paths: Mutex::new(Vec::new()),
            generation_error_type: Mutex::new(None),
            removal_error_type: Mutex::new(None),
        }
    }

    // Helper to set a specific generation error for the next call
    pub fn set_generation_error(&self, error: Option<ThumbnailServiceError>) {
        *self.generation_error_type.lock().unwrap() = error;
        *self.generate_should_fail.lock().unwrap() =
            self.generation_error_type.lock().unwrap().is_some();
    }

    // Helper to set a specific removal error for the next call
    pub fn set_removal_error(&self, error: Option<ThumbnailServiceError>) {
        *self.removal_error_type.lock().unwrap() = error;
        *self.remove_should_fail.lock().unwrap() =
            self.removal_error_type.lock().unwrap().is_some();
    }
}

impl Default for MockThumbnailService {
    fn default() -> Self {
        Self::new(false, false)
    }
}

impl ThumbnailServiceTrait for MockThumbnailService {
    fn remove_thumbnails_for_image(
        &mut self, // Takes &mut self as per trait
        image_filename: &str,
        image_map_name: &str,
        data_dir: &Path, // Used to construct path for error, and for filtering created_paths
    ) -> Result<(), ThumbnailServiceError> {
        if *self.remove_should_fail.lock().unwrap() {
            if let Some(err) = self.removal_error_type.lock().unwrap().take() {
                return Err(err);
            }
            // Default error if specific one isn't set
            let dummy_path = data_dir
                .join(image_map_name)
                .join(".thumbnails")
                .join("mock_thumb_for_error.webp");
            return Err(ThumbnailServiceError::FileRemoval(
                dummy_path,
                SerializableIoError {
                    kind: std::io::ErrorKind::Other,
                    message: "Mock thumbnail deletion error from tests_common".to_string(),
                },
            ));
        }

        let image_filename_stem_to_match = Path::new(image_filename)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();

        let mut created_paths_guard = self.created_thumbnail_paths.lock().unwrap();

        created_paths_guard.retain(|thumb_path| {
            let mut should_keep = true;
            let mut matches_criteria = false;

            if let Some(path_stem) = thumb_path.file_stem() {
                if path_stem.to_string_lossy().starts_with(&*image_filename_stem_to_match) {
                    if let Some(thumbnails_dir) = thumb_path.parent() { // e.g., /data_dir/map_name/.thumbnails
                        if thumbnails_dir.ends_with(".thumbnails") {
                             if let Some(map_dir) = thumbnails_dir.parent() { // e.g., /data_dir/map_name
                                if map_dir.file_name().map_or(false, |name| name.to_string_lossy() == image_map_name) {
                                    // Further check if map_dir is under data_dir (optional, for robustness)
                                    if map_dir.starts_with(data_dir.join(image_map_name)) {
                                         matches_criteria = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if matches_criteria {
                match std::fs::remove_file(thumb_path) {
                    Ok(_) => {
                        log::debug!("[MockShared] Successfully deleted thumbnail file: {:?}", thumb_path);
                        should_keep = false;
                    }
                    Err(_e) => {
                        // In a mock, if fs::remove_file fails, it might indicate a test setup issue
                        // or that the file wasn't created as expected. We'll let it be kept.
                        // The test asserting deletion should fail if the file still exists.
                        log::warn!("[MockShared] Failed to delete thumbnail file {:?} during mock operation. It might not exist.", thumb_path);
                    }
                }
            }
            should_keep
        });

        Ok(())
    }

    // The deprecated request_thumbnail_generation method has been removed as part of the transition to full-size WebP images

    fn get_cached_texture_info(&self, _key: &str) -> Option<(egui::TextureHandle, (u32, u32))> {
        // Mock implementation: by default, returns None.
        None
    }

    fn has_texture(&self, _key: &str) -> bool {
        // Mock implementation for tests
        false
    }

    fn load_texture_from_file(
        &mut self,
        file_path: &Path,
        _cache_key: &str,
        _ctx: &egui::Context,
    ) -> Result<(), ThumbnailServiceError> {
        // Mock implementation for tests
        if *self.generate_should_fail.lock().unwrap() {
            Err(ThumbnailServiceError::ImageOpen(
                file_path.to_path_buf(),
                SerializableImageError {
                    message: "Mock texture loading error".to_string(),
                },
            ))
        } else {
            // Track the path in created_thumbnail_paths for testing verification
            self.created_thumbnail_paths
                .lock()
                .unwrap()
                .push(file_path.to_path_buf());
            Ok(())
        }
    }

    fn convert_to_full_webp(
        &self,
        original_image_path: &Path,
        output_dir: &Path,
    ) -> Result<PathBuf, ThumbnailServiceError> {
        // Similar logic to generate_thumbnail_file but for full-size WebP conversion
        if *self.generate_should_fail.lock().unwrap() {
            if let Some(err) = self.generation_error_type.lock().unwrap().take() {
                return Err(err);
            }
            // Default error if specific one isn't set
            return Err(ThumbnailServiceError::ImageSave(
                PathBuf::from("dummy_path_save_fail.webp"),
                SerializableImageError::from(&image::ImageError::Encoding(
                    image::error::EncodingError::new(
                        image::error::ImageFormatHint::Exact(image::ImageFormat::WebP),
                        "Mock full WebP conversion error from tests_common".to_string(),
                    ),
                )),
            ));
        }

        let file_stem = original_image_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        let webp_filename = format!("{}.webp", file_stem); // No size suffix for full-size WebP
        let webp_path = output_dir.join(webp_filename);

        // Create output directory if it doesn't exist
        if !output_dir.exists() {
            fs::create_dir_all(output_dir).map_err(|e| {
                ThumbnailServiceError::DirectoryCreation(
                    output_dir.to_path_buf(),
                    SerializableIoError::from(e),
                )
            })?;
        }

        // Create a dummy file
        fs::write(&webp_path, "mock full webp content").map_err(|e| {
            ThumbnailServiceError::ImageSave(
                webp_path.clone(),
                SerializableImageError {
                    message: format!("Mock fs::write failed for full WebP: {}", e),
                },
            )
        })?;

        self.created_thumbnail_paths
            .lock()
            .unwrap()
            .push(webp_path.clone());

        Ok(webp_path)
    }
}

// --- Shared Test Environment Setup ---

/// Holds instances of services and other common items needed for integration-style unit tests.
#[cfg(test)] // Only compile this when running tests
pub struct PersistenceTestEnvironment {
    pub persistence_service: Arc<PersistenceService>,
    pub mock_thumbnail_service: Arc<Mutex<MockThumbnailService>>,
    pub temp_dir: TempDir,
    pub data_dir_path: PathBuf,
}

#[cfg(test)] // Only compile this when running tests
pub fn setup_persistence_test_env() -> PersistenceTestEnvironment {
    let temp_dir = tempdir().expect("Failed to create temp_dir for persistence test environment");
    let data_dir_path = temp_dir.path().to_path_buf();

    let mock_ts = Arc::new(Mutex::new(MockThumbnailService::new(false, false)));

    let ps = Arc::new(
        PersistenceService::new(data_dir_path.clone())
            .expect("Failed to create PersistenceService for persistence test environment"),
    );

    PersistenceTestEnvironment {
        persistence_service: ps,
        mock_thumbnail_service: mock_ts,
        temp_dir,
        data_dir_path,
    }
}

#[cfg(test)] // Only compile this when running tests
pub struct TestServiceEnvironment {
    pub image_service: Arc<ImageService>,
    pub persistence_service: Arc<PersistenceService>,
    pub mock_thumbnail_service: Arc<Mutex<MockThumbnailService>>, // Concrete mock for direct manipulation
    pub temp_dir: TempDir, // Manages the temporary directory for this environment
    pub data_dir_path: PathBuf, // Convenience accessor for the data directory path
}

/// Sets up a common test environment with PersistenceService, MockThumbnailService, and ImageService.
///
/// The `PersistenceService` will use a temporary directory that is cleaned up
/// when the returned `TestServiceEnvironment` (specifically its `temp_dir` field) is dropped.
/// The `MockThumbnailService` is the shared mock, initialized to not fail by default.
/// The `ImageService` is configured to use these mock/test instances.
#[cfg(test)] // Only compile this when running tests
pub fn setup_test_environment() -> TestServiceEnvironment {
    let persistence_env = setup_persistence_test_env();

    // ImageService expects Arc<Mutex<dyn ThumbnailServiceTrait>>.
    // We cast our Arc<Mutex<MockThumbnailService>> from persistence_env to satisfy this.
    let thumbnail_service_trait_obj: Arc<Mutex<dyn ThumbnailServiceTrait>> =
        persistence_env.mock_thumbnail_service.clone();

    let is = Arc::new(ImageService::new(
        persistence_env.persistence_service.clone(),
        thumbnail_service_trait_obj,
    ));

    TestServiceEnvironment {
        image_service: is,
        persistence_service: persistence_env.persistence_service,
        mock_thumbnail_service: persistence_env.mock_thumbnail_service,
        temp_dir: persistence_env.temp_dir,
        data_dir_path: persistence_env.data_dir_path,
    }
}

// Test environment for ThumbnailService tests
#[derive(Debug)]
pub struct ThumbnailTestEnvironment {
    pub temp_dir: TempDir,
    pub source_dir: PathBuf,
    pub output_dir: PathBuf,
}

impl ThumbnailTestEnvironment {
    pub fn source_path(&self, filename: &str) -> PathBuf {
        self.source_dir.join(filename)
    }

    pub fn output_path(&self, filename: &str) -> PathBuf {
        self.output_dir.join(filename)
    }
}

#[cfg(test)] // Only compile this when running tests
pub fn setup_thumbnail_test_env() -> ThumbnailTestEnvironment {
    let temp_dir = TempDir::new().expect("Failed to create temp_dir for thumbnail test");
    let source_dir = temp_dir.path().join("source_files");
    std::fs::create_dir_all(&source_dir)
        .expect("Failed to create source_dir for thumbnail test env");
    let output_dir = temp_dir.path().join("output_thumbnails");
    std::fs::create_dir_all(&output_dir)
        .expect("Failed to create output_dir for thumbnail test env");
    ThumbnailTestEnvironment {
        temp_dir,
        source_dir,
        output_dir,
    }
}

// Helper function to create a dummy PNG image file for testing
// Moved from local test modules (e.g., thumbnail_service::tests, image_service::tests)
#[cfg(test)] // Only compile this when running tests
pub fn create_dummy_image_file(dir: &Path, filename: &str, width: u32, height: u32) -> PathBuf {
    let path = dir.join(filename);
    let img = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_fn(width, height, |x, y| {
        if (x + y) % 2 == 0 {
            Rgba([0, 0, 0, 255]) // Black
        } else {
            Rgba([255, 255, 255, 255]) // White
        }
    });
    img.save_with_format(&path, ImageFormat::Png)
        .expect("Failed to save dummy image in tests_common::create_dummy_image_file");
    path
}
