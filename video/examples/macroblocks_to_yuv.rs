//! Convert Eva macroblock files → planar YUV 4:2:0 for playback.
//!
//! # What this does
//!
//! Reassembles `orig_y_enc` / `orig_u_enc` / `orig_v_enc` into a standard **planar
//! YUV 4:2:0** file you can play with ffplay or re-encode to mp4.
//!
//! Optionally applies the same **brightness** edit Eva proves (`BRIGHTNESS=416`) so
//! you can preview the edited video off-chain.
//!
//! # Usage
//!
//! ```bash
//! # Original pixels
//! cargo run --release -p video --example macroblocks_to_yuv -- \
//!   ./data_parsed/runway_demo runway_orig.yuv 352 288 30
//!
//! # Preview after brightness edit (matches BrightnessCfg(416) in edit_bright_only)
//! BRIGHTNESS=416 cargo run --release -p video --example macroblocks_to_yuv -- \
//!   ./data_parsed/runway_demo runway_bright.yuv 352 288 30
//!
//! ffplay -f rawvideo -pix_fmt yuv420p -s 352x288 runway_bright.yuv
//! ```

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use video::macroblock_yuv::{macroblocks_per_frame, macroblocks_to_yuv420, read_macroblock_dir};

fn usage() -> &'static str {
    "Usage: macroblocks_to_yuv <macroblock_dir> <output.yuv> <width> <height> [num_frames]\n\
     \n\
     Reassembles orig_*_enc into planar YUV420p.\n\
     Set BRIGHTNESS=<u16> to apply the Eva brightness edit on export.\n\
     If num_frames is omitted, inferred from orig_y_enc size."
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 4 {
        eprintln!("{}", usage());
        return ExitCode::FAILURE;
    }

    let input_dir = PathBuf::from(&args[0]);
    let output = PathBuf::from(&args[1]);
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

    let brightness = env::var("BRIGHTNESS").ok().and_then(|s| s.parse().ok());

    let (orig_y, orig_u, orig_v) = match read_macroblock_dir(&input_dir) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("read {}: {e}", input_dir.display());
            return ExitCode::FAILURE;
        }
    };

    let mbs_per_frame = match macroblocks_per_frame(width, height) {
        Ok(n) => n,
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
        if orig_y.len() % (mbs_per_frame * 256) != 0 {
            eprintln!(
                "orig_y_enc size {} not aligned to frame macroblocks; pass num_frames",
                orig_y.len()
            );
            return ExitCode::FAILURE;
        }
        orig_y.len() / (mbs_per_frame * 256)
    };

    let yuv = match macroblocks_to_yuv420(
        &orig_y,
        &orig_u,
        &orig_v,
        width,
        height,
        num_frames,
        brightness,
    ) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = fs::write(&output, &yuv) {
        eprintln!("write {}: {e}", output.display());
        return ExitCode::FAILURE;
    }

    println!("Wrote {} ({} bytes, {num_frames} frames at {width}×{height})", output.display(), yuv.len());
    if let Some(b) = brightness {
        println!("  applied brightness scale {b} (Eva BrightnessCfg)");
    }
    println!();
    println!("Play:");
    println!("  ffplay -f rawvideo -pix_fmt yuv420p -s {width}x{height} {}", output.display());

    ExitCode::SUCCESS
}
