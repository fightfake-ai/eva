//! Convert between planar YUV 4:2:0 frames and Eva macroblock dump files.
//!
//! Eva stores one macroblock per slot in row-major macroblock order (left→right,
//! top→bottom). Each slot is 16×16 luma + 8×8 U + 8×8 V bytes in separate files:
//! `orig_y_enc`, `orig_u_enc`, `orig_v_enc`.

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use crate::edit::constraints::{Brightness, BrightnessCfg, EditGadget};
use crate::encode::Matrix;

pub const MB_Y_BYTES: usize = 256;
pub const MB_UV_BYTES: usize = 64;

fn validate_dims(width: usize, height: usize) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err("width and height must be positive".into());
    }
    if width % 16 != 0 || height % 16 != 0 {
        return Err(format!("width and height must be multiples of 16 (got {width}×{height})"));
    }
    Ok(())
}

/// Number of macroblocks in one frame.
pub fn macroblocks_per_frame(width: usize, height: usize) -> Result<usize, String> {
    validate_dims(width, height)?;
    Ok((width / 16) * (height / 16))
}

/// Bytes in one planar YUV 4:2:0 frame.
pub fn yuv420_frame_bytes(width: usize, height: usize) -> Result<usize, String> {
    validate_dims(width, height)?;
    Ok(width * height * 3 / 2)
}

/// Macroblock column/row (in 16×16 units) for linear index `mb_idx`.
pub fn macroblock_xy(width: usize, mb_idx: usize) -> (usize, usize) {
    let cols = width / 16;
    let mb_x = mb_idx % cols;
    let mb_y = mb_idx / cols;
    (mb_x, mb_y)
}

/// Copy one 16×16 luma macroblock from a full Y plane into `out` (256 bytes, row-major).
fn extract_y_block(y_plane: &[u8], width: usize, mb_x: usize, mb_y: usize, out: &mut [u8; MB_Y_BYTES]) {
    for row in 0..16 {
        let y = mb_y * 16 + row;
        let src = &y_plane[y * width + mb_x * 16..y * width + mb_x * 16 + 16];
        out[row * 16..row * 16 + 16].copy_from_slice(src);
    }
}

/// Copy one 8×8 chroma macroblock from a chroma plane into `out`.
fn extract_uv_block(
    plane: &[u8],
    chroma_width: usize,
    mb_x: usize,
    mb_y: usize,
    out: &mut [u8; MB_UV_BYTES],
) {
    for row in 0..8 {
        let y = mb_y * 8 + row;
        let src = &plane[y * chroma_width + mb_x * 8..y * chroma_width + mb_x * 8 + 8];
        out[row * 8..row * 8 + 8].copy_from_slice(src);
    }
}

/// Paste one 16×16 luma macroblock into a full Y plane.
fn insert_y_block(y_plane: &mut [u8], width: usize, mb_x: usize, mb_y: usize, block: &[u8; MB_Y_BYTES]) {
    for row in 0..16 {
        let y = mb_y * 16 + row;
        y_plane[y * width + mb_x * 16..y * width + mb_x * 16 + 16]
            .copy_from_slice(&block[row * 16..row * 16 + 16]);
    }
}

fn insert_uv_block(
    plane: &mut [u8],
    chroma_width: usize,
    mb_x: usize,
    mb_y: usize,
    block: &[u8; MB_UV_BYTES],
) {
    for row in 0..8 {
        let y = mb_y * 8 + row;
        plane[y * chroma_width + mb_x * 8..y * chroma_width + mb_x * 8 + 8]
            .copy_from_slice(&block[row * 8..row * 8 + 8]);
    }
}

