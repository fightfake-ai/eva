//! P-MV closure vs edit position (early / mid / late) on the same 720p orig capture.
//!
//! ```bash
//! cargo run --release -p video --example pmv_closure_position -- \
//!   --out docs/results/pmv-closure-position-720p.csv
//! ```

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use video::{
    edit_window_presets, hd_center_box, island_counts, load_mv_dir, load_syntax_dir, pmv_closure,
    HD_FRAMES, HD_HEIGHT, HD_WIDTH, IslandSpec,
};

fn main() -> ExitCode {
    let mut out = PathBuf::from("docs/results/pmv-closure-position-720p.csv");
    let mut dump_dir = PathBuf::from("docs/fixtures/jm-dumps-720p/orig");

    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                out = PathBuf::from(&args[i]);
            }
            "--dumps" => {
                i += 1;
                dump_dir = PathBuf::from(&args[i]);
            }
            _ => {}
        }
        i += 1;
    }

    let syn = match load_syntax_dir(&dump_dir) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("load dumps: {e}");
            return ExitCode::FAILURE;
        }
    };
    let total = syn.pred_y.len() / 256;
    let mv = match load_mv_dir(&dump_dir, total) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let mut rows = vec![
        "position,frame_start,frame_end,post_frames,halo_mbs,geometric,closure,added_post,added_pre,closure_pct".to_string(),
    ];

    for preset in edit_window_presets(HD_FRAMES) {
        let post_frames = HD_FRAMES.saturating_sub(preset.frame_end);
        for halo in [0usize, 1, 2] {
            let spec = IslandSpec {
                width: HD_WIDTH,
                height: HD_HEIGHT,
                num_frames: HD_FRAMES,
                frame_start: preset.frame_start,
                frame_end: preset.frame_end,
                rect: hd_center_box(),
                halo_mbs: halo,
            };
            let geo = match island_counts(&spec) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::FAILURE;
                }
            };
            let counts = match pmv_closure(&spec, &mv, &syn.type_enc) {
                Ok((_, c)) => c,
                Err(e) => {
                    eprintln!("{} halo={halo}: {e}", preset.label);
                    return ExitCode::FAILURE;
                }
            };
            let pct = 100.0 * counts.closure_fraction(geo.total);
            eprintln!(
                "{} [{},{}) halo={halo} post={post_frames} closure={} ({pct:.1}%) added_post={}",
                preset.label,
                preset.frame_start,
                preset.frame_end,
                counts.closure,
                counts.added_post,
            );
            rows.push(format!(
                "{},{},{},{},{},{},{},{},{},{:.3}",
                preset.label,
                preset.frame_start,
                preset.frame_end,
                post_frames,
                halo,
                counts.geometric,
                counts.closure,
                counts.added_post,
                counts.added_pre,
                pct
            ));
        }
    }

    if let Some(parent) = out.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if fs::write(&out, rows.join("\n") + "\n").is_err() {
        eprintln!("write {} failed", out.display());
        return ExitCode::FAILURE;
    }
    eprintln!("wrote {}", out.display());
    ExitCode::SUCCESS
}
