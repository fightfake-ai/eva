//! Export per-MB grids (pixel edit, decode diff, optional syntax) as JSON for canvases.
//!
//! ```bash
//! cargo run --release -p video --example mb_grid_export -- \
//!   --out docs/results/mb-grid-synthetic-8f-intra.json
//!
//! # With GOP=8 decode layer
//! cargo run --release -p video --example mb_grid_export -- \
//!   --gop 8 --out docs/results/mb-grid-synthetic-8f-gop8.json
//!
//! # With JM syntax dumps (directories containing pred_y_enc, coeff_y_enc, type_enc)
//! cargo run --release -p video --example mb_grid_export -- \
//!   --syntax-orig-dir ./dumps/orig --syntax-edit-dir ./dumps/edit \
//!   --out mb-grid-syntax.json
//! ```

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use video::{
    build_mb_grid_report, ffmpeg_available, hd_center_box, load_yuv_or_fixture, mb_grid_to_json,
    redact_window, HD_FRAMES, HD_HEIGHT, HD_WIDTH, IslandSpec,
};

fn usage() -> &'static str {
    "Usage: mb_grid_export [--yuv FILE] [--width W] [--height H] [--frames N] \\\n\
     \t[--box x y w h] [--frame-start F] [--frame-end F] [--halo N] \\\n\
     \t[--gop N] [--qp Q] [--syntax-orig-dir DIR] [--syntax-edit-dir DIR] --out FILE.json"
}

fn parse_args() -> Result<
    (
        Option<PathBuf>,
        IslandSpec,
        usize,
        u8,
        Option<PathBuf>,
        Option<PathBuf>,
        PathBuf,
    ),
    String,
> {
    let mut yuv = None;
    let mut width = HD_WIDTH;
    let mut height = HD_HEIGHT;
    let mut frames = HD_FRAMES;
    let mut box_xywh = {
        let b = hd_center_box();
        (b.x, b.y, b.w, b.h)
    };
    let mut frame_start = 0usize;
    let mut frame_end = 0usize;
    let mut frame_window_set = false;
    let mut halo = 1usize;
    let mut gop = 1usize;
    let mut qp = 23u8;
    let mut syntax_orig = None;
    let mut syntax_edit = None;
    let mut out = None;

    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--yuv" => {
                i += 1;
                yuv = Some(PathBuf::from(args.get(i).ok_or("--yuv needs path")?));
            }
            "--width" => {
                i += 1;
                width = args[i].parse().map_err(|_| "bad --width")?;
            }
            "--height" => {
                i += 1;
                height = args[i].parse().map_err(|_| "bad --height")?;
            }
            "--frames" => {
                i += 1;
                frames = args[i].parse().map_err(|_| "bad --frames")?;
            }
            "--box" => {
                i += 1;
                box_xywh.0 = args.get(i).ok_or("--box needs x")?.parse().map_err(|_| "bad x")?;
                i += 1;
                box_xywh.1 = args.get(i).ok_or("--box needs y")?.parse().map_err(|_| "bad y")?;
                i += 1;
                box_xywh.2 = args.get(i).ok_or("--box needs w")?.parse().map_err(|_| "bad w")?;
                i += 1;
                box_xywh.3 = args.get(i).ok_or("--box needs h")?.parse().map_err(|_| "bad h")?;
            }
            "--frame-start" => {
                i += 1;
                frame_start = args[i].parse().map_err(|_| "bad --frame-start")?;
                frame_window_set = true;
            }
            "--frame-end" => {
                i += 1;
                frame_end = args[i].parse().map_err(|_| "bad --frame-end")?;
                frame_window_set = true;
            }
            "--halo" => {
                i += 1;
                halo = args[i].parse().map_err(|_| "bad --halo")?;
            }
            "--gop" => {
                i += 1;
                gop = args[i].parse().map_err(|_| "bad --gop")?;
            }
            "--qp" => {
                i += 1;
                let v: usize = args[i].parse().map_err(|_| "bad --qp")?;
                if v > 51 {
                    return Err("qp must be 0..=51".into());
                }
                qp = v as u8;
            }
            "--syntax-orig-dir" => {
                i += 1;
                syntax_orig = Some(PathBuf::from(
                    args.get(i).ok_or("--syntax-orig-dir needs path")?,
                ));
            }
            "--syntax-edit-dir" => {
                i += 1;
                syntax_edit = Some(PathBuf::from(
                    args.get(i).ok_or("--syntax-edit-dir needs path")?,
                ));
            }
            "--out" => {
                i += 1;
                out = Some(PathBuf::from(args.get(i).ok_or("--out needs path")?));
            }
            "-h" | "--help" => return Err(usage().into()),
            other => return Err(format!("unknown arg: {other}")),
        }
        i += 1;
    }
    let out = out.ok_or("--out FILE.json is required")?;
    let (fs, fe) = if frame_window_set {
        (frame_start, frame_end)
    } else {
        redact_window(frames)
    };
    let spec = IslandSpec {
        width,
        height,
        num_frames: frames,
        frame_start: fs,
        frame_end: fe,
        rect: video::PixelRect {
            x: box_xywh.0,
            y: box_xywh.1,
            w: box_xywh.2,
            h: box_xywh.3,
        },
        halo_mbs: halo,
    };
    spec.validate()?;
    Ok((yuv, spec, gop, qp, syntax_orig, syntax_edit, out))
}

