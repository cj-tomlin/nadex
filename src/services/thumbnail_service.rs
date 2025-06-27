use std::collections::{HashMap, HashSet, VecDeque};
use std::error::Error as StdError; // Alias for clarity
use std::fmt;
use std::fs::{self}; // Added File for tests, OpenOptions moved to tests module
use std::path::{Path, PathBuf};

use std::sync::{
    Mutex,
    mpsc::{Receiver, Sender},
};
use std::thread;
// For image processing

use egui;
use image::{self, GenericImageView}; // Keep image crate import, add GenericImageView
use log;

// --- SerializableIoError ---
#[derive(Debug, Clone)]
pub struct SerializableIoError {
    pub kind: std::io::ErrorKind,
    pub message: String,
}

impl From<std::io::Error> for SerializableIoError {
    fn from(error: std::io::Error) -> Self {
        SerializableIoError {
            kind: error.kind(),
            message: error.to_string(),
        }
    }
}

impl fmt::Display for SerializableIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IO Error (Kind: {:?}): {}", self.kind, self.message)
    }
}

impl StdError for SerializableIoError {}
// --- End SerializableIoError ---

// --- SerializableImageError ---
#[derive(Debug, Clone)]
pub struct SerializableImageError {
    pub message: String,
}

impl From<&image::ImageError> for SerializableImageError {
    fn from(error: &image::ImageError) -> Self {
        SerializableImageError {
            message: error.to_string(),
        }
    }
}

impl fmt::Display for SerializableImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Image Error: {}", self.message)
    }
}
impl StdError for SerializableImageError {}
// --- End SerializableImageError ---

// --- ThumbnailServiceError ---
#[derive(Debug, Clone)]
pub enum ThumbnailServiceError {
    DirectoryCreation(PathBuf, SerializableIoError),
    ImageOpen(PathBuf, SerializableImageError),
    ImageSave(PathBuf, SerializableImageError),
    FileRemoval(PathBuf, SerializableIoError), // For file removal errors
    Other(String),                             // For general errors
}

impl fmt::Display for ThumbnailServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ThumbnailServiceError::DirectoryCreation(path, err) => write!(
                f,
                "Directory creation failed for '{}': {}",
                path.display(),
                err
            ),
            ThumbnailServiceError::ImageOpen(path, err) => {
                write!(f, "Image open failed for '{}': {}", path.display(), err)
            }
            ThumbnailServiceError::ImageSave(path, err) => {
                write!(f, "Image save failed for '{}': {}", path.display(), err)
            }
            ThumbnailServiceError::FileRemoval(path, err) => {
                write!(f, "File removal failed for '{}': {}", path.display(), err)
            }
            ThumbnailServiceError::Other(msg) => {
                write!(f, "Thumbnail service error: {}", msg)
            }
        }
    }
}

impl StdError for ThumbnailServiceError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            ThumbnailServiceError::DirectoryCreation(_, err) => Some(err),
            ThumbnailServiceError::ImageOpen(_, err) => Some(err),
            ThumbnailServiceError::ImageSave(_, err) => Some(err),
            ThumbnailServiceError::FileRemoval(_, err) => Some(err),
            ThumbnailServiceError::Other(_) => None, // No underlying source error
        }
    }
}
// --- End ThumbnailServiceError ---

// Full-size WebP images are now used instead of multiple thumbnails at different sizes
const MAX_THUMB_CACHE_SIZE: usize = 24; // Example value

// --- Structs for Asynchronous Thumbnail Loading (if used by cache/UI directly) ---
#[derive(Debug)]
pub struct ThumbnailLoadJob {
    pub image_file_path: PathBuf,
    pub thumb_storage_dir: PathBuf,
    pub target_size: u32,
}

#[derive(Debug)]
pub struct ThumbnailLoadResult {
    pub thumb_path_key: String,
    pub color_image: Option<egui::ColorImage>, // Requires egui import if used
    pub dimensions: Option<(u32, u32)>,
    pub error: Option<String>,
}
// --- End Async Structs ---

/// Constructs the canonical path for a thumbnail file.
///
/// When `size` is 0, it creates a path for a full-size WebP image without a size suffix.
/// Otherwise, it creates a path with the traditional size suffix for backward compatibility.
pub fn module_construct_thumbnail_path(img_path: &Path, thumb_dir: &Path, size: u32) -> PathBuf {
    let stem = img_path.file_stem().unwrap_or_default().to_string_lossy();

    // For full-size WebP (size=0), don't add a size suffix
    if size == 0 {
        thumb_dir.join(format!("{}.webp", stem))
    } else {
        // For backward compatibility, keep the size suffix for old thumbnails
        thumb_dir.join(format!("{}_{}.webp", stem, size))
    }
}

// --- ThumbnailCache struct and impl ---
// This cache is for egui::TextureHandle, so it's UI-specific.
// If ThumbnailService is purely backend, this might live elsewhere or be simpler.
pub struct ThumbnailCache {
    textures: HashMap<String, (egui::TextureHandle, (u32, u32))>,
    order: VecDeque<String>,
    loading_in_progress: HashSet<String>,
}

impl fmt::Debug for ThumbnailCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ThumbnailCache")
            .field(
                "textures",
                &format!("<{} Egui TextureHandles>", self.textures.len()),
            )
            .field("order", &self.order)
            .field("loading_in_progress", &self.loading_in_progress)
            .finish()
    }
}

impl ThumbnailCache {
    pub fn new() -> Self {
        Self {
            textures: HashMap::new(),
            order: VecDeque::with_capacity(MAX_THUMB_CACHE_SIZE),
            loading_in_progress: HashSet::new(),
        }
    }

