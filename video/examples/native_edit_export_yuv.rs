//! Export Eva macroblock files to playable planar YUV 4:2:0.
//!
//! # What this does
//!
//! 1. **Optional native edit** — if `BRIGHTNESS=<u16>` is set, runs
//!    [`Brightness::edit_native`] on every macroblock (reference implementation, not in-circuit).
//! 2. **Export** — stitches `orig_*_enc` into standard planar YUV for ffplay / ffmpeg.
//!
//! Without `BRIGHTNESS`, step 1 is skipped (identity) and only reassembly runs.
//!
//! # Usage
//!
//! ```bash
//! # Original pixels (no edit)
//! cargo run --release -p video --example native_edit_export_yuv -- \
//!   ./data_parsed/my_clip my_clip_orig.yuv 352 288 30
//!
//! # Native brightness edit + export (matches BrightnessCfg(416) in edit_bright_only)
//! BRIGHTNESS=416 cargo run --release -p video --example native_edit_export_yuv -- \
//!   ./data_parsed/my_clip my_clip_edited.yuv 352 288 30
//!
//! ffplay -f rawvideo -pix_fmt yuv420p -s 352x288 my_clip_edited.yuv
//! ```

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use video::macroblock_yuv::{
    macroblocks_per_frame, macroblocks_to_yuv420, native_brightness_export_yuv420,
    read_macroblock_dir,
};

fn usage() -> &'static str {
    "Usage: native_edit_export_yuv <macroblock_dir> <output.yuv> <width> <height> [num_frames]\n\
     \n\
     Optional native edit (BRIGHTNESS=<u16>) then export orig_*_enc to planar YUV420p.\n\
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

    let yuv = match brightness {
        Some(scale) => native_brightness_export_yuv420(
            &orig_y,
            &orig_u,
            &orig_v,
            width,
            height,
            num_frames,
            scale,
        ),
        None => macroblocks_to_yuv420(&orig_y, &orig_u, &orig_v, width, height, num_frames),
    };
    let yuv = match yuv {
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
        println!("  native edit: brightness scale {b} (Brightness::edit_native)");
    } else {
        println!("  no native edit (reassemble only)");
    }
    println!();
    println!("Play:");
    println!("  ffplay -f rawvideo -pix_fmt yuv420p -s {width}x{height} {}", output.display());

    ExitCode::SUCCESS
}
