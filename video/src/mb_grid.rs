//! Per-macroblock grids for visualization: edit mask, decode diff, syntax diff.

use std::fmt::Write as FmtWrite;
use std::path::Path;

use crate::island::{mb_role, IslandSpec, MbRole};
use crate::macroblock_yuv::{macroblock_xy, yuv420_frame_bytes};
use crate::neighbor_impact::syntax_mb_changed;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PerMbLumaDiff {
    pub changed: bool,
    pub max_delta_y: u8,
}

#[derive(Clone, Debug)]
pub struct MbFrameGrid {
    pub frame: usize,
    pub cols: usize,
    pub rows: usize,
    /// Row-major MB index; `role_code` matches [`role_to_code`].
    pub role: Vec<u8>,
    /// Orig vs edited YUV before any encode (true only on painted gadget MBs).
    pub pixel_edited: Vec<bool>,
    /// Decoded luma differs between two independent encodes (proxy for smear, not syntax).
    pub decode_changed: Vec<bool>,
    pub max_delta_y: Vec<u8>,
    /// JM dump diff: pred/coeff/type bytewise (`None` if no syntax layer).
    pub syntax_changed: Option<Vec<bool>>,
}

#[derive(Clone, Debug)]
pub struct MbGridReport {
    pub spec: IslandSpec,
    pub frames: Vec<MbFrameGrid>,
}