    #[allow(dead_code)]
    fn prune(&mut self) {
        while self.order.len() > MAX_THUMB_CACHE_SIZE {
            if let Some(oldest_key) = self.order.pop_back() {
                self.textures.remove(&oldest_key);
            } else {
                break;
            }
        }
    }

    // This method is specific to clearing cache entries.
    // The actual file deletion is handled by ThumbnailServiceTrait::remove_thumbnails_for_image
    pub fn remove_image_thumbnails(
        &mut self,
        image_filename: &str,
        image_map_name: &str,
        data_dir: &Path,
    ) {
        let map_data_dir = data_dir.join(image_map_name);
        let original_image_path_in_data = map_data_dir.join(image_filename);
        let thumb_storage_dir = map_data_dir.join(".thumbnails");

        // Handle only the full-size WebP (size=0)
        let full_webp_path = module_construct_thumbnail_path(
            &original_image_path_in_data,
            &thumb_storage_dir,
            0, // Size 0 indicates full-size WebP
        );
        let key = full_webp_path.to_string_lossy().into_owned();
        self.textures.remove(&key);
        self.loading_in_progress.remove(&key);
        self.order.retain(|k| self.textures.contains_key(k));
    }

    // Other cache methods like get, insert, mark_loading etc. would go here
    // For brevity, they are omitted but would be necessary for a functional UI cache.
    #[allow(dead_code)]
    pub fn get_texture_info(&mut self, key: &str) -> Option<&(egui::TextureHandle, (u32, u32))> {
        if let Some(index) = self.order.iter().position(|x| x == key) {
            let k = self.order.remove(index).unwrap();
            self.order.push_front(k);
        }
        self.textures.get(key)
    }
}
// --- End ThumbnailCache ---

// --- ThumbnailServiceTrait ---
pub trait ThumbnailServiceTrait: Send + Sync + fmt::Debug {
    /// Convert the source image to a single WebP file at full resolution
    #[allow(dead_code)] // This is used for the thumbnail cache
    fn convert_to_full_webp(
        &self,
        source_image_path: &Path,
        thumbnail_storage_dir: &Path,
    ) -> Result<PathBuf, ThumbnailServiceError>;

    /// Convert the source image to WebP format at a specific output path
    fn convert_to_webp_at_path(
        &self,
        source_image_path: &Path,
        output_path: &Path,
    ) -> Result<(), ThumbnailServiceError>;

    /// Remove all thumbnails for a specific image
    fn remove_thumbnails_for_image(
        &mut self,
        image_filename: &str,
        image_map_name: &str,
        data_dir: &Path,
    ) -> Result<(), ThumbnailServiceError>;

    fn get_cached_texture_info(&self, key: &str) -> Option<(egui::TextureHandle, (u32, u32))>;

    /// Check if a texture exists in the cache
    fn has_texture(&self, key: &str) -> bool;

    /// Load a texture from file into the cache
    fn load_texture_from_file(
        &mut self,
        file_path: &Path,
        cache_key: &str,
        ctx: &egui::Context,
    ) -> Result<(), ThumbnailServiceError>;
}
// --- End ThumbnailServiceTrait ---

// --- ConcreteThumbnailService ---
#[derive(Debug)]
pub struct ConcreteThumbnailService {
    cache: Mutex<ThumbnailCache>, // If service manages UI cache directly
    job_sender: Sender<ThumbnailLoadJob>, // For async generation
                                  // job_receiver is typically held by a worker thread pool manager
}

impl ConcreteThumbnailService {
    pub fn new(job_sender: Sender<ThumbnailLoadJob>) -> Self {
        Self {
            cache: Mutex::new(ThumbnailCache::new()),
            job_sender,
        }
    }

    // Method to be called by the UI/main thread when a thumbnail result is received
    pub fn process_completed_job(
        &mut self,
        key: String, // This is the thumb_path_key from ThumbnailLoadResult
        color_image: egui::ColorImage,
        dimensions: (u32, u32),
        ctx: &egui::Context, // egui::Context for texture creation
    ) {
        log::debug!("Processing completed job for key: {}", key);

        // Create texture handle from ColorImage
        let texture_handle = ctx.load_texture(
            &key,                         // Use the unique key as the texture name
            color_image,                  // The actual image data
            egui::TextureOptions::LINEAR, // Default filtering
        );

        let mut cache = self.cache.lock().unwrap();
        // Ensure the key for cache.textures and cache.order is the same string instance or correctly cloned.
        let cache_key_for_insert = key.clone();
        let cache_key_for_order = key;

        cache
            .textures
            .insert(cache_key_for_insert, (texture_handle, dimensions));
        log::info!(
            "Inserted texture into cache for key: {}, new cache size: {}",
            cache_key_for_order,
            cache.textures.len()
        );
        cache.order.push_back(cache_key_for_order); // Add to LRU tracking
        cache.prune(); // Maintain cache size
    }

