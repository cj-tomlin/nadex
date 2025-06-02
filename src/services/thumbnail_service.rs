// src/services/thumbnail_service.rs
use std::collections::{HashMap, HashSet, VecDeque};
use std::error::Error as StdError; // Alias for clarity
use std::fmt;

use rayon::spawn;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use egui; // For Ui, TextureHandle, ColorImage, TextureOptions
use image;
use log;

const MAX_THUMB_CACHE_SIZE: usize = 18; // Example value

/// Allowed thumbnail widths (pixels).
pub(crate) const ALLOWED_THUMB_SIZES: [u32; 3] = [960, 720, 480]; // Example values

// --- Structs for Asynchronous Thumbnail Loading ---
#[derive(Debug)]
pub struct ThumbnailLoadJob {
    pub image_file_path: PathBuf, // Path to the original image
    pub thumb_storage_dir: PathBuf,
    pub target_size: u32,
}

#[derive(Debug)]
pub struct ThumbnailLoadResult {
    pub thumb_path_key: String, // Key for the cache (usually thumb_path.to_string_lossy())
    pub color_image: Option<egui::ColorImage>,
    pub dimensions: Option<(u32, u32)>,
    pub error: Option<String>,
}

/// Constructs the canonical path for a thumbnail file.
pub(crate) fn module_construct_thumbnail_path(
    img_path: &Path,
    thumb_dir: &Path,
    size: u32,
) -> PathBuf {
    let stem = img_path.file_stem().unwrap_or_default().to_string_lossy();
    thumb_dir.join(format!("{}_{}.webp", stem, size))
}

#[derive(Debug)]
pub enum ThumbnailServiceError {
    DirectoryCreationFailed(PathBuf, std::io::Error),
    ImageOpenFailed(PathBuf, image::ImageError),
    ImageSaveFailed(PathBuf, image::ImageError),
}

impl fmt::Display for ThumbnailServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ThumbnailServiceError::DirectoryCreationFailed(path, err) => write!(
                f,
                "Failed to create thumbnail directory {}: {}",
                path.display(),
                err
            ),
            ThumbnailServiceError::ImageOpenFailed(path, err) => write!(
                f,
                "Failed to open image {} for thumbnail generation: {}",
                path.display(),
                err
            ),
            ThumbnailServiceError::ImageSaveFailed(path, err) => {
                write!(f, "Failed to save thumbnail {}: {}", path.display(), err)
            }
        }
    }
}
impl StdError for ThumbnailServiceError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            ThumbnailServiceError::DirectoryCreationFailed(_, err) => Some(err),
            ThumbnailServiceError::ImageOpenFailed(_, err) => Some(err),
            ThumbnailServiceError::ImageSaveFailed(_, err) => Some(err),
        }
    }
}

// --- ThumbnailCache struct and impl ---
pub struct ThumbnailCache {
    textures: HashMap<String, (egui::TextureHandle, (u32, u32))>, // Key: thumb_path_str
    order: VecDeque<String>,                                      // For LRU: stores thumb_path_str
    loading_in_progress: HashSet<String>, // Tracks thumb_path_str of items being loaded
}

impl ThumbnailCache {
    pub fn new() -> Self {
        Self {
            textures: HashMap::new(),
            order: VecDeque::with_capacity(MAX_THUMB_CACHE_SIZE),
            loading_in_progress: HashSet::new(),
        }
    }

    fn prune(&mut self) {
        while self.order.len() > MAX_THUMB_CACHE_SIZE {
            if let Some(oldest_key) = self.order.pop_back() {
                if self.textures.remove(&oldest_key).is_some() {
                    // log::debug!("Cache PRUNED: {}", oldest_key);
                }
            } else {
                break;
            }
        }
    }

