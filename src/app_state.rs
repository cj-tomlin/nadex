// src/app_state.rs

use crate::persistence::{ImageManifest, ImageMeta, NadeType}; // Removed load_manifest import
use crate::ui::image_grid_view::ThumbnailCache;
use crate::ui::edit_view::EditFormData; // Assuming EditFormData is pub
use crate::app_logic::upload_processor::UploadTask; // Assuming UploadTask is pub
use crate::services::persistence_service::PersistenceService;
use std::path::PathBuf;
use eframe::egui;

// Make sure EditFormData and UploadTask are public in their respective modules.

// #[derive(Debug)] // Default might not be appropriate anymore due to complex initialization. Manually implemented due to TextureHandle.
pub struct AppState {
    // Filtering UI state
    pub selected_nade_type: Option<NadeType>,
    // Upload modal state
    pub show_upload_modal: bool,
    pub upload_modal_file: Option<PathBuf>,
    pub upload_modal_nade_type: NadeType,
    pub upload_modal_notes: String,
    pub upload_modal_position: String,
    pub uploads: Vec<UploadTask>,
    pub current_map: String,
    pub current_map_images: Vec<ImageMeta>,

    // List of available maps
    pub maps: Vec<&'static str>,
    // Map of map name -> Vec of image file names (not full paths)
    pub image_manifest: ImageManifest,
    // For displaying error messages
    pub error_message: Option<String>,
    // App data dir
    pub data_dir: PathBuf,
    // User grid preferences
    pub grid_image_size: f32,
    // Window state (future: persist)
    pub thumbnail_cache: ThumbnailCache,
    pub selected_image_for_detail: Option<ImageMeta>,
    pub detail_view_texture_handle: Option<egui::TextureHandle>,
    pub editing_image_meta: Option<ImageMeta>,
    pub edit_form_data: Option<EditFormData>,
    pub show_delete_confirmation: Option<ImageMeta>,
    pub detail_view_error: Option<String>,

    // Services
    pub persistence_service: PersistenceService,
}

impl AppState {
    pub fn new() -> Self {
        let mut data_dir = dirs::data_local_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        data_dir.push("nadex");
        std::fs::create_dir_all(&data_dir).ok(); // Ensure the directory exists
        // Initialize PersistenceService first, as it might be needed for other setup or loading
        let persistence_service = PersistenceService::new(data_dir.clone())
            .expect("Failed to initialize PersistenceService. Ensure data directory is accessible.");

        let manifest = persistence_service.load_manifest();

        Self {
            selected_nade_type: None,
            uploads: Vec::new(),
            current_map: "de_ancient".to_string(),
            current_map_images: Vec::new(),
            show_upload_modal: false,
            upload_modal_file: None,
            upload_modal_nade_type: NadeType::Smoke,
            upload_modal_notes: String::new(),
            upload_modal_position: String::new(),
            maps: vec![
                "de_ancient",
                "de_anubis",
                "de_cache",
                "de_dust2",
                "de_inferno",
                "de_mirage",
                "de_nuke",
                "de_overpass",
                "de_train",
                "de_vertigo",
            ],
            image_manifest: manifest,
            error_message: None,
            data_dir, // Comes from initialization above
            grid_image_size: 480.0,
            thumbnail_cache: ThumbnailCache::new(),
            selected_image_for_detail: None,
            detail_view_texture_handle: None,
            editing_image_meta: None,
            edit_form_data: None,
            show_delete_confirmation: None,
            detail_view_error: None,
            persistence_service, // Add the initialized service
        }
    }

    pub fn filter_images_for_current_map(&mut self) {
        self.current_map_images = self
            .image_manifest
            .images
            .get(&self.current_map)
            .map_or_else(Vec::new, |images_for_map| {
                let mut sorted_images = images_for_map.clone();
                sorted_images.sort_by(|a, b| a.filename.cmp(&b.filename));
                sorted_images
            });
    }
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("selected_nade_type", &self.selected_nade_type)
            .field("show_upload_modal", &self.show_upload_modal)
            .field("upload_modal_file", &self.upload_modal_file)
            .field("upload_modal_nade_type", &self.upload_modal_nade_type)
            .field("upload_modal_notes", &self.upload_modal_notes)
            .field("upload_modal_position", &self.upload_modal_position)
            .field("uploads", &self.uploads)
            .field("current_map", &self.current_map)
            .field("current_map_images", &self.current_map_images)
            .field("maps", &self.maps)
            .field("image_manifest", &self.image_manifest)
            .field("error_message", &self.error_message)
            .field("data_dir", &self.data_dir)
            .field("grid_image_size", &self.grid_image_size)
            .field("thumbnail_cache", &self.thumbnail_cache)
            .field("selected_image_for_detail", &self.selected_image_for_detail)
            .field("detail_view_texture_handle", &self.detail_view_texture_handle.as_ref().map(|_| "TextureHandle (present)"))
            .field("editing_image_meta", &self.editing_image_meta)
            .field("edit_form_data", &self.edit_form_data)
            .field("show_delete_confirmation", &self.show_delete_confirmation)
            .field("detail_view_error", &self.detail_view_error)
            .field("persistence_service", &self.persistence_service) // Add persistence_service
            .finish()
    }
}

// We might need a Default impl if NadexApp::default() still relies on it for AppState,
// but direct initialization with new() is cleaner given the logic involved.
// If Default is strictly needed later, we can add:
// impl Default for AppState {
//     fn default() -> Self {
//         Self::new()
//     }
// }