    #[allow(dead_code)] // Renamed and refactored from get_or_request_thumbnail_texture
    // This method is responsible for initiating the asynchronous thumbnail generation.
    // Renamed to _internal_ to distinguish from the trait method.
    fn _internal_request_thumbnail_generation(
        &mut self,
        image_file_path: PathBuf,   // Full path to the original image
        thumb_storage_dir: PathBuf, // Directory where thumbnails are stored
        target_size: u32,           // Target width/height for the thumbnail
    ) -> Result<(), String> {
        let thumb_path_key =
            module_construct_thumbnail_path(&image_file_path, &thumb_storage_dir, target_size)
                .to_string_lossy()
                .into_owned();

        let mut cache = self.cache.lock().unwrap();

        // 1. Check if texture is already loaded
        if cache.get_texture_info(&thumb_path_key).is_some() {
            log::debug!(
                "Thumbnail for key '{}' already in cache. Skipping request.",
                thumb_path_key
            );
            return Ok(()); // Already cached, no need to request
        }

        // 2. Check if texture is already loading
        if cache.loading_in_progress.contains(&thumb_path_key) {
            log::debug!(
                "Thumbnail for key '{}' is already being loaded. Skipping request.",
                thumb_path_key
            );
            return Ok(()); // Already loading
        }

        // 3. If not loaded and not loading, request load
        let job = ThumbnailLoadJob {
            image_file_path,   // Consumed
            thumb_storage_dir, // Consumed
            target_size,
        };

        match self.job_sender.send(job) {
            Ok(_) => {
                cache.loading_in_progress.insert(thumb_path_key.clone());
                log::debug!("Sent thumbnail load job for key: {}", thumb_path_key);
                Ok(())
            }
            Err(e) => {
                log::error!(
                    "Failed to send thumbnail load job for key '{}': {}",
                    thumb_path_key,
                    e
                );
                Err(format!("Failed to send thumbnail load job: {}", e))
            }
        }
    }

    #[allow(dead_code)] // TODO: Implement fully
    pub fn process_loaded_thumbnails(
        &mut self,
        ctx: &egui::Context,
        results: Vec<ThumbnailLoadResult>,
    ) -> bool {
        let mut new_textures_loaded = false;
        let mut cache = self.cache.lock().unwrap();

        for result in results {
            cache.loading_in_progress.remove(&result.thumb_path_key);

            if let Some(err_msg) = result.error {
                log::error!(
                    "Thumbnail loading failed for {}: {}",
                    result.thumb_path_key,
                    err_msg
                );
                continue;
            }

            if let (Some(color_image), Some(dimensions)) = (result.color_image, result.dimensions) {
                let texture_handle = ctx.load_texture(
                    &result.thumb_path_key,       // Use a unique name for the texture
                    color_image,                  // The egui::ColorImage
                    egui::TextureOptions::LINEAR, // Default options
                );
                cache
                    .textures
                    .insert(result.thumb_path_key.clone(), (texture_handle, dimensions));
                cache.order.push_front(result.thumb_path_key); // Add to front for LRU
                cache.prune(); // Prune if over max size
                new_textures_loaded = true;
                log::debug!(
                    "Successfully loaded and cached texture for: {}",
                    cache.order.front().unwrap()
                );
            } else {
                log::warn!(
                    "ThumbnailLoadResult for {} was missing image or dimensions despite no error.",
                    result.thumb_path_key
                );
            }
        }
        new_textures_loaded
    }
}

impl ThumbnailServiceTrait for ConcreteThumbnailService {
    fn convert_to_full_webp(
        &self,
        original_image_path: &Path,
        output_dir: &Path,
    ) -> Result<PathBuf, ThumbnailServiceError> {
        _static_convert_to_full_webp(original_image_path, output_dir)
    }

    fn convert_to_webp_at_path(
        &self,
        source_image_path: &Path,
        output_path: &Path,
    ) -> Result<(), ThumbnailServiceError> {
        _static_convert_to_webp_at_path(source_image_path, output_path)
    }

    fn remove_thumbnails_for_image(
        &mut self,
        image_filename: &str,
        image_map_name: &str,
        data_dir: &Path,
    ) -> Result<(), ThumbnailServiceError> {
        let map_data_dir = data_dir.join(image_map_name);
        let original_image_path_in_data = map_data_dir.join(image_filename); // Used to form thumb names
        let thumb_storage_dir = map_data_dir.join(".thumbnails");
        let mut first_error: Option<ThumbnailServiceError> = None;

        if thumb_storage_dir.exists() && thumb_storage_dir.is_dir() {
            // For the full-size WebP (size=0)
            let full_webp_path = module_construct_thumbnail_path(
                &original_image_path_in_data,
                &thumb_storage_dir,
                0, // Size 0 indicates full-size WebP
            );

            if full_webp_path.exists() {
                if let Err(e) = fs::remove_file(&full_webp_path) {
                    log::warn!(
                        "Failed to remove full-size WebP file {:?}: {}. Will attempt to continue.",
                        full_webp_path,
                        e
                    );
                    if first_error.is_none() {
                        first_error = Some(ThumbnailServiceError::FileRemoval(
                            full_webp_path.clone(),
                            e.into(),
                        ));
                    }
                }
            }
        }

        // Also clear from cache
        if let Ok(mut cache) = self.cache.lock() {
            cache.remove_image_thumbnails(image_filename, image_map_name, data_dir);
        } else {
            log::error!("Failed to lock thumbnail cache for clearing entries during removal.");
            // Decide if this itself should be an error. For now, prioritize file system errors.
        }

        if let Some(err) = first_error {
            Err(err)
        } else {
            Ok(())
        }
    }

    // The deprecated request_thumbnail_generation method has been removed as part of the transition to full-size WebP images

    fn get_cached_texture_info(&self, key: &str) -> Option<(egui::TextureHandle, (u32, u32))> {
        let mut cache = self.cache.lock().unwrap();
        // Cloned because TextureHandle is Arc-like and dimensions are (u32, u32) which is Copy.
        // The TextureHandle itself is cloneable (it's an Arc internally).
        cache.get_texture_info(key).cloned()
    }

    fn has_texture(&self, key: &str) -> bool {
        if let Ok(cache) = self.cache.lock() {
            cache.textures.contains_key(key)
        } else {
            false
        }
    }

