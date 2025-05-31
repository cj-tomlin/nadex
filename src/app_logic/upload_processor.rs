// src/app_logic/upload_processor.rs

 // To allow `app: &mut crate::NadexApp`
use crate::persistence::ImageMeta;
use eframe::egui;
use log;
use std::sync::mpsc::TryRecvError;
use std::time::{Instant, SystemTime}; // Added SystemTime

pub const UPLOAD_TIMEOUT_SECONDS: f32 = 30.0;
pub const UPLOAD_NOTIFICATION_DURATION_SECONDS: f32 = 5.0;

#[derive(Debug, PartialEq)]
pub enum UploadStatus {
    InProgress,
    Success,
    Failed(String),
}

// Receiver needs to be public if UploadTask is constructed outside this module,
// but NadexApp::copy_image_to_data_threaded creates it.
// For now, keep UploadTask fields as they are. If NadexApp needs to inspect status
// directly, we might need to make UploadStatus pub(crate) or pub.
// Given UploadTask is a field in NadexApp, it and its fields (like status)
// might need to be public or pub(crate) depending on how it's used in main.rs.
// Let's assume for now that NadexApp will only interact with Vec<UploadTask> via this module.
// However, NadexApp.uploads is Vec<UploadTask>, so UploadTask and UploadStatus need to be visible to main.rs
#[derive(Debug)]
pub struct UploadTask {
    pub map: String, // Made fields public for access from main.rs
    pub rx: std::sync::mpsc::Receiver<Result<ImageMeta, String>>, // Made public
    pub status: UploadStatus, // Made public
    pub finished_time: Option<Instant>, // Made public
    pub start_time: Instant, // Made public
}

impl UploadTask {
    // Helper constructor if needed, though NadexApp currently constructs it directly.
    // For now, direct construction in NadexApp is fine.
}

pub fn process_upload_tasks(app_state: &mut crate::app_state::AppState, ctx: &egui::Context) {
    let mut newly_completed_meta_list: Vec<ImageMeta> = Vec::new();
    let now = Instant::now();

    app_state.uploads.retain_mut(|upload_task| {
        if upload_task.finished_time.is_none() {
            match upload_task.rx.try_recv() {
                Ok(Ok(newly_uploaded_meta)) => {
                    log::info!(
                        "Upload channel received for: \"{}\" to map \"{}\"",
                        newly_uploaded_meta.filename,
                        newly_uploaded_meta.map
                    );
                    newly_completed_meta_list.push(newly_uploaded_meta.clone());
                    upload_task.status = UploadStatus::Success;
                    upload_task.finished_time = Some(now);
                }
                Ok(Err(err_msg)) => {
                    upload_task.status = UploadStatus::Failed(err_msg.clone());
                    upload_task.finished_time = Some(now);
                    log::error!("Upload failed: {}", err_msg);
                    app_state.error_message = Some(err_msg);
                    ctx.request_repaint();
                }
                Err(TryRecvError::Empty) => {
                    if now.duration_since(upload_task.start_time).as_secs_f32() > UPLOAD_TIMEOUT_SECONDS {
                        upload_task.status = UploadStatus::Failed("Upload timed out".to_string());
                        upload_task.finished_time = Some(now);
                        log::warn!("Upload timed out for: {:?}", upload_task.map);
                        ctx.request_repaint();
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    upload_task.status = UploadStatus::Failed("Upload channel disconnected".to_string());
                    upload_task.finished_time = Some(now);
                    log::error!("Upload channel disconnected for: {:?}", upload_task.map);
                    ctx.request_repaint();
                }
            }
        }

        if let Some(finished_time) = upload_task.finished_time {
            let elapsed_since_finish = now.duration_since(finished_time);

            if elapsed_since_finish.as_secs_f32() > UPLOAD_NOTIFICATION_DURATION_SECONDS {
                return false;
            } else {
                let (text_color, bg_color, message) = match &upload_task.status {
                    UploadStatus::Success => (
                        egui::Color32::WHITE,
                        egui::Color32::from_black_alpha(200),
                        format!("Upload to '{}' successful!", upload_task.map),
                    ),
                    UploadStatus::Failed(e) => (
                        egui::Color32::WHITE,
                        egui::Color32::from_black_alpha(200),
                        format!("Upload to '{}' failed: {}.", upload_task.map, e),
                    ),
                    UploadStatus::InProgress => {
                        log::warn!("Notification: Task for '{}' has finished_time but status is InProgress.", upload_task.map);
                        (
                            egui::Color32::LIGHT_BLUE,
                            egui::Color32::from_black_alpha(180),
                            format!("Upload '{}': processing...", upload_task.map),
                        )
                    }
                };

                let notification_frame = egui::Frame::default()
                    .fill(bg_color)
                    .rounding(egui::Rounding::same(8.0))
                    .inner_margin(egui::Margin::same(12.0));

                let area_id = format!("upload_notification_{}_{:?}", upload_task.map, upload_task.start_time);
                egui::Area::new(area_id.into())
                    .anchor(egui::Align2::RIGHT_TOP, [-24.0_f32, 24.0_f32])
                    .show(ctx, |ui| {
                        notification_frame.show(ui, |ui| {
                            ui.label(egui::RichText::new(message).color(text_color));
                        });
                    });
                return true;
            }
        } else {
            return true;
        }
    });

    let mut manifest_updated = false;
    let mut refresh_grid_for_current_map = false;

    if !newly_completed_meta_list.is_empty() {
        for meta in newly_completed_meta_list {
            log::info!(
                "Processing collected uploaded meta for: '{}'",
                meta.filename
            );
            app_state.image_manifest
                .images
                .entry(meta.map.clone())
                .or_default()
                .push(meta.clone());
            app_state.image_manifest
                .maps
                .entry(meta.map.clone())
                .or_insert_with(|| crate::persistence::MapMeta {
                    // Ensure MapMeta is accessible
                    last_accessed: SystemTime::now(),
                })
                .last_accessed = SystemTime::now();
            manifest_updated = true;

            if meta.map == app_state.current_map {
                refresh_grid_for_current_map = true;
            }
        }

        if manifest_updated {
            if let Err(e) = app_state.persistence_service.save_manifest(&app_state.image_manifest) {
                log::error!("Error saving manifest after processing uploads: {}", e);
                app_state.error_message = Some(format!("Failed to save manifest: {}", e));
            } else {
                log::info!("Manifest saved successfully after processing recent uploads.");
            }
        }

        if refresh_grid_for_current_map && manifest_updated {
            log::info!(
                "Grid refresh needed for current map ('{}'), calling filter_images_for_current_map.",
                app_state.current_map
            );
            app_state.filter_images_for_current_map();
            ctx.request_repaint();
        } else if manifest_updated {
            ctx.request_repaint();
        }
    }
}
