//! Splice-decode spike: merge JM syntax (copy J + island I), decode one frame, compare.
//!
//! Primary metrics compare **decoded planes to each other** (not source YUV — JM is lossy at
//! QP 30). `spliced vs full_orig` outside `I` should be 0 when copy-J merge is correct.
//!
//! ```bash
//! ./scripts/fetch_fixture_720p.sh
//! cargo run --release -p video --example splice_decode_spike -- \
//!   --frame 14 --out docs/results/splice-decode-720p.csv
//! ```

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use video::{
    compare_decoded_frame_to_yuv, compare_decoded_planes, decode_frame_luma, expected_splice_yuv,
    finalize_role_means, hd_center_box, load_syntax_dir, load_yuv_or_fixture, merge_syntax,
    pixel_outside_island, redact_window, redact_yuv, SpliceDecodeMode, SpliceDecodeReport,
    HD_FRAMES, HD_HEIGHT, HD_WIDTH, IslandSpec,
};

fn main() -> ExitCode {
    let mut frame = 14usize;
    let mut out = PathBuf::from("docs/results/splice-decode-720p.csv");
    let mut orig_dir = PathBuf::from("docs/fixtures/jm-dumps-720p/orig");
    let mut edit_dir = PathBuf::from("docs/fixtures/jm-dumps-720p/edited");
    let mut run_jm = false;

    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--frame" => {
                i += 1;
                frame = args[i].parse().unwrap_or(14);
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
            "--run-jm" => run_jm = true,
            _ => {}
        }
        i += 1;
    }

    let (fs, fe) = redact_window(HD_FRAMES);
    let mut rows: Vec<String> = vec![
        "scenario,compare_to,halo_mbs,decode_mode,gadget_max,halo_max,skip_in_max,outside_i_max,island_max,full_max,full_mean".to_string(),
    ];

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

        let orig_yuv = match load_yuv_or_fixture(None, spec.width, spec.height, spec.num_frames) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        let edited = match redact_yuv(&orig_yuv, &spec) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };

        if pixel_outside_island(&orig_yuv, &edited, &spec).unwrap_or(1) != 0 {
            eprintln!("warning: pixel edit outside I with halo={halo}");
        }

        if run_jm || !orig_dir.is_dir() || !edit_dir.is_dir() {
            if let Err(e) = run_jm_dumps(&orig_yuv, &edited, &spec, &orig_dir, &edit_dir) {
                eprintln!("JM dumps failed: {e}");
                return ExitCode::FAILURE;
            }
        }

        let orig_syn = match load_syntax_dir(&orig_dir) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("load orig dumps: {e}");
                eprintln!("hint: run mb_walkthrough_export first, or pass --run-jm");
                return ExitCode::FAILURE;
            }
        };
        let edit_syn = match load_syntax_dir(&edit_dir) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("load edit dumps: {e}");
                return ExitCode::FAILURE;
            }
        };

        let merged = match merge_syntax(&orig_syn, &edit_syn, &spec) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("merge failed: {e}");
                return ExitCode::FAILURE;
            }
        };

        let expected = match expected_splice_yuv(&orig_yuv, &edited, &spec) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("expected yuv: {e}");
                return ExitCode::FAILURE;
            }
        };

        let plane = spec.width * spec.height;
        let frame_bytes = plane + plane / 2;
        let frame_off = frame * frame_bytes;
        let ref_plane = &expected[frame_off..frame_off + plane];

        for mode in [SpliceDecodeMode::Naive, SpliceDecodeMode::ScanIntra4x4] {
            let mut dec_orig = vec![0u8; plane];
            let mut dec_edit = vec![0u8; plane];
            let mut dec_spliced = vec![0u8; plane];

            if decode_frame_luma(&orig_syn, &spec, frame, mode, &mut dec_orig).is_err()
                || decode_frame_luma(&edit_syn, &spec, frame, mode, &mut dec_edit).is_err()
                || decode_frame_luma(&merged, &spec, frame, mode, &mut dec_spliced).is_err()
            {
                eprintln!("decode failed halo={halo} {mode:?}");
                continue;
            }

            let mode_label = match mode {
                SpliceDecodeMode::Naive => "naive_pred_dump",
                SpliceDecodeMode::ScanIntra4x4 => "scan_intra4x4",
            };

            for (scenario, decoded, compare_to, reference) in [
                ("full_orig", &dec_orig, "pixel_oracle", ref_plane),
                ("full_edited", &dec_edit, "pixel_oracle", ref_plane),
                ("spliced", &dec_spliced, "pixel_oracle", ref_plane),
                ("spliced", &dec_spliced, "full_orig", &dec_orig),
                ("spliced", &dec_spliced, "full_edited", &dec_edit),
            ] {
                let rep_result = match compare_to {
                    "pixel_oracle" => compare_decoded_frame_to_yuv(
                        decoded, reference, &spec, frame, mode, halo,
                    ),
                    _ => compare_decoded_planes(
                        decoded,
                        reference,
                        &spec,
                        frame,
                        halo,
                        mode_label,
                    ),
                };
                let mut rep = match rep_result {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("compare {scenario} vs {compare_to}: {e}");
                        continue;
                    }
                };
                finalize_role_means(&mut rep);
                log_report(scenario, compare_to, halo, &rep);
                rows.push(row(scenario, compare_to, halo, &rep));
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

fn log_report(scenario: &str, compare_to: &str, halo: usize, rep: &SpliceDecodeReport) {
    eprintln!(
        "{scenario} vs {compare_to} halo={halo} {} outside_i_max={} island_max={} full_max={} full_mean={:.2}",
        rep.decode_mode,
        rep.outside_island_max(),
        rep.island_max(),
        rep.full_frame_max,
        rep.full_frame_mean
    );
}

fn row(scenario: &str, compare_to: &str, halo: usize, rep: &SpliceDecodeReport) -> String {
    format!(
        "{},{},{},{},{},{},{},{},{},{},{:.3}",
        scenario,
        compare_to,
        halo,
        rep.decode_mode,
        rep.gadget.max_abs_y,
        rep.halo.max_abs_y,
        rep.skip_in_edited.max_abs_y,
        rep.outside_island.max_abs_y,
        rep.island.max_abs_y,
        rep.full_frame_max,
        rep.full_frame_mean
    )
}

fn run_jm_dumps(
    orig: &[u8],
    edited: &[u8],
    spec: &IslandSpec,
    orig_dir: &Path,
    edit_dir: &Path,
) -> Result<(), String> {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts/jm_syntax_dumps.sh");
    let work = env::temp_dir().join(format!(
        "eva-splice-yuv-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&work).map_err(|e| format!("mkdir: {e}"))?;
    let orig_path = work.join("orig.yuv");
    let edit_path = work.join("edit.yuv");
    fs::write(&orig_path, orig).map_err(|e| format!("write: {e}"))?;
    fs::write(&edit_path, edited).map_err(|e| format!("write: {e}"))?;

    for (yuv, dir) in [(&orig_path, orig_dir), (&edit_path, edit_dir)] {
        let out = Command::new(&script)
            .args([
                "--yuv",
                yuv.to_str().unwrap(),
                "--out",
                dir.to_str().unwrap(),
                "--width",
                &spec.width.to_string(),
                "--height",
                &spec.height.to_string(),
                "--frames",
                &spec.num_frames.to_string(),
                "--bframes",
                "0",
            ])
            .output()
            .map_err(|e| format!("spawn jm: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "{}\n{}",
                String::from_utf8_lossy(&out.stderr),
                String::from_utf8_lossy(&out.stdout)
            ));
        }
    }
    let _ = fs::remove_dir_all(&work);
    Ok(())
}