    fn load_texture_from_file(
        &mut self,
        file_path: &Path,
        cache_key: &str,
        ctx: &egui::Context,
    ) -> Result<(), ThumbnailServiceError> {
        if !file_path.exists() {
            return Err(ThumbnailServiceError::ImageOpen(
                file_path.to_path_buf(),
                SerializableImageError {
                    message: "File does not exist".to_string(),
                },
            ));
        }

        // Load the image from file
        log::info!("Loading image from path: {:?}", file_path);
        let img = image::open(file_path).map_err(|e| {
            log::error!("Failed to load image: {:?}", e);
            ThumbnailServiceError::ImageOpen(file_path.to_path_buf(), (&e).into())
        })?;

        let dimensions = (img.width(), img.height());
        let rgba_img = img.into_rgba8();

        // Get size and raw bytes from image
        let size = [rgba_img.width() as usize, rgba_img.height() as usize];
        let rgba_bytes = rgba_img.as_raw(); // This gives &[u8] which is what from_rgba_unmultiplied expects

        log::info!(
            "Creating texture with dimensions: {}x{}",
            dimensions.0,
            dimensions.1
        );

        // Create the texture
        let texture = ctx.load_texture(
            cache_key,
            egui::ColorImage::from_rgba_unmultiplied(size, rgba_bytes),
            egui::TextureOptions::default(),
        );

        // Store in cache
        if let Ok(mut cache) = self.cache.lock() {
            cache
                .textures
                .insert(cache_key.to_string(), (texture, dimensions));
            cache.order.push_back(cache_key.to_string());
            log::info!("Texture cached successfully with key: {}", cache_key);
        } else {
            return Err(ThumbnailServiceError::Other(
                "Failed to lock cache".to_string(),
            ));
        }

        Ok(())
    }
}

// --- Thumbnail Worker Thread ---
pub fn spawn_thumbnail_worker_thread(
    job_receiver: Receiver<ThumbnailLoadJob>,
    result_sender: Sender<ThumbnailLoadResult>,
) {
    thread::spawn(move || {
        log::info!("Thumbnail worker thread started.");
        for job in job_receiver {
            // Loop will terminate if sender disconnects and channel is empty
            log::debug!(
                "Worker received job for path: {}",
                job.image_file_path.display()
            );

            // Construct the key for the result, similar to how it's done in get_or_request_thumbnail_texture
            let thumb_path_key = module_construct_thumbnail_path(
                &job.image_file_path,
                &job.thumb_storage_dir,
                job.target_size,
            )
            .to_string_lossy()
            .into_owned();

            let (image_data, dimensions, error_message) = match process_job_to_color_image(&job) {
                Ok((color_image, dims)) => (Some(color_image), Some(dims), None),
                Err(err_msg) => {
                    log::error!(
                        "Error processing thumbnail for {:?}: {}",
                        job.image_file_path,
                        err_msg
                    );
                    (None, None, Some(err_msg))
                }
            };

            let result = ThumbnailLoadResult {
                thumb_path_key,          // Key to identify the thumbnail in the cache
                color_image: image_data, // Option<egui::ColorImage>
                dimensions,              // Option<(u32, u32)>
                error: error_message,
            };

            if result_sender.send(result).is_err() {
                log::error!("Thumbnail worker: Result receiver disconnected. Shutting down.");
                break; // Exit loop if we can't send results
            }
            log::debug!("Worker sent result for: {:?}", job.image_file_path);
        }
        log::info!("Thumbnail worker thread finished.");
    });
}
// --- End ConcreteThumbnailService ---

// Helper function for the worker thread
fn process_job_to_color_image(
    job: &ThumbnailLoadJob,
) -> Result<(egui::ColorImage, (u32, u32)), String> {
    // 1. Open the image
    let img = image::open(&job.image_file_path)
        .map_err(|e| format!("Failed to open image {:?}: {}", job.image_file_path, e))?;

    // 2. Resize (thumbnail maintains aspect ratio)
    let resized_img = img.thumbnail(job.target_size, job.target_size);
    let (width, height) = resized_img.dimensions();

    // 3. Convert to egui::ColorImage
    let rgba_image = resized_img.to_rgba8(); // This is an ImageBuffer<Rgba<u8>, Vec<u8>>

    let mut color_pixels = Vec::with_capacity((width * height) as usize);
    for pixel_data in rgba_image.pixels() {
        // pixel_data is &Rgba<u8>
        color_pixels.push(egui::Color32::from_rgba_unmultiplied(
            pixel_data[0],
            pixel_data[1],
            pixel_data[2],
            pixel_data[3],
        ));
    }

    let color_image = egui::ColorImage {
        size: [width as usize, height as usize],
        pixels: color_pixels,
    };

    Ok((color_image, (width, height)))
}

// Private static helper for converting an image to WebP at a specific output path
fn _static_convert_to_webp_at_path(
    source_image_path: &Path,
    output_path: &Path,
) -> Result<(), ThumbnailServiceError> {
    if !source_image_path.exists() {
        return Err(ThumbnailServiceError::ImageOpen(
            source_image_path.to_path_buf(),
            SerializableImageError {
                message: "Source image file does not exist.".to_string(),
            },
        ));
    }

    // Ensure the output directory exists
    if let Some(parent) = output_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| {
                ThumbnailServiceError::DirectoryCreation(parent.to_path_buf(), e.into())
            })?;
        }
    }

    // Open the source image
    let img = image::open(source_image_path).map_err(|e| {
        ThumbnailServiceError::ImageOpen(source_image_path.to_path_buf(), (&e).into())
    })?;

    // Convert to RGBA8 for consistent WebP encoding
    let rgba_img = img.to_rgba8();

    // Save directly to the specified output path with WebP format
    rgba_img
        .save_with_format(output_path, image::ImageFormat::WebP)
        .map_err(|e| ThumbnailServiceError::ImageSave(output_path.to_path_buf(), (&e).into()))?;

    log::info!("WebP conversion complete: {}", output_path.display());

    Ok(())
}

