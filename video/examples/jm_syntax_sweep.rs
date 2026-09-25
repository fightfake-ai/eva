//! JM syntax smear sweep: compare orig vs edited YUV under several encoder knobs.
//!
//! ```bash
//! cargo run --release -p video --example jm_syntax_sweep -- \
//!   --out docs/results/jm-syntax-sweep-720p.csv
//! ```

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use video::{
    build_mb_grid_report, hd_center_box, hd_center_box_mb_aligned, island_counts, load_yuv_or_fixture,
    redact_window, IslandSpec, MbGridReport, MbRole, PixelRect, HD_FRAMES, HD_HEIGHT, HD_WIDTH,
};

struct Preset {
    id: &'static str,
    label: &'static str,
    args: &'static [&'static str],
}

struct BoxVariant {
    id: &'static str,
    rect: fn() -> PixelRect,
}

const PRESETS: &[Preset] = &[
    Preset {
        id: "ipp_baseline",
        label: "JM IPP baseline (I@0, P rest)",
        args: &[],
    },
    Preset {
        id: "all_intra",
        label: "JM all-intra (IntraPeriod=1)",
        args: &["--all-intra"],
    },
    Preset {
        id: "gop8",
        label: "JM GOP=8 (IntraPeriod=8)",
        args: &["--gop", "8"],
    },
    Preset {
        id: "constrained_intra",
        label: "JM IPP + constrained intra pred",
        args: &["--constrained"],
    },
    Preset {
        id: "strict_cqp",
        label: "JM IPP + strict CQP (no QP search)",
        args: &["--strict-cqp", "--qp", "28"],
    },
    Preset {
        id: "independent_frames",
        label: "JM per-frame independent encode",
        args: &["--independent-frames", "--strict-cqp", "--qp", "28"],
    },
];

const BOX_VARIANTS: &[BoxVariant] = &[
    BoxVariant {
        id: "center",
        rect: hd_center_box,
    },
    BoxVariant {
        id: "mb_aligned",
        rect: hd_center_box_mb_aligned,
    },
];

#[derive(Clone, Copy, Debug)]
struct SmearCounts {
    syntax_no_pixel: usize,
    syntax_outside_i: usize,
    syntax_halo: usize,
    syntax_edited_frames: usize,
    decode_no_pixel: usize,
    decode_outside_i: usize,
}

