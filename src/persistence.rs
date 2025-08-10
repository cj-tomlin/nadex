use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::SystemTime;
use strum_macros::EnumIter;

// We've migrated to a single full-size WebP image approach
// No need to re-export ALLOWED_THUMB_SIZES anymore

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default, EnumIter)]
pub enum NadeType {
    #[default]
    Smoke,
    Flash,
    Molotov,
    Grenade,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageMeta {
    pub filename: String,
    pub map: String,
    pub nade_type: NadeType,
    pub notes: String,    // How to throw
    pub position: String, // Where this nade is for (e.g., "A Main Smoke")
    #[serde(default)]
    pub order: usize, // Order position for reordering images
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MapMeta {
    pub last_accessed: SystemTime,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct ImageManifest {
    pub images: HashMap<String, Vec<ImageMeta>>, // map_name -> Vec<ImageMeta>
    pub maps: HashMap<String, MapMeta>,          // map_name -> MapMeta
    #[serde(default)]
    pub webp_migration_completed: bool, // Tracks if the one-time WebP migration has been performed
}

impl ImageManifest {
    pub fn clone_and_add(&self, mut new_image: ImageMeta, map_name: &str) -> Self {
        let mut new_manifest = self.clone();
        let images_for_map = new_manifest.images.entry(map_name.to_string()).or_default();

        // Assign order value based on current number of images for this map
        new_image.order = images_for_map.len();

        images_for_map.push(new_image);
        // Optionally, update map metadata if needed, e.g., last_accessed
        // For now, just adding the image.
        new_manifest
    }

    /// Migrate existing images to have proper order values
    /// This ensures backward compatibility with manifests created before the order field
    pub fn migrate_image_order(&mut self) {
        for (_map_name, images) in self.images.iter_mut() {
            // Check if images need order migration (all have order 0 or inconsistent ordering)
            let needs_migration = images.len() > 1 && 
                (images.iter().all(|img| img.order == 0) || 
                 !Self::has_consistent_ordering(images));
            
            if needs_migration {
                // Sort by filename first to maintain some consistency
                images.sort_by(|a, b| a.filename.cmp(&b.filename));
                
                // Assign sequential order values
                for (idx, image) in images.iter_mut().enumerate() {
                    image.order = idx;
                }
            }
        }
    }

    /// Check if images have consistent ordering (no duplicates, sequential)
    fn has_consistent_ordering(images: &[ImageMeta]) -> bool {
        if images.is_empty() {
            return true;
        }
        
        let mut orders: Vec<usize> = images.iter().map(|img| img.order).collect();
        orders.sort_unstable();
        
        // Check if orders are sequential starting from 0
        orders.iter().enumerate().all(|(idx, &order)| order == idx)
    }
}