// Private static helper for converting an image to full-size WebP
fn _static_convert_to_full_webp(
    original_image_path: &Path,
    output_dir: &Path,
) -> Result<PathBuf, ThumbnailServiceError> {
    if !original_image_path.exists() {
        return Err(ThumbnailServiceError::ImageOpen(
            original_image_path.to_path_buf(),
            SerializableImageError {
                message: "Original image file does not exist.".to_string(),
            },
        ));
    }

    if output_dir.is_file() {
        return Err(ThumbnailServiceError::DirectoryCreation(
            output_dir.to_path_buf(),
            SerializableIoError {
                kind: std::io::ErrorKind::AlreadyExists,
                message: "Intended output directory path exists as a file.".to_string(),
            },
        ));
    }

    if !output_dir.exists() {
        fs::create_dir_all(output_dir).map_err(|e| {
            ThumbnailServiceError::DirectoryCreation(output_dir.to_path_buf(), e.into())
        })?;
    }

    let img = image::open(original_image_path).map_err(|e| {
        ThumbnailServiceError::ImageOpen(original_image_path.to_path_buf(), (&e).into())
    })?;

    // Use the original dimensions - no resizing
    let file_stem = original_image_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();

    // Construct the output path with .webp extension
    let output_path = output_dir.join(format!("{}.webp", file_stem));

    // Save the image as WebP at full resolution with optimized quality
    // The image crate uses reasonable default quality settings for WebP
    // We explicitly convert to RGBA8 for consistent results
    let rgba_img = img.to_rgba8();

    // Save with WebP format for good balance of quality and file size
    rgba_img
        .save_with_format(&output_path, image::ImageFormat::WebP)
        .map_err(|e| ThumbnailServiceError::ImageSave(output_path.clone(), (&e).into()))?;

    // Note: The image crate uses default WebP encoding parameters which are good for most cases
    // If more control is needed, we could use a crate like webp or libwebp-sys directly

    log::info!(
        "Optimized WebP conversion complete: {}",
        output_path.display()
    );

    Ok(output_path)
}

// Private static helper for actual thumbnail generation
fn _static_do_generate_thumbnail_file(
    original_image_path: &Path,
    thumb_storage_dir: &Path,
    target_width: u32,
) -> Result<PathBuf, ThumbnailServiceError> {
    if !original_image_path.exists() {
        return Err(ThumbnailServiceError::ImageOpen(
            original_image_path.to_path_buf(),
            SerializableImageError {
                message: "Original image file does not exist.".to_string(),
            },
        ));
    }

    if thumb_storage_dir.is_file() {
        return Err(ThumbnailServiceError::DirectoryCreation(
            thumb_storage_dir.to_path_buf(),
            SerializableIoError {
                kind: std::io::ErrorKind::AlreadyExists,
                message: "Intended thumbnail directory path exists as a file.".to_string(),
            },
        ));
    }

    if !thumb_storage_dir.exists() {
        fs::create_dir_all(thumb_storage_dir).map_err(|e| {
            ThumbnailServiceError::DirectoryCreation(thumb_storage_dir.to_path_buf(), e.into())
        })?;
    }

    let img = image::open(original_image_path).map_err(|e| {
        ThumbnailServiceError::ImageOpen(original_image_path.to_path_buf(), (&e).into())
    })?;

    let original_width = img.width();
    let original_height = img.height();
    let target_height =
        (original_height as f32 * (target_width as f32 / original_width as f32)) as u32;

    let thumbnail = img.thumbnail_exact(target_width, target_height);

    let thumb_path_with_ext =
        module_construct_thumbnail_path(original_image_path, thumb_storage_dir, target_width);

    thumbnail
        .save_with_format(&thumb_path_with_ext, image::ImageFormat::WebP)
        .map_err(|e| ThumbnailServiceError::ImageSave(thumb_path_with_ext.clone(), (&e).into()))?;

    Ok(thumb_path_with_ext)
}

// --- Tests ---
#[cfg(test)]
mod tests {
    use super::*; // Import items from outer module
    use crate::tests_common::{
        create_dummy_image_file, // ThumbnailTestEnvironment removed as unused by name
        setup_thumbnail_test_env,
    };
    // use image::{ImageBuffer, ImageFormat, Rgba}; // Was for local create_dummy_image_file
    use std::fs::{self, File, OpenOptions}; // Keep fs and File for test setup, added OpenOptions
    #[cfg(windows)]
    use std::os::windows::fs::OpenOptionsExt;
    // use tempfile::TempDir; // No longer directly used in tests after refactor
    use image::GenericImageView; // Added for img.dimensions()

    // Helper to get a basic ConcreteThumbnailService for tests that don't use async sender
    fn new_test_thumbnail_service() -> ConcreteThumbnailService {
        let (tx, _rx) = std::sync::mpsc::channel(); // Dummy channel for tests not focusing on async
        ConcreteThumbnailService::new(tx)
    }

