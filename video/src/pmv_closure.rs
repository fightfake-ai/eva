//! P-frame motion-vector closure: expand geometric island `I` by inter prediction reach.
//!
//! Reads `mv_enc` (16 L0 4×4 blocks per MB) plus `type_enc` from JM dumps. Starting from
//! gadget ∪ halo, any inter MB whose L0 MV touches a seeded MB joins `I` (fixpoint).

use std::collections::HashSet;
use std::path::Path;

use crate::island::{island_indices, mb_role, IslandSpec, MbRole};
use crate::macroblock_yuv::MB_Y_BYTES;
use crate::neighbor_experiment::redact_window;

pub const MV_BYTES_PER_MB: usize = 96;
pub const BLOCKS_PER_MB: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockMotion {
    pub ref_frame: i16,
    pub mv_x: i16,
    pub mv_y: i16,
}

#[derive(Clone, Debug)]
pub struct MvBundle {
    pub blocks: Vec<[BlockMotion; BLOCKS_PER_MB]>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PmvClosureCounts {
    pub geometric: usize,
    pub closure: usize,
    pub added_total: usize,
    pub added_post: usize,
    pub added_edited_outside_halo: usize,
    pub added_pre: usize,
}

impl PmvClosureCounts {
    pub fn closure_fraction(&self, total: usize) -> f64 {
        if total == 0 {
            0.0
        } else {
            self.closure as f64 / total as f64
        }
    }
}

pub fn load_mv_dir(dir: &Path, num_mbs: usize) -> Result<MvBundle, String> {
    let raw = std::fs::read(dir.join("mv_enc"))
        .map_err(|e| format!("read {}: {e}", dir.join("mv_enc").display()))?;
    let need = num_mbs * MV_BYTES_PER_MB;
    if raw.len() < need {
        return Err(format!(
            "mv_enc has {} bytes, need {need} ({num_mbs} MBs × {} B)",
            raw.len(),
            MV_BYTES_PER_MB
        ));
    }
    let mut blocks = Vec::with_capacity(num_mbs);
    for g in 0..num_mbs {
        let off = g * MV_BYTES_PER_MB;
        let mut mb = [BlockMotion {
            ref_frame: -1,
            mv_x: 0,
            mv_y: 0,
        }; BLOCKS_PER_MB];
        for (i, slot) in mb.iter_mut().enumerate() {
            let b = off + i * 6;
            slot.ref_frame = i16::from_le_bytes([raw[b], raw[b + 1]]);
            slot.mv_x = i16::from_le_bytes([raw[b + 2], raw[b + 3]]);
            slot.mv_y = i16::from_le_bytes([raw[b + 4], raw[b + 5]]);
        }
        blocks.push(mb);
    }
    Ok(MvBundle { blocks })
}

fn is_intra_mb(types: &[u8; 6]) -> bool {
    if types[0] == 2 {
        return true;
    }
    matches!(types[1], 9 | 10 | 13)
}

fn block_touches_seed(
    ref_frame: i16,
    px: usize,
    py: usize,
    mv_x: i16,
    mv_y: i16,
    spec: &IslandSpec,
    seed: &HashSet<usize>,
    mbs: usize,
    cols: usize,
) -> bool {
    if ref_frame < 0 {
        return false;
    }
    let rf = ref_frame as usize;
    if rf >= spec.num_frames {
        return false;
    }
    let rx0 = px as i32 + (mv_x as i32 >> 2);
    let ry0 = py as i32 + (mv_y as i32 >> 2);
    let rx1 = rx0 + 3;
    let ry1 = ry0 + 3;
    if rx1 < 0 || ry1 < 0 {
        return false;
    }
    let mb_x0 = (rx0.max(0) as usize) / 16;
    let mb_y0 = (ry0.max(0) as usize) / 16;
    let mb_x1 = (rx1.max(0) as usize) / 16;
    let mb_y1 = (ry1.max(0) as usize) / 16;
    let rows = spec.rows();
    for mby in mb_y0..=mb_y1.min(rows.saturating_sub(1)) {
        for mbx in mb_x0..=mb_x1.min(cols.saturating_sub(1)) {
            let g = rf * mbs + mby * cols + mbx;
            if seed.contains(&g) {
                return true;
            }
        }
    }
    false
}

fn mb_inter_touches_seed(
    global: usize,
    spec: &IslandSpec,
    mv: &MvBundle,
    types: &[u8],
    seed: &HashSet<usize>,
    mbs: usize,
    cols: usize,
) -> Result<bool, String> {
    let (frame, mb_x, mb_y) = spec.locate(global)?;
    let t6 = &types[global * 6..global * 6 + 6];
    if is_intra_mb(t6.try_into().unwrap()) {
        return Ok(false);
    }
    let blocks = &mv.blocks[global];
    for (i, bm) in blocks.iter().enumerate() {
        let bx = i % 4;
        let by = i / 4;
        let px = mb_x * 16 + bx * 4;
        let py = mb_y * 16 + by * 4;
        if block_touches_seed(bm.ref_frame, px, py, bm.mv_x, bm.mv_y, spec, seed, mbs, cols) {
            return Ok(true);
        }
    }
    let _ = frame;
    Ok(false)
}

/// Fixpoint expansion of geometric `I` using L0 MV reachability (conservative MB overlap).
pub fn pmv_closure(
    spec: &IslandSpec,
    mv: &MvBundle,
    type_enc: &[u8],
) -> Result<(HashSet<usize>, PmvClosureCounts), String> {
    spec.validate()?;
    let total = spec.total_mbs()?;
    if mv.blocks.len() != total {
        return Err(format!(
            "mv bundle has {} MBs, spec expects {total}",
            mv.blocks.len()
        ));
    }
    if type_enc.len() != total * 6 {
        return Err(format!(
            "type_enc length {} != {total} × 6",
            type_enc.len()
        ));
    }

    let mbs = spec.mbs_per_frame()?;
    let cols = spec.cols();
    let mut seed: HashSet<usize> = island_indices(spec)?.into_iter().collect();
    let geometric = seed.len();

    loop {
        let mut added = Vec::new();
        for g in 0..total {
            if seed.contains(&g) {
                continue;
            }
            if mb_inter_touches_seed(g, spec, mv, type_enc, &seed, mbs, cols)? {
                added.push(g);
            }
        }
        if added.is_empty() {
            break;
        }
        for g in added {
            seed.insert(g);
        }
    }

    let mut added_post = 0usize;
    let mut added_edited = 0usize;
    let mut added_pre = 0usize;
    let geo_set: HashSet<usize> = island_indices(spec)?.into_iter().collect();
    for &g in &seed {
        if geo_set.contains(&g) {
            continue;
        }
        match mb_role(spec, g)? {
            MbRole::SkipPost => added_post += 1,
            MbRole::SkipPre => added_pre += 1,
            MbRole::SkipInEditedFrames | MbRole::Gadget | MbRole::Halo => added_edited += 1,
        }
    }

    let closure = seed.len();
    Ok((
        seed,
        PmvClosureCounts {
            geometric,
            closure,
            added_total: closure.saturating_sub(geometric),
            added_post,
            added_edited_outside_halo: added_edited,
            added_pre,
        },
    ))
}

pub fn validate_syntax_lengths(
    pred_y: usize,
    coeff_y: usize,
    type_enc: usize,
    mv_enc: usize,
) -> Result<usize, String> {
    if pred_y != coeff_y || pred_y % MB_Y_BYTES != 0 {
        return Err("pred/coeff length mismatch".into());
    }
    let n = pred_y / MB_Y_BYTES;
    if type_enc != n * 6 {
        return Err(format!("type_enc length {type_enc} != {n} × 6"));
    }
    if mv_enc != n * MV_BYTES_PER_MB {
        return Err(format!(
            "mv_enc length {mv_enc} != {n} × {} (regenerate JM dumps)",
            MV_BYTES_PER_MB
        ));
    }
    Ok(n)
}

/// Named edit windows for comparing P-MV closure vs edit position in the clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditWindowPreset {
    pub label: &'static str,
    pub frame_start: usize,
    pub frame_end: usize,
}