/// Planar YUV 4:2:0 bytes (`num_frames` concatenated) → Eva macroblock files layout.
pub fn yuv420_to_macroblocks(
    yuv: &[u8],
    width: usize,
    height: usize,
    num_frames: usize,
) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), String> {
    validate_dims(width, height)?;
    let frame_bytes = yuv420_frame_bytes(width, height)?;
    let expected = frame_bytes
        .checked_mul(num_frames)
        .ok_or_else(|| "frame count overflow".to_string())?;
    if yuv.len() < expected {
        return Err(format!(
            "input too short: need {expected} bytes for {num_frames} frame(s) at {width}×{height}, got {}",
            yuv.len()
        ));
    }

    let mbs = macroblocks_per_frame(width, height)?;
    let chroma_w = width / 2;
    let mut orig_y = Vec::with_capacity(mbs * num_frames * MB_Y_BYTES);
    let mut orig_u = Vec::with_capacity(mbs * num_frames * MB_UV_BYTES);
    let mut orig_v = Vec::with_capacity(mbs * num_frames * MB_UV_BYTES);

    for f in 0..num_frames {
        let base = f * frame_bytes;
        let y_plane = &yuv[base..base + width * height];
        let u_off = base + width * height;
        let u_plane = &yuv[u_off..u_off + chroma_w * (height / 2)];
        let v_off = u_off + chroma_w * (height / 2);
        let v_plane = &yuv[v_off..v_off + chroma_w * (height / 2)];

        for mb_idx in 0..mbs {
            let (mb_x, mb_y) = macroblock_xy(width, mb_idx);
            let mut y_block = [0u8; MB_Y_BYTES];
            let mut u_block = [0u8; MB_UV_BYTES];
            let mut v_block = [0u8; MB_UV_BYTES];
            extract_y_block(y_plane, width, mb_x, mb_y, &mut y_block);
            extract_uv_block(u_plane, chroma_w, mb_x, mb_y, &mut u_block);
            extract_uv_block(v_plane, chroma_w, mb_x, mb_y, &mut v_block);
            orig_y.extend_from_slice(&y_block);
            orig_u.extend_from_slice(&u_block);
            orig_v.extend_from_slice(&v_block);
        }
    }

    Ok((orig_y, orig_u, orig_v))
}

/// Eva macroblock files → planar YUV 4:2:0 bytes for `num_frames` frame(s).
///
/// If `brightness_scale` is `Some(s)`, applies [`Brightness`] per macroblock on export
/// (for previewing the edited video off-chain).
pub fn macroblocks_to_yuv420(
    orig_y: &[u8],
    orig_u: &[u8],
    orig_v: &[u8],
    width: usize,
    height: usize,
    num_frames: usize,
    brightness_scale: Option<u16>,
) -> Result<Vec<u8>, String> {
    validate_dims(width, height)?;
    let mbs = macroblocks_per_frame(width, height)?;
    let total_mbs = mbs
        .checked_mul(num_frames)
        .ok_or_else(|| "frame count overflow".to_string())?;

    let need_y = total_mbs * MB_Y_BYTES;
    let need_uv = total_mbs * MB_UV_BYTES;
    if orig_y.len() < need_y || orig_u.len() < need_uv || orig_v.len() < need_uv {
        return Err(format!(
            "macroblock files too short for {num_frames} frame(s) at {width}×{height}: \
             need y={need_y} u={need_uv} v={need_uv} bytes, got y={} u={} v={}",
            orig_y.len(),
            orig_u.len(),
            orig_v.len()
        ));
    }

    let frame_bytes = yuv420_frame_bytes(width, height)?;
    let chroma_w = width / 2;
    let mut out = vec![0u8; frame_bytes * num_frames];
    let brightness = brightness_scale.map(BrightnessCfg);

    for f in 0..num_frames {
        let frame_base = f * frame_bytes;
        let frame = &mut out[frame_base..frame_base + frame_bytes];
        let (y_plane, uv) = frame.split_at_mut(width * height);
        let (u_plane, v_plane) = uv.split_at_mut(chroma_w * (height / 2));

        for mb_idx in 0..mbs {
            let global = f * mbs + mb_idx;
            let (mb_x, mb_y) = macroblock_xy(width, mb_idx);

            let y = Matrix::<u8, 16, 16>::from_vec(
                orig_y[global * MB_Y_BYTES..(global + 1) * MB_Y_BYTES].to_vec(),
            );
            let u = Matrix::<u8, 8, 8>::from_vec(
                orig_u[global * MB_UV_BYTES..(global + 1) * MB_UV_BYTES].to_vec(),
            );
            let v = Matrix::<u8, 8, 8>::from_vec(
                orig_v[global * MB_UV_BYTES..(global + 1) * MB_UV_BYTES].to_vec(),
            );

            let (y, u, v) = if let Some(ref cfg) = brightness {
                Brightness::edit_native(&y, &u, &v, cfg)
            } else {
                (y, u, v)
            };

            let y_arr: [u8; MB_Y_BYTES] = y.iter().copied().collect::<Vec<_>>().try_into().unwrap();
            let u_arr: [u8; MB_UV_BYTES] = u.iter().copied().collect::<Vec<_>>().try_into().unwrap();
            let v_arr: [u8; MB_UV_BYTES] = v.iter().copied().collect::<Vec<_>>().try_into().unwrap();

            insert_y_block(y_plane, width, mb_x, mb_y, &y_arr);
            insert_uv_block(u_plane, chroma_w, mb_x, mb_y, &u_arr);
            insert_uv_block(v_plane, chroma_w, mb_x, mb_y, &v_arr);
        }
    }

    Ok(out)
}