    pub fn remove_image_thumbnails(
        &mut self,
        image_filename: &str,
        image_map_name: &str,
        data_dir: &PathBuf,
    ) {
        let map_data_dir = data_dir.join(image_map_name);
        let original_image_path_in_data = map_data_dir.join(image_filename); // Path to the main image
        let thumb_storage_dir = map_data_dir.join(".thumbnails");

        for &size in ALLOWED_THUMB_SIZES.iter() {
            let expected_thumb_path = module_construct_thumbnail_path(
                &original_image_path_in_data,
                &thumb_storage_dir,
                size,
            );
            let expected_thumb_path_str = expected_thumb_path.to_string_lossy().into_owned();

            if self.textures.remove(&expected_thumb_path_str).is_some() {
                self.order.retain(|k| k != &expected_thumb_path_str);
            }
            self.loading_in_progress.remove(&expected_thumb_path_str); // Also remove if it was loading
        }
    }
}

impl fmt::Debug for ThumbnailCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ThumbnailCache")
            .field("textures_count", &self.textures.len())
            .field("order_count", &self.order.len())
            .field("loading_in_progress_count", &self.loading_in_progress.len())
            .finish()
    }
}

// --- ThumbnailService struct and impl ---
pub struct ThumbnailService {
    cache: ThumbnailCache,
    job_sender: Sender<ThumbnailLoadJob>,
    result_receiver: Receiver<ThumbnailLoadResult>,
}

// Manual Debug impl for ThumbnailService because Sender/Receiver don't derive Debug
impl fmt::Debug for ThumbnailService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ThumbnailService")
            .field("cache", &self.cache)
            // Not showing sender/receiver details
            .finish_non_exhaustive()
    }
}

impl ThumbnailService {
    pub fn new() -> Self {
        let (job_sender, job_receiver) = mpsc::channel::<ThumbnailLoadJob>();
        let (result_sender, result_receiver) = mpsc::channel::<ThumbnailLoadResult>();

        thread::spawn(move || {
            // Worker thread logic
            for job in job_receiver {
                let final_thumb_path = module_construct_thumbnail_path(
                    &job.image_file_path, // This is the *original* image path for generation purposes
                    &job.thumb_storage_dir,
                    job.target_size,
                );
                let thumb_path_key = final_thumb_path.to_string_lossy().into_owned();

                // Attempt to load existing thumbnail first
                match Self::static_load_attempt(&final_thumb_path) {
                    Ok(loaded_data) => {
                        // Successfully loaded, send result directly
                        log::info!(
                            "WorkerLoop: Successfully loaded existing thumbnail: {}",
                            final_thumb_path.display()
                        );
                        let result_payload = ThumbnailLoadResult {
                            thumb_path_key, // thumb_path_key is already defined from final_thumb_path
                            color_image: Some(loaded_data.0),
                            dimensions: Some(loaded_data.1),
                            error: None,
                        };
                        if let Err(e) = result_sender.send(result_payload) {
                            log::warn!(
                                "WorkerLoop: Result receiver dropped for direct load of {}. Error: {}. Worker thread stopping.",
                                final_thumb_path.display(),
                                e
                            );
                            break; // Exit the main `for job in job_receiver` loop
                        }
                    }
                    Err(open_err) => {
                        // Failed to open, try to generate it in a rayon task
                        log::info!(
                            "WorkerLoop: Thumbnail {} not found or failed to open ({}). Spawning rayon task for generation from {}.",
                            final_thumb_path.display(),
                            open_err,
                            job.image_file_path.display()
                        );

                        let rayon_result_sender = result_sender.clone();
                        let rayon_job_image_file_path = job.image_file_path.clone();
                        let rayon_job_thumb_storage_dir = job.thumb_storage_dir.clone();
                        let rayon_job_target_size = job.target_size;
                        let rayon_thumb_path_key = thumb_path_key.clone(); // Cloned from outer scope

                        spawn(move || {
                            // This path is for logging the initial target path before generation
                            let initial_target_path_for_rayon_log = module_construct_thumbnail_path(
                                &rayon_job_image_file_path,
                                &rayon_job_thumb_storage_dir,
                                rayon_job_target_size,
                            );

                            let generation_and_load_result_rayon: Result<
                                (egui::ColorImage, (u32, u32)),
                                String,
                            >;

                            match Self::static_generate_thumbnail_file(
                                &rayon_job_image_file_path,
                                &rayon_job_thumb_storage_dir,
                                rayon_job_target_size,
                            ) {
                                Ok(generated_path_buf) => {
                                    log::info!(
                                        "Rayon: Successfully generated thumbnail: {}",
                                        generated_path_buf.display()
                                    );
                                    generation_and_load_result_rayon = Self::static_load_attempt(&generated_path_buf).map_err(|load_err_after_gen| {
                                        format!("Rayon: Generated thumbnail {} but failed to load it: {}", generated_path_buf.display(), load_err_after_gen)
                                    });
                                }
                                Err(gen_err) => {
                                    let err_msg = format!(
                                        "Rayon: Failed to generate thumbnail {}: {}",
                                        initial_target_path_for_rayon_log.display(),
                                        gen_err
                                    );
                                    log::error!("{}", err_msg);
                                    generation_and_load_result_rayon = Err(err_msg);
                                }
                            }

                            let send_result_payload_rayon = match generation_and_load_result_rayon {
                                Ok((color_image, dimensions)) => ThumbnailLoadResult {
                                    thumb_path_key: rayon_thumb_path_key.clone(),
                                    color_image: Some(color_image),
                                    dimensions: Some(dimensions),
                                    error: None,
                                },
                                Err(err_str) => {
                                    log::warn!(
                                        "Rayon: Thumbnail processing failed for {}: {}. Sending error result.",
                                        rayon_thumb_path_key,
                                        err_str
                                    );
                                    ThumbnailLoadResult {
                                        thumb_path_key: rayon_thumb_path_key.clone(),
                                        color_image: None,
                                        dimensions: None,
                                        error: Some(err_str),
                                    }
                                }
                            };

                            if let Err(e) = rayon_result_sender.send(send_result_payload_rayon) {
                                log::error!(
                                    "Rayon: Failed to send thumbnail result for {}: {}",
                                    rayon_thumb_path_key,
                                    e
                                );
                            }
                        });
                        // Main worker loop does not wait and does not send anything here.
                    }
                }
            }
            log::info!("Thumbnail worker thread finished.");
        });

        Self {
            cache: ThumbnailCache::new(),
            job_sender,
            result_receiver,
        }
    }

