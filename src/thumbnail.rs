use std::fs;
use std::path::{Path, PathBuf};
use image::{DynamicImage, GenericImageView, ImageFormat, imageops::FilterType};

/// Allowed thumbnail widths (pixels). Should be divisors of 1920 for best quality.
pub const ALLOWED_THUMB_SIZES: [u32; 5] = [1920, 1440, 960, 720, 480];

/// Returns the path for a thumbnail of a given image at a given size
pub fn thumbnail_path(image_path: &Path, thumb_dir: &Path, size: u32) -> PathBuf {
    let stem = image_path.file_stem().unwrap().to_string_lossy();
    let ext = "png"; // Always save thumbnails as png
    thumb_dir.join(format!("{}_{}.{}", stem, size, ext))
}

/// Generates all allowed thumbnails for an image (call this on upload)
pub fn generate_all_thumbnails(orig_img_path: &Path, thumb_dir: &Path) {
    if let Ok(img) = image::open(orig_img_path) {
        let aspect = img.height() as f32 / img.width() as f32;
        fs::create_dir_all(thumb_dir).ok();
        for &size in ALLOWED_THUMB_SIZES.iter() {
            let thumb = img.resize(size, (size as f32 * aspect) as u32, FilterType::Lanczos3);
            let thumb_path = thumbnail_path(orig_img_path, thumb_dir, size);
            let _ = thumb.save_with_format(&thumb_path, ImageFormat::Png);
        }
    }
}

/// Returns the thumbnail path if it exists
pub fn get_thumbnail(orig_img_path: &Path, thumb_dir: &Path, size: u32) -> Option<PathBuf> {
    let thumb_path = thumbnail_path(orig_img_path, thumb_dir, size);
    if thumb_path.exists() {
        Some(thumb_path)
    } else {
        None
    }
}
