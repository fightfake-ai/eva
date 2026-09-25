//! Compare geometric vs P-MV-closure syntax merge on post-edit frames.
//!
//! Frame 16 (first post-window P frame). Metrics on **closure ∩ frame** MBs show whether
//! P-MV merge picks up edited syntax where geometric merge keeps orig.
//!
//! ```bash
//! cargo run --release -p video --example splice_pmv_spike -- \
//!   --frame 16 --out docs/results/splice-pmv-720p.csv
//! ```

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use video::{
    compare_decoded_mb_set, compare_decoded_planes, decode_frame_luma, finalize_role_means,
    hd_center_box, load_mv_dir, load_syntax_dir, merge_syntax, merge_syntax_pmv, pmv_closure,
    redact_window, PmvClosureCounts, SpliceDecodeMode, SpliceDecodeReport, HD_FRAMES, HD_HEIGHT,
    HD_WIDTH, IslandSpec,
};

fn main() -> ExitCode {
    let mut frame = 16usize;
    let mut out = PathBuf::from("docs/results/splice-pmv-720p.csv");
    let mut orig_dir = PathBuf::from("docs/fixtures/jm-dumps-720p/orig");
    let mut edit_dir = PathBuf::from("docs/fixtures/jm-dumps-720p/edited");

    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--frame" => {
                i += 1;
                frame = args[i].parse().unwrap_or(16);
            }
            "--out" => {
                i += 1;
                out = PathBuf::from(&args[i]);
            }
            "--orig-dumps" => {
                i += 1;
                orig_dir = PathBuf::from(&args[i]);
            }
            "--edit-dumps" => {
                i += 1;
                edit_dir = PathBuf::from(&args[i]);
            }
            _ => {}
        }
        i += 1;
    }

    let (fs, fe) = redact_window(HD_FRAMES);
    let orig_syn = match load_syntax_dir(&orig_dir) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("load orig: {e}");
            return ExitCode::FAILURE;
        }
    };
    let edit_syn = match load_syntax_dir(&edit_dir) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("load edited: {e}");
            return ExitCode::FAILURE;
        }
    };
    let n = orig_syn.pred_y.len() / 256;
    let mv = match load_mv_dir(&orig_dir, n) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let mut rows = vec![
        "merge_mode,halo_mbs,compare_to,skip_post_max,closure_frame_max,closure_frame_mbs,closure_total,added_post".to_string(),
    ];

    let plane = HD_WIDTH * HD_HEIGHT;
    let mode = SpliceDecodeMode::Naive;

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

        let (closure, counts) = match pmv_closure(&spec, &mv, &orig_syn.type_enc) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("closure halo={halo}: {e}");
                return ExitCode::FAILURE;
            }
        };
        let mbs = spec.mbs_per_frame().unwrap();
        let base = frame * mbs;
        let on_frame: HashSet<usize> = closure
            .iter()
            .copied()
            .filter(|&g| g >= base && g < base + mbs)
            .collect();

        let merged_geo = merge_syntax(&orig_syn, &edit_syn, &spec).unwrap();
        let merged_pmv = merge_syntax_pmv(&orig_syn, &edit_syn, &spec, &mv)
            .unwrap()
            .0;

        let mut dec_orig = vec![0u8; plane];
        let mut dec_edit = vec![0u8; plane];
        let mut dec_geo = vec![0u8; plane];
        let mut dec_pmv = vec![0u8; plane];

        if decode_frame_luma(&orig_syn, &spec, frame, mode, &mut dec_orig).is_err()
            || decode_frame_luma(&edit_syn, &spec, frame, mode, &mut dec_edit).is_err()
            || decode_frame_luma(&merged_geo, &spec, frame, mode, &mut dec_geo).is_err()
            || decode_frame_luma(&merged_pmv, &spec, frame, mode, &mut dec_pmv).is_err()
        {
            eprintln!("decode failed halo={halo}");
            continue;
        }

        for (merge_mode, decoded) in [("geometric", &dec_geo), ("pmv_closure", &dec_pmv)] {
            for compare_to in ["full_orig", "full_edited"] {
                let reference = if compare_to == "full_orig" {
                    &dec_orig
                } else {
                    &dec_edit
                };
                let mut rep = compare_decoded_planes(
                    decoded,
                    reference,
                    &spec,
                    frame,
                    halo,
                    "naive_pred_dump",
                )
                .unwrap();
                finalize_role_means(&mut rep);
                let (cf_max, _cf_mean, cf_mbs) =
                    compare_decoded_mb_set(decoded, reference, &spec, frame, &on_frame).unwrap();
                log_line(merge_mode, halo, compare_to, &rep, cf_max, cf_mbs, &counts);
                rows.push(row(
                    merge_mode,
                    halo,
                    compare_to,
                    &rep,
                    cf_max,
                    cf_mbs,
                    &counts,
                ));
            }
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

fn log_line(
    merge_mode: &str,
    halo: usize,
    compare_to: &str,
    rep: &SpliceDecodeReport,
    closure_frame_max: u8,
    closure_frame_mbs: usize,
    counts: &PmvClosureCounts,
) {
    eprintln!(
        "{merge_mode} vs {compare_to} halo={halo} skip_post_max={} closure_frame_max={} closure_frame_mbs={} closure_total={}",
        rep.skip_post.max_abs_y,
        closure_frame_max,
        closure_frame_mbs,
        counts.closure,
    );
}

fn row(
    merge_mode: &str,
    halo: usize,
    compare_to: &str,
    rep: &SpliceDecodeReport,
    closure_frame_max: u8,
    closure_frame_mbs: usize,
    counts: &PmvClosureCounts,
) -> String {
    format!(
        "{},{},{},{},{},{},{},{}",
        merge_mode,
        halo,
        compare_to,
        rep.skip_post.max_abs_y,
        closure_frame_max,
        closure_frame_mbs,
        counts.closure,
        counts.added_post,
    )
}
