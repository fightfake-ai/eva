//! Convert a still image (PNG/JPEG/…) → Eva macroblock files (`num_frames = 1`).
//!
//! # What this does
//!
//! 1. Decodes the image with the `image` crate
//! 2. Crops to the largest top-left multiple of 16
//! 3. Converts RGB → planar YUV 4:2:0
//! 4. Tiles into Eva `orig_*_enc` macroblock dumps
//!
//! # Usage
//!
//! ```bash
//! cargo run --release -p video --example image_to_macroblocks -- \
//!   photo.png ./data_parsed/photo
//! ```
//!
//! Then prove with Eva examples (`QUICK=1`, `DATA_PATH=…`) or toolkit:
//!
//! ```bash
//! fightfake prove-edit --input photo.png --gadget brightness --out-dir out/
//! ```

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use video::{
    crop_to_macroblock_grid, rgb8_to_yuv420p, write_macroblock_dir, yuv420_to_macroblocks,
};

fn usage() -> &'static str {
    "Usage: image_to_macroblocks <input.png|jpg|…> <output_dir>\n\
     \n\
     Decodes a still image, crops to multiples of 16, writes Eva orig_*_enc."
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("{}", usage());
        return ExitCode::FAILURE;
    }

    let input = PathBuf::from(&args[0]);
    let output_dir = PathBuf::from(&args[1]);

    let img = match image::open(&input) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("decode {}: {e}", input.display());
            return ExitCode::FAILURE;
        }
    };
    let rgb = img.to_rgb8();
    let (ow, oh) = rgb.dimensions();
    let (width, height) = crop_to_macroblock_grid(ow as usize, oh as usize);
    if width < 16 || height < 16 {
        eprintln!("image too small after crop: {width}×{height} (need ≥ 16×16)");
        return ExitCode::FAILURE;
    }
    if width != ow as usize || height != oh as usize {
        eprintln!("cropping {ow}×{oh} → {width}×{height} (top-left, multiples of 16)");
    }

    let cropped = if width == ow as usize && height == oh as usize {
        rgb
    } else {
        image::imageops::crop_imm(&rgb, 0, 0, width as u32, height as u32).to_image()
    };

    let yuv = match rgb8_to_yuv420p(cropped.as_raw(), width, height) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let (orig_y, orig_u, orig_v) = match yuv420_to_macroblocks(&yuv, width, height, 1) {
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

    let mbs = (width / 16) * (height / 16);
    println!("Wrote Eva macroblocks → {}", output_dir.display());
    println!("  frames=1  macroblocks={mbs}  resolution={width}×{height}");
    println!("  orig_y_enc  {} bytes", orig_y.len());
    println!("  orig_u_enc  {} bytes", orig_u.len());
    println!("  orig_v_enc  {} bytes", orig_v.len());
    println!();
    println!("Next (Eva example):");
    println!("  export DATA_PATH=/path/to/data_parsed");
    println!(
        "  VIDEO={} QUICK=1 cargo run --release -p video --example edit_bright_only",
        output_dir.file_name().unwrap().to_string_lossy()
    );
    println!();
    println!("Or toolkit (Level-0 or eva-backend):");
    println!(
        "  fightfake prove-edit --input {} --gadget brightness --out-dir out/",
        input.display()
    );

    ExitCode::SUCCESS
}
