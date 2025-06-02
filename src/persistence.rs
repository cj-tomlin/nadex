use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::SystemTime;
use strum_macros::EnumIter;

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

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct ImageManifest {
    pub images: HashMap<String, Vec<ImageMeta>>, // map_name -> Vec<ImageMeta>
    pub maps: HashMap<String, MapMeta>,          // map_name -> MapMeta
}

