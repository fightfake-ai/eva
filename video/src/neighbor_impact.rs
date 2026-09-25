//! Measure how far a redact + (optional) re-encode smears, per macroblock.
//!
//! Two different questions:
//!
//! 1. **Pixel gadget** — compare orig YUV vs redacted YUV. Only gadget MBs
//!    should differ. This is a sanity check, not an encode experiment.
//! 2. **Re-encode contamination** — compare two decoded YUVs (orig encoded vs
//!    redacted-then-encoded). Counts outside the geometric island are the
//!    *ffmpeg full-re-encode* leak, an upper bound on splice `I`. Syntax-accurate
//!    membership needs JM dumps or a bitstream parser (see `syntax_mb_changed`).

use crate::island::{island_counts, mb_role, IslandSpec, MbRole};
use crate::macroblock_yuv::{macroblock_xy, yuv420_frame_bytes, MB_UV_BYTES, MB_Y_BYTES};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RoleImpact {
    pub n_mbs: usize,
    pub n_changed: usize,
    pub max_abs_y: u8,
}

impl RoleImpact {
    fn observe(&mut self, max_abs_y: u8, threshold: u8) {
        self.n_mbs += 1;
        if max_abs_y > threshold {
            self.n_changed += 1;
        }
        if max_abs_y > self.max_abs_y {
            self.max_abs_y = max_abs_y;
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImpactReport {
    pub threshold: u8,
    pub gadget: RoleImpact,
    pub halo: RoleImpact,
    pub skip_in_edited_frames: RoleImpact,
    pub skip_pre: RoleImpact,
    pub skip_post: RoleImpact,
}

impl ImpactReport {
    pub fn island_changed(&self) -> usize {
        self.gadget.n_changed + self.halo.n_changed
    }

    /// MBs that changed *outside* geometric `I`. For pixel-redact this should be 0.
    /// For two independent x264 encodes it is the GOP / rate-control smear.
    pub fn outside_island_changed(&self) -> usize {
        self.skip_in_edited_frames.n_changed + self.skip_pre.n_changed + self.skip_post.n_changed
    }
}

fn luma_max_abs(
    a: &[u8],
    b: &[u8],
    width: usize,
    frame: usize,
    mb_x: usize,
    mb_y: usize,
    frame_bytes: usize,
) -> u8 {
    let y_base = frame * frame_bytes;
    let mut max_d = 0u8;
    for row in 0..16 {
        let y = mb_y * 16 + row;
        let off = y_base + y * width + mb_x * 16;
        for col in 0..16 {
            let d = a[off + col].abs_diff(b[off + col]);
            if d > max_d {
                max_d = d;
            }
        }
    }
    max_d
}

/// Per-MB luma max-abs between two planar YUV 4:2:0 clips, classified by island role.
pub fn compare_yuv_to_island(
    orig: &[u8],
    other: &[u8],
    spec: &IslandSpec,
    threshold: u8,
) -> Result<ImpactReport, String> {
    spec.validate()?;
    let frame_bytes = yuv420_frame_bytes(spec.width, spec.height)?;
    let need = frame_bytes
        .checked_mul(spec.num_frames)
        .ok_or_else(|| "frame count overflow".to_string())?;
    if orig.len() < need || other.len() < need {
        return Err(format!(
            "YUV too short: need {need} bytes, got orig={} other={}",
            orig.len(),
            other.len()
        ));
    }
    let mbs = spec.mbs_per_frame()?;
    let mut report = ImpactReport {
        threshold,
        ..Default::default()
    };
    for frame in 0..spec.num_frames {
        for mb in 0..mbs {
            let global = frame * mbs + mb;
            let (mb_x, mb_y) = macroblock_xy(spec.width, mb);
            let d = luma_max_abs(orig, other, spec.width, frame, mb_x, mb_y, frame_bytes);
            let slot = match mb_role(spec, global)? {
                MbRole::Gadget => &mut report.gadget,
                MbRole::Halo => &mut report.halo,
                MbRole::SkipInEditedFrames => &mut report.skip_in_edited_frames,
                MbRole::SkipPre => &mut report.skip_pre,
                MbRole::SkipPost => &mut report.skip_post,
            };
            slot.observe(d, threshold);
        }
    }
    debug_assert_eq!(
        report.gadget.n_mbs
            + report.halo.n_mbs
            + report.skip_in_edited_frames.n_mbs
            + report.skip_pre.n_mbs
            + report.skip_post.n_mbs,
        island_counts(spec)?.total
    );
    Ok(report)
}

/// Which Eva syntax MBs differ (`pred` / `coeff` / `type_enc` bytewise).
///
/// `changed[i]` is true if any of the per-MB slices differ. Lengths must match.
pub fn syntax_mb_changed(
    pred_y_a: &[u8],
    pred_y_b: &[u8],
    coeff_y_a: &[u8],
    coeff_y_b: &[u8],
    type_a: &[u8],
    type_b: &[u8],
) -> Result<Vec<bool>, String> {
    if pred_y_a.len() != pred_y_b.len()
        || coeff_y_a.len() != coeff_y_b.len()
        || type_a.len() != type_b.len()
    {
        return Err("syntax dump lengths differ".into());
    }
    if pred_y_a.len() % MB_Y_BYTES != 0 || type_a.len() % 6 != 0 {
        return Err("syntax dumps are not whole macroblocks".into());
    }
    let n = pred_y_a.len() / MB_Y_BYTES;
    if coeff_y_a.len() / MB_Y_BYTES != n || type_a.len() / 6 != n {
        return Err("pred/coeff/type macroblock counts differ".into());
    }
    let mut out = vec![false; n];
    for i in 0..n {
        let py = i * MB_Y_BYTES;
        let ty = i * 6;
        out[i] = pred_y_a[py..py + MB_Y_BYTES] != pred_y_b[py..py + MB_Y_BYTES]
            || coeff_y_a[py..py + MB_Y_BYTES] != coeff_y_b[py..py + MB_Y_BYTES]
            || type_a[ty..ty + 6] != type_b[ty..ty + 6];
    }
    let _ = MB_UV_BYTES;
    Ok(out)
}

/// Classify a `syntax_mb_changed` mask with an island spec (same length as total MBs).
pub fn classify_syntax_changes(
    changed: &[bool],
    spec: &IslandSpec,
) -> Result<ImpactReport, String> {
    if changed.len() != spec.total_mbs()? {
        return Err(format!(
            "changed mask length {} != total MBs {}",
            changed.len(),
            spec.total_mbs()?
        ));
    }
    let mut report = ImpactReport {
        threshold: 0,
        ..Default::default()
    };
    for (g, &ch) in changed.iter().enumerate() {
        let slot = match mb_role(spec, g)? {
            MbRole::Gadget => &mut report.gadget,
            MbRole::Halo => &mut report.halo,
            MbRole::SkipInEditedFrames => &mut report.skip_in_edited_frames,
            MbRole::SkipPre => &mut report.skip_pre,
            MbRole::SkipPost => &mut report.skip_post,
        };
        slot.observe(u8::from(ch), 0);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::native_macroblocks::native_redact_edit_macroblocks;
    use crate::island::PixelRect;
    use crate::macroblock_yuv::{macroblocks_to_yuv420, yuv420_to_macroblocks};

    fn gradient_yuv(width: usize, height: usize, frames: usize) -> Vec<u8> {
        let fb = width * height * 3 / 2;
        let mut yuv = vec![128u8; fb * frames];
        for f in 0..frames {
            let base = f * fb;
            for y in 0..height {
                for x in 0..width {
                    yuv[base + y * width + x] = ((x + y + f * 3) % 256) as u8;
                }
            }
        }
        yuv
    }

    #[test]
    fn pixel_redact_only_touches_gadget() {
        let spec = IslandSpec {
            width: 64,
            height: 48,
            num_frames: 3,
            frame_start: 1,
            frame_end: 2,
            rect: PixelRect {
                x: 16,
                y: 16,
                w: 16,
                h: 16,
            },
            halo_mbs: 1,
        };
        let orig = gradient_yuv(spec.width, spec.height, spec.num_frames);
        let (y, u, v) =
            yuv420_to_macroblocks(&orig, spec.width, spec.height, spec.num_frames).unwrap();
        let (ey, eu, ev) = native_redact_edit_macroblocks(
            &y,
            &u,
            &v,
            spec.width,
            spec.height,
            spec.num_frames,
            spec.rect.x,
            spec.rect.y,
            spec.rect.w,
            spec.rect.h,
            spec.frame_start,
            spec.frame_end,
            0,
        )
        .unwrap();
        let edited =
            macroblocks_to_yuv420(&ey, &eu, &ev, spec.width, spec.height, spec.num_frames).unwrap();
        let r = compare_yuv_to_island(&orig, &edited, &spec, 0).unwrap();
        assert_eq!(r.gadget.n_changed, r.gadget.n_mbs);
        assert!(r.gadget.n_mbs > 0);
        assert_eq!(r.halo.n_changed, 0);
        assert_eq!(r.outside_island_changed(), 0);
    }

    #[test]
    fn syntax_mask_classifies() {
        let spec = IslandSpec {
            width: 16,
            height: 16,
            num_frames: 2,
            frame_start: 0,
            frame_end: 1,
            rect: PixelRect {
                x: 0,
                y: 0,
                w: 16,
                h: 16,
            },
            halo_mbs: 0,
        };
        let changed = vec![true, false];
        let r = classify_syntax_changes(&changed, &spec).unwrap();
        assert_eq!(r.gadget.n_changed, 1);
        assert_eq!(r.skip_post.n_changed, 0);
        assert_eq!(r.skip_post.n_mbs, 1);
    }

    #[test]
    fn skip_macroblock_pixel_leaves_match_capture() {
        use crate::merkle::capture_pixel_tree;

        let spec = IslandSpec {
            width: 32,
            height: 32,
            num_frames: 1,
            frame_start: 0,
            frame_end: 1,
            rect: PixelRect {
                x: 0,
                y: 0,
                w: 16,
                h: 16,
            },
            halo_mbs: 1,
        };
        let orig = gradient_yuv(spec.width, spec.height, spec.num_frames);
        let (y, u, v) =
            yuv420_to_macroblocks(&orig, spec.width, spec.height, spec.num_frames).unwrap();
        let (ey, eu, ev) = native_redact_edit_macroblocks(
            &y,
            &u,
            &v,
            spec.width,
            spec.height,
            spec.num_frames,
            0,
            0,
            16,
            16,
            0,
            1,
            0,
        )
        .unwrap();
        let t0 = capture_pixel_tree(&y, &u, &v).unwrap();
        let t1 = capture_pixel_tree(&ey, &eu, &ev).unwrap();
        assert_ne!(t0.root(), t1.root());
        assert_ne!(t0.proof(0).unwrap().leaf, t1.proof(0).unwrap().leaf);
        // Halo MBs keep original pixels (syntax may change later; not in this tree).
        for i in 1..4 {
            assert_eq!(
                t0.proof(i).unwrap().leaf,
                t1.proof(i).unwrap().leaf,
                "halo/skip pixel leaf {i} should be identity"
            );
            assert!(t0.proof(i).unwrap().verify(&t0.root()));
        }
        assert!(!t0.proof(1).unwrap().verify(&t1.root()));
    }
}