pub fn role_to_code(role: MbRole) -> u8 {
    match role {
        MbRole::Gadget => 0,
        MbRole::Halo => 1,
        MbRole::SkipInEditedFrames => 2,
        MbRole::SkipPre => 3,
        MbRole::SkipPost => 4,
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

/// Per-MB luma max-abs between two clips (global MB order).
pub fn compare_yuv_per_mb(
    orig: &[u8],
    other: &[u8],
    spec: &IslandSpec,
    threshold: u8,
) -> Result<Vec<PerMbLumaDiff>, String> {
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
    let mut out = Vec::with_capacity(spec.num_frames * mbs);
    for frame in 0..spec.num_frames {
        for mb in 0..mbs {
            let (mb_x, mb_y) = macroblock_xy(spec.width, mb);
            let d = luma_max_abs(orig, other, spec.width, frame, mb_x, mb_y, frame_bytes);
            out.push(PerMbLumaDiff {
                changed: d > threshold,
                max_delta_y: d,
            });
        }
    }
    Ok(out)
}

/// Which MBs had pixels painted (orig vs edited YUV, threshold 0).
pub fn pixel_edit_per_mb(orig: &[u8], edited: &[u8], spec: &IslandSpec) -> Result<Vec<bool>, String> {
    Ok(compare_yuv_per_mb(orig, edited, spec, 0)?
        .into_iter()
        .map(|d| d.changed)
        .collect())
}

fn load_syntax_bundle(dir: &Path) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), String> {
    let pred = std::fs::read(dir.join("pred_y_enc"))
        .map_err(|e| format!("read {}: {e}", dir.join("pred_y_enc").display()))?;
    let coeff = std::fs::read(dir.join("coeff_y_enc"))
        .map_err(|e| format!("read {}: {e}", dir.join("coeff_y_enc").display()))?;
    let types = std::fs::read(dir.join("type_enc"))
        .map_err(|e| format!("read {}: {e}", dir.join("type_enc").display()))?;
    Ok((pred, coeff, types))
}

/// Build per-frame grids. `decode_*` are optional second YUV pair (e.g. two ffmpeg decodes).
pub fn build_mb_grid_report(
    orig: &[u8],
    edited: &[u8],
    spec: &IslandSpec,
    decode_a: Option<&[u8]>,
    decode_b: Option<&[u8]>,
    decode_threshold: u8,
    syntax_orig_dir: Option<&Path>,
    syntax_edit_dir: Option<&Path>,
) -> Result<MbGridReport, String> {
    spec.validate()?;
    let pixel = pixel_edit_per_mb(orig, edited, spec)?;
    let decode = match (decode_a, decode_b) {
        (Some(a), Some(b)) => Some(compare_yuv_per_mb(a, b, spec, decode_threshold)?),
        _ => None,
    };
    let syntax = match (syntax_orig_dir, syntax_edit_dir) {
        (Some(a), Some(b)) => {
            let (pred_a, coeff_a, type_a) = load_syntax_bundle(a)?;
            let (pred_b, coeff_b, type_b) = load_syntax_bundle(b)?;
            Some(syntax_mb_changed(
                &pred_a, &pred_b, &coeff_a, &coeff_b, &type_a, &type_b,
            )?)
        }
        (None, None) => None,
        _ => return Err("pass both --syntax-orig-dir and --syntax-edit-dir".into()),
    };

    let mbs = spec.mbs_per_frame()?;
    let cols = spec.cols();
    let rows = spec.rows();
    let mut frames = Vec::with_capacity(spec.num_frames);
    for frame in 0..spec.num_frames {
        let base = frame * mbs;
        let mut role = Vec::with_capacity(mbs);
        let mut pixel_edited = Vec::with_capacity(mbs);
        let mut decode_changed = Vec::with_capacity(mbs);
        let mut max_delta_y = Vec::with_capacity(mbs);
        let mut syntax_changed = syntax.as_ref().map(|_| Vec::with_capacity(mbs));
        for mb in 0..mbs {
            let g = base + mb;
            role.push(role_to_code(mb_role(spec, g)?));
            pixel_edited.push(pixel[g]);
            if let Some(ref dec) = decode {
                decode_changed.push(dec[g].changed);
                max_delta_y.push(dec[g].max_delta_y);
            } else {
                decode_changed.push(false);
                max_delta_y.push(0);
            }
            if let Some(ref mut syn) = syntax_changed {
                syn.push(syntax.as_ref().unwrap()[g]);
            }
        }
        frames.push(MbFrameGrid {
            frame,
            cols,
            rows,
            role,
            pixel_edited,
            decode_changed,
            max_delta_y,
            syntax_changed,
        });
    }
    Ok(MbGridReport { spec: *spec, frames })
}

fn bool_row_to_bits(v: &[bool]) -> String {
    v.iter().map(|&b| if b { '1' } else { '0' }).collect()
}

fn role_row_to_chars(v: &[u8]) -> String {
    v.iter()
        .map(|&c| match c {
            0 => 'G',
            1 => 'H',
            2 => 'S',
            3 => 'P',
            4 => 'T',
            _ => '?',
        })
        .collect()
}

/// JSON for canvas embedding (`docs/results/mb-grid-*.json`).
pub fn mb_grid_to_json(report: &MbGridReport, label: &str, decode_label: &str) -> String {
    let s = &report.spec;
    let mut out = String::new();
    writeln!(
        out,
        "{{\n  \"label\": {:?},\n  \"decode_label\": {:?},\n  \"width\": {},\n  \"height\": {},\n  \"cols\": {},\n  \"rows\": {},\n  \"num_frames\": {},\n  \"frame_start\": {},\n  \"frame_end\": {},\n  \"box\": [{}, {}, {}, {}],\n  \"halo\": {},\n  \"has_syntax\": {},\n  \"frames\": [",
        label,
        decode_label,
        s.width,
        s.height,
        s.cols(),
        s.rows(),
        s.num_frames,
        s.frame_start,
        s.frame_end,
        s.rect.x,
        s.rect.y,
        s.rect.w,
        s.rect.h,
        s.halo_mbs,
        report.frames.first().and_then(|f| f.syntax_changed.as_ref()).is_some(),
    )
    .unwrap();
    for (i, f) in report.frames.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write!(
            out,
            "\n    {{\"f\": {}, \"role\": {:?}, \"pixel\": {:?}, \"decode\": {:?}",
            f.frame,
            role_row_to_chars(&f.role),
            bool_row_to_bits(&f.pixel_edited),
            bool_row_to_bits(&f.decode_changed),
        )
        .unwrap();
        if let Some(ref syn) = f.syntax_changed {
            write!(out, ", \"syntax\": {:?}", bool_row_to_bits(syn)).unwrap();
        }
        out.push('}');
    }
    out.push_str("\n  ]\n}\n");
    out
}

