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
    pub fn clone_and_add(&self, new_image: ImageMeta, map_name: &str) -> Self {
        let mut new_manifest = self.clone();
        new_manifest
            .images
            .entry(map_name.to_string())
            .or_default()
            .push(new_image);
        // Optionally, update map metadata if needed, e.g., last_accessed
        // For now, just adding the image.
        new_manifest
    }
}
