//! Shared helpers for neighbor-impact / island-size experiments.
//!
//! Used by `neighbor_reencode` and `experiment_matrix` examples. Measures
//! geometric island `I`, pixel redact locality, and (with ffmpeg) full
//! re-encode contamination vs splice copy-syntax.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::island::{island_counts, IslandCounts, IslandSpec, PixelRect};
use crate::macroblock_yuv::{macroblocks_to_yuv420, yuv420_frame_bytes, yuv420_to_macroblocks};
use crate::native_redact_edit_macroblocks;
use crate::neighbor_impact::{compare_yuv_to_island, ImpactReport};

/// One row in a sweep (`experiment_matrix` CSV).
#[derive(Clone, Debug)]
pub struct ExperimentRow {
    pub label: String,
    pub spec: IslandSpec,
    pub counts: IslandCounts,
    pub pixel_outside_i: usize,
    pub intra_outside_i_d0: Option<usize>,
    pub intra_outside_i_d2: Option<usize>,
    pub intra_halo_changed_d0: Option<usize>,
    pub gop_outside_i_d0: Option<usize>,
    pub gop_post_changed_d0: Option<usize>,
    pub gop_outside_i_d2: Option<usize>,
    pub gop_post_changed_d2: Option<usize>,
}

/// Default neighbor-experiment resolution (720p HD). CIF (352×288) was only used early on
/// because the Eva paper dataset (Foreman) is CIF — not because it is representative.
pub const HD_WIDTH: usize = 1280;
pub const HD_HEIGHT: usize = 720;
pub const HD_FRAMES: usize = 30;

/// Repo-local 720p fixture (see `docs/fixtures/README.md`). Gitignored; generate with script.
pub const DEFAULT_FIXTURE_YUV: &str = "docs/fixtures/sample_720p_30f.yuv";

/// Center redact box (~50% of 720p frame).
pub fn hd_center_box() -> PixelRect {
    PixelRect {
        x: 320,
        y: 180,
        w: 640,
        h: 360,
    }
}

/// Same coverage as [`hd_center_box`], snapped outward to 16×16 MB grid (whole MBs blackened).
pub fn hd_center_box_mb_aligned() -> PixelRect {
    hd_center_box().snap_outward_to_macroblocks()
}

pub fn hd_small_box() -> PixelRect {
    PixelRect {
        x: 480,
        y: 270,
        w: 320,
        h: 240,
    }
}

pub fn hd_wide_box() -> PixelRect {
    PixelRect {
        x: 160,
        y: 120,
        w: 960,
        h: 400,
    }
}

/// Two-frame redact window centered in the clip.
pub fn redact_window(num_frames: usize) -> (usize, usize) {
    let mid = num_frames / 2;
    (mid.saturating_sub(1), (mid + 1).min(num_frames))
}

pub fn load_yuv_or_fixture(
    yuv_path: Option<&Path>,
    width: usize,
    height: usize,
    frames: usize,
) -> Result<Vec<u8>, String> {
    match yuv_path {
        Some(p) => load_yuv420(p, width, height, frames),
        None => {
            let fixture = Path::new(DEFAULT_FIXTURE_YUV);
            if fixture.is_file() {
                eprintln!("using fixture {}", fixture.display());
                load_yuv420(fixture, width, height, frames)
            } else {
                eprintln!(
                    "no --yuv and no {} — synthetic {width}×{height} gradient",
                    DEFAULT_FIXTURE_YUV
                );
                Ok(synthetic_gradient_yuv(width, height, frames))
            }
        }
    }
}

pub fn synthetic_gradient_yuv(width: usize, height: usize, frames: usize) -> Vec<u8> {
    let fb = width * height * 3 / 2;
    let mut yuv = vec![128u8; fb * frames];
    for f in 0..frames {
        let base = f * fb;
        for y in 0..height {
            for x in 0..width {
                yuv[base + y * width + x] = ((x + y + f * 17) % 220 + 16) as u8;
            }
        }
    }
    yuv
}

