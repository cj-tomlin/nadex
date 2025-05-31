use std::sync::Arc;

use super::persistence_service::PersistenceService;

#[derive(Debug)]
pub struct ImageService {
    // Potentially, a reference to PersistenceService if needed directly
    // persistence: Arc<PersistenceService>, 
} 

impl ImageService {
    pub fn new(/*persistence: Arc<PersistenceService>*/) -> Self {
        // Self { persistence }
        Self {}
    }

    // Placeholder for future methods
    // pub fn get_image_details(&self, image_id: &str) -> Option<ImageDetails> { None }

    pub fn get_images_for_map_sorted(
        &self,
        image_manifest: &crate::persistence::ImageManifest,
        map_name: &str,
    ) -> Vec<crate::persistence::ImageMeta> {
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
        manifest: &mut crate::persistence::ImageManifest,
        original_image_meta: &crate::persistence::ImageMeta,
        form_data: &crate::ui::edit_view::EditFormData,
    ) -> Result<(), String> {
        let map_name = &original_image_meta.map;

        if let Some(images_in_map) = manifest.images.get_mut(map_name) {
            if let Some(image_to_update) = images_in_map
                .iter_mut()
                .find(|img| img.filename == form_data.filename)
            {
                // Ensure the filename from form_data matches the original, as a sanity check
                // though typically filename is not editable in this form.
                if original_image_meta.filename != form_data.filename {
                    return Err(format!(
                        "Filename mismatch: original '{}', form data '{}'. Cannot update.",
                        original_image_meta.filename,
                        form_data.filename
                    ));
                }

                image_to_update.nade_type = form_data.nade_type;
                image_to_update.position = form_data.position.clone();
                image_to_update.notes = form_data.notes.clone();
                Ok(())
            } else {
                Err(format!(
                    "Image with filename '{}' not found in map '{}'.",
                    form_data.filename,
                    map_name
                ))
            }
        } else {
            Err(format!("Map '{}' not found in manifest.", map_name))
        }
    }
}

// Placeholder for a more detailed image struct if needed
// #[derive(Debug)]
// pub struct ImageDetails {
//     pub meta: crate::persistence::ImageMeta,
//     // pub full_path: PathBuf,
//     // pub thumbnail_paths: HashMap<u32, PathBuf>,
// }
