//! Convert planar YUV 4:2:0 → Eva macroblock files for lossless proofs.
//!
//! # What this does
//!
//! Eva's prover reads per-macroblock pixel dumps (`orig_y_enc`, `orig_u_enc`,
//! `orig_v_enc`), not `.mp4` or a single `.yuv` frame directly. This tool:
//!
//! 1. Reads raw **planar YUV 4:2:0** (Y plane, then U, then V — as ffmpeg outputs)
//! 2. Splits each frame into 16×16 luma + 8×8 chroma **macroblocks** in Eva order
//! 3. Writes the three `orig_*_enc` files under an output folder
//!
//! Use that folder as `DATA_PATH/<name>/` when running `edit_bright_only` etc.
//!
//! # Prepare input with ffmpeg
//!
//! ```bash
//! ffmpeg -i runway.mp4 -vf scale=352:288 -pix_fmt yuv420p -frames:v 30 runway.yuv
//! ```
//!
//! Width and height must be multiples of 16.
//!
//! # Usage
//!
//! ```bash
//! cargo run --release -p video --example yuv_to_macroblocks -- \
//!   runway.yuv ./data_parsed/runway_demo 352 288 30
//! ```

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use video::macroblock_yuv::{write_macroblock_dir, yuv420_frame_bytes, yuv420_to_macroblocks};

fn usage() -> &'static str {
    "Usage: yuv_to_macroblocks <input.yuv> <output_dir> <width> <height> [num_frames]\n\
     \n\
     Converts planar YUV420p to Eva orig_y_enc / orig_u_enc / orig_v_enc.\n\
     If num_frames is omitted, inferred from file size."
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 4 {
        eprintln!("{}", usage());
        return ExitCode::FAILURE;
    }

    let input = PathBuf::from(&args[0]);
    let output_dir = PathBuf::from(&args[1]);
    let width: usize = match args[2].parse() {
        Ok(w) => w,
        Err(_) => {
            eprintln!("invalid width: {}", args[2]);
            return ExitCode::FAILURE;
        }
    };
    let height: usize = match args[3].parse() {
        Ok(h) => h,
        Err(_) => {
            eprintln!("invalid height: {}", args[3]);
            return ExitCode::FAILURE;
        }
    };

    let yuv = match fs::read(&input) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read {}: {e}", input.display());
            return ExitCode::FAILURE;
        }
    };

    let frame_bytes = match yuv420_frame_bytes(width, height) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let num_frames = if args.len() >= 5 {
        match args[4].parse() {
            Ok(n) => n,
            Err(_) => {
                eprintln!("invalid num_frames: {}", args[4]);
                return ExitCode::FAILURE;
            }
        }
    } else {
        if yuv.len() % frame_bytes != 0 {
            eprintln!(
                "file size {} is not a multiple of one frame ({} bytes); pass num_frames explicitly",
                yuv.len(),
                frame_bytes
            );
            return ExitCode::FAILURE;
        }
        yuv.len() / frame_bytes
    };

    let (orig_y, orig_u, orig_v) = match yuv420_to_macroblocks(&yuv, width, height, num_frames) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = write_macroblock_dir(&output_dir, &orig_y, &orig_u, &orig_v) {
        eprintln!("write {}: {e}", output_dir.display());
        return ExitCode::FAILURE;
    }

    let mbs = (width / 16) * (height / 16) * num_frames;
    println!("Wrote Eva macroblocks → {}", output_dir.display());
    println!("  frames={num_frames}  macroblocks={mbs}  resolution={width}×{height}");
    println!("  orig_y_enc  {} bytes", orig_y.len());
    println!("  orig_u_enc  {} bytes", orig_u.len());
    println!("  orig_v_enc  {} bytes", orig_v.len());
    println!();
    println!("Next:");
    println!("  export DATA_PATH=/path/to/data_parsed");
    println!("  QUICK=1 cargo run --release -p video --example edit_bright_only");
    println!("  (point examples at folder name: {})", output_dir.file_name().unwrap().to_string_lossy());

    ExitCode::SUCCESS
}
