use crate::app_actions::AppAction; // For sending actions
use crate::persistence::{ImageManifest, ImageMeta, NadeType};
use crate::services::persistence_service::PersistenceService;
use crate::services::persistence_service::PersistenceServiceError;
#[cfg(test)]
use crate::services::thumbnail_service::{SerializableImageError, SerializableIoError};
use crate::services::thumbnail_service::{ThumbnailServiceError, ThumbnailServiceTrait};
use crate::ui::edit_view::EditFormData; // Ensure EditFormData is in scope
use image::{self, GenericImageView}; // For image dimension validation and GenericImageView trait
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex}; // For sender type

#[derive(Debug)]
pub enum ImageServiceError {
    Persistence(std::io::Error),
    NotFound(String),
    InputError(String),
    Thumbnail(ThumbnailServiceError), // For thumbnail generation/processing errors
    ThumbnailDeletion(ThumbnailServiceError), // New variant for deletion errors
    Other(String),
}

impl std::fmt::Display for ImageServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageServiceError::Persistence(io_err) => write!(f, "Persistence error: {}", io_err),
            ImageServiceError::NotFound(msg) => write!(f, "Not found: {}", msg),
            ImageServiceError::InputError(msg) => write!(f, "Invalid input: {}", msg),
            ImageServiceError::Thumbnail(err) => write!(f, "Thumbnail service error: {}", err),
            ImageServiceError::ThumbnailDeletion(err) => {
                write!(f, "Thumbnail deletion error: {}", err)
            }
            ImageServiceError::Other(msg) => write!(f, "Image service error: {}", msg),
        }
    }
}

impl From<PersistenceServiceError> for ImageServiceError {
    fn from(err: PersistenceServiceError) -> Self {
        match err {
            PersistenceServiceError::IoError(io_err) => ImageServiceError::Persistence(io_err),
            PersistenceServiceError::InvalidInput(msg) => ImageServiceError::InputError(msg),
            PersistenceServiceError::SerializationError(msg) => {
                ImageServiceError::Other(format!("Manifest serialization error: {}", msg))
            }
            PersistenceServiceError::ThumbnailGenerationFailed(thumb_err) => {
                ImageServiceError::Thumbnail(thumb_err)
            }
            PersistenceServiceError::ThumbnailDeletionFailed(io_err) => {
                ImageServiceError::ThumbnailDeletion(io_err)
            }
        }
    }
}

impl From<std::io::Error> for ImageServiceError {
    fn from(err: std::io::Error) -> Self {
        ImageServiceError::Persistence(err)
    }
}

#[derive(Debug)] // Added derive Debug for ImageService
pub struct ImageService {
    persistence_service: Arc<PersistenceService>,
    thumbnail_service: Arc<Mutex<dyn ThumbnailServiceTrait>>, // Use trait for mocking
}

impl ImageService {
    pub fn new(
        persistence_service: Arc<PersistenceService>,
        thumbnail_service: Arc<Mutex<dyn ThumbnailServiceTrait>>, // Use trait for mocking
    ) -> Self {
        Self {
            persistence_service,
            thumbnail_service, // Initialize thumbnail_service
        }
    }

    // pub fn get_image_details(&self, image_id: &str) -> Option<ImageDetails> { None }

    pub fn get_images_for_map_sorted(
        &self,
        image_manifest: &ImageManifest,
        map_name: &str,
    ) -> Vec<ImageMeta> {
        image_manifest
            .images
            .get(map_name)
            .map_or_else(Vec::new, |images_for_map| {
                let mut sorted_images = images_for_map.clone();
                // Sort by order field first, then by filename as fallback
                sorted_images.sort_by(|a, b| {
                    a.order.cmp(&b.order).then_with(|| a.filename.cmp(&b.filename))
                });
                sorted_images
            })
    }