    #[test]
    fn test_generate_thumbnail_file_success() {
        let env = setup_thumbnail_test_env(); // Use shared setup
        let source_image_filename = "test_image.png";
        // Use env.source_dir for creating the dummy image
        let source_image_path =
            create_dummy_image_file(&env.source_dir, source_image_filename, 200, 150);

        // Use env.output_dir as the thumb_storage_dir. Clone it as the function expects &Path.
        let thumb_storage_dir = env.output_dir.clone();
        // The setup_thumbnail_test_env creates the output_dir, and the service is responsible for creating subdirs if needed.

        let thumbnail_service = new_test_thumbnail_service();

        let result = thumbnail_service.convert_to_full_webp(&source_image_path, &thumb_storage_dir);

        assert!(
            result.is_ok(),
            "convert_to_full_webp failed: {:?}",
            result.err()
        );
        let thumb_path = result.unwrap();

        assert!(
            thumb_path.exists(),
            "Thumbnail file was not created at {:?}",
            thumb_path
        );

        // For full-size WebP, the expected filename is just the file stem with .webp extension
        let expected_thumb_filename = format!(
            "{}.webp",
            source_image_path.file_stem().unwrap().to_str().unwrap()
        );
        assert_eq!(
            thumb_path.file_name().unwrap().to_str().unwrap(),
            expected_thumb_filename
        );

        match image::open(&thumb_path) {
            Ok(img) => {
                // Full-size WebP should maintain the original image width (or be scaled to max width if larger)
                assert!(img.width() > 0, "Generated WebP has invalid width");
            }
            Err(e) => panic!("Failed to open generated thumbnail {:?}: {}", thumb_path, e),
        }
    }

    #[test]
    fn test_generate_thumbnail_file_invalid_source_path() {
        let env = setup_thumbnail_test_env(); // Use shared setup
        let invalid_source_path = env.source_dir.join("non_existent_image.png"); // Use env.source_dir
        let thumb_storage_dir = env.output_dir.clone(); // Use env.output_dir

        let thumbnail_service = new_test_thumbnail_service();
        let result =
            thumbnail_service.convert_to_full_webp(&invalid_source_path, &thumb_storage_dir);

        assert!(result.is_err());
        match result.err().unwrap() {
            ThumbnailServiceError::ImageOpen(path, _) => {
                assert_eq!(path, invalid_source_path);
            }
            other_error => panic!("Expected ImageOpen error, got {:?}", other_error),
        }
    }

    #[test]
    fn test_generate_thumbnail_file_directory_creation_error() {
        let env = setup_thumbnail_test_env(); // Use shared setup
        // Use env.source_dir for creating the dummy image
        let source_image_path = create_dummy_image_file(&env.source_dir, "source.png", 100, 100);

        // Define a path where the service expects to create/use a directory for thumbnails.
        // We will create this path as a file beforehand to cause a conflict.
        let conflicting_path_for_thumb_dir = env
            .temp_dir
            .path()
            .join("conflicting_thumb_storage_as_file");

        File::create(&conflicting_path_for_thumb_dir) // Create this path as a file
            .expect("Test setup: failed to create file at conflicting_path_for_thumb_dir");
        assert!(conflicting_path_for_thumb_dir.is_file());

        let thumbnail_service = new_test_thumbnail_service();

        // Pass the path (which is now a file) as the intended thumbnail storage directory.
        let result = thumbnail_service
            .convert_to_full_webp(&source_image_path, &conflicting_path_for_thumb_dir);

        assert!(result.is_err());
        match result.err().unwrap() {
            ThumbnailServiceError::DirectoryCreation(returned_path, io_error_details) => {
                assert_eq!(
                    returned_path, conflicting_path_for_thumb_dir,
                    "The path in DirectoryCreation error should match the conflicting path"
                );
                eprintln!(
                    "Correctly failed with DirectoryCreation for path '{}': Kind: {:?}, Msg: {}",
                    returned_path.display(),
                    io_error_details.kind,
                    io_error_details.message
                );
            }
            other_error => {
                panic!(
                    "Expected ThumbnailServiceError::DirectoryCreation, but got {:?}",
                    other_error
                );
            }
        }
    }

    #[test]
    fn test_generate_thumbnail_file_image_save_error() {
        let env = setup_thumbnail_test_env(); // Use shared setup
        let source_image_filename = "test_image_save_conflict.png";
        // Use env.source_dir for creating the dummy image
        let source_image_path =
            create_dummy_image_file(&env.source_dir, source_image_filename, 200, 150);

        // Use env.output_dir as the thumb_storage_dir.
        // The setup_thumbnail_test_env creates this directory.
        let thumb_storage_dir = env.output_dir.clone();

        // For full-size WebP, the naming format is just the file stem with .webp extension
        let expected_thumb_filename = format!(
            "{}.webp",
            source_image_path.file_stem().unwrap().to_str().unwrap()
        );
        let conflicting_thumb_path_as_dir = thumb_storage_dir.join(expected_thumb_filename);

        fs::create_dir_all(&conflicting_thumb_path_as_dir)
            .expect("Test setup: failed to create conflicting directory at thumbnail file path");
        assert!(
            conflicting_thumb_path_as_dir.is_dir(),
            "Test setup: conflicting path should be a directory"
        );

        let thumbnail_service = new_test_thumbnail_service();

        let result = thumbnail_service.convert_to_full_webp(&source_image_path, &thumb_storage_dir);

        assert!(
            result.is_err(),
            "Expected an error when target thumbnail path is a directory, but got Ok"
        );

        match result.err().unwrap() {
            ThumbnailServiceError::ImageSave(returned_path, image_error_details) => {
                assert_eq!(
                    returned_path, conflicting_thumb_path_as_dir,
                    "The path in ImageSave error should match the conflicting thumbnail path"
                );
                eprintln!(
                    "Correctly failed with ImageSave for path '{}': {}",
                    returned_path.display(),
                    image_error_details.message
                );
                assert!(!image_error_details.message.is_empty());
            }
            other_error => {
                panic!(
                    "Expected ThumbnailServiceError::ImageSave, but got {:?}",
                    other_error
                );
            }
        }
    }

