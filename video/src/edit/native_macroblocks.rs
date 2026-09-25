//! Native (reference) edit transforms on Eva macroblock byte streams.
//!
//! These functions apply [`EditGadget::edit_native`] per macroblock. They do not run in-circuit
//! and do not export to planar YUV — see [`crate::macroblock_yuv::macroblocks_to_yuv420`] for that.

use crate::edit::constraints::{Brightness, BrightnessCfg, EditGadget, RedactRect, RedactRectCfg};
use crate::encode::Matrix;
use crate::macroblock_yuv::{macroblock_xy, validate_macroblock_inputs, MB_UV_BYTES, MB_Y_BYTES};

/// Apply [`Brightness::edit_native`] to every macroblock in `orig_*` byte streams.
///
/// Input and output use the same Eva macroblock layout (`orig_y_enc` / `orig_u_enc` / `orig_v_enc`
/// shape). Reference only — matches in-circuit edit at [`BrightnessCfg`](super::constraints::BrightnessCfg)(`brightness_scale`).
pub fn native_brightness_edit_macroblocks(
    orig_y: &[u8],
    orig_u: &[u8],
    orig_v: &[u8],
    width: usize,
    height: usize,
    num_frames: usize,
    brightness_scale: u16,
) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), String> {
    let (_, need_y, need_uv) =
        validate_macroblock_inputs(orig_y, orig_u, orig_v, width, height, num_frames)?;
    let total_mbs = need_y / MB_Y_BYTES;
    let cfg = BrightnessCfg(brightness_scale);

    let mut out_y = vec![0u8; need_y];
    let mut out_u = vec![0u8; need_uv];
    let mut out_v = vec![0u8; need_uv];

    for global in 0..total_mbs {
        let y = Matrix::<u8, 16, 16>::from_vec(
            orig_y[global * MB_Y_BYTES..(global + 1) * MB_Y_BYTES].to_vec(),
        );
        let u = Matrix::<u8, 8, 8>::from_vec(
            orig_u[global * MB_UV_BYTES..(global + 1) * MB_UV_BYTES].to_vec(),
        );
        let v = Matrix::<u8, 8, 8>::from_vec(
            orig_v[global * MB_UV_BYTES..(global + 1) * MB_UV_BYTES].to_vec(),
        );

        let (y, u, v) = Brightness::edit_native(&y, &u, &v, &cfg);

        let y_arr: [u8; MB_Y_BYTES] = y.iter().copied().collect::<Vec<_>>().try_into().unwrap();
        let u_arr: [u8; MB_UV_BYTES] = u.iter().copied().collect::<Vec<_>>().try_into().unwrap();
        let v_arr: [u8; MB_UV_BYTES] = v.iter().copied().collect::<Vec<_>>().try_into().unwrap();

        out_y[global * MB_Y_BYTES..(global + 1) * MB_Y_BYTES].copy_from_slice(&y_arr);
        out_u[global * MB_UV_BYTES..(global + 1) * MB_UV_BYTES].copy_from_slice(&u_arr);
        out_v[global * MB_UV_BYTES..(global + 1) * MB_UV_BYTES].copy_from_slice(&v_arr);
    }

    Ok((out_y, out_u, out_v))
}

