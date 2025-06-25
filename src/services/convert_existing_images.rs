use crate::services::thumbnail_service::{
    ConcreteThumbnailService, ThumbnailServiceError, ThumbnailServiceTrait,
};
use log::info;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Utility function to convert existing images to full-size WebP
pub fn convert_all_images_to_full_webp(
    app_data_dir: &Path,
    image_manifest: &crate::persistence::ImageManifest,
    thumbnail_service: &Arc<Mutex<ConcreteThumbnailService>>,
) -> Result<usize, String> {
    let mut converted_count = 0;

    info!("Starting conversion of all existing images to full-size WebP...");

    // Iterate through all maps and images
    for (map_name, images) in &image_manifest.images {
        let map_dir = app_data_dir.join(map_name);
        let thumbnails_dir = map_dir.join(".thumbnails");

        if !thumbnails_dir.exists() {
            if let Err(err) = std::fs::create_dir_all(&thumbnails_dir) {
                return Err(format!(
                    "Failed to create thumbnails directory for map {}: {}",
                    map_name, err
                ));
            }
        }

        // Process each image in the map
        for image_meta in images {
            let image_path = map_dir.join(&image_meta.filename);
            if !image_path.exists() {
                info!("Image not found, skipping: {:?}", image_path);
                continue;
            }

            // Construct the expected WebP path
            use crate::services::thumbnail_service::module_construct_thumbnail_path;
            let webp_path = module_construct_thumbnail_path(&image_path, &thumbnails_dir, 0);

            // Only convert if the WebP file doesn't already exist
            if !webp_path.exists() {
                info!("WebP version doesn't exist, converting: {:?}", image_path);
                if let Ok(service) = thumbnail_service.lock() {
                    match service.convert_to_full_webp(&image_path, &thumbnails_dir) {
                        Ok(webp_path) => {
                            converted_count += 1;
                            info!(
                                "Successfully converted to WebP: {:?} → {}",
                                image_path,
                                webp_path.display()
                            );
                        }
                        Err(e) => {
                            // Log details based on error type for better diagnostics
                            match &e {
                                ThumbnailServiceError::ImageOpen(path, err) => {
                                    info!(
                                        "Failed to open image for WebP conversion: {} - Error: {}",
                                        path.display(),
                                        err
                                    );
                                }
                                ThumbnailServiceError::DirectoryCreation(path, err) => {
                                    info!(
                                        "Failed to create directory for WebP conversion: {} - Error: {}",
                                        path.display(),
                                        err
                                    );
                                }
                                ThumbnailServiceError::ImageSave(path, err) => {
                                    info!(
                                        "Failed to save WebP image: {} - Error: {}",
                                        path.display(),
                                        err
                                    );
                                }
                                _ => {
                                    info!(
                                        "Failed to convert to WebP: {:?} - Error: {}",
                                        image_path, e
                                    );
                                }
                            }

                            // Continue with other images despite this failure
                        }
                    }
                } else {
                    return Err("Failed to acquire lock on thumbnail service".to_string());
                }
            } else {
                info!("WebP version already exists, skipping: {:?}", webp_path);
            }
        }
    }

    info!(
        "Conversion complete. Converted {} images to full-size WebP.",
        converted_count
    );
    Ok(converted_count)
}