    #[test]
    fn test_remove_thumbnails_success() {
        let env = setup_thumbnail_test_env(); // Use shared setup for TempDir
        let data_dir_root = env.temp_dir.path(); // Base path for test operations
        let image_filename = "test_image_for_removal.png";
        let image_map_name = "map_for_removal";

        let thumb_storage_dir = data_dir_root.join(image_map_name).join(".thumbnails");
        fs::create_dir_all(&thumb_storage_dir)
            .expect("Test setup: failed to create thumb_storage_dir");

        let source_image_path = env.source_dir.join(image_filename);
        let image_stem = source_image_path.file_stem().unwrap().to_str().unwrap();

        // Create a single full-size WebP file instead of multiple sized thumbnails
        let webp_path = thumb_storage_dir.join(format!("{}.webp", image_stem));
        File::create(&webp_path).expect("Test setup: failed to create dummy WebP file");
        assert!(webp_path.exists());
        let expected_paths = vec![webp_path];

        let mut thumbnail_service = new_test_thumbnail_service();
        let result = thumbnail_service.remove_thumbnails_for_image(
            image_filename,
            image_map_name,
            data_dir_root,
        );

        assert!(
            result.is_ok(),
            "remove_thumbnails_for_image failed: {:?}",
            result.err()
        );
        for path in expected_paths {
            assert!(!path.exists(), "Thumbnail file {:?} was not removed", path);
        }
    }

    #[test]
    fn test_remove_thumbnails_io_error() {
        let env = setup_thumbnail_test_env(); // Use shared setup for TempDir
        let data_dir_root = env.temp_dir.path().to_path_buf(); // Base path for test operations
        let image_filename = "my_test_image_lock.jpg";
        let image_map_name = "map_lock_test";

        let image_stem = Path::new(image_filename)
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap();

        // Construct the directory where thumbnails are expected by remove_thumbnails_for_image
        let actual_thumb_storage_dir = data_dir_root.join(image_map_name).join(".thumbnails");

        fs::create_dir_all(&actual_thumb_storage_dir)
            .expect("Test setup: failed to create actual_thumb_storage_dir");

        // With the new approach, we have a single full-size WebP file
        let webp_path = actual_thumb_storage_dir.join(format!("{}.webp", image_stem));

        File::create(&webp_path).expect("Test setup: failed to create dummy WebP file");

        assert!(webp_path.exists());

        // Lock the file by opening it with exclusive access (share_mode(0))
        #[cfg(windows)]
        let _locked_file_handle = OpenOptions::new()
            .read(true)
            .share_mode(0) // Exclusive lock on Windows
            .open(&webp_path)
            .expect("Test setup: failed to open (lock) the WebP file with exclusive access");

        // For non-Windows, fall back to a simple open, hoping it provides some lock,
        // or acknowledge this test might be less effective.
        #[cfg(not(windows))]
        let _locked_file_handle =
            File::open(&webp_path).expect("Test setup: failed to open (lock) the WebP file");

        let mut thumbnail_service = new_test_thumbnail_service(); // Made mut
        let result = thumbnail_service.remove_thumbnails_for_image(
            image_filename, // Pass &str directly
            image_map_name, // Pass &str directly
            &data_dir_root, // PathBuf to &Path
        );

        assert!(
            result.is_err(),
            "Expected remove_thumbnails_for_image to fail due to locked file"
        );

        // The _locked_file_handle goes out of scope here (or at end of test),
        // releasing the lock. No explicit cleanup of permissions needed for this locking mechanism.

        match result.err().unwrap() {
            ThumbnailServiceError::FileRemoval(returned_path, io_error_details) => {
                assert_eq!(
                    returned_path, webp_path,
                    "The path in FileRemoval error should match the locked thumbnail path"
                );
                eprintln!(
                    "Correctly failed with FileRemoval for path '{}': Kind: {:?}, Msg: {}",
                    returned_path.display(),
                    io_error_details.kind,
                    io_error_details.message
                );
                let msg_lower = io_error_details.message.to_lowercase();
                assert!(
                    io_error_details.kind == std::io::ErrorKind::PermissionDenied
                        || msg_lower.contains("being used by another process")
                        || msg_lower.contains("access is denied"),
                    "Unexpected IO error kind or message for locked file: {:?} - {}",
                    io_error_details.kind,
                    io_error_details.message
                );
            }
            other_error => {
                panic!(
                    "Expected ThumbnailServiceError::FileRemoval, but got {:?}",
                    other_error
                );
            }
        }

        // With the new single full-size WebP approach, we only have one file to check
        assert!(webp_path.exists(), "Locked WebP file should still exist");
    }

    #[test]
    fn test_convert_to_webp_at_path_success() {
        let env = setup_thumbnail_test_env();
        let service = new_test_thumbnail_service(); // Get an instance of ConcreteThumbnailService

        // Create a test image
        let original_image_width = 1024;
        let original_image_height = 768;
        let source_image_path = create_dummy_image_file(
            &env.source_dir,
            "test_convert_at_path.png",
            original_image_width,
            original_image_height,
        );

        // Create destination path
        let dest_webp_path = env.output_dir.join("converted_direct.webp");

        // Test convert_to_webp_at_path
        let result = service.convert_to_webp_at_path(&source_image_path, &dest_webp_path);

        // Verify conversion succeeded
        assert!(
            result.is_ok(),
            "convert_to_webp_at_path failed: {:?}",
            result.err()
        );

        // Verify that the WebP file was created at the specified location
        assert!(
            dest_webp_path.exists(),
            "WebP file was not created at the specified location"
        );

        // Verify the saved file is actually a WebP image
        let webp_bytes = std::fs::read(&dest_webp_path).expect("Failed to read WebP file");
        let format = image::guess_format(&webp_bytes).expect("Failed to determine image format");
        assert_eq!(
            format,
            image::ImageFormat::WebP,
            "Saved image is not in WebP format"
        );
    }