fn main() -> ExitCode {
    let mut out = PathBuf::from("docs/results/jm-syntax-sweep-720p.csv");
    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--out" {
            i += 1;
            out = PathBuf::from(&args[i]);
        }
        i += 1;
    }

    let orig = match load_yuv_or_fixture(None, HD_WIDTH, HD_HEIGHT, HD_FRAMES) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts/jm_syntax_dumps.sh");
    if !script.is_file() {
        eprintln!("missing {}", script.display());
        return ExitCode::FAILURE;
    }

    let dump_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/fixtures/jm-sweep-720p");
    let work = env::temp_dir().join(format!(
        "eva-sweep-yuv-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    if fs::create_dir_all(&work).is_err() {
        eprintln!("mkdir temp failed");
        return ExitCode::FAILURE;
    }
    let orig_yuv = work.join("orig.yuv");
    if fs::write(&orig_yuv, &orig).is_err() {
        eprintln!("write temp yuv failed");
        return ExitCode::FAILURE;
    }

    let (fs, fe) = redact_window(HD_FRAMES);

    let mut rows = Vec::new();
    rows.push(format!(
        "preset_id,preset_label,box_id,syntax_no_pixel,syntax_outside_i,syntax_halo,syntax_edited_frames_only,decode_gop1_no_pixel,decode_gop1_outside_i,decode_gop8_no_pixel,decode_gop8_outside_i,gadget_mb,island_mb"
    ));

    for preset in PRESETS {
        let preset_dir = dump_root.join(preset.id);
        let orig_dir = preset_dir.join("orig");
        let mut orig_encoded = false;

        for box_var in BOX_VARIANTS {
            let rect = (box_var.rect)();
            let spec = IslandSpec {
                width: HD_WIDTH,
                height: HD_HEIGHT,
                num_frames: HD_FRAMES,
                frame_start: fs,
                frame_end: fe,
                rect,
                halo_mbs: 1,
            };
            let counts = island_counts(&spec).expect("island counts");
            let edited = match video::redact_yuv(&orig, &spec) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("redact failed ({} {}): {e}", preset.id, box_var.id);
                    continue;
                }
            };
            let edit_yuv = work.join(format!("edit_{}.yuv", box_var.id));
            if fs::write(&edit_yuv, &edited).is_err() {
                eprintln!("write edit yuv failed");
                continue;
            }

            let decode_gop1 = decode_counts(&orig, &edited, &spec, 1, 23);
            let decode_gop8 = decode_counts(&orig, &edited, &spec, 8, 23);

            eprintln!(
                "=== {} · box={} (gadget={} island={}) ===",
                preset.label, box_var.id, counts.gadget, counts.island()
            );

            if !orig_encoded {
                if let Err(e) = run_jm_preset(&script, &orig_yuv, &orig_dir, &spec, preset.args) {
                    eprintln!("orig encode failed ({}): {e}", preset.id);
                    continue;
                }
                orig_encoded = true;
            }

            let edit_dir = preset_dir.join(format!("edited_{}", box_var.id));
            if let Err(e) = run_jm_preset(&script, &edit_yuv, &edit_dir, &spec, preset.args) {
                eprintln!("edit encode failed ({} {}): {e}", preset.id, box_var.id);
                continue;
            }

            let report = match build_mb_grid_report(
                &orig,
                &edited,
                &spec,
                None,
                None,
                0,
                Some(orig_dir.as_path()),
                Some(edit_dir.as_path()),
            ) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("report failed ({} {}): {e}", preset.id, box_var.id);
                    continue;
                }
            };
            let c = smear_counts(&report, &spec);
            eprintln!(
                "  syntax_no_pixel={} outside_i={} edited_only={}",
                c.syntax_no_pixel, c.syntax_outside_i, c.syntax_edited_frames
            );
            rows.push(format!(
                "{},{},{},{},{},{},{},{},{},{},{},{},{}",
                preset.id,
                csv_escape(preset.label),
                box_var.id,
                c.syntax_no_pixel,
                c.syntax_outside_i,
                c.syntax_halo,
                c.syntax_edited_frames,
                decode_gop1.syntax_no_pixel,
                decode_gop1.syntax_outside_i,
                decode_gop8.syntax_no_pixel,
                decode_gop8.syntax_outside_i,
                counts.gadget,
                counts.island(),
            ));
        }
    }

    let _ = fs::remove_dir_all(&work);
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

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn run_jm_preset(
    script: &Path,
    yuv: &Path,
    out_dir: &Path,
    spec: &IslandSpec,
    extra: &[&str],
) -> Result<(), String> {
    fs::create_dir_all(out_dir).map_err(|e| format!("mkdir: {e}"))?;
    let mut cmd = Command::new(script);
    cmd.args([
        "--yuv",
        yuv.to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
        "--width",
        &spec.width.to_string(),
        "--height",
        &spec.height.to_string(),
        "--frames",
        &spec.num_frames.to_string(),
        "--bframes",
        "0",
    ]);
    for arg in extra {
        cmd.arg(arg);
    }
    let out = cmd.output().map_err(|e| format!("spawn: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        ));
    }
    Ok(())
}

fn smear_counts(report: &MbGridReport, spec: &IslandSpec) -> SmearCounts {
    let mut c = SmearCounts {
        syntax_no_pixel: 0,
        syntax_outside_i: 0,
        syntax_halo: 0,
        syntax_edited_frames: 0,
        decode_no_pixel: 0,
        decode_outside_i: 0,
    };
    let mbs = spec.mbs_per_frame().unwrap_or(0);
    for fg in &report.frames {
        for mb in 0..mbs {
            let g = fg.frame * mbs + mb;
            let role = video::mb_role(spec, g).unwrap_or(MbRole::SkipPre);
            let in_edited = fg.frame >= spec.frame_start && fg.frame < spec.frame_end;
            if let Some(ref syn) = fg.syntax_changed {
                if syn[mb] && !fg.pixel_edited[mb] {
                    c.syntax_no_pixel += 1;
                    if in_edited {
                        c.syntax_edited_frames += 1;
                    }
                    if matches!(role, MbRole::Halo) {
                        c.syntax_halo += 1;
                    }
                    if !matches!(role, MbRole::Gadget | MbRole::Halo) {
                        c.syntax_outside_i += 1;
                    }
                }
            }
            if fg.decode_changed[mb] && !fg.pixel_edited[mb] {
                c.decode_no_pixel += 1;
                if !matches!(role, MbRole::Gadget | MbRole::Halo) {
                    c.decode_outside_i += 1;
                }
            }
        }
    }
    c
}

fn decode_counts(
    orig: &[u8],
    edited: &[u8],
    spec: &IslandSpec,
    gop: usize,
    qp: u8,
) -> SmearCounts {
    let (a, b) = match decoded_pair(orig, edited, spec, gop, qp) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("decode gop={gop} skipped: {e}");
            return SmearCounts {
                syntax_no_pixel: 0,
                syntax_outside_i: 0,
                syntax_halo: 0,
                syntax_edited_frames: 0,
                decode_no_pixel: 0,
                decode_outside_i: 0,
            };
        }
    };
    let report = build_mb_grid_report(orig, edited, spec, Some(&a), Some(&b), 0, None, None)
        .expect("decode report");
    let mut c = smear_counts(&report, &spec);
    c.syntax_no_pixel = c.decode_no_pixel;
    c.syntax_outside_i = c.decode_outside_i;
    c.syntax_halo = 0;
    c.syntax_edited_frames = 0;
    c
}