/// `early` / `mid` (centered) / `late` two-frame windows on a 30-frame-style clip.
pub fn edit_window_presets(num_frames: usize) -> Vec<EditWindowPreset> {
    let (mid_s, mid_e) = redact_window(num_frames);
    let early_end = 4.min(num_frames);
    let late_start = num_frames.saturating_sub(6);
    let late_end = num_frames.saturating_sub(4).max(late_start + 1);
    vec![
        EditWindowPreset {
            label: "early",
            frame_start: 2.min(num_frames.saturating_sub(1)),
            frame_end: early_end.max(3),
        },
        EditWindowPreset {
            label: "mid",
            frame_start: mid_s,
            frame_end: mid_e,
        },
        EditWindowPreset {
            label: "late",
            frame_start: late_start.min(late_end.saturating_sub(1)),
            frame_end: late_end.min(num_frames),
        },
    ]
}

/// Run P-MV closure for one [`IslandSpec`] (orig capture MVs + types).
pub fn pmv_closure_for_spec(
    spec: &IslandSpec,
    mv: &MvBundle,
    type_enc: &[u8],
) -> Result<PmvClosureCounts, String> {
    pmv_closure(spec, mv, type_enc).map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::island::PixelRect;

    fn tiny_spec(halo: usize) -> IslandSpec {
        IslandSpec {
            width: 64,
            height: 32,
            num_frames: 3,
            frame_start: 1,
            frame_end: 2,
            rect: PixelRect {
                x: 16,
                y: 0,
                w: 16,
                h: 16,
            },
            halo_mbs: halo,
        }
    }

    #[test]
    fn edit_window_presets_cover_three_positions() {
        let p = edit_window_presets(30);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].label, "early");
        assert_eq!(p[0].frame_start, 2);
        assert_eq!(p[0].frame_end, 4);
        assert_eq!(p[1].label, "mid");
        assert_eq!(p[1].frame_start, 14);
        assert_eq!(p[1].frame_end, 16);
        assert_eq!(p[2].label, "late");
        assert_eq!(p[2].frame_start, 24);
        assert_eq!(p[2].frame_end, 26);
    }

    #[test]
    fn pmv_expands_post_frame_referencing_island() {
        let spec = tiny_spec(0);
        let total = spec.total_mbs().unwrap();
        let mbs = spec.mbs_per_frame().unwrap();
        let gadget = spec.frame_start * mbs + 1; // mb (1,0) frame 1

        let mut mv_blocks = vec![
            [BlockMotion {
                ref_frame: -1,
                mv_x: 0,
                mv_y: 0,
            }; BLOCKS_PER_MB];
            total
        ];
        // Frame 2, mb (1,0): MV from frame 1 pointing at gadget MB pixel (16,0)
        let consumer = 2 * mbs + 1;
        mv_blocks[consumer][0] = BlockMotion {
            ref_frame: 1,
            mv_x: 0,
            mv_y: 0,
        };

        let type_enc = vec![1u8, 1, 0, 0, 28, 28].repeat(total);
        let mv = MvBundle { blocks: mv_blocks };
        let (set, counts) = pmv_closure(&spec, &mv, &type_enc).unwrap();
        assert!(set.contains(&gadget));
        assert!(set.contains(&consumer));
        assert_eq!(counts.geometric, 1);
        assert_eq!(counts.added_post, 1);
        assert_eq!(counts.closure, 2);
    }
}