    pub fn update_image_metadata(
        &self,
        manifest: &mut ImageManifest,
        original_image_meta: &ImageMeta,
        form_data: &EditFormData,
    ) -> Result<(), ImageServiceError> {
        let map_name = &original_image_meta.map;

        // Ensure the filename from form_data matches the original, as a sanity check
        // Filename should not be editable through this form, but good to verify.
        if original_image_meta.filename != form_data.filename {
            return Err(ImageServiceError::InputError(format!(
                "Filename mismatch: original '{}', form data '{}'. Cannot update.",
                original_image_meta.filename, form_data.filename
            )));
        }

        if let Some(images_in_map) = manifest.images.get_mut(map_name) {
            if let Some(image_to_update) = images_in_map
                .iter_mut()
                .find(|img| img.filename == original_image_meta.filename)
            // Find by original filename
            {
                image_to_update.nade_type = form_data.nade_type;
                image_to_update.position = form_data.position.clone();
                image_to_update.notes = form_data.notes.clone();

                // After updating in-memory manifest, save it to disk
                self.persistence_service.save_manifest(manifest)?;
                Ok(())
            } else {
                Err(ImageServiceError::NotFound(format!(
                    "Image with filename '{}' not found in map '{}'.",
                    original_image_meta.filename, // Use original filename for error message
                    map_name
                )))
            }
        } else {
            Err(ImageServiceError::NotFound(format!(
                "Map '{}' not found in manifest.",
                map_name
            )))
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upload_image(
        &self,
        original_file_path: &Path,
        map_name: &str,
        nade_type: NadeType,
        position_details: &str,
        throw_instructions: &str,
    ) -> Result<ImageMeta, ImageServiceError> {
        // 1. Validate the image (open and check dimensions)
        let img = image::open(original_file_path).map_err(|e| {
            ImageServiceError::InputError(format!(
                "Failed to open image '{}' for validation: {}",
                original_file_path.display(),
                e
            ))
        })?;
        let dims = img.dimensions();
        // TODO: Make these dimensions configurable if necessary
        const MIN_WIDTH: u32 = 256;
        const MIN_HEIGHT: u32 = 256;
        const MAX_WIDTH: u32 = 8192;
        const MAX_HEIGHT: u32 = 8192;

        if dims.0 < MIN_WIDTH || dims.1 < MIN_HEIGHT {
            return Err(ImageServiceError::InputError(format!(
                "Image dimensions ({}x{}) for '{}' are too small. Minimum required is {}x{}.",
                dims.0,
                dims.1,
                original_file_path.display(),
                MIN_WIDTH,
                MIN_HEIGHT
            )));
        }

        if dims.0 > MAX_WIDTH || dims.1 > MAX_HEIGHT {
            return Err(ImageServiceError::InputError(format!(
                "Image dimensions ({}x{}) for '{}' are too large. Maximum allowed is {}x{}.",
                dims.0,
                dims.1,
                original_file_path.display(),
                MAX_WIDTH,
                MAX_HEIGHT
            )));
        }

        // 2. Copy image to data directory and get unique filename.
        //    This also triggers thumbnail generation via PersistenceService.
        let (_dest_path, unique_filename) = self.persistence_service.copy_image_to_data(
            original_file_path,
            map_name,
            nade_type,                      // Pass through nade_type
            throw_instructions.to_string(), // Pass through as notes
            position_details.to_string(),   // Pass through as position
            &self.thumbnail_service,
        )?;

        // 3. Create ImageMeta
        let new_image_meta = ImageMeta {
            filename: unique_filename,
            map: map_name.to_string(),
            nade_type,
            notes: throw_instructions.to_string(),
            position: position_details.to_string(),
            order: 0, // Will be set properly when added to manifest
        };

        // Manifest update and saving are handled by persistence_service.copy_image_to_data.
        Ok(new_image_meta)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn orchestrate_full_upload_process(
        self: Arc<Self>, // Take Arc<Self> to move into the thread
        file_path: PathBuf,
        map_name: String,
        nade_type: NadeType,
        position: String,
        notes: String,
        initial_manifest: ImageManifest, // Pass the current manifest state
        app_action_sender: mpsc::Sender<AppAction>,
    ) {
        log::info!(
            "ImageService: Orchestrating full upload for map: {}, file: {:?}",
            map_name,
            file_path
        );

        let self_clone_for_save = Arc::clone(&self); // Clone self for the potential second thread
        let app_action_sender_clone_for_save = app_action_sender.clone(); // Clone sender for the potential second thread

        // Spawn the first background thread for image processing.
        std::thread::spawn(move || {
            log::info!(
                "ImageService Orchestration(T1): Starting image processing for {:?}",
                file_path
            );

            let upload_result = self.upload_image(
                &file_path, &map_name, // map_name is borrowed here
                nade_type, &position, &notes,
            );

            match upload_result {
                Ok(new_image_meta) => {
                    log::info!(
                        "ImageService Orchestration(T1): Image processing successful: {:?}",
                        new_image_meta.filename
                    );

                    // Send action to UI to update its in-memory manifest immediately
                    let ui_update_action = AppAction::UploadSucceededBackgroundTask {
                        new_image_meta: new_image_meta.clone(), // Clone for UI action
                        map_name: map_name.clone(),             // Clone map_name for UI action
                    };
                    if let Err(e) = app_action_sender.send(ui_update_action) {
                        log::error!(
                            "ImageService Orchestration(T1): Failed to send UploadSucceededBackgroundTask: {}",
                            e
                        );
                        // Even if this send fails, proceed to try saving the manifest
                    }

                    // Prepare manifest for saving
                    let manifest_for_saving =
                        initial_manifest.clone_and_add(new_image_meta, &map_name);

                    log::info!("ImageService Orchestration(T1): Triggering manifest save.");
                    // Call save_manifest_async (which spawns its own thread)
                    self_clone_for_save.save_manifest_async(
                        manifest_for_saving,
                        app_action_sender_clone_for_save, // Use the cloned sender
                    );
                }
                Err(e) => {
                    log::error!(
                        "ImageService Orchestration(T1): Image processing failed: {}",
                        e
                    );
                    let fail_action = AppAction::UploadFailed {
                        error_message: Some(format!(
                            "Image processing failed (ImageService): {}",
                            e
                        )),
                    };
                    if let Err(send_err) = app_action_sender.send(fail_action) {
                        log::error!(
                            "ImageService Orchestration(T1): Failed to send UploadFailed action: {}",
                            send_err
                        );
                    }
                }
            }
        });
    }

    pub fn save_manifest_async(
        self: Arc<Self>,                 // Take Arc<Self> to move into the thread
        manifest_to_save: ImageManifest, // Pass the manifest by value (it's cloned by caller)
        app_action_sender: mpsc::Sender<AppAction>,
    ) {
        log::info!("ImageService: Queuing background manifest save.");
        // self (Arc<ImageService>) is moved into the thread.
        // persistence_service is accessed via self.
        std::thread::spawn(move || {
            log::info!("ImageService Background(2): Starting manifest save.");
            let save_result = self.persistence_service.save_manifest(&manifest_to_save);

            let manifest_completion_action = match save_result {
                Ok(_) => {
                    log::info!("ImageService Background(2): Manifest save successful.");
                    AppAction::ManifestSaveCompleted {
                        success: true,
                        error_message: None, // Corrected: No error message on success
                    }
                }
                Err(e) => {
                    log::error!("ImageService Background(2): Manifest save failed: {}", e);
                    AppAction::ManifestSaveCompleted {
                        success: false,
                        error_message: Some(format!(
                            "Failed to save manifest (ImageService): {}",
                            e
                        )),
                    }
                }
            };

            if let Err(e) = app_action_sender.send(manifest_completion_action) {
                log::error!(
                    "ImageService Background(2): Failed to send manifest save completion action: {}",
                    e
                );
            }
        });
    }

    pub fn delete_image(
        &self,
        image_to_delete: &ImageMeta,
        manifest: &mut ImageManifest,
    ) -> Result<(), ImageServiceError> {
        // 1. Delete image file and its thumbnails from disk
        self.persistence_service.delete_image_and_thumbnails(
            &image_to_delete.map,      // This field holds the actual map name
            &image_to_delete.filename, // This field holds the actual image filename
            &self.thumbnail_service,
        )?;

        // 2. Remove ImageMeta from the manifest
        if let Some(images_in_map) = manifest.images.get_mut(&image_to_delete.map) {
            images_in_map.retain(|meta| meta.filename != image_to_delete.filename);
            // Note: We are not removing the map from manifest.maps even if images_in_map becomes empty.
            // This is consistent with previous direct deletion logic.
        } else {
            // This case should ideally not happen if image_to_delete was valid and came from the manifest.
            // However, good to log if it does.
            log::warn!(
                "Attempted to delete image from map '{}' which was not found in manifest. Filename: {}",
                image_to_delete.map,
                image_to_delete.filename
            );
            // Optionally, return an error here if strict consistency is required, e.g.:
            // return Err(ImageServiceError::NotFound(format!(
            //     "Map '{}' not found in manifest during delete operation for file '{}'.",
            //     image_to_delete.map, image_to_delete.filename
            // )));
        }

        // 3. Save the updated manifest
        self.persistence_service.save_manifest(manifest)?;

        Ok(())
    }
}

// Placeholder for a more detailed image struct if needed
// #[derive(Debug)]
// pub struct ImageDetails {
//     pub meta: crate::persistence::ImageMeta,
//     // pub full_path: PathBuf,
//     // pub thumbnail_paths: HashMap<u32, PathBuf>,
// }
#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::NadeType;
    use crate::services::thumbnail_service::ThumbnailServiceError;
    #[cfg(test)]
    use crate::tests_common::{create_dummy_image_file, setup_test_environment};
    use std::fs;
    use std::path::Path;
    use std::time::SystemTime;

    // Test for successful image upload
    #[test]
    fn test_upload_image_success() {
        let env = setup_test_environment();
        // ... (rest of the code remains the same)
        // image_service, persistence_service, mock_thumbnail_service, data_dir_path, and temp_dir are from env

        let map_name = "test_map_upload";
        let nade_type = NadeType::Smoke;
        let position_details = "Site A";
        let throw_instructions = "Align with box, aim at sky, run throw.";

        // Create the map directory and thumbnails directory
        let map_dir = env.data_dir_path.join(map_name);
        let thumb_dir = map_dir.join(".thumbnails");
        std::fs::create_dir_all(&map_dir).expect("Failed to create map directory");
        std::fs::create_dir_all(&thumb_dir).expect("Failed to create thumbnails directory");

        // Use env.temp_dir.path() for temporary source files not part of the app's data structure
        let source_image_dir = env.temp_dir.path().join("source_files");
        fs::create_dir_all(&source_image_dir).expect("Failed to create source_image_dir");
        let original_file_path =
            create_dummy_image_file(&source_image_dir, "test_upload_img.png", 1920, 1440);

        let result = env.image_service.upload_image(
            &original_file_path,
            map_name,
            nade_type,
            position_details,
            throw_instructions,
        );

        assert!(result.is_ok(), "upload_image failed: {:?}", result.err());
        let image_meta = result.unwrap();

        assert_eq!(image_meta.map, map_name);
        assert_eq!(image_meta.nade_type, nade_type);
        assert_eq!(image_meta.position, position_details);
        assert_eq!(image_meta.notes, throw_instructions);
        assert!(!image_meta.filename.is_empty());

        // Use env.data_dir_path for paths within the application's data structure
        let expected_image_path_in_data =
            env.data_dir_path.join(map_name).join(&image_meta.filename);
        assert!(
            expected_image_path_in_data.exists(),
            "Uploaded image file does not exist in data directory: {:?}",
            expected_image_path_in_data
        );

        let expected_thumb_dir = env.data_dir_path.join(map_name).join(".thumbnails");
        assert!(
            expected_thumb_dir.exists(),
            ".thumbnails directory was not created: {:?}",
            expected_thumb_dir
        );

        // Check if a full-size WebP file was created by the mock
        let webp_file_path = expected_thumb_dir.join(format!(
            "{}.webp",
            Path::new(&image_meta.filename)
                .file_stem()
                .unwrap()
                .to_str()
                .unwrap()
        ));
        assert!(
            webp_file_path.exists(),
            "Mock full-size WebP file {:?} does not exist. Mock created paths: {:?}",
            webp_file_path,
            env.mock_thumbnail_service
                .lock()
                .unwrap()
                .created_thumbnail_paths
                .lock()
                .unwrap()
        );
    }

    #[test]
    fn test_delete_image_success() {
        let env = setup_test_environment();
        // image_service, persistence_service, mock_thumbnail_service, data_dir_path, and temp_dir are from env

        let map_name = "test_map_delete";
        let nade_type = NadeType::Smoke;
        let position_details = "Site A";
        let throw_instructions = "Align with box, aim at sky, run throw.";

        // Create the map directory and thumbnails directory
        let map_dir = env.data_dir_path.join(map_name);
        let thumb_dir = map_dir.join(".thumbnails");
        std::fs::create_dir_all(&map_dir).expect("Failed to create map directory");
        std::fs::create_dir_all(&thumb_dir).expect("Failed to create thumbnails directory");

        // Use env.temp_dir.path() for temporary source files not part of the app's data structure
        let source_image_dir = env.temp_dir.path().join("source_files");
        fs::create_dir_all(&source_image_dir).expect("Failed to create source_image_dir");
        let original_file_path =
            create_dummy_image_file(&source_image_dir, "test_delete_img.png", 1920, 1440);

        let upload_result = env.image_service.upload_image(
            &original_file_path,
            map_name,
            nade_type,
            position_details,
            throw_instructions,
        );
        assert!(
            upload_result.is_ok(),
            "Setup for delete_image: upload_image failed: {:?}",
            upload_result.err()
        );
        let image_to_delete_meta = upload_result.unwrap();

        // Create and save an initial manifest
        let mut manifest = ImageManifest::default(); // Use default for a clean start
        manifest = manifest.clone_and_add(image_to_delete_meta.clone(), &image_to_delete_meta.map);
        env.persistence_service
            .save_manifest(&manifest)
            .expect("Failed to save initial manifest for delete test");

        // Verify files exist before deletion
        let uploaded_image_path_in_data = env
            .data_dir_path
            .join(map_name)
            .join(&image_to_delete_meta.filename);
        assert!(
            uploaded_image_path_in_data.exists(),
            "Uploaded image file for deletion {:?} does not exist before delete call",
            uploaded_image_path_in_data
        );
        let thumb_dir_for_delete = env.data_dir_path.join(map_name).join(".thumbnails");
        let expected_thumb_path_before_delete = thumb_dir_for_delete.join(format!(
            "{}.webp",
            Path::new(&image_to_delete_meta.filename)
                .file_stem()
                .unwrap()
                .to_str()
                .unwrap()
        ));
        assert!(
            expected_thumb_path_before_delete.exists(),
            "Expected thumbnail file {:?} does not exist before delete call. Mock created paths: {:?}",
            expected_thumb_path_before_delete,
            env.mock_thumbnail_service
                .lock()
                .unwrap()
                .created_thumbnail_paths
                .lock()
                .unwrap()
        );

        // Action: Call delete_image
        let delete_result = env
            .image_service
            .delete_image(&image_to_delete_meta, &mut manifest);

        // Assertions
        assert!(
            delete_result.is_ok(),
            "delete_image failed: {:?}",
            delete_result.err()
        );

        assert!(
            !uploaded_image_path_in_data.exists(),
            "Original image file {:?} was not deleted",
            uploaded_image_path_in_data
        );
        assert!(
            !expected_thumb_path_before_delete.exists(),
            "Thumbnail file {:?} was not deleted. Mock created paths after delete: {:?}",
            expected_thumb_path_before_delete,
            env.mock_thumbnail_service
                .lock()
                .unwrap()
                .created_thumbnail_paths
                .lock()
                .unwrap()
        );

        assert!(manifest.images.get(map_name).map_or(true, |v| {
            !v.iter()
                .any(|im| im.filename == image_to_delete_meta.filename)
        }));

        let reloaded_manifest: ImageManifest = env.persistence_service.load_manifest();
        assert!(reloaded_manifest.images.get(map_name).map_or(true, |v| {
            !v.iter()
                .any(|im| im.filename == image_to_delete_meta.filename)
        }));
    }

    #[test]
    fn test_upload_image_invalid_dimensions() {
        let env = setup_test_environment();

        let map_name = "test_map_invalid_dims";
        let source_image_dir = env.temp_dir.path().join("source_files_invalid_dims");
        fs::create_dir_all(&source_image_dir).expect("Failed to create source_image_dir");

        // Create an image with dimensions too small
        let small_image_path =
            create_dummy_image_file(&source_image_dir, "small_img.png", 100, 100); // Invalid

        let result_small = env.image_service.upload_image(
            &small_image_path,
            map_name,
            NadeType::Smoke,
            "Site B",
            "Too small.",
        );
        assert!(result_small.is_err());
        match result_small.err().unwrap() {
            ImageServiceError::InputError(msg) => {
                let sub1 = "Image dimensions (100x100)";
                let sub2 = "are too small";
                let contains_sub1 = msg.contains(sub1);
                let contains_sub2 = msg.contains(sub2);
                assert!(
                    contains_sub1 && contains_sub2,
                    "InputError message check failed for 'too small'. Expected to contain '{}' (found: {}) AND '{}' (found: {}). Actual full message: '{}'",
                    sub1,
                    contains_sub1,
                    sub2,
                    contains_sub2,
                    msg
                );
            }
            e => panic!("Expected InputError for small image, got {:?}", e),
        }

        // Create an image with dimensions too large
        let large_image_path =
            create_dummy_image_file(&source_image_dir, "large_img.png", 8193, 256); // Invalid, width > MAX_WIDTH
        let result_large = env.image_service.upload_image(
            &large_image_path,
            map_name,
            NadeType::Smoke,
            "Site C",
            "Too large.",
        );
        assert!(result_large.is_err());
        match result_large.err().unwrap() {
            ImageServiceError::InputError(msg) => {
                let sub1_large = "Image dimensions (8193x256)";
                let sub2_large = "are too large";
                let contains_sub1_large = msg.contains(sub1_large);
                let contains_sub2_large = msg.contains(sub2_large);
                assert!(
                    contains_sub1_large && contains_sub2_large,
                    "InputError message check failed for 'too large'. Expected to contain '{}' (found: {}) AND '{}' (found: {}). Actual full message: '{}'",
                    sub1_large,
                    contains_sub1_large,
                    sub2_large,
                    contains_sub2_large,
                    msg
                );
            }
            e => panic!("Expected InputError for large image, got {:?}", e),
        }
    }

    #[test]
    fn test_upload_image_thumbnail_error() {
        let env = setup_test_environment();

        // Configure the mock from the environment to fail thumbnail generation
        {
            let mock_service_locked = env.mock_thumbnail_service.lock().unwrap();
            *mock_service_locked.generate_should_fail.lock().unwrap() = true;
            *mock_service_locked.generation_error_type.lock().unwrap() =
                Some(ThumbnailServiceError::ImageSave(
                    PathBuf::from("mock_thumb_save_path.jpg"),
                    SerializableImageError {
                        message: "Simulated thumbnail generation error via ImageSave".to_string(),
                    },
                ));
        }

        let map_name = "test_map_thumb_fail";
        let source_image_dir = env.temp_dir.path().join("source_files_thumb_fail");
        fs::create_dir_all(&source_image_dir).expect("Failed to create source_image_dir");
        let original_file_path =
            create_dummy_image_file(&source_image_dir, "thumb_fail_img.png", 1920, 1440);

        let result = env.image_service.upload_image(
            &original_file_path,
            map_name,
            NadeType::Smoke,
            "Position",
            "Instructions",
        );

        assert!(result.is_err());
        match result.err().unwrap() {
            ImageServiceError::Thumbnail(ThumbnailServiceError::ImageSave(ref path, ref err)) => {
                assert!(path.to_string_lossy().contains("mock_thumb_save_path.jpg"));
                assert!(
                    err.message
                        .contains("Simulated thumbnail generation error via ImageSave"),
                    "Unexpected error message: {}",
                    err.message
                );
            }
            e => panic!(
                "Expected ImageServiceError::ThumbnailServiceError(GenerationFailed), got {:?}",
                e
            ),
        }

        // Verify that the original image was NOT saved if thumbnail generation failed.
        // The actual filename in data_dir might be different due to timestamping.
        // We check if the map-specific directory is empty or doesn't contain the original filename stem.
        let map_data_dir = env.data_dir_path.join(map_name);
        let original_filename_stem = original_file_path.file_stem().unwrap().to_str().unwrap();
        let original_extension = original_file_path
            .extension()
            .unwrap_or_default()
            .to_str()
            .unwrap_or_default();

        let mut image_found_in_data_dir = false;
        if map_data_dir.exists() {
            if let Ok(entries) = fs::read_dir(&map_data_dir) {
                for entry in entries.flatten() {
                    let entry_name = entry.file_name().to_string_lossy().into_owned();
                    if entry_name.starts_with(original_filename_stem)
                        && entry_name.ends_with(original_extension)
                    {
                        // Check if it's not a thumbnail directory
                        if !entry_name.contains(".thumbnails") {
                            image_found_in_data_dir = true;
                            break;
                        }
                    }
                }
            }
        }
        assert!(
            !image_found_in_data_dir,
            "Original image file appears to exist in {:?} (or a file starting with '{}') despite thumbnail generation failure.",
            map_data_dir, original_filename_stem
        );

        // Also check that no mock thumbnails were 'created' according to the mock's internal state.
        let mock_thumb_paths = env
            .mock_thumbnail_service
            .lock()
            .unwrap()
            .created_thumbnail_paths
            .lock()
            .unwrap()
            .clone();
        assert!(
            mock_thumb_paths.is_empty(),
            "MockThumbnailService has tracked thumbnail paths after a failed generation: {:?}",
            mock_thumb_paths
        );
    }

    #[test]
    fn test_delete_image_not_found_in_manifest() {
        let env = setup_test_environment();
        let image_service = env.image_service;
        // persistence_service is available via env.persistence_service
        // mock_thumbnail_service is available via env.mock_thumbnail_service
        // data_dir_root is env.data_dir_path
        // temp_dir is env.temp_dir

        let map_name = "test_map_not_found";
        let image_filename = "non_existent_image.png";

        // Create an ImageMeta for an image that won't be in the manifest
        let image_meta_not_in_manifest = ImageMeta {
            filename: image_filename.to_string(),
            map: map_name.to_string(),
            nade_type: NadeType::Molotov,
            position: "A Site".to_string(),
            notes: "Default plant molly".to_string(),
            order: 0,
        };

        let mut manifest = ImageManifest {
            images: std::collections::HashMap::new(),
            maps: std::collections::HashMap::new(),
            webp_migration_completed: false,
        }; // Empty manifest
        // Ensure the map exists in the manifest.maps, but no images for it
        manifest.maps.insert(
            map_name.to_string(),
            crate::persistence::MapMeta {
                last_accessed: SystemTime::now(),
            },
        );

        // Action: Call delete_image
        // Current behavior: logs a warning, doesn't remove anything from manifest.images, saves manifest, returns Ok.
        let result = image_service.delete_image(&image_meta_not_in_manifest, &mut manifest);

        // Assertions based on current behavior (no error, manifest unchanged regarding this specific image)
        match result {
            Err(ImageServiceError::Persistence(io_err)) => {
                assert_eq!(
                    io_err.kind(),
                    std::io::ErrorKind::NotFound,
                    "Expected a NotFound IO error, but got {:?}",
                    io_err
                );
            }
            Ok(_) => panic!(
                "delete_image should have failed for a non-existent image, but it succeeded."
            ),
            Err(other_err) => panic!(
                "delete_image failed with an unexpected error type: {:?}",
                other_err
            ),
        }

        // Verify the manifest on disk does not contain the image (it never did)
        let reloaded_manifest = env.persistence_service.load_manifest();
        assert!(
            reloaded_manifest.images.get(map_name).map_or(true, |v| !v
                .iter()
                .any(|im| im.filename == image_meta_not_in_manifest.filename)),
            "ImageMeta for non-existent image should not be in the manifest."
        );

        // If ImageService::delete_image were changed to return Err for this case:
        // assert!(result.is_err());
        // match result.err().unwrap() {
        //     ImageServiceError::NotFound(msg) => {
        //         assert!(msg.contains(map_name) && msg.contains(image_filename));
        //     }
        //     e => panic!("Expected ImageServiceError::NotFound, got {:?}", e),
        // }
    }
    // Ensure the main tests module closing brace is here if it was removed by mistake

    #[test]
    fn test_delete_image_thumbnail_removal_error() {
        let env = setup_test_environment();
        let map_name = "test_map_thumb_delete_fail";

        // 1. Upload an image successfully first
        let source_image_dir = env
            .temp_dir
            .path()
            .join("source_files_thumb_delete_fail_setup");
        fs::create_dir_all(&source_image_dir).expect("Failed to create source_image_dir for setup");
        let original_upload_path =
            create_dummy_image_file(&source_image_dir, "image_for_delete_test.png", 1920, 1440);

        let upload_result = env.image_service.upload_image(
            &original_upload_path,
            map_name,
            NadeType::Smoke,
            "A Site",
            "Standard smoke for A site execute.",
        );
        assert!(
            upload_result.is_ok(),
            "Setup upload failed: {:?}",
            upload_result.err()
        );
        let image_meta_to_delete = upload_result.unwrap();

        let mut manifest = ImageManifest::default();
        manifest = manifest.clone_and_add(image_meta_to_delete.clone(), &image_meta_to_delete.map);
        env.persistence_service
            .save_manifest(&manifest)
            .expect("Failed to save manifest for setup");

        // 2. Configure mock to fail on thumbnail removal
        {
            let mock_service_locked = env.mock_thumbnail_service.lock().unwrap();
            *mock_service_locked.remove_should_fail.lock().unwrap() = true;
            *mock_service_locked.removal_error_type.lock().unwrap() =
                Some(ThumbnailServiceError::FileRemoval(
                    PathBuf::from("mock_thumb_removal_path.jpg"),
                    SerializableIoError {
                        kind: std::io::ErrorKind::Other,
                        message: "Simulated thumbnail removal error via FileRemoval".to_string(),
                    },
                ));
        }

        // 3. Attempt to delete the image
        let delete_result = env
            .image_service
            .delete_image(&image_meta_to_delete, &mut manifest);

        // 4. Assertions
        assert!(
            delete_result.is_err(),
            "Expected delete_image to fail due to thumbnail removal error"
        );
        match delete_result.err().unwrap() {
            ImageServiceError::ThumbnailDeletion(ThumbnailServiceError::FileRemoval(
                ref path,
                ref err,
            )) => {
                assert!(
                    path.to_string_lossy()
                        .contains("mock_thumb_removal_path.jpg")
                );
                assert!(
                    err.message
                        .contains("Simulated thumbnail removal error via FileRemoval"),
                    "Unexpected error message: {}",
                    err.message
                );
            }
            e => panic!(
                "Expected ImageServiceError::ThumbnailServiceError(RemovalFailed), got {:?}",
                e
            ),
        }

        // 5. Verify original image file still exists (assuming transactional failure or early exit)
        let expected_image_path_in_data = env
            .data_dir_path
            .join(map_name)
            .join(&image_meta_to_delete.filename);
        assert!(
            expected_image_path_in_data.exists(),
            "Original image file {:?} should still exist after failed thumbnail deletion, but it does not.",
            expected_image_path_in_data
        );

        // 6. Verify manifest (reloaded from disk) still contains the image
        let reloaded_manifest: ImageManifest = env.persistence_service.load_manifest();
        let image_still_in_manifest = reloaded_manifest.images.get(map_name).map_or(false, |v| {
            v.iter()
                .any(|im| im.filename == image_meta_to_delete.filename)
        });
        assert!(
            image_still_in_manifest,
            "ImageMeta should still be in the manifest on disk after failed thumbnail deletion."
        );

        // 7. Verify mock's created_thumbnail_paths might still contain the path if remove failed before cleanup
        // This depends on the mock's internal logic for remove_file.
        // If remove_file in mock just returns error without removing from its list, then it should be there.
        // If it removes from list then errors, it should be empty.
        // Current MockThumbnailService::remove_file returns error and does NOT remove from created_thumbnail_paths if it's configured to fail.
        let mock_paths_after_failed_remove = env
            .mock_thumbnail_service
            .lock()
            .unwrap()
            .created_thumbnail_paths
            .lock()
            .unwrap()
            .clone();
        let expected_thumb_filename_stem = Path::new(&image_meta_to_delete.filename)
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap();
        let thumbnail_was_tracked = mock_paths_after_failed_remove
            .iter()
            .any(|p| p.to_string_lossy().contains(expected_thumb_filename_stem));
        assert!(
            thumbnail_was_tracked,
            "MockThumbnailService should still track the thumbnail path ({:?}) after a simulated removal failure, but paths are: {:?}",
            expected_thumb_filename_stem, mock_paths_after_failed_remove
        );
    }

    #[test]
    fn test_update_image_metadata_not_found() {
        let env = setup_test_environment();
        let image_service = env.image_service;
        // persistence_service is env.persistence_service
        // mock_thumbnail_service is env.mock_thumbnail_service
        // data_dir_root is env.data_dir_path
        // temp_dir is env.temp_dir

        let map_name = "test_map_update_not_found".to_string();
        let original_filename = "non_existent_for_update.png".to_string();

        let image_meta_not_in_manifest = ImageMeta {
            filename: original_filename.clone(),
            map: map_name.clone(),
            nade_type: NadeType::Smoke,
            position: "Original Position".to_string(),
            notes: "Original Notes".to_string(),
            order: 0,
        };

        let mut manifest = ImageManifest::default();
        // Ensure the map exists, but the image does not within that map.
        manifest.images.insert(map_name.clone(), Vec::new());

        let form_data = EditFormData {
            filename: original_filename.clone(),
            nade_type: NadeType::Molotov,
            position: "Updated Position".to_string(),
            notes: "Updated Notes".to_string(),
        };

        let result = image_service.update_image_metadata(
            &mut manifest,
            &image_meta_not_in_manifest,
            &form_data,
        );

        assert!(result.is_err());
        match result.err().unwrap() {
            ImageServiceError::NotFound(msg) => {
                assert!(msg.contains(&original_filename));
                assert!(msg.contains(&map_name));
            }
            e => panic!("Expected ImageServiceError::NotFound, got {:?}", e),
        }
    }

    #[test]
    fn test_update_image_metadata_success() {
        let env = setup_test_environment();
        let image_service = env.image_service; // Arc<ImageService>
        let persistence_service = env.persistence_service; // Arc<PersistenceService>
        // mock_thumbnail_service is env.mock_thumbnail_service
        // data_dir_path is env.data_dir_path
        // temp_dir is env.temp_dir

        let map_name = "test_map_update_success";
        let original_nade_type = NadeType::Smoke;
        let original_position = "Original Position";
        let original_notes = "Original Notes";

        // 1. Upload an image first to have something to update
        let source_image_dir = env.temp_dir.path().join("source_files_for_update");
        fs::create_dir_all(&source_image_dir)
            .expect("Failed to create source_image_dir for update_meta_success test");
        let original_file_path_for_upload =
            create_dummy_image_file(&source_image_dir, "image_to_be_updated.png", 800, 600);

        let upload_result = image_service.upload_image(
            &original_file_path_for_upload,
            map_name,
            original_nade_type,
            original_position,
            original_notes,
        );
        assert!(
            upload_result.is_ok(),
            "Setup for update_image_metadata_success: upload_image failed: {:?}",
            upload_result.err()
        );
        let image_to_update_meta = upload_result.unwrap();

        // 2. Load the manifest that was saved by upload_image (via persistence_service)
        let mut manifest: ImageManifest = persistence_service.load_manifest();
        assert!(
            manifest.images.get(map_name).map_or(false, |v| v
                .iter()
                .any(|im| im.filename == image_to_update_meta.filename)),
            "Uploaded image not found in manifest before update."
        );

        // 3. Prepare new data for the update
        let updated_nade_type = NadeType::Molotov;
        let updated_position = "Updated Position A";
        let updated_notes = "These are the updated notes for the image.";

        let form_data = EditFormData {
            filename: image_to_update_meta.filename.clone(), // Filename must match
            nade_type: updated_nade_type,
            position: updated_position.to_string(),
            notes: updated_notes.to_string(),
        };

        // 4. Call update_image_metadata
        let update_result = image_service.update_image_metadata(
            &mut manifest, // Pass the loaded manifest
            &image_to_update_meta,
            &form_data,
        );

        // 5. Assertions
        assert!(
            update_result.is_ok(),
            "update_image_metadata failed: {:?}",
            update_result.err()
        );

        // 5a. Verify the in-memory manifest is updated
        let updated_meta_in_memory = manifest.images.get(map_name).and_then(|v| {
            v.iter()
                .find(|im| im.filename == image_to_update_meta.filename)
        });

        assert!(
            updated_meta_in_memory.is_some(),
            "Image not found in in-memory manifest after update."
        );
        let meta_mem = updated_meta_in_memory.unwrap();
        assert_eq!(meta_mem.nade_type, updated_nade_type);
        assert_eq!(meta_mem.position, updated_position);
        assert_eq!(meta_mem.notes, updated_notes);
        assert_eq!(meta_mem.map, map_name); // Should not change
        assert_eq!(meta_mem.filename, image_to_update_meta.filename); // Should not change

        // 5b. Verify the manifest on disk is updated
        // ImageService::update_image_metadata calls self.persistence_service.save_manifest(&manifest)
        let reloaded_manifest: ImageManifest = persistence_service.load_manifest();
        let updated_meta_on_disk = reloaded_manifest.images.get(map_name).and_then(|v| {
            v.iter()
                .find(|im| im.filename == image_to_update_meta.filename)
        });

        assert!(
            updated_meta_on_disk.is_some(),
            "Image not found in reloaded manifest after update."
        );
        let meta_disk = updated_meta_on_disk.unwrap();
        assert_eq!(meta_disk.nade_type, updated_nade_type);
        assert_eq!(meta_disk.position, updated_position);
        assert_eq!(meta_disk.notes, updated_notes);
    }

    #[test]
    fn test_get_images_for_map_sorted_empty_map() {
        let env = setup_test_environment();
        let image_service = env.image_service;
        let map_name = "de_dust2_empty";

        let mut manifest = ImageManifest::default();
        manifest.images.insert(map_name.to_string(), Vec::new()); // Map exists, but no images

        let result = image_service.get_images_for_map_sorted(&manifest, map_name);
        assert!(
            result.is_empty(),
            "Expected empty vector for empty map, got {:?}",
            result
        );
    }

    #[test]
    fn test_get_images_for_map_sorted_map_not_found() {
        let env = setup_test_environment();
        let image_service = env.image_service;
        let manifest = ImageManifest::default(); // Empty manifest

        let result = image_service.get_images_for_map_sorted(&manifest, "de_inferno_non_existent");
        assert!(
            result.is_empty(),
            "Expected empty vector for non-existent map, got {:?}",
            result
        );
    }

    #[test]
    fn test_get_images_for_map_sorted_single_image() {
        let env = setup_test_environment();
        let image_service = env.image_service;
        let map_name = "de_mirage_single";

        let image1_meta = ImageMeta {
            filename: "image_a.png".to_string(),
            map: map_name.to_string(),
            nade_type: NadeType::Smoke,
            position: "A Site".to_string(),
            notes: "Notes for A".to_string(),
            order: 0,
        };

        let mut manifest = ImageManifest::default();
        manifest
            .images
            .insert(map_name.to_string(), vec![image1_meta.clone()]);

        let result = image_service.get_images_for_map_sorted(&manifest, map_name);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].filename, image1_meta.filename);
    }

