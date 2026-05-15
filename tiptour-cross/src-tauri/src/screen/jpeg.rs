// JPEG encoding for screen frames.
//
// Gemini Live doesn't need 4K screenshots — the model downsamples its
// vision input internally anyway. We cap the longest side at 1024px and
// encode at quality 70, which keeps single frames well under 100KB even
// for content-dense screens.

use image::codecs::jpeg::JpegEncoder;
use image::{ImageBuffer, Rgba};

use super::capture::RawFrame;

const JPEG_QUALITY: u8 = 70;
const MAX_DIMENSION: u32 = 1024;

pub fn encode_frame_to_jpeg(frame: &RawFrame) -> Result<Vec<u8>, String> {
    // Convert BGRA → RGBA so the `image` crate can consume it. (image
    // doesn't ship a Bgra8 buffer type; we swap in-place per pixel.)
    let mut rgba = frame.bgra.clone();
    if rgba.len() % 4 != 0 {
        return Err(format!(
            "BGRA buffer length {} is not a multiple of 4",
            rgba.len()
        ));
    }
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }

    let buffer: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_vec(frame.width, frame.height, rgba)
            .ok_or_else(|| "Failed to construct ImageBuffer from BGRA bytes".to_string())?;

    // Aspect-preserving resize down to MAX_DIMENSION on the longest side.
    // If the frame is already smaller we skip the resize entirely.
    let dynamic = image::DynamicImage::ImageRgba8(buffer);
    let resized = if frame.width.max(frame.height) > MAX_DIMENSION {
        dynamic.resize(MAX_DIMENSION, MAX_DIMENSION, image::imageops::FilterType::Triangle)
    } else {
        dynamic
    };

    // JPEG doesn't carry alpha — convert to RGB8 first to avoid the
    // encoder having to drop the channel implicitly.
    let rgb = resized.to_rgb8();

    let mut out = Vec::with_capacity(64 * 1024);
    let mut encoder = JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY);
    encoder
        .encode_image(&rgb)
        .map_err(|error| format!("JPEG encode failed: {error}"))?;
    Ok(out)
}
