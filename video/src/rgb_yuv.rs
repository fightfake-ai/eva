//! RGB8 → planar YUV 4:2:0 (full-range BT.601 / JPEG-style).
//!
//! Used by still-image ingest. Does not pull in an image decoder — callers
//! decode PNG/JPEG themselves and pass raw RGB8 bytes.

/// Convert tightly packed RGB8 (`width * height * 3` bytes) to planar YUV420p.
///
/// `width` and `height` must be even (YUV 4:2:0). For Eva, also pass multiples
/// of 16 into [`crate::yuv420_to_macroblocks`].
pub fn rgb8_to_yuv420p(rgb: &[u8], width: usize, height: usize) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
        return Err(format!(
            "rgb8_to_yuv420p: width×height must be even and non-zero (got {width}×{height})"
        ));
    }
    if rgb.len() != width * height * 3 {
        return Err(format!(
            "rgb8_to_yuv420p: expected {} RGB bytes, got {}",
            width * height * 3,
            rgb.len()
        ));
    }

    let y_size = width * height;
    let uv_size = (width / 2) * (height / 2);
    let mut out = vec![0u8; y_size + 2 * uv_size];
    let (y_plane, rest) = out.split_at_mut(y_size);
    let (u_plane, v_plane) = rest.split_at_mut(uv_size);

    for row in 0..height {
        for col in 0..width {
            let i = (row * width + col) * 3;
            let r = rgb[i] as i32;
            let g = rgb[i + 1] as i32;
            let b = rgb[i + 2] as i32;
            let y = (77 * r + 150 * g + 29 * b + 128) >> 8;
            y_plane[row * width + col] = y.clamp(0, 255) as u8;
        }
    }

    for row in 0..(height / 2) {
        for col in 0..(width / 2) {
            let mut r_sum = 0i32;
            let mut g_sum = 0i32;
            let mut b_sum = 0i32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let px = ((row * 2 + dy) * width + (col * 2 + dx)) * 3;
                    r_sum += rgb[px] as i32;
                    g_sum += rgb[px + 1] as i32;
                    b_sum += rgb[px + 2] as i32;
                }
            }
            let r = r_sum / 4;
            let g = g_sum / 4;
            let b = b_sum / 4;
            let u = ((-43 * r - 85 * g + 128 * b + 128) >> 8) + 128;
            let v = ((128 * r - 107 * g - 21 * b + 128) >> 8) + 128;
            let idx = row * (width / 2) + col;
            u_plane[idx] = u.clamp(0, 255) as u8;
            v_plane[idx] = v.clamp(0, 255) as u8;
        }
    }

    Ok(out)
}

/// Crop dimensions down to the largest top-left multiple of 16.
pub fn crop_to_macroblock_grid(width: usize, height: usize) -> (usize, usize) {
    ((width / 16) * 16, (height / 16) * 16)
}