    #[test]
    fn test_concrete_service_convert_to_full_webp_success() {
        let env = setup_thumbnail_test_env();
        let service = new_test_thumbnail_service(); // Get an instance of ConcreteThumbnailService

        let original_image_width = 1920;
        let original_image_height = 1080;
        let source_image_path = create_dummy_image_file(
            &env.source_dir,
            "test_image_concrete.png",
            original_image_width,
            original_image_height,
        );

        // Test convert_to_full_webp
        let result = service.convert_to_full_webp(&source_image_path, &env.output_dir);

        assert!(
            result.is_ok(),
            "convert_to_full_webp failed: {:?}",
            result.err()
        );

        let webp_path = result.unwrap();
        assert!(
            webp_path.exists(),
            "WebP file does not exist at {:?}",
            webp_path
        );

        // Verify it's a valid image
        let img = image::open(&webp_path).expect("Failed to open generated WebP file");
        let (width, height) = img.dimensions();

        // Full-size WebP may be capped at a maximum width/height, but shouldn't be larger than original
        assert!(
            width <= original_image_width && height <= original_image_height,
            "WebP dimensions ({},{}) are larger than original ({},{}) for file {:?}",
            width,
            height,
            original_image_width,
            original_image_height,
            webp_path
        );

        // Check file extension is .webp
        assert_eq!(
            webp_path.extension().unwrap(),
            "webp",
            "File extension should be .webp"
        );

        // Check the filename pattern (no size suffix)
        let expected_filename = format!(
            "{}.webp",
            source_image_path.file_stem().unwrap().to_str().unwrap()
        );
        assert_eq!(
            webp_path.file_name().unwrap().to_str().unwrap(),
            expected_filename,
            "Full-size WebP filename should have no size suffix"
        );
    }

    #[test]
    fn test_convert_to_full_webp_corrupt_image() {
        let env = setup_thumbnail_test_env();
        let service = new_test_thumbnail_service();

        // Create a corrupt image file (just text data, not valid image format)
        let corrupt_image_path = env.source_dir.join("corrupt_image.png");
        fs::write(&corrupt_image_path, "This is not valid image data")
            .expect("Failed to create corrupt test file");

        // Try to convert the corrupt image
        let result = service.convert_to_full_webp(&corrupt_image_path, &env.output_dir);

        // Should fail with ImageOpen error
        assert!(
            result.is_err(),
            "Expected error when converting corrupt image"
        );
        match result.err().unwrap() {
            ThumbnailServiceError::ImageOpen(path, _) => {
                assert_eq!(
                    path, corrupt_image_path,
                    "Error path should match corrupt image path"
                );
            }
            other_error => panic!("Expected ImageOpen error, got {:?}", other_error),
        }
    }

    #[test]
    fn test_convert_to_full_webp_edge_case_tiny_image() {
        let env = setup_thumbnail_test_env();
        let service = new_test_thumbnail_service();

        // Create a very small 1x1 pixel image
        let tiny_image_path = create_dummy_image_file(
            &env.source_dir,
            "tiny_image.png",
            1, // Width: 1 pixel
            1, // Height: 1 pixel
        );

        // Convert the tiny image
        let result = service.convert_to_full_webp(&tiny_image_path, &env.output_dir);

        // Should succeed despite tiny dimensions
        assert!(
            result.is_ok(),
            "Failed to convert tiny 1x1 image: {:?}",
            result.err()
        );

        let webp_path = result.unwrap();
        assert!(
            webp_path.exists(),
            "WebP output for tiny image doesn't exist"
        );

        // Verify dimensions were preserved
        if let Ok(img) = image::open(&webp_path) {
            let (width, height) = img.dimensions();
            assert_eq!(width, 1, "Tiny WebP should maintain 1px width");
            assert_eq!(height, 1, "Tiny WebP should maintain 1px height");
        } else {
            panic!("Failed to open generated WebP for tiny image");
        }
    }

    #[test]
    fn test_convert_to_full_webp_large_image() {
        let env = setup_thumbnail_test_env();
        let service = new_test_thumbnail_service();

        // Create a very large image (8K resolution)
        let large_width = 7680;
        let large_height = 4320;
        let large_image_path = create_dummy_image_file(
            &env.source_dir,
            "large_image.png",
            large_width,
            large_height,
        );

        // Convert the large image
        let result = service.convert_to_full_webp(&large_image_path, &env.output_dir);

        // Should successfully handle large images
        assert!(
            result.is_ok(),
            "Failed to convert large image: {:?}",
            result.err()
        );

        let webp_path = result.unwrap();
        assert!(
            webp_path.exists(),
            "WebP output for large image doesn't exist"
        );

        // Verify image dimensions were preserved
        if let Ok(img) = image::open(&webp_path) {
            let (width, height) = img.dimensions();
            // Large images might be resized in the process, but should maintain aspect ratio
            if width != large_width {
                let expected_height = (height as f32 * (large_width as f32 / width as f32)) as u32;
                assert!(
                    (height as i32 - expected_height as i32).abs() <= 1,
                    "Aspect ratio not maintained for large image conversion"
                );
            } else {
                assert_eq!(width, large_width, "Large WebP should maintain width");
                assert_eq!(height, large_height, "Large WebP should maintain height");
            }
        } else {
            panic!("Failed to open generated WebP for large image");
        }
    }
}
