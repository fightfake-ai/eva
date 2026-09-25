//! P-MV closure report: how much geometric `I` grows when following inter MV reach.
//!
//! Requires JM dumps with `mv_enc` (regenerate after updating `third_party/jm-eva/eva_dump.c`):
//!
//! ```bash
//! ./scripts/fetch_fixture_720p.sh
//! ./scripts/jm_syntax_dumps.sh --yuv docs/fixtures/sample_720p_30f.yuv \
//!   --out docs/fixtures/jm-dumps-720p/orig --width 1280 --height 720 --frames 30
//! cargo run --release -p video --example pmv_closure_report -- \
//!   --out docs/results/pmv-closure-720p.csv
//! ```

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use video::{
    hd_center_box, island_counts, load_mv_dir, load_syntax_dir, pmv_closure, redact_window,
    HD_FRAMES, HD_HEIGHT, HD_WIDTH, IslandSpec,
};

fn main() -> ExitCode {
    let mut out = PathBuf::from("docs/results/pmv-closure-720p.csv");
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

    let (fs, fe) = redact_window(HD_FRAMES);
    let mut rows = vec![
        "halo_mbs,geometric,closure,added_total,added_post,added_edited_outside_halo,added_pre,closure_pct".to_string(),
    ];

    let syn = match load_syntax_dir(&dump_dir) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("load dumps: {e}");
            eprintln!("hint: regenerate with scripts/jm_syntax_dumps.sh (needs mv_enc)");
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

    for halo in [0usize, 1, 2] {
        let spec = IslandSpec {
            width: HD_WIDTH,
            height: HD_HEIGHT,
            num_frames: HD_FRAMES,
            frame_start: fs,
            frame_end: fe,
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

        let (_, counts) = match pmv_closure(&spec, &mv, &syn.type_enc) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("pmv_closure halo={halo}: {e}");
                return ExitCode::FAILURE;
            }
        };

        let pct = 100.0 * counts.closure_fraction(geo.total);
        eprintln!(
            "halo={halo} geometric={} closure={} added_post={} added_edited_outside={} ({pct:.2}%)",
            counts.geometric,
            counts.closure,
            counts.added_post,
            counts.added_edited_outside_halo,
        );
        rows.push(format!(
            "{},{},{},{},{},{},{},{:.3}",
            halo,
            counts.geometric,
            counts.closure,
            counts.added_total,
            counts.added_post,
            counts.added_edited_outside_halo,
            counts.added_pre,
            pct
        ));
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
