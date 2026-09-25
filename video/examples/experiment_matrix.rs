//! Sweep island geometry + neighbor leak across several redact scenarios.
//!
//! Default: **1280×720 × 30 frames**, using `docs/fixtures/sample_720p_30f.yuv` when present.
//!
//! ```bash
//! # Default (720p fixture or synthetic HD gradient)
//! cargo run --release -p video --example experiment_matrix -- \
//!   --out docs/results/neighbor-matrix-720p.csv
//!
//! # Your own clip
//! cargo run --release -p video --example experiment_matrix -- \
//!   --yuv ./clip.yuv --width 1920 --height 1080 --frames 60 --out matrix.csv
//! ```

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use video::{
    csv_header, ffmpeg_available, hd_center_box, hd_small_box, hd_wide_box, load_yuv_or_fixture,
    redact_window, row_to_csv, run_experiment, HD_FRAMES, HD_HEIGHT, HD_WIDTH, IslandSpec,
};

struct Scenario {
    label: &'static str,
    rect: video::PixelRect,
    gop: usize,
}

fn usage() -> &'static str {
    "Usage: experiment_matrix [--yuv FILE] [--width W] [--height H] [--frames N] [--qp Q] [--out CSV]\n\
     \n\
     Runs a fixed scenario matrix (geometry + pixel + ffmpeg if available).\n\
     Default: 1280×720 × 30 frames; loads docs/fixtures/sample_720p_30f.yuv if present."
}

fn parse_args() -> Result<(Option<PathBuf>, usize, usize, usize, u8, Option<PathBuf>), String> {
    let mut yuv = None;
    let mut width = HD_WIDTH;
    let mut height = HD_HEIGHT;
    let mut frames = HD_FRAMES;
    let mut qp = 23u8;
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
            "--qp" => {
                i += 1;
                let v: usize = args[i].parse().map_err(|_| "bad --qp")?;
                if v > 51 {
                    return Err("qp must be 0..=51".into());
                }
                qp = v as u8;
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
    Ok((yuv, width, height, frames, qp, out))
}

fn scenarios(_num_frames: usize) -> Vec<Scenario> {
    vec![
        Scenario {
            label: "hd_center_box",
            rect: hd_center_box(),
            gop: 8,
        },
        Scenario {
            label: "small_box",
            rect: hd_small_box(),
            gop: 8,
        },
        Scenario {
            label: "wide_box",
            rect: hd_wide_box(),
            gop: 8,
        },
        Scenario {
            label: "hd_center_gop1",
            rect: hd_center_box(),
            gop: 1,
        },
    ]
}

fn main() -> ExitCode {
    let parsed = match parse_args() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let (yuv_path, width, height, num_frames, qp, out_path) = parsed;
    let (frame_start, frame_end) = redact_window(num_frames);

    let yuv = match load_yuv_or_fixture(yuv_path.as_deref(), width, height, num_frames) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let ffmpeg = ffmpeg_available();
    if !ffmpeg {
        eprintln!("warning: ffmpeg not found — CSV will omit re-encode columns");
    }

    let mut lines = vec![csv_header().to_string()];
    for sc in scenarios(num_frames) {
        let spec = IslandSpec {
            width,
            height,
            num_frames,
            frame_start,
            frame_end,
            rect: sc.rect,
            halo_mbs: 1,
        };
        match run_experiment(sc.label, &yuv, spec, sc.gop, qp, ffmpeg) {
            Ok(row) => {
                eprintln!(
                    "ok {}  I={:.2}%  pixel_out={}  intra_out_d0={:?}  gop_post_d0={:?}",
                    sc.label,
                    100.0 * row.counts.island_fraction(),
                    row.pixel_outside_i,
                    row.intra_outside_i_d0,
                    row.gop_post_changed_d0,
                );
                lines.push(row_to_csv(&row, sc.gop));
            }
            Err(e) => {
                eprintln!("{}: {e}", sc.label);
                return ExitCode::FAILURE;
            }
        }
    }

    let body = lines.join("\n");
    if let Some(path) = out_path {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                let _ = fs::create_dir_all(parent);
            }
        }
        if let Err(e) = fs::write(&path, &body) {
            eprintln!("write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        eprintln!("wrote {}", path.display());
    } else {
        println!("{body}");
    }
    ExitCode::SUCCESS
}