/// Write `orig_y_enc`, `orig_u_enc`, `orig_v_enc` under `dir` (creates dir if needed).
pub fn write_macroblock_dir(dir: &Path, orig_y: &[u8], orig_u: &[u8], orig_v: &[u8]) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    File::create(dir.join("orig_y_enc"))?.write_all(orig_y)?;
    File::create(dir.join("orig_u_enc"))?.write_all(orig_u)?;
    File::create(dir.join("orig_v_enc"))?.write_all(orig_v)?;
    Ok(())
}

/// Read macroblock files from `dir`.
pub fn read_macroblock_dir(dir: &Path) -> std::io::Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    Ok((
        fs::read(dir.join("orig_y_enc"))?,
        fs::read(dir.join("orig_u_enc"))?,
        fs::read(dir.join("orig_v_enc"))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_frame(width: usize, height: usize) -> Vec<u8> {
        let mut yuv = vec![0u8; yuv420_frame_bytes(width, height).unwrap()];
        let chroma_w = width / 2;
        for y in 0..height {
            for x in 0..width {
                yuv[y * width + x] = ((x + y) % 256) as u8;
            }
        }
        let u_base = width * height;
        for y in 0..height / 2 {
            for x in 0..chroma_w {
                yuv[u_base + y * chroma_w + x] = (x % 256) as u8;
            }
        }
        let v_base = u_base + chroma_w * (height / 2);
        for y in 0..height / 2 {
            for x in 0..chroma_w {
                yuv[v_base + y * chroma_w + x] = (y % 256) as u8;
            }
        }
        yuv
    }

    #[test]
    fn roundtrip_yuv_macroblocks_yuv() {
        let width = 352;
        let height = 288;
        let src = synthetic_frame(width, height);
        let (y, u, v) = yuv420_to_macroblocks(&src, width, height, 1).unwrap();
        let back = macroblocks_to_yuv420(&y, &u, &v, width, height, 1, None).unwrap();
        assert_eq!(src, back);
    }

    #[test]
    fn macroblock_xy_matches_eva_indexing() {
        let width = 352;
        let (mb_x, mb_y) = macroblock_xy(width, 22);
        assert_eq!((mb_x, mb_y), (0, 1));
        let (mb_x, mb_y) = macroblock_xy(width, 23);
        assert_eq!((mb_x, mb_y), (1, 1));
    }
}
