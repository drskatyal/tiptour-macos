// dHash perceptual hash for skipping near-identical screen frames.
//
// Port of TipTour/ScreenshotPerceptualHash.swift. The algorithm is the
// canonical 9x8 difference hash: downscale to 9x8 grayscale, then for
// each row compare neighbouring pixels left-to-right; bit i is set if
// pixel[i] > pixel[i+1]. That produces 8 comparisons per row × 8 rows
// = 64 bits packed into a u64.
//
// Default same-scene threshold of 5 bits matches the Swift constant.

use super::capture::RawFrame;

pub const DEFAULT_SAME_SCENE_THRESHOLD: u32 = 5;

const DOWNSCALED_WIDTH: usize = 9;
const DOWNSCALED_HEIGHT: usize = 8;

pub fn perceptual_hash(frame: &RawFrame) -> u64 {
    let downscaled = downscale_to_grayscale(frame);
    pack_dhash(&downscaled)
}

fn downscale_to_grayscale(frame: &RawFrame) -> [u8; DOWNSCALED_WIDTH * DOWNSCALED_HEIGHT] {
    // Nearest-neighbour downscale into the 9x8 target. Cheap and good
    // enough — dHash is robust to interpolation choice because we only
    // care about ordering between neighbouring cells.
    let mut out = [0u8; DOWNSCALED_WIDTH * DOWNSCALED_HEIGHT];
    if frame.width == 0 || frame.height == 0 || frame.bgra.len() < (frame.width * frame.height * 4) as usize {
        return out;
    }
    let src_width = frame.width as usize;
    let src_height = frame.height as usize;
    for target_row in 0..DOWNSCALED_HEIGHT {
        let source_y = (target_row * src_height) / DOWNSCALED_HEIGHT;
        for target_column in 0..DOWNSCALED_WIDTH {
            let source_x = (target_column * src_width) / DOWNSCALED_WIDTH;
            let pixel_offset = (source_y * src_width + source_x) * 4;
            let b = frame.bgra[pixel_offset] as u32;
            let g = frame.bgra[pixel_offset + 1] as u32;
            let r = frame.bgra[pixel_offset + 2] as u32;
            // Luma709 approximation — bit-shift weighted average.
            let luma = ((r * 54 + g * 183 + b * 19) >> 8) as u8;
            out[target_row * DOWNSCALED_WIDTH + target_column] = luma;
        }
    }
    out
}

fn pack_dhash(pixels: &[u8; DOWNSCALED_WIDTH * DOWNSCALED_HEIGHT]) -> u64 {
    let mut hash: u64 = 0;
    for row in 0..DOWNSCALED_HEIGHT {
        for column in 0..(DOWNSCALED_WIDTH - 1) {
            let left = pixels[row * DOWNSCALED_WIDTH + column];
            let right = pixels[row * DOWNSCALED_WIDTH + column + 1];
            if left > right {
                let bit_position = row * (DOWNSCALED_WIDTH - 1) + column;
                hash |= 1u64 << bit_position;
            }
        }
    }
    hash
}

pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

pub fn should_skip(previous: Option<u64>, current: u64, threshold: u32) -> bool {
    match previous {
        Some(previous_hash) => hamming_distance(previous_hash, current) <= threshold,
        None => false,
    }
}