fn main() -> ExitCode {
    let parsed = match parse_args() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let (yuv_path, spec, gop, qp, syntax_orig, syntax_edit, out_path) = parsed;

    let orig = match load_yuv_or_fixture(yuv_path.as_deref(), spec.width, spec.height, spec.num_frames) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let edited = match video::redact_yuv(&orig, &spec) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("redact: {e}");
            return ExitCode::FAILURE;
        }
    };

    let (decode_a, decode_b, decode_label) = if ffmpeg_available() {
        match decoded_pair(&orig, &edited, &spec, gop, qp) {
            Ok(pair) => (
                Some(pair.0),
                Some(pair.1),
                format!("ffmpeg GOP={gop} decode ΔY>0"),
            ),
            Err(e) => {
                eprintln!("decode: {e}");
                (None, None, "pixel only".into())
            }
        }
    } else {
        eprintln!("warning: ffmpeg not found — JSON will omit decode layer");
        (None, None, "pixel only".into())
    };

    let report = match build_mb_grid_report(
        &orig,
        &edited,
        &spec,
        decode_a.as_deref(),
        decode_b.as_deref(),
        0,
        syntax_orig.as_deref(),
        syntax_edit.as_deref(),
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("grid: {e}");
            return ExitCode::FAILURE;
        }
    };

    let json = mb_grid_to_json(&report, "sample_720p_30f", &decode_label);
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = fs::create_dir_all(parent);
        }
    }
    if let Err(e) = fs::write(&out_path, &json) {
        eprintln!("write {}: {e}", out_path.display());
        return ExitCode::FAILURE;
    }
    eprintln!("wrote {}", out_path.display());
    ExitCode::SUCCESS
}

/// Encode/decode orig and edited; return decoded YUV pair for grid diff.
fn decoded_pair(
    orig: &[u8],
    edited: &[u8],
    spec: &IslandSpec,
    gop: usize,
    qp: u8,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    use std::time::{SystemTime, UNIX_EPOCH};
    use video::yuv420_frame_bytes;

    let dir = {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("eva-mbgrid-{nanos}"))
    };
    fs::create_dir_all(&dir).map_err(|e| format!("temp: {e}"))?;
    let tag = format!("gop{gop}");
    let orig_path = dir.join(format!("{tag}-orig.yuv"));
    let edit_path = dir.join(format!("{tag}-edit.yuv"));
    let orig_dec = dir.join(format!("{tag}-orig-dec.yuv"));
    let edit_dec = dir.join(format!("{tag}-edit-dec.yuv"));
    fs::write(&orig_path, orig).map_err(|e| e.to_string())?;
    fs::write(&edit_path, edited).map_err(|e| e.to_string())?;
    encode_decode_file(&orig_path, &orig_dec, spec, gop, qp)?;
    encode_decode_file(&edit_path, &edit_dec, spec, gop, qp)?;
    let a = fs::read(&orig_dec).map_err(|e| e.to_string())?;
    let b = fs::read(&edit_dec).map_err(|e| e.to_string())?;
    let need = yuv420_frame_bytes(spec.width, spec.height)? * spec.num_frames;
    if a.len() < need || b.len() < need {
        return Err("decoded YUV too short".into());
    }
    let _ = fs::remove_dir_all(&dir);
    Ok((a[..need].to_vec(), b[..need].to_vec()))
}

fn encode_decode_file(
    yuv: &Path,
    decoded: &Path,
    spec: &IslandSpec,
    gop: usize,
    qp: u8,
) -> Result<(), String> {
    use std::process::Command;
    let mp4 = decoded.with_extension("mp4");
    let size = format!("{}x{}", spec.width, spec.height);
    let run = |args: &[&str]| -> Result<(), String> {
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
    };
    run(&[
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
    run(&[
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
