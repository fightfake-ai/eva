//! Print geometric island sizes (gadget ∪ 1-MB halo) for a redact box.
//!
//! This is the *encode-skip* prove set `I` without inter-prediction. P-frame
//! leak is measured by `neighbor_reencode`, not by this geometry.
//!
//! ```bash
//! cargo run --release -p video --example island_report -- \
//!   352 288 30  96 80 176 144  2 5  1
//! ```

use std::env;
use std::process::ExitCode;

use video::{island_counts, IslandSpec, PixelRect};

fn usage() -> &'static str {
    "Usage: island_report <width> <height> <num_frames> \\\n\
     \t<x> <y> <w> <h> <frame_start> <frame_end> [halo_mbs]\n\
     \n\
     Pixel rectangle (x,y,w,h) on frames [frame_start, frame_end).\n\
     halo_mbs defaults to 1 (Chebyshev ring). 0 = gadget only."
}

fn parse_usize(s: &str, name: &str) -> Result<usize, String> {
    s.parse().map_err(|_| format!("invalid {name}: {s}"))
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 9 {
        eprintln!("{}", usage());
        return ExitCode::FAILURE;
    }
    if args.len() > 10 {
        eprintln!("too many arguments");
        eprintln!("{}", usage());
        return ExitCode::FAILURE;
    }

    let spec = (|| {
        Ok::<_, String>(IslandSpec {
            width: parse_usize(&args[0], "width")?,
            height: parse_usize(&args[1], "height")?,
            num_frames: parse_usize(&args[2], "num_frames")?,
            rect: PixelRect {
                x: parse_usize(&args[3], "x")?,
                y: parse_usize(&args[4], "y")?,
                w: parse_usize(&args[5], "w")?,
                h: parse_usize(&args[6], "h")?,
            },
            frame_start: parse_usize(&args[7], "frame_start")?,
            frame_end: parse_usize(&args[8], "frame_end")?,
            halo_mbs: if args.len() == 10 {
                parse_usize(&args[9], "halo_mbs")?
            } else {
                1
            },
        })
    })();

    let spec = match spec {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let counts = match island_counts(&spec) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    println!(
        "clip {}×{} × {} frames  ({} MBs/frame, {} total)",
        spec.width,
        spec.height,
        spec.num_frames,
        counts.total / spec.num_frames,
        counts.total
    );
    println!(
        "redact ({},{}) {}×{}  frames [{}, {})  halo {}",
        spec.rect.x,
        spec.rect.y,
        spec.rect.w,
        spec.rect.h,
        spec.frame_start,
        spec.frame_end,
        spec.halo_mbs
    );
    println!();
    println!("role                      MBs");
    println!("gadget (box)           {:8}", counts.gadget);
    println!("halo (neighbors)       {:8}", counts.halo);
    println!(
        "I = gadget ∪ halo      {:8}  ({:.2}% of clip)",
        counts.island(),
        100.0 * counts.island_fraction()
    );
    println!("skip on edited frames  {:8}", counts.skip_in_edited_frames);
    println!("skip pre-window        {:8}", counts.skip_pre);
    println!(
        "skip post-window       {:8}  (P-leak candidates)",
        counts.skip_post
    );
    println!("J = all skip           {:8}", counts.skip());

    ExitCode::SUCCESS
}