    /// Processes results from the background thumbnail loading thread.
    /// Returns true if any textures were loaded (signaling a need for UI repaint).
    pub fn process_background_loads(&mut self, ctx: &egui::Context) -> bool {
        let mut loaded_any = false;
        while let Ok(result) = self.result_receiver.try_recv() {
            self.cache
                .loading_in_progress
                .remove(&result.thumb_path_key);
            if let Some(color_image) = result.color_image {
                if let Some((image_width, image_height)) = result.dimensions {
                    let texture_name = result.thumb_path_key.clone();
                    let texture_handle =
                        ctx.load_texture(texture_name, color_image, egui::TextureOptions::LINEAR);
                    self.cache.textures.insert(
                        result.thumb_path_key.clone(),
                        (texture_handle, (image_width, image_height)),
                    );
                    self.cache.order.push_front(result.thumb_path_key);
                    self.cache.prune();
                    loaded_any = true;
                } else {
                    log::error!(
                        "Thumbnail result for {} had image but no dimensions.",
                        result.thumb_path_key
                    );
                }
            } else if let Some(error_msg) = result.error {
                log::error!(
                    "Failed to load thumbnail {}: {}",
                    result.thumb_path_key,
                    error_msg
                );
            }
        }
        loaded_any
    }

    pub fn send_thumbnail_job(
        &self,
        job: ThumbnailLoadJob,
    ) -> Result<(), mpsc::SendError<ThumbnailLoadJob>> {
        self.job_sender.send(job)
    }