pub fn load_yuv420(path: &Path, width: usize, height: usize, frames: usize) -> Result<Vec<u8>, String> {
    let need = yuv420_frame_bytes(width, height)?
        .checked_mul(frames)
        .ok_or_else(|| "frame count overflow".to_string())?;
    let buf = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if buf.len() < need {
        return Err(format!(
            "{} too short: need {need} bytes for {frames} frame(s), got {}",
            path.display(),
            buf.len()
        ));
    }
    Ok(buf[..need].to_vec())
}

pub fn redact_yuv(orig: &[u8], spec: &IslandSpec) -> Result<Vec<u8>, String> {
    let (y, u, v) = yuv420_to_macroblocks(orig, spec.width, spec.height, spec.num_frames)?;
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
    )?;
    macroblocks_to_yuv420(&ey, &eu, &ev, spec.width, spec.height, spec.num_frames)
}

pub fn outside_island_changed(r: &ImpactReport) -> usize {
    r.outside_island_changed()
}

pub fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn run_ffmpeg(args: &[&str]) -> Result<(), String> {
    let out = Command::new("ffmpeg")
        .args(args)
        .output()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "ffmpeg failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

fn encode_decode_yuv(
    yuv: &Path,
    decoded: &Path,
    width: usize,
    height: usize,
    gop: usize,
    qp: u8,
) -> Result<(), String> {
    let mp4 = decoded.with_extension("mp4");
    let size = format!("{width}x{height}");
    run_ffmpeg(&[
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "yuv420p",
        "-s:v",
        &size,
        "-r",
        "30",
        "-i",
        yuv.to_str().unwrap(),
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-tune",
        "zerolatency",
        "-qp",
        &qp.to_string(),
        "-g",
        &gop.to_string(),
        "-bf",
        "0",
        "-pix_fmt",
        "yuv420p",
        mp4.to_str().unwrap(),
    ])?;
    run_ffmpeg(&[
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-i",
        mp4.to_str().unwrap(),
        "-pix_fmt",
        "yuv420p",
        decoded.to_str().unwrap(),
    ])
}

fn temp_workdir() -> Result<PathBuf, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("eva-neighbor-{nanos}"));
    fs::create_dir_all(&dir).map_err(|e| format!("temp dir: {e}"))?;
    Ok(dir)
}

/// Compare decoded luma after independent x264 encodes of `orig` vs `edited`.
pub fn compare_reencoded_decoded(
    orig: &[u8],
    edited: &[u8],
    spec: &IslandSpec,
    gop: usize,
    qp: u8,
) -> Result<(ImpactReport, ImpactReport), String> {
    let dir = temp_workdir()?;
    let tag = format!("gop{gop}");
    let orig_path = dir.join(format!("{tag}-orig.yuv"));
    let edit_path = dir.join(format!("{tag}-edit.yuv"));
    let orig_dec = dir.join(format!("{tag}-orig-dec.yuv"));
    let edit_dec = dir.join(format!("{tag}-edit-dec.yuv"));
    fs::write(&orig_path, orig).map_err(|e| e.to_string())?;
    fs::write(&edit_path, edited).map_err(|e| e.to_string())?;
    encode_decode_yuv(
        &orig_path,
        &orig_dec,
        spec.width,
        spec.height,
        gop,
        qp,
    )?;
    encode_decode_yuv(
        &edit_path,
        &edit_dec,
        spec.width,
        spec.height,
        gop,
        qp,
    )?;
    let a = fs::read(&orig_dec).map_err(|e| e.to_string())?;
    let b = fs::read(&edit_dec).map_err(|e| e.to_string())?;
    let d0 = compare_yuv_to_island(&a, &b, spec, 0)?;
    let d2 = compare_yuv_to_island(&a, &b, spec, 2)?;
    let _ = fs::remove_dir_all(&dir);
    Ok((d0, d2))
}

