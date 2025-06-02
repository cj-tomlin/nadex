use crate::app_actions::AppAction; // For sending actions
use crate::persistence::{ImageManifest, ImageMeta, NadeType};
use crate::services::persistence_service::PersistenceService;
use crate::services::persistence_service::PersistenceServiceError;
use crate::services::thumbnail_service::ThumbnailService;
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
    Other(String),
}

impl std::fmt::Display for ImageServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageServiceError::Persistence(io_err) => write!(f, "Persistence error: {}", io_err),
            ImageServiceError::NotFound(msg) => write!(f, "Not found: {}", msg),
            ImageServiceError::InputError(msg) => write!(f, "Invalid input: {}", msg),
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
    thumbnail_service: Arc<Mutex<ThumbnailService>>, // Added thumbnail_service field
}

impl ImageService {
    pub fn new(
        persistence_service: Arc<PersistenceService>,
        thumbnail_service: Arc<Mutex<ThumbnailService>>, // Added thumbnail_service param
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
                sorted_images.sort_by(|a, b| a.filename.cmp(&b.filename));
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
        const EXPECTED_WIDTH: u32 = 1920;
        const EXPECTED_HEIGHT: u32 = 1440;
        if dims.0 != EXPECTED_WIDTH || dims.1 != EXPECTED_HEIGHT {
            return Err(ImageServiceError::InputError(format!(
                "Invalid image dimensions for '{}': got {:?}, expected {}x{}.",
                original_file_path.display(),
                dims,
                EXPECTED_WIDTH,
                EXPECTED_HEIGHT
            )));
        }

        // 2. Copy image to data directory and get unique filename.
        //    This also triggers thumbnail generation via PersistenceService.
        let (_dest_path, unique_filename) = self.persistence_service.copy_image_to_data(
            original_file_path,
            map_name,
            &self.thumbnail_service,
        )?;

        // 3. Create ImageMeta
        let new_image_meta = ImageMeta {
            filename: unique_filename,
            map: map_name.to_string(),
            nade_type,
            notes: throw_instructions.to_string(),
            position: position_details.to_string(),
        };

        // Manifest update and saving are now handled by the caller.
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
            &mut *self.thumbnail_service.lock().map_err(|e| {
                ImageServiceError::Other(format!(
                    "Failed to lock thumbnail_service for delete: {}",
                    e
                ))
            })?,
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
