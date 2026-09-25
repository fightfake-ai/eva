//! Merge JM syntax dumps (copy `J`, replace `I`) and approximate luma decode for splice spikes.
//!
//! Uses per-MB `pred_y_enc` + `coeff_y_enc` + `type_enc` from JM dumps. Two decode modes:
//! - **Naive**: residual from coeffs added to dumped predictor (fast sanity check).
//! - **Scan**: raster-order recon buffer; intra 4×4 vertical / horizontal / DC from neighbors.
//!
//! Merge modes:
//! - **Geometric** — replace gadget ∪ halo on edited frames (`merge_syntax`).
//! - **P-MV closure** — replace every slot in MV-expanded `I` (`merge_syntax_pmv`).

use std::collections::HashSet;
use std::path::Path;

use crate::encode::constants::{BASE_OFFSET_BP_SLICE, BASE_OFFSET_I_SLICE, Q_BITS_4};
use crate::island::{island_indices, mb_role, IslandSpec, MbRole};
use crate::macroblock_yuv::{macroblock_xy, yuv420_frame_bytes, MB_Y_BYTES};
use crate::neighbor_impact::compare_yuv_to_island;
use crate::pmv_closure::{load_mv_dir, pmv_closure, MvBundle, PmvClosureCounts};

/// One clip's JM luma syntax dumps (`pred_y_enc`, `coeff_y_enc`, `type_enc`, optional sidecars).
#[derive(Clone, Debug)]
pub struct SyntaxBundle {
    pub pred_y: Vec<u8>,
    pub coeff_y: Vec<u8>,
    pub type_enc: Vec<u8>,
    /// 16 bytes/MB when present (`intra_modes_enc`).
    pub intra_modes: Vec<u8>,
    /// 8 bytes/MB when present (`b8x8_enc`).
    pub b8x8_enc: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RoleLumaStats {
    pub n_mbs: usize,
    pub max_abs_y: u8,
    pub mean_abs_y: f64,
    pub over_4: usize,
    pub over_16: usize,
}

#[derive(Clone, Debug, Default)]
pub struct SpliceDecodeReport {
    pub frame: usize,
    pub halo_mbs: usize,
    pub decode_mode: &'static str,
    pub gadget: RoleLumaStats,
    pub halo: RoleLumaStats,
    pub skip_in_edited: RoleLumaStats,
    pub skip_pre: RoleLumaStats,
    pub skip_post: RoleLumaStats,
    pub island: RoleLumaStats,
    pub outside_island: RoleLumaStats,
    pub full_frame_max: u8,
    pub full_frame_mean: f64,
}

impl SpliceDecodeReport {
    pub fn island_max(&self) -> u8 {
        self.island.max_abs_y
    }