/// Run pixel + optional ffmpeg passes for one scenario.
pub fn run_experiment(
    label: &str,
    orig: &[u8],
    spec: IslandSpec,
    gop: usize,
    qp: u8,
    with_ffmpeg: bool,
) -> Result<ExperimentRow, String> {
    spec.validate()?;
    let counts = island_counts(&spec)?;
    let edited = redact_yuv(orig, &spec)?;
    let pix = compare_yuv_to_island(orig, &edited, &spec, 0)?;
    let mut row = ExperimentRow {
        label: label.to_string(),
        spec,
        counts,
        pixel_outside_i: outside_island_changed(&pix),
        intra_outside_i_d0: None,
        intra_outside_i_d2: None,
        intra_halo_changed_d0: None,
        gop_outside_i_d0: None,
        gop_post_changed_d0: None,
        gop_outside_i_d2: None,
        gop_post_changed_d2: None,
    };

    if !with_ffmpeg {
        return Ok(row);
    }

    let (intra0, intra2) = compare_reencoded_decoded(orig, &edited, &spec, 1, qp)?;
    row.intra_outside_i_d0 = Some(outside_island_changed(&intra0));
    row.intra_outside_i_d2 = Some(outside_island_changed(&intra2));
    row.intra_halo_changed_d0 = Some(intra0.halo.n_changed);

    if gop > 1 {
        let (gop0, gop2) = compare_reencoded_decoded(orig, &edited, &spec, gop, qp)?;
        row.gop_outside_i_d0 = Some(outside_island_changed(&gop0));
        row.gop_post_changed_d0 = Some(gop0.skip_post.n_changed);
        row.gop_outside_i_d2 = Some(outside_island_changed(&gop2));
        row.gop_post_changed_d2 = Some(gop2.skip_post.n_changed);
    }

    Ok(row)
}

pub fn csv_header() -> &'static str {
    "label,width,height,frames,box_x,box_y,box_w,box_h,frame_start,frame_end,halo,\
gadget_mb,halo_mb,island_mb,total_mb,island_pct,\
pixel_outside_i,\
intra_outside_i_d0,intra_outside_i_d2,intra_halo_d0,\
gop,gop_outside_i_d0,gop_post_d0,gop_outside_i_d2,gop_post_d2"
}

pub fn row_to_csv(r: &ExperimentRow, gop: usize) -> String {
    let s = &r.spec;
    let intra0 = r
        .intra_outside_i_d0
        .map(|v| v.to_string())
        .unwrap_or_else(|| "".into());
    let intra2 = r
        .intra_outside_i_d2
        .map(|v| v.to_string())
        .unwrap_or_else(|| "".into());
    let ihalo = r
        .intra_halo_changed_d0
        .map(|v| v.to_string())
        .unwrap_or_else(|| "".into());
    let g0 = r
        .gop_outside_i_d0
        .map(|v| v.to_string())
        .unwrap_or_else(|| "".into());
    let gp0 = r
        .gop_post_changed_d0
        .map(|v| v.to_string())
        .unwrap_or_else(|| "".into());
    let g2 = r
        .gop_outside_i_d2
        .map(|v| v.to_string())
        .unwrap_or_else(|| "".into());
    let gp2 = r
        .gop_post_changed_d2
        .map(|v| v.to_string())
        .unwrap_or_else(|| "".into());
    format!(
        "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{:.4},{},{},{},{},{},{},{},{},{}",
        csv_escape(&r.label),
        s.width,
        s.height,
        s.num_frames,
        s.rect.x,
        s.rect.y,
        s.rect.w,
        s.rect.h,
        s.frame_start,
        s.frame_end,
        s.halo_mbs,
        r.counts.gadget,
        r.counts.halo,
        r.counts.island(),
        r.counts.total,
        100.0 * r.counts.island_fraction(),
        r.pixel_outside_i,
        intra0,
        intra2,
        ihalo,
        gop,
        g0,
        gp0,
        g2,
        gp2,
    )
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::island::PixelRect;

    #[test]
    fn pixel_redact_is_local() {
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
        let row = run_experiment("t", &orig, spec, 8, 23, false).unwrap();
        assert_eq!(row.pixel_outside_i, 0);
        assert!(row.counts.island() > row.counts.gadget);
    }
}