    pub fn get_or_request_thumbnail_texture(
        &mut self,
        // ctx: &egui::Context, // Not needed directly here, textures are loaded in process_background_loads
        image_file_path: &PathBuf,
        thumb_storage_dir: &PathBuf,
        target_size: u32,
    ) -> Option<&(egui::TextureHandle, (u32, u32))> {
        let thumb_path =
            module_construct_thumbnail_path(image_file_path, thumb_storage_dir, target_size);
        let thumb_path_key = thumb_path.to_string_lossy().into_owned();

        // 1. Check cache
        if let Some(cached_data) = self.cache.textures.get(&thumb_path_key) {
            // If found, update LRU order and return
            self.cache.order.retain(|k| k != &thumb_path_key);
            self.cache.order.push_front(thumb_path_key.clone());
            return Some(cached_data);
        }

        // 2. Check if already loading
        if self.cache.loading_in_progress.contains(&thumb_path_key) {
            return None; // Already requested, waiting for background thread
        }

        // 3. If not cached and not loading, request it
        self.cache
            .loading_in_progress
            .insert(thumb_path_key.clone());

        let job = ThumbnailLoadJob {
            image_file_path: image_file_path.clone(),
            thumb_storage_dir: thumb_storage_dir.clone(),
            target_size,
            // thumb_path is constructed by the worker now
        };

        if let Err(e) = self.job_sender.send(job) {
            log::error!(
                "Failed to send thumbnail load job for {}: {}",
                thumb_path_key,
                e
            );
            self.cache.loading_in_progress.remove(&thumb_path_key); // Remove from loading if send failed
        }

        None // Request sent, thumbnail not available yet
    }

    // Method to generate a single thumbnail (can be called by ImageService)
    fn static_load_attempt(
        path_to_load: &PathBuf,
    ) -> Result<(egui::ColorImage, (u32, u32)), String> {
        match image::open(path_to_load) {
            Ok(img) => {
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [img.width() as usize, img.height() as usize],
                    img.to_rgba8().as_flat_samples().as_slice(),
                );
                Ok((color_image, (img.width(), img.height())))
            }
            Err(e) => Err(format!(
                "Failed to open thumbnail {}: {}",
                path_to_load.display(),
                e
            )),
        }
    }
    //
    // This function is now static-like, not requiring `&mut self`.
    fn static_generate_thumbnail_file(
        original_image_path: &PathBuf,
        thumb_storage_dir: &PathBuf,
        target_width: u32,
    ) -> Result<PathBuf, ThumbnailServiceError> {
        if !thumb_storage_dir.exists() {
            std::fs::create_dir_all(thumb_storage_dir).map_err(|e| {
                ThumbnailServiceError::DirectoryCreationFailed(thumb_storage_dir.clone(), e)
            })?;
        }

        let img = image::open(original_image_path)
            .map_err(|e| ThumbnailServiceError::ImageOpenFailed(original_image_path.clone(), e))?;

        let aspect_ratio = img.width() as f32 / img.height() as f32;
        let target_height = (target_width as f32 / aspect_ratio) as u32;

        let thumbnail = img.resize_exact(
            target_width,
            target_height,
            image::imageops::FilterType::Lanczos3,
        );
        let thumb_path =
            module_construct_thumbnail_path(original_image_path, thumb_storage_dir, target_width);

        thumbnail
            .save_with_format(&thumb_path, image::ImageFormat::WebP)
            .map_err(|e| ThumbnailServiceError::ImageSaveFailed(thumb_path.clone(), e))?;

        Ok(thumb_path)
    }

    // Method to generate all standard thumbnail sizes for an image

    // Method to remove all thumbnails associated with an original image file
    pub fn remove_thumbnails_for_image(
        &mut self,
        image_filename: &str,
        image_map_name: &str,
        data_dir: &PathBuf,
    ) -> Result<(), std::io::Error> {
        let map_data_dir = data_dir.join(image_map_name);
        let original_image_path_in_data = map_data_dir.join(image_filename);
        let thumb_storage_dir = map_data_dir.join(".thumbnails");

        if thumb_storage_dir.exists() {
            for &size in ALLOWED_THUMB_SIZES.iter() {
                let thumb_path = module_construct_thumbnail_path(
                    &original_image_path_in_data,
                    &thumb_storage_dir,
                    size,
                );
                if thumb_path.exists() {
                    std::fs::remove_file(&thumb_path)?;
                }
            }
        }
        self.cache
            .remove_image_thumbnails(image_filename, image_map_name, data_dir);
        Ok(())
    }
}