/// Apply [`RedactRect::edit_native`] per macroblock for a fixed pixel rectangle
/// on frames `[frame_start, frame_end)`.
pub fn native_redact_edit_macroblocks(
    orig_y: &[u8],
    orig_u: &[u8],
    orig_v: &[u8],
    width: usize,
    height: usize,
    num_frames: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    frame_start: usize,
    frame_end: usize,
    fill_y: u8,
) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), String> {
    let (mbs_per_frame, need_y, need_uv) =
        validate_macroblock_inputs(orig_y, orig_u, orig_v, width, height, num_frames)?;
    let total_mbs = need_y / MB_Y_BYTES;
    let f_end = frame_end.min(num_frames);

    let mut out_y = vec![0u8; need_y];
    let mut out_u = vec![0u8; need_uv];
    let mut out_v = vec![0u8; need_uv];

    for global in 0..total_mbs {
        let frame = global / mbs_per_frame;
        let (mb_x, mb_y) = macroblock_xy(width, global % mbs_per_frame);
        let origin_x = mb_x * 16;
        let origin_y = mb_y * 16;
        let in_frame_range = frame >= frame_start && frame < f_end;
        let x1 = x.min(width);
        let y1 = y.min(height);
        let x2 = x.saturating_add(w).min(width);
        let y2 = y.saturating_add(h).min(height);
        let cfg = RedactRectCfg::from_rectangle(
            origin_x,
            origin_y,
            in_frame_range,
            x1,
            y1,
            x2,
            y2,
            fill_y,
        );

        let y_mat = Matrix::<u8, 16, 16>::from_vec(
            orig_y[global * MB_Y_BYTES..(global + 1) * MB_Y_BYTES].to_vec(),
        );
        let u_mat = Matrix::<u8, 8, 8>::from_vec(
            orig_u[global * MB_UV_BYTES..(global + 1) * MB_UV_BYTES].to_vec(),
        );
        let v_mat = Matrix::<u8, 8, 8>::from_vec(
            orig_v[global * MB_UV_BYTES..(global + 1) * MB_UV_BYTES].to_vec(),
        );

        let (y_out, u_out, v_out) = RedactRect::edit_native(&y_mat, &u_mat, &v_mat, &cfg);

        let y_arr: [u8; MB_Y_BYTES] = y_out
            .iter()
            .copied()
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let u_arr: [u8; MB_UV_BYTES] = u_out
            .iter()
            .copied()
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let v_arr: [u8; MB_UV_BYTES] = v_out
            .iter()
            .copied()
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();

        out_y[global * MB_Y_BYTES..(global + 1) * MB_Y_BYTES].copy_from_slice(&y_arr);
        out_u[global * MB_UV_BYTES..(global + 1) * MB_UV_BYTES].copy_from_slice(&u_arr);
        out_v[global * MB_UV_BYTES..(global + 1) * MB_UV_BYTES].copy_from_slice(&v_arr);
    }

    Ok((out_y, out_u, out_v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macroblock_yuv::{macroblocks_to_yuv420, yuv420_to_macroblocks};

    fn synthetic_frame(width: usize, height: usize) -> Vec<u8> {
        let mut yuv = vec![0u8; width * height * 3 / 2];
        let chroma_w = width / 2;
        for y in 0..height {
            for x in 0..width {
                yuv[y * width + x] = (x % 256) as u8;
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
    fn native_brightness_edit_then_export() {
        let width = 352;
        let height = 288;
        let src = synthetic_frame(width, height);
        let (y, u, v) = yuv420_to_macroblocks(&src, width, height, 1).unwrap();
        let (edited_y, edited_u, edited_v) =
            native_brightness_edit_macroblocks(&y, &u, &v, width, height, 1, 416).unwrap();
        assert_ne!(edited_y, y);
        assert_eq!(edited_u, u);
        assert_eq!(edited_v, v);
        let exported =
            macroblocks_to_yuv420(&edited_y, &edited_u, &edited_v, width, height, 1).unwrap();
        assert_eq!(exported.len(), src.len());
    }

    #[test]
    fn native_redact_fills_aligned_box_only() {
        let width = 32;
        let height = 32;
        let src = synthetic_frame(width, height);
        let (y, u, v) = yuv420_to_macroblocks(&src, width, height, 1).unwrap();
        let (ey, eu, ev) = super::native_redact_edit_macroblocks(
            &y, &u, &v, width, height, 1, 0, 0, 16, 16, 0, 1, 0,
        )
        .unwrap();
        assert_eq!(&ey[..MB_Y_BYTES], &[0u8; MB_Y_BYTES]);
        assert_eq!(&ey[MB_Y_BYTES..], &y[MB_Y_BYTES..]);
        assert_eq!(&eu[MB_UV_BYTES..], &u[MB_UV_BYTES..]);
        assert_eq!(&ev[MB_UV_BYTES..], &v[MB_UV_BYTES..]);
    }
}