    #[test]
    fn test_get_images_for_map_sorted_multiple_images_sorted() {
        let env = setup_test_environment();
        let image_service = env.image_service;
        let map_name = "de_overpass_multiple";

        let image_c_meta = ImageMeta {
            filename: "image_c.png".to_string(),
            map: map_name.to_string(),
            nade_type: NadeType::Flash,
            position: "Long".to_string(),
            notes: "Notes for C".to_string(),
            order: 0,
        };
        let image_a_meta = ImageMeta {
            filename: "image_a.png".to_string(),
            map: map_name.to_string(),
            nade_type: NadeType::Smoke,
            position: "Connector".to_string(),
            notes: "Notes for A".to_string(),
            order: 1,
        };
        let image_b_meta = ImageMeta {
            filename: "image_b.png".to_string(),
            map: map_name.to_string(),
            nade_type: NadeType::Molotov,
            position: "Monster".to_string(),
            notes: "Notes for B".to_string(),
            order: 2,
        };

        let mut manifest = ImageManifest::default();
        manifest.images.insert(
            map_name.to_string(),
            vec![
                image_c_meta.clone(),
                image_a_meta.clone(),
                image_b_meta.clone(),
            ],
        );

        let result = image_service.get_images_for_map_sorted(&manifest, map_name);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].filename, "image_a.png");
        assert_eq!(result[1].filename, "image_b.png");
        assert_eq!(result[2].filename, "image_c.png");
    }
} // Closes `mod tests`