#[derive(Clone, Copy, Debug)]
pub struct MbCoord {
    pub frame: usize,
    pub mb_x: usize,
    pub mb_y: usize,
}

impl MbCoord {
    pub fn pixel_origin(&self) -> (usize, usize) {
        (self.mb_x * 16, self.mb_y * 16)
    }
}

fn write_mb_list(out: &mut String, name: &str, coords: &[MbCoord]) {
    writeln!(out, "  \"{name}\": [").unwrap();
    for (i, c) in coords.iter().enumerate() {
        let (px, py) = c.pixel_origin();
        if i > 0 {
            out.push(',');
        }
        write!(
            out,
            "\n    {{\"frame\": {}, \"mb_x\": {}, \"mb_y\": {}, \"px\": {}, \"py\": {}}}",
            c.frame, c.mb_x, c.mb_y, px, py
        )
        .unwrap();
    }
    if coords.is_empty() {
        out.push_str("\n  ");
    } else {
        out.push('\n');
    }
    writeln!(out, "  ],").unwrap();
}

/// Detailed walkthrough JSON: source, edit, gadget/halo lists, decode + syntax smear without pixel edit.
pub fn mb_walkthrough_to_json(
    report: &MbGridReport,
    source: &str,
    source_url: &str,
    decode_label: &str,
    syntax_label: &str,
) -> String {
    let s = &report.spec;
    let mut gadget = Vec::new();
    let mut halo = Vec::new();
    let mut decode_no_pixel = Vec::new();
    let mut decode_no_pixel_outside_i = Vec::new();
    let mut decode_halo_no_pixel = Vec::new();
    let mut syntax_no_pixel = Vec::new();
    let mut syntax_no_pixel_outside_i = Vec::new();
    let mut syntax_halo_no_pixel = Vec::new();
    let has_syntax = report.frames.first().and_then(|f| f.syntax_changed.as_ref()).is_some();

    for fg in &report.frames {
        let mbs = fg.role.len();
        for mb in 0..mbs {
            let (mb_x, mb_y) = macroblock_xy(s.width, mb);
            let coord = MbCoord {
                frame: fg.frame,
                mb_x,
                mb_y,
            };
            if fg.pixel_edited[mb] {
                gadget.push(coord);
            }
            if fg.role[mb] == role_to_code(MbRole::Halo) && !fg.pixel_edited[mb] {
                halo.push(coord);
            }
            if fg.decode_changed[mb] && !fg.pixel_edited[mb] {
                decode_no_pixel.push(coord);
                let role_code = fg.role[mb];
                if role_code == role_to_code(MbRole::Halo) {
                    decode_halo_no_pixel.push(coord);
                }
                if role_code != role_to_code(MbRole::Gadget) && role_code != role_to_code(MbRole::Halo) {
                    decode_no_pixel_outside_i.push(coord);
                }
            }
            if let Some(ref syn) = fg.syntax_changed {
                if syn[mb] && !fg.pixel_edited[mb] {
                    syntax_no_pixel.push(coord);
                    let role_code = fg.role[mb];
                    if role_code == role_to_code(MbRole::Halo) {
                        syntax_halo_no_pixel.push(coord);
                    }
                    if role_code != role_to_code(MbRole::Gadget) && role_code != role_to_code(MbRole::Halo) {
                        syntax_no_pixel_outside_i.push(coord);
                    }
                }
            }
        }
    }

    let mut out = String::new();
    writeln!(
        out,
        "{{\n  \"source\": {:?},\n  \"source_url\": {:?},\n  \"decode_label\": {:?},\n  \"syntax_label\": {:?},\n  \"has_syntax\": {},\n  \"width\": {},\n  \"height\": {},\n  \"cols\": {},\n  \"rows\": {},\n  \"num_frames\": {},\n  \"frame_start\": {},\n  \"frame_end\": {},\n  \"box\": [{}, {}, {}, {}],\n  \"halo_mbs\": {},\n  \"fill_y\": 0,\n  \"counts\": {{\n    \"gadget_mb_slots\": {},\n    \"halo_mb_slots\": {},\n    \"island_mb_slots\": {},\n    \"decode_no_pixel\": {},\n    \"decode_no_pixel_outside_i\": {},\n    \"decode_halo_no_pixel\": {},\n    \"syntax_no_pixel\": {},\n    \"syntax_no_pixel_outside_i\": {},\n    \"syntax_halo_no_pixel\": {}\n  }},",
        source,
        source_url,
        decode_label,
        syntax_label,
        has_syntax,
        s.width,
        s.height,
        s.cols(),
        s.rows(),
        s.num_frames,
        s.frame_start,
        s.frame_end,
        s.rect.x,
        s.rect.y,
        s.rect.w,
        s.rect.h,
        s.halo_mbs,
        gadget.len(),
        halo.len(),
        gadget.len() + halo.len(),
        decode_no_pixel.len(),
        decode_no_pixel_outside_i.len(),
        decode_halo_no_pixel.len(),
        syntax_no_pixel.len(),
        syntax_no_pixel_outside_i.len(),
        syntax_halo_no_pixel.len(),
    )
    .unwrap();
    write_mb_list(&mut out, "gadget_mbs", &gadget);
    write_mb_list(&mut out, "halo_mbs", &halo);
    write_mb_list(&mut out, "decode_no_pixel_mbs", &decode_no_pixel);
    write_mb_list(&mut out, "decode_no_pixel_outside_i_mbs", &decode_no_pixel_outside_i);
    write_mb_list(&mut out, "decode_halo_no_pixel_mbs", &decode_halo_no_pixel);
    write_mb_list(&mut out, "syntax_no_pixel_mbs", &syntax_no_pixel);
    write_mb_list(&mut out, "syntax_no_pixel_outside_i_mbs", &syntax_no_pixel_outside_i);
    write_mb_list(&mut out, "syntax_halo_no_pixel_mbs", &syntax_halo_no_pixel);
    out.push_str("  \"frames\": [");
    for (i, f) in report.frames.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write!(
            out,
            "\n    {{\"f\": {}, \"role\": {:?}, \"pixel\": {:?}, \"decode\": {:?}",
            f.frame,
            role_row_to_chars(&f.role),
            bool_row_to_bits(&f.pixel_edited),
            bool_row_to_bits(&f.decode_changed),
        )
        .unwrap();
        if let Some(ref syn) = f.syntax_changed {
            write!(out, ", \"syntax\": {:?}", bool_row_to_bits(syn)).unwrap();
        }
        out.push('}');
    }
    out.push_str("\n  ]\n}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::island::PixelRect;
    use crate::neighbor_experiment::synthetic_gradient_yuv;
    use crate::native_redact_edit_macroblocks;
    use crate::macroblock_yuv::{macroblocks_to_yuv420, yuv420_to_macroblocks};

    #[test]
    fn pixel_grid_only_gadget_edited() {
        let spec = IslandSpec {
            width: 64,
            height: 48,
            num_frames: 2,
            frame_start: 0,
            frame_end: 1,
            rect: PixelRect {
                x: 16,
                y: 16,
                w: 16,
                h: 16,
            },
            halo_mbs: 1,
        };
        let orig = synthetic_gradient_yuv(spec.width, spec.height, spec.num_frames);
        let (y, u, v) =
            yuv420_to_macroblocks(&orig, spec.width, spec.height, spec.num_frames).unwrap();
        let (ey, eu, ev) = native_redact_edit_macroblocks(
            &y, &u, &v, spec.width, spec.height, spec.num_frames, 16, 16, 16, 16, 0, 1, 0,
        )
        .unwrap();
        let edited =
            macroblocks_to_yuv420(&ey, &eu, &ev, spec.width, spec.height, spec.num_frames).unwrap();
        let report = build_mb_grid_report(&orig, &edited, &spec, None, None, 0, None, None).unwrap();
        let f0 = &report.frames[0];
        assert_eq!(f0.pixel_edited.iter().filter(|&&b| b).count(), 1);
        assert!(f0.role.contains(&role_to_code(MbRole::Halo)));
    }
}
