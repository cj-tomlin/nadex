// src/app_actions.rs

use crate::persistence::{NadeType, ImageMeta};
use crate::ui::edit_view::EditFormData; // Added import
use std::path::PathBuf; // Added import

#[derive(Debug, Clone)]
pub enum AppAction {
    // --- Map Actions ---
    SelectMap(String),

    // --- Upload Actions ---
    SubmitUpload {
        file_path: PathBuf,
        map_name: String,
        nade_type: NadeType,
        position: String,
        notes: String,
    },
    SetProcessingUpload(bool),
    UploadSucceededBackgroundTask { // Sent from image upload thread to main thread
        new_image_meta: ImageMeta,
        map_name: String, // map_name is needed to update the manifest correctly
    },
    UploadFailed { // Sent from image upload thread to main thread
        error_message: Option<String>,
    },
    ManifestSaveCompleted { // Sent from manifest save thread to main thread
        success: bool,
        error_message: Option<String>,
    },

    // --- UI Actions (from TopBar, etc.) ---
    SetGridImageSize(f32),
    ShowUploadModal,
    SetNadeFilter(Option<NadeType>),
    ImageGridImageClicked(ImageMeta),

    // --- Detail Modal Actions ---
    DetailModalClose,
    DetailModalRequestEdit(ImageMeta),
    DetailModalRequestDelete(ImageMeta),

    // --- Edit Modal Actions ---
    EditModalSave(EditFormData),
    EditModalCancel,

    // --- Delete Confirmation Modal Actions ---
    DeleteConfirm,
    DeleteCancel,

    // Add other action categories and specific actions as needed
    // Example: Modal Actions, etc.
}