fn decoded_pair(
    orig: &[u8],
    edited: &[u8],
    spec: &IslandSpec,
    gop: usize,
    qp: u8,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    use std::time::{SystemTime, UNIX_EPOCH};
    use video::yuv420_frame_bytes;

    let tag = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = env::temp_dir().join(format!("eva-sweep-dec-{tag}"));
    fs::create_dir_all(&dir).map_err(|e| format!("mkdir: {e}"))?;
    let orig_path = dir.join("orig.yuv");
    let edit_path = dir.join("edit.yuv");
    let orig_dec = dir.join("orig-dec.yuv");
    let edit_dec = dir.join("edit-dec.yuv");
    let need = yuv420_frame_bytes(spec.width, spec.height)? * spec.num_frames;
    fs::write(&orig_path, orig).map_err(|e| format!("write: {e}"))?;
    fs::write(&edit_path, edited).map_err(|e| format!("write: {e}"))?;
    encode_decode_ffmpeg(&orig_path, &orig_dec, spec, gop, qp)?;
    encode_decode_ffmpeg(&edit_path, &edit_dec, spec, gop, qp)?;
    let a = fs::read(&orig_dec).map_err(|e| format!("read: {e}"))?;
    let b = fs::read(&edit_dec).map_err(|e| format!("read: {e}"))?;
    let _ = fs::remove_dir_all(&dir);
    if a.len() < need || b.len() < need {
        return Err("decoded yuv too short".into());
    }
    Ok((a[..need].to_vec(), b[..need].to_vec()))
}

fn encode_decode_ffmpeg(
    yuv: &Path,
    out: &Path,
    spec: &IslandSpec,
    gop: usize,
    qp: u8,
) -> Result<(), String> {
    let gop_s = gop.to_string();
    let qp_s = qp.to_string();
    let frames_s = spec.num_frames.to_string();
    let size = format!("{}x{}", spec.width, spec.height);
    let mp4 = out.with_extension("mp4");
    let enc = Command::new("ffmpeg")
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv420p",
            "-s",
            &size,
            "-i",
            yuv.to_str().unwrap(),
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-bf",
            "0",
            "-g",
            &gop_s,
            "-qp",
            &qp_s,
            "-frames:v",
            &frames_s,
            mp4.to_str().unwrap(),
        ])
        .output()
        .map_err(|e| format!("ffmpeg enc: {e}"))?;
    if !enc.status.success() {
        return Err(String::from_utf8_lossy(&enc.stderr).into());
    }
    let dec = Command::new("ffmpeg")
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            mp4.to_str().unwrap(),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv420p",
            "-frames:v",
            &frames_s,
            out.to_str().unwrap(),
        ])
        .output()
        .map_err(|e| format!("ffmpeg dec: {e}"))?;
    if !dec.status.success() {
        return Err(String::from_utf8_lossy(&dec.stderr).into());
    }
    Ok(())
}