    pub fn outside_island_max(&self) -> u8 {
        self.outside_island.max_abs_y
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpliceDecodeMode {
    /// `recon = pred_dump + IDCT(dequant(coeff))` per MB (ignores mixed neighbor recon).
    Naive,
    /// Raster recon buffer; intra 4×4 vertical / horizontal / DC from decoded neighbors.
    ScanIntra4x4,
}

pub fn load_syntax_dir(dir: &Path) -> Result<SyntaxBundle, String> {
    let pred_y = std::fs::read(dir.join("pred_y_enc"))
        .map_err(|e| format!("read {}: {e}", dir.join("pred_y_enc").display()))?;
    let coeff_y = std::fs::read(dir.join("coeff_y_enc"))
        .map_err(|e| format!("read {}: {e}", dir.join("coeff_y_enc").display()))?;
    let type_enc = std::fs::read(dir.join("type_enc"))
        .map_err(|e| format!("read {}: {e}", dir.join("type_enc").display()))?;
    validate_bundle(&pred_y, &coeff_y, &type_enc)?;
    let n = pred_y.len() / MB_Y_BYTES;
    let intra_modes = read_optional_sidecar(dir, "intra_modes_enc", n * 16)?;
    let b8x8_enc = read_optional_sidecar(dir, "b8x8_enc", n * 8)?;
    Ok(SyntaxBundle {
        pred_y,
        coeff_y,
        type_enc,
        intra_modes,
        b8x8_enc,
    })
}

fn read_optional_sidecar(dir: &Path, name: &str, need: usize) -> Result<Vec<u8>, String> {
    let path = dir.join(name);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if data.len() != need {
        return Err(format!(
            "{} length {} != expected {need}",
            path.display(),
            data.len()
        ));
    }
    Ok(data)
}

fn validate_bundle(pred_y: &[u8], coeff_y: &[u8], type_enc: &[u8]) -> Result<(), String> {
    if pred_y.len() != coeff_y.len() || pred_y.len() % MB_Y_BYTES != 0 {
        return Err("pred/coeff length mismatch or not whole macroblocks".into());
    }
    let n = pred_y.len() / MB_Y_BYTES;
    if type_enc.len() != n * 6 {
        return Err(format!(
            "type_enc length {} != {n} macroblocks × 6",
            type_enc.len()
        ));
    }
    Ok(())
}

/// Slot `g ∈ island` → `edited`; else → `orig`.
pub fn merge_syntax_set(
    orig: &SyntaxBundle,
    edited: &SyntaxBundle,
    spec: &IslandSpec,
    island: &HashSet<usize>,
) -> Result<SyntaxBundle, String> {
    validate_bundle(&orig.pred_y, &orig.coeff_y, &orig.type_enc)?;
    validate_bundle(&edited.pred_y, &edited.coeff_y, &edited.type_enc)?;
    if orig.pred_y.len() != edited.pred_y.len() {
        return Err("orig/edited dump lengths differ".into());
    }
    let n = orig.pred_y.len() / MB_Y_BYTES;
    let total = spec.total_mbs()?;
    if n != total {
        return Err(format!("dump has {n} MBs, spec expects {total}"));
    }

    let mut pred_y = orig.pred_y.clone();
    let mut coeff_y = orig.coeff_y.clone();
    let mut type_enc = orig.type_enc.clone();
    let mut intra_modes = merge_sidecar(&orig.intra_modes, &edited.intra_modes, n * 16)?;
    let mut b8x8_enc = merge_sidecar(&orig.b8x8_enc, &edited.b8x8_enc, n * 8)?;

    for &g in island {
        if g >= n {
            return Err(format!("island index {g} out of range (n={n})"));
        }
        let py = g * MB_Y_BYTES;
        let ty = g * 6;
        pred_y[py..py + MB_Y_BYTES].copy_from_slice(&edited.pred_y[py..py + MB_Y_BYTES]);
        coeff_y[py..py + MB_Y_BYTES].copy_from_slice(&edited.coeff_y[py..py + MB_Y_BYTES]);
        type_enc[ty..ty + 6].copy_from_slice(&edited.type_enc[ty..ty + 6]);
        copy_island_sidecar(&mut intra_modes, &edited.intra_modes, g, 16);
        copy_island_sidecar(&mut b8x8_enc, &edited.b8x8_enc, g, 8);
    }

    Ok(SyntaxBundle {
        pred_y,
        coeff_y,
        type_enc,
        intra_modes,
        b8x8_enc,
    })
}

fn merge_sidecar(orig: &[u8], edited: &[u8], need: usize) -> Result<Vec<u8>, String> {
    if orig.is_empty() && edited.is_empty() {
        return Ok(Vec::new());
    }
    if !orig.is_empty() && edited.is_empty() {
        if orig.len() != need {
            return Err(format!("sidecar orig length {} != {need}", orig.len()));
        }
        return Ok(orig.to_vec());
    }
    if orig.len() != need || edited.len() != need {
        return Err(format!(
            "sidecar length mismatch (orig {}, edited {}, need {need})",
            orig.len(),
            edited.len()
        ));
    }
    Ok(orig.to_vec())
}

fn copy_island_sidecar(out: &mut [u8], edited: &[u8], g: usize, bytes_per_mb: usize) {
    if out.is_empty() || edited.is_empty() {
        return;
    }
    let o = g * bytes_per_mb;
    out[o..o + bytes_per_mb].copy_from_slice(&edited[o..o + bytes_per_mb]);
}

/// Write Eva glue inputs for JM (`EVA_GLUE_DIR`).
pub fn export_glue_dir(bundle: &SyntaxBundle, dir: &Path) -> Result<(), String> {
    validate_bundle(&bundle.pred_y, &bundle.coeff_y, &bundle.type_enc)?;
    std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    std::fs::write(dir.join("pred_y_enc"), &bundle.pred_y)
        .map_err(|e| format!("write pred_y_enc: {e}"))?;
    std::fs::write(dir.join("coeff_y_enc"), &bundle.coeff_y)
        .map_err(|e| format!("write coeff_y_enc: {e}"))?;
    std::fs::write(dir.join("type_enc"), &bundle.type_enc)
        .map_err(|e| format!("write type_enc: {e}"))?;
    if !bundle.intra_modes.is_empty() {
        std::fs::write(dir.join("intra_modes_enc"), &bundle.intra_modes)
            .map_err(|e| format!("write intra_modes_enc: {e}"))?;
    }
    if !bundle.b8x8_enc.is_empty() {
        std::fs::write(dir.join("b8x8_enc"), &bundle.b8x8_enc)
            .map_err(|e| format!("write b8x8_enc: {e}"))?;
    }
    Ok(())
}

/// Export merged syntax plus orig `mv_enc` sidecar for CABAC glue encode.
pub fn export_glue_dir_with_mv(
    bundle: &SyntaxBundle,
    mv: &[u8],
    dir: &Path,
) -> Result<(), String> {
    let n = bundle.pred_y.len() / MB_Y_BYTES;
    let need = n * crate::pmv_closure::MV_BYTES_PER_MB;
    if mv.len() != need {
        return Err(format!("mv_enc length {} != {need}", mv.len()));
    }
    export_glue_dir(bundle, dir)?;
    std::fs::write(dir.join("mv_enc"), mv).map_err(|e| format!("write mv_enc: {e}"))?;
    Ok(())
}

/// Geometric island: gadget ∪ halo on edited frames.
pub fn merge_syntax(
    orig: &SyntaxBundle,
    edited: &SyntaxBundle,
    spec: &IslandSpec,
) -> Result<SyntaxBundle, String> {
    let geo: HashSet<usize> = island_indices(spec)?.into_iter().collect();
    merge_syntax_set(orig, edited, spec, &geo)
}

/// P-MV closure island from **orig** capture MVs; replace those slots with edited syntax.
pub fn merge_syntax_pmv(
    orig: &SyntaxBundle,
    edited: &SyntaxBundle,
    spec: &IslandSpec,
    mv: &MvBundle,
) -> Result<(SyntaxBundle, PmvClosureCounts), String> {
    let (island, counts) = pmv_closure(spec, mv, &orig.type_enc)?;
    let merged = merge_syntax_set(orig, edited, spec, &island)?;
    Ok((merged, counts))
}

/// Load orig syntax + `mv_enc`, run P-MV merge against edited syntax.
pub fn merge_syntax_pmv_dirs(
    orig_dir: &Path,
    edit_dir: &Path,
    spec: &IslandSpec,
) -> Result<(SyntaxBundle, PmvClosureCounts), String> {
    let orig = load_syntax_dir(orig_dir)?;
    let edited = load_syntax_dir(edit_dir)?;
    let n = orig.pred_y.len() / MB_Y_BYTES;
    let mv = load_mv_dir(orig_dir, n)?;
    merge_syntax_pmv(&orig, &edited, spec, &mv)
}

/// Decode one frame's luma plane into `out_y` (must hold `width × height` bytes).
pub fn decode_frame_luma(
    bundle: &SyntaxBundle,
    spec: &IslandSpec,
    frame: usize,
    mode: SpliceDecodeMode,
    out_y: &mut [u8],
) -> Result<(), String> {
    spec.validate()?;
    if frame >= spec.num_frames {
        return Err(format!("frame {frame} >= num_frames {}", spec.num_frames));
    }
    let total_mbs = spec.total_mbs()?;
    let need = total_mbs * MB_Y_BYTES;
    if bundle.pred_y.len() < need {
        return Err(format!(
            "syntax dump has {} MB bytes, need {need} for {} frames",
            bundle.pred_y.len(),
            spec.num_frames
        ));
    }
    let plane = spec.width * spec.height;
    if out_y.len() < plane {
        return Err(format!("out_y need {plane} bytes"));
    }

    let mbs = spec.mbs_per_frame()?;
    let base = frame * mbs;

    match mode {
        SpliceDecodeMode::Naive => {
            for mb in 0..mbs {
                let g = base + mb;
                let (mb_x, mb_y) = macroblock_xy(spec.width, mb);
                decode_mb_luma_naive(
                    &bundle.pred_y[g * MB_Y_BYTES..(g + 1) * MB_Y_BYTES],
                    &bundle.coeff_y[g * MB_Y_BYTES..(g + 1) * MB_Y_BYTES],
                    &bundle.type_enc[g * 6..g * 6 + 6],
                    spec.width,
                    mb_x,
                    mb_y,
                    out_y,
                )?;
            }
        }
        SpliceDecodeMode::ScanIntra4x4 => {
            decode_frame_luma_scan(bundle, spec, frame, out_y)?;
        }
    }
    Ok(())
}

fn decode_frame_luma_scan(
    bundle: &SyntaxBundle,
    spec: &IslandSpec,
    frame: usize,
    out_y: &mut [u8],
) -> Result<(), String> {
    let mbs = spec.mbs_per_frame()?;
    let cols = spec.cols();
    let rows = spec.rows();
    let base = frame * mbs;

    for mb_y in 0..rows {
        for mb_x in 0..cols {
            let g = base + mb_y * cols + mb_x;
            let types = &bundle.type_enc[g * 6..g * 6 + 6];
            let qp = types[4] as usize;
            let is_intra = types[0] == 2;
            let mb_type = types[1];
            let pred = &bundle.pred_y[g * MB_Y_BYTES..(g + 1) * MB_Y_BYTES];
            let coeff = &bundle.coeff_y[g * MB_Y_BYTES..(g + 1) * MB_Y_BYTES];
            let residual = coeff_to_residual(coeff, qp, is_intra, mb_type == 10)?;

            for by in 0..4 {
                for bx in 0..4 {
                    for y in 0..4 {
                        for x in 0..4 {
                            let py = mb_y * 16 + by * 4 + y;
                            let px = mb_x * 16 + bx * 4 + x;
                            let idx = y * 4 + x;
                            let mut recon = if mb_type == 9 || mb_type == 10 {
                                intra4x4_pred_sample(
                                    out_y,
                                    spec.width,
                                    px,
                                    py,
                                    types[3],
                                ) + residual[by * 4 + bx][idx]
                            } else {
                                pred[(by * 4 + y) * 16 + (bx * 4 + x)] as i32
                                    + residual[by * 4 + bx][idx]
                            };
                            if recon < 0 {
                                recon = 0;
                            } else if recon > 255 {
                                recon = 255;
                            }
                            out_y[py * spec.width + px] = recon as u8;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// H.264 intra 4×4 prediction modes (subset): 0 vertical, 1 horizontal, 2 DC.
fn intra4x4_pred_sample(
    recon: &[u8],
    width: usize,
    px: usize,
    py: usize,
    mode: u8,
) -> i32 {
    match mode {
        0 => {
            if px == 0 {
                128
            } else {
                recon[py * width + (px - 1)] as i32
            }
        }
        1 => {
            if py == 0 {
                128
            } else {
                recon[(py - 1) * width + px] as i32
            }
        }
        _ => {
            let left = if px > 0 {
                recon[py * width + (px - 1)] as i32
            } else {
                128
            };
            let top = if py > 0 {
                recon[(py - 1) * width + px] as i32
            } else {
                128
            };
            (left + top + 1) >> 1
        }
    }
}

fn decode_mb_luma_naive(
    pred: &[u8],
    coeff: &[u8],
    types: &[u8],
    width: usize,
    mb_x: usize,
    mb_y: usize,
    out_y: &mut [u8],
) -> Result<(), String> {
    let qp = types[4] as usize;
    let is_intra = types[0] == 2;
    let is_i16 = types[1] == 10;
    let residual = coeff_to_residual(coeff, qp, is_intra, is_i16)?;
    for by in 0..4 {
        for bx in 0..4 {
            for y in 0..4 {
                for x in 0..4 {
                    let py = mb_y * 16 + by * 4 + y;
                    let px = mb_x * 16 + bx * 4 + x;
                    let idx = y * 4 + x;
                    let p = pred[(by * 4 + y) * 16 + (bx * 4 + x)] as i32;
                    let mut v = p + residual[by * 4 + bx][idx];
                    if v < 0 {
                        v = 0;
                    } else if v > 255 {
                        v = 255;
                    }
                    out_y[py * width + px] = v as u8;
                }
            }
        }
    }
    Ok(())
}

fn coeff_to_residual(
    coeff: &[u8],
    qp: usize,
    is_intra: bool,
    is_i16: bool,
) -> Result<[[i32; 16]; 16], String> {
    if coeff.len() != MB_Y_BYTES {
        return Err("coeff slice not 256 bytes".into());
    }
    let qp_mod = qp % 6;
    let qp_div = qp / 6;
    let d = Q_BITS_4 + qp_div;
    let base_offset = if is_intra {
        BASE_OFFSET_I_SLICE
    } else {
        BASE_OFFSET_BP_SLICE
    };
    let offset = (base_offset << (qp_div + Q_BITS_4 - 11)) as i32;

    let scale0 = [13107i16, 11916, 10082, 9362, 8192, 7282][qp_mod];
    let scale1 = [8066, 7490, 6554, 5825, 5243, 4559][qp_mod];
    let scale2 = [5243, 4660, 4194, 3647, 3355, 2893][qp_mod];

    let mut blocks = [[0i32; 16]; 16];
    for by in 0..4 {
        for bx in 0..4 {
            let mut levels = [0i16; 16];
            for y in 0..4 {
                for x in 0..4 {
                    let b = coeff[(by * 4 + y) * 16 + (bx * 4 + x)];
                    levels[y * 4 + x] = (b as i16) - 128;
                }
            }
            if is_i16 {
                // DC hadamard path omitted — approximate spike; I16MB may show higher error.
            }
            let mut dequant = [0i32; 16];
            for i in 0..16 {
                let scale = match i {
                    0 | 2 | 8 | 10 => scale0,
                    5 | 7 | 13 | 15 => scale2,
                    _ => scale1,
                };
                dequant[i] = inv_quant_core(levels[i], scale, offset, d);
            }
            blocks[by * 4 + bx] = idct_4x4(dequant);
        }
    }
    Ok(blocks)
}

fn inv_quant_core(level: i16, m: i16, a: i32, d: usize) -> i32 {
    if level == 0 {
        return 0;
    }
    let mut v = (level.abs() as i32) << d;
    v -= a;
    v /= m as i32;
    if level < 0 { -v } else { v }
}

/// H.264 4×4 inverse integer transform (output residual samples).
fn idct_4x4(coeff: [i32; 16]) -> [i32; 16] {
    let mut tmp = [0i32; 16];
    for i in 0..4 {
        let c0 = coeff[i * 4];
        let c1 = coeff[i * 4 + 1];
        let c2 = coeff[i * 4 + 2];
        let c3 = coeff[i * 4 + 3];
        let e0 = c0 + c2;
        let e1 = c0 - c2;
        let e2 = (c1 >> 1) - c3;
        let e3 = c1 + (c3 >> 1);
        tmp[i] = e0 + e3;
        tmp[4 + i] = e1 + e2;
        tmp[8 + i] = e1 - e2;
        tmp[12 + i] = e0 - e3;
    }
    let mut out = [0i32; 16];
    for i in 0..4 {
        let t0 = tmp[i];
        let t1 = tmp[4 + i];
        let t2 = tmp[8 + i];
        let t3 = tmp[12 + i];
        let e0 = t0 + t2;
        let e1 = t0 - t2;
        let e2 = (t1 >> 1) - t3;
        let e3 = t1 + (t3 >> 1);
        out[i * 4] = (e0 + e3 + 32) >> 6;
        out[i * 4 + 1] = (e1 + e2 + 32) >> 6;
        out[i * 4 + 2] = (e1 - e2 + 32) >> 6;
        out[i * 4 + 3] = (e0 - e3 + 32) >> 6;
    }
    out
}

/// Compare two decoded luma planes (same frame geometry), bucketed by island role.
pub fn compare_decoded_planes(
    decoded: &[u8],
    reference: &[u8],
    spec: &IslandSpec,
    frame: usize,
    halo_mbs: usize,
    decode_mode: &'static str,
) -> Result<SpliceDecodeReport, String> {
    let mut spec_role = *spec;
    spec_role.halo_mbs = halo_mbs;

    let plane = spec.width * spec.height;
    if reference.len() < plane {
        return Err("reference plane too short".into());
    }
    if decoded.len() < plane {
        return Err("decoded plane too short".into());
    }

    let mut report = SpliceDecodeReport {
        frame,
        halo_mbs,
        decode_mode,
        ..Default::default()
    };

    let mbs = spec.mbs_per_frame()?;
    let base = frame * mbs;
    let mut sum_all = 0u64;
    let mut n_all = 0usize;

    for mb in 0..mbs {
        let g = base + mb;
        let (mb_x, mb_y) = macroblock_xy(spec.width, mb);
        let mut max_d = 0u8;
        let mut sum_d = 0u64;
        for row in 0..16 {
            let y = mb_y * 16 + row;
            let off = y * spec.width + mb_x * 16;
            for col in 0..16 {
                let d = decoded[off + col].abs_diff(reference[off + col]);
                if d > max_d {
                    max_d = d;
                }
                sum_d += d as u64;
                if d > report.full_frame_max {
                    report.full_frame_max = d;
                }
            }
        }
        let mean = sum_d as f64 / 256.0;
        sum_all += sum_d;
        n_all += 256;

        let slot = match mb_role(&spec_role, g)? {
            MbRole::Gadget => &mut report.gadget,
            MbRole::Halo => &mut report.halo,
            MbRole::SkipInEditedFrames => &mut report.skip_in_edited,
            MbRole::SkipPre => &mut report.skip_pre,
            MbRole::SkipPost => &mut report.skip_post,
        };
        observe_role(slot, max_d, mean);
        if mb_role(&spec_role, g)?.in_island() {
            observe_role(&mut report.island, max_d, mean);
        } else {
            observe_role(&mut report.outside_island, max_d, mean);
        }
    }
    report.full_frame_mean = sum_all as f64 / n_all as f64;
    Ok(report)
}

/// Compare only macroblocks whose global index is in `globals` (typically closure ∩ frame).
pub fn compare_decoded_mb_set(
    decoded: &[u8],
    reference: &[u8],
    spec: &IslandSpec,
    frame: usize,
    globals: &HashSet<usize>,
) -> Result<(u8, f64, usize), String> {
    let plane = spec.width * spec.height;
    if reference.len() < plane || decoded.len() < plane {
        return Err("plane too short".into());
    }
    let mbs = spec.mbs_per_frame()?;
    let base = frame * mbs;
    let mut max_d = 0u8;
    let mut sum_d = 0u64;
    let mut n_px = 0usize;
    for mb in 0..mbs {
        let g = base + mb;
        if !globals.contains(&g) {
            continue;
        }
        let (mb_x, mb_y) = macroblock_xy(spec.width, mb);
        for row in 0..16 {
            let y = mb_y * 16 + row;
            let off = y * spec.width + mb_x * 16;
            for col in 0..16 {
                let d = decoded[off + col].abs_diff(reference[off + col]);
                if d > max_d {
                    max_d = d;
                }
                sum_d += d as u64;
                n_px += 1;
            }
        }
    }
    let mean = if n_px == 0 {
        0.0
    } else {
        sum_d as f64 / n_px as f64
    };
    Ok((max_d, mean, n_px / 256))
}

/// Compare decoded frame luma plane against reference luma plane (same size `width × height`).
pub fn compare_decoded_frame_to_yuv(
    decoded_y: &[u8],
    reference_plane: &[u8],
    spec: &IslandSpec,
    frame: usize,
    mode: SpliceDecodeMode,
    halo_mbs: usize,
) -> Result<SpliceDecodeReport, String> {
    let decode_mode = match mode {
        SpliceDecodeMode::Naive => "naive_pred_dump",
        SpliceDecodeMode::ScanIntra4x4 => "scan_intra4x4",
    };
    compare_decoded_planes(
        decoded_y,
        reference_plane,
        spec,
        frame,
        halo_mbs,
        decode_mode,
    )
}

fn observe_role(slot: &mut RoleLumaStats, max_d: u8, mean: f64) {
    slot.n_mbs += 1;
    if max_d > slot.max_abs_y {
        slot.max_abs_y = max_d;
    }
    slot.mean_abs_y += mean;
    if max_d > 4 {
        slot.over_4 += 1;
    }
    if max_d > 16 {
        slot.over_16 += 1;
    }
}

pub fn finalize_role_means(report: &mut SpliceDecodeReport) {
    for slot in [
        &mut report.gadget,
        &mut report.halo,
        &mut report.skip_in_edited,
        &mut report.skip_pre,
        &mut report.skip_post,
        &mut report.island,
        &mut report.outside_island,
    ] {
        if slot.n_mbs > 0 {
            slot.mean_abs_y /= slot.n_mbs as f64;
        }
    }
}

/// Build expected reference for edited frames: `orig` on J, `edited` on I (pixel oracle).
pub fn expected_splice_yuv(orig: &[u8], edited: &[u8], spec: &IslandSpec) -> Result<Vec<u8>, String> {
    spec.validate()?;
    let frame_bytes = yuv420_frame_bytes(spec.width, spec.height)?;
    let need = frame_bytes * spec.num_frames;
    if orig.len() < need || edited.len() < need {
        return Err("YUV too short".into());
    }
    let mut out = orig[..need].to_vec();
    let mbs = spec.mbs_per_frame()?;
    for frame in spec.frame_start_clamped()..spec.frame_end_clamped() {
        for mb in 0..mbs {
            let g = frame * mbs + mb;
            if mb_role(spec, g)?.in_island() {
                let (mb_x, mb_y) = macroblock_xy(spec.width, mb);
                copy_mb_luma(edited, &mut out, spec.width, frame, mb_x, mb_y, frame_bytes)?;
            }
        }
    }
    Ok(out)
}

fn copy_mb_luma(
    src: &[u8],
    dst: &mut [u8],
    width: usize,
    frame: usize,
    mb_x: usize,
    mb_y: usize,
    frame_bytes: usize,
) -> Result<(), String> {
    let frame_off = frame * frame_bytes;
    for row in 0..16 {
        let y = mb_y * 16 + row;
        let off = frame_off + y * width + mb_x * 16;
        dst[off..off + 16].copy_from_slice(&src[off..off + 16]);
    }
    Ok(())
}

/// Sanity: pixel edit stays outside I.
pub fn pixel_outside_island(orig: &[u8], edited: &[u8], spec: &IslandSpec) -> Result<usize, String> {
    Ok(compare_yuv_to_island(orig, edited, spec, 0)?.outside_island_changed())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::island::PixelRect;

    fn tiny_spec(halo: usize) -> IslandSpec {
        IslandSpec {
            width: 48,
            height: 32,
            num_frames: 1,
            frame_start: 0,
            frame_end: 1,
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
    fn merge_uses_edited_only_on_island() {
        let spec = tiny_spec(0);
        let n = spec.total_mbs().unwrap();
        let mut orig = SyntaxBundle {
            pred_y: vec![1u8; n * 256],
            coeff_y: vec![128u8; n * 256],
            type_enc: vec![0u8; n * 6],
            intra_modes: Vec::new(),
            b8x8_enc: Vec::new(),
        };
        let mut edited = SyntaxBundle {
            pred_y: vec![2u8; n * 256],
            coeff_y: vec![130u8; n * 256],
            type_enc: vec![9u8; n * 6],
            intra_modes: Vec::new(),
            b8x8_enc: Vec::new(),
        };
        orig.pred_y[1 * 256] = 9;
        edited.pred_y[1 * 256] = 7;
        let merged = merge_syntax(&orig, &edited, &spec).unwrap();
        assert_eq!(merged.pred_y[0], 1);
        assert_eq!(merged.pred_y[256], 7);
        assert_eq!(merged.type_enc[6], 9);
        assert_eq!(merged.pred_y[2 * 256], 1);
    }

    #[test]
    fn spliced_matches_orig_outside_island_on_decode() {
        let spec = tiny_spec(1);
        let n = spec.total_mbs().unwrap();
        let mut orig = SyntaxBundle {
            pred_y: (0..n * 256).map(|i| (i % 251) as u8).collect(),
            coeff_y: vec![128u8; n * 256],
            type_enc: vec![1, 1, 0, 0, 30, 30].repeat(n),
            intra_modes: Vec::new(),
            b8x8_enc: Vec::new(),
        };
        let edited = SyntaxBundle {
            pred_y: vec![200u8; n * 256],
            coeff_y: vec![140u8; n * 256],
            type_enc: vec![1, 1, 0, 0, 30, 30].repeat(n),
            intra_modes: Vec::new(),
            b8x8_enc: Vec::new(),
        };
        let merged = merge_syntax(&orig, &edited, &spec).unwrap();
        let plane = spec.width * spec.height;
        let mut dec_orig = vec![0u8; plane];
        let mut dec_spliced = vec![0u8; plane];
        decode_frame_luma(
            &orig,
            &spec,
            0,
            SpliceDecodeMode::Naive,
            &mut dec_orig,
        )
        .unwrap();
        decode_frame_luma(
            &merged,
            &spec,
            0,
            SpliceDecodeMode::Naive,
            &mut dec_spliced,
        )
        .unwrap();
        let mut rep = compare_decoded_planes(
            &dec_spliced,
            &dec_orig,
            &spec,
            0,
            1,
            "test",
        )
        .unwrap();
        finalize_role_means(&mut rep);
        assert_eq!(
            rep.outside_island.max_abs_y, 0,
            "copy-J splice should decode identically outside I"
        );
    }

    #[test]
    fn pmv_merge_replaces_post_consumer_with_edited() {
        use crate::island::PixelRect;
        use crate::pmv_closure::{BlockMotion, MvBundle, BLOCKS_PER_MB};

        let spec = IslandSpec {
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
            halo_mbs: 0,
        };
        let total = spec.total_mbs().unwrap();
        let mbs = spec.mbs_per_frame().unwrap();
        let consumer = spec.frame_end_clamped() * mbs + 1;
        assert!(consumer < total);

        let mut mv_blocks = vec![
            [BlockMotion {
                ref_frame: -1,
                mv_x: 0,
                mv_y: 0,
            }; BLOCKS_PER_MB];
            total
        ];
        mv_blocks[consumer][0] = BlockMotion {
            ref_frame: 1,
            mv_x: 0,
            mv_y: 0,
        };

        let orig = SyntaxBundle {
            pred_y: vec![1u8; total * 256],
            coeff_y: vec![128u8; total * 256],
            type_enc: vec![1u8, 1, 0, 0, 28, 28].repeat(total),
            intra_modes: Vec::new(),
            b8x8_enc: Vec::new(),
        };
        let mut edited = SyntaxBundle {
            pred_y: vec![9u8; total * 256],
            coeff_y: vec![140u8; total * 256],
            type_enc: vec![1u8, 1, 0, 0, 28, 28].repeat(total),
            intra_modes: Vec::new(),
            b8x8_enc: Vec::new(),
        };
        edited.pred_y[consumer * 256] = 42;

        let mv = MvBundle { blocks: mv_blocks };
        let geo = merge_syntax(&orig, &edited, &spec).unwrap();
        assert_eq!(geo.pred_y[consumer * 256], 1, "geometric merge keeps orig on post frame");

        let (pmv, counts) = merge_syntax_pmv(&orig, &edited, &spec, &mv).unwrap();
        assert_eq!(counts.added_post, 1);
        assert_eq!(pmv.pred_y[consumer * 256], 42, "pmv merge uses edited on MV consumer");
    }
}
