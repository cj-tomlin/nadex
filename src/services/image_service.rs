use std::sync::{Arc, Mutex};
use crate::persistence::{ImageManifest, ImageMeta, NadeType};
use crate::services::persistence_service::PersistenceService;
use crate::services::thumbnail_service::{ThumbnailService, ThumbnailServiceError};
use crate::services::persistence_service::PersistenceServiceError;
use crate::ui::edit_view::EditFormData; // Ensure EditFormData is in scope
use std::path::Path;
use image::{self, GenericImageView}; // For image dimension validation and GenericImageView trait

#[derive(Debug)]
pub enum ImageServiceError {
    Persistence(std::io::Error),
    Thumbnail(ThumbnailServiceError),
    NotFound(String),
    InputError(String),
    Other(String),
}

impl std::fmt::Display for ImageServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageServiceError::Persistence(io_err) => write!(f, "Persistence error: {}", io_err),
            ImageServiceError::Thumbnail(msg) => write!(f, "Thumbnail generation error: {}", msg),
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
            PersistenceServiceError::ThumbnailGenerationFailed(thumb_err) => ImageServiceError::Thumbnail(thumb_err),
            PersistenceServiceError::InvalidInput(msg) => ImageServiceError::InputError(msg),
            PersistenceServiceError::SerializationError(msg) => ImageServiceError::Other(format!("Manifest serialization error: {}", msg)),
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
        thumbnail_service: Arc<Mutex<ThumbnailService>> // Added thumbnail_service param
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
                original_image_meta.filename,
                form_data.filename
            )));
        }

        if let Some(images_in_map) = manifest.images.get_mut(map_name) {
            if let Some(image_to_update) = images_in_map
                .iter_mut()
                .find(|img| img.filename == original_image_meta.filename) // Find by original filename
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
                "Map '{}' not found in manifest.", map_name
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
        let (_dest_path, unique_filename) = self
            .persistence_service
            .copy_image_to_data(original_file_path, map_name, &self.thumbnail_service)?;

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

    pub fn delete_image(
        &self,
        image_to_delete: &ImageMeta,
        manifest: &mut ImageManifest,
    ) -> Result<(), ImageServiceError> {
        // 1. Delete image file and its thumbnails from disk
        self.persistence_service.delete_image_and_thumbnails(
            &image_to_delete.map,      // This field holds the actual map name
            &image_to_delete.filename, // This field holds the actual image filename
            &mut *self.thumbnail_service.lock().map_err(|e| ImageServiceError::Other(format!("Failed to lock thumbnail_service for delete: {}", e)))?,
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
