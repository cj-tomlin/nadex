use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use strum_macros::EnumIter;
use chrono::Utc;

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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MapMeta {
    pub last_accessed: SystemTime,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct ImageManifest {
    pub images: HashMap<String, Vec<ImageMeta>>, // map_name -> Vec<ImageMeta>
    pub maps: HashMap<String, MapMeta>,          // map_name -> MapMeta
}

pub fn save_manifest(manifest: &ImageManifest, data_dir: &Path) -> std::io::Result<()> {
    let manifest_path = data_dir.join("manifest.json");
    let json = serde_json::to_string_pretty(manifest).unwrap();
    fs::write(manifest_path, json)
}

pub fn load_manifest(data_dir: &Path) -> ImageManifest {
    let manifest_path = data_dir.join("manifest.json");
    if manifest_path.exists() {
        let json = fs::read_to_string(manifest_path).unwrap_or_default();
        serde_json::from_str(&json).unwrap_or_default()
    } else {
        ImageManifest::default()
    }
}

pub fn ensure_map_dir(data_dir: &Path, map: &str) -> std::io::Result<PathBuf> {
    let map_dir = data_dir.join(map);
    fs::create_dir_all(&map_dir)?;
    Ok(map_dir)
}

pub fn copy_image_to_data(src: &Path, data_dir: &Path, map: &str) -> std::io::Result<(PathBuf, String)> {
    let map_dir = ensure_map_dir(data_dir, map)?;
    
    let original_filename = src.file_name().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid source path"))?;
    let stem = Path::new(original_filename).file_stem().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "Could not extract file stem"))?.to_string_lossy();
    let extension = Path::new(original_filename).extension().map_or_else(|| "", |ext| ext.to_str().unwrap_or(""));

    let timestamp = Utc::now().format("%Y%m%d%H%M%S%3f").to_string(); // YYYYMMDDHHMMSSmmm (milliseconds)
    let unique_filename_str = if extension.is_empty() {
        format!("{}_{}", stem, timestamp)
    } else {
        format!("{}_{}.{}", stem, timestamp, extension)
    };

    let dest = map_dir.join(&unique_filename_str);
    fs::copy(src, &dest)?;
    Ok((dest, unique_filename_str))
}
