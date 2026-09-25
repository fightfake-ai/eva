//! Neighbor-impact experiment: geometric island vs pixel redact vs ffmpeg re-encode.
//!
//! Answers “how many neighbor blocks are affected?” in three layers:
//!
//! 1. **Geometry** — box ∪ 1-MB Chebyshev ring (`I`). Deterministic.
//! 2. **Pixel redact** — orig YUV vs redacted YUV. Only gadget MBs should differ.
//! 3. **Full re-encode** (if `ffmpeg` is on PATH) — encode orig and edited
//!    independently (intra-only and with a GOP), decode, count MBs whose luma
//!    changed *outside* `I`. That is an upper bound: splice would copy skip
//!    syntax, so it would not re-quantize those MBs. Syntax-accurate counts
//!    need paired JM dumps (`syntax_mb_changed`).
//!
//! ```bash
//! cargo run --release -p video --example neighbor_reencode -- \
//!   352 288 8  96 80 176 144  2 4
//!
//! cargo run --release -p video --example neighbor_reencode -- \
//!   352 288 8  96 80 176 144  2 4 --yuv clip.yuv --gop 8
//! ```

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{SystemTime, UNIX_EPOCH};

use video::{
    compare_yuv_to_island, island_counts, macroblocks_to_yuv420, native_redact_edit_macroblocks,
    yuv420_frame_bytes, yuv420_to_macroblocks, ImpactReport, IslandSpec, PixelRect,
};

fn usage() -> &'static str {
    "Usage: neighbor_reencode <width> <height> <num_frames> \\\n\
     \t<x> <y> <w> <h> <frame_start> <frame_end> [--yuv FILE] [--gop N] [--halo N] [--qp N]"
}

struct Args {
    spec: IslandSpec,
    yuv: Option<PathBuf>,
    gop: usize,
    qp: u8,
}

fn parse_usize(s: &str, name: &str) -> Result<usize, String> {
    s.parse().map_err(|_| format!("invalid {name}: {s}"))
}

fn parse_args() -> Result<Args, String> {
    let raw: Vec<String> = env::args().skip(1).collect();
    if raw.len() < 9 {
        return Err(usage().into());
    }
    let mut spec = IslandSpec {
        width: parse_usize(&raw[0], "width")?,
        height: parse_usize(&raw[1], "height")?,
        num_frames: parse_usize(&raw[2], "num_frames")?,
        rect: PixelRect {
            x: parse_usize(&raw[3], "x")?,
            y: parse_usize(&raw[4], "y")?,
            w: parse_usize(&raw[5], "w")?,
            h: parse_usize(&raw[6], "h")?,
        },
        frame_start: parse_usize(&raw[7], "frame_start")?,
        frame_end: parse_usize(&raw[8], "frame_end")?,
        halo_mbs: 1,
    };
    let mut yuv = None;
    let mut gop = 8usize;
    let mut qp = 23u8;
    let mut i = 9; // flags after the 9 positional fields
    while i < raw.len() {
        match raw[i].as_str() {
            "--yuv" => {
                i += 1;
                yuv = Some(PathBuf::from(raw.get(i).ok_or("--yuv needs a path")?));
            }
            "--gop" => {
                i += 1;
                gop = parse_usize(raw.get(i).ok_or("--gop needs a value")?, "gop")?;
                if gop == 0 {
                    return Err("gop must be >= 1".into());
                }
            }
            "--halo" => {
                i += 1;
                spec.halo_mbs = parse_usize(raw.get(i).ok_or("--halo needs a value")?, "halo")?;
            }
            "--qp" => {
                i += 1;
                let v = parse_usize(raw.get(i).ok_or("--qp needs a value")?, "qp")?;
                if v > 51 {
                    return Err("qp must be 0..=51".into());
                }
                qp = v as u8;
            }
            other => return Err(format!("unknown argument: {other}")),
        }
        i += 1;
    }
    spec.validate()?;
    Ok(Args { spec, yuv, gop, qp })
}

fn gradient_yuv(width: usize, height: usize, frames: usize) -> Vec<u8> {
    let fb = width * height * 3 / 2;
    let mut yuv = vec![128u8; fb * frames];
    for f in 0..frames {
        let base = f * fb;
        for y in 0..height {
            for x in 0..width {
                yuv[base + y * width + x] = ((x + y + f * 17) % 220 + 16) as u8;
            }
        }
    }
    yuv
}

fn load_or_synth(args: &Args) -> Result<Vec<u8>, String> {
    let need = yuv420_frame_bytes(args.spec.width, args.spec.height)? * args.spec.num_frames;
    match &args.yuv {
        Some(path) => {
            let buf = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
            if buf.len() < need {
                return Err(format!(
                    "{} is too short: need {need} bytes for {} frames, got {}",
                    path.display(),
                    args.spec.num_frames,
                    buf.len()
                ));
            }
            Ok(buf[..need].to_vec())
        }
        None => Ok(gradient_yuv(
            args.spec.width,
            args.spec.height,
            args.spec.num_frames,
        )),
    }
}

fn redact_yuv(orig: &[u8], spec: &IslandSpec) -> Result<Vec<u8>, String> {
    let (y, u, v) = yuv420_to_macroblocks(orig, spec.width, spec.height, spec.num_frames)?;
    let (ey, eu, ev) = native_redact_edit_macroblocks(
        &y,
        &u,
        &v,
        spec.width,
        spec.height,
        spec.num_frames,
        spec.rect.x,
        spec.rect.y,
        spec.rect.w,
        spec.rect.h,
        spec.frame_start,
        spec.frame_end,
        0,
    )?;
    macroblocks_to_yuv420(&ey, &eu, &ev, spec.width, spec.height, spec.num_frames)
}

fn print_role_table(title: &str, r: &ImpactReport) {
    println!("{title}  (luma max-abs > {})", r.threshold);
    println!(
        "  {:<22} {:>8} {:>8} {:>8}",
        "role", "MBs", "changed", "max|ΔY|"
    );
    for (name, s) in [
        ("gadget", &r.gadget),
        ("halo", &r.halo),
        ("skip edited frames", &r.skip_in_edited_frames),
        ("skip pre", &r.skip_pre),
        ("skip post (P cand.)", &r.skip_post),
    ] {
        println!(
            "  {name:<22} {:>8} {:>8} {:>8}",
            s.n_mbs, s.n_changed, s.max_abs_y
        );
    }
    println!(
        "  outside I changed: {}   (0 means the 1-MB ring covered this measurement)",
        r.outside_island_changed()
    );
}

fn ffmpeg_ok() -> bool {
    Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn run_ffmpeg(args: &[&str]) -> Result<(), String> {
    let out = Command::new("ffmpeg")
        .args(args)
        .output()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "ffmpeg failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

fn encode_decode(
    yuv: &Path,
    decoded: &Path,
    width: usize,
    height: usize,
    gop: usize,
    qp: u8,
) -> Result<(), String> {
    let mp4 = decoded.with_extension("mp4");
    let size = format!("{width}x{height}");
    let qp_s = qp.to_string();
    let gop_s = gop.to_string();
    run_ffmpeg(&[
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "yuv420p",
        "-s:v",
        &size,
        "-r",
        "30",
        "-i",
        yuv.to_str().unwrap(),
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-tune",
        "zerolatency",
        "-qp",
        &qp_s,
        "-g",
        &gop_s,
        "-bf",
        "0",
        "-pix_fmt",
        "yuv420p",
        mp4.to_str().unwrap(),
    ])?;
    run_ffmpeg(&[
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-i",
        mp4.to_str().unwrap(),
        "-pix_fmt",
        "yuv420p",
        decoded.to_str().unwrap(),
    ])
}

fn tmp_dir() -> Result<PathBuf, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = env::temp_dir().join(format!("eva-neighbor-{nanos}"));
    fs::create_dir_all(&dir).map_err(|e| format!("temp dir: {e}"))?;
    Ok(dir)
}

fn reencode_pass(
    label: &str,
    orig: &[u8],
    edited: &[u8],
    spec: &IslandSpec,
    gop: usize,
    qp: u8,
    dir: &Path,
) -> Result<(), String> {
    let orig_path = dir.join(format!("{label}-orig.yuv"));
    let edit_path = dir.join(format!("{label}-edit.yuv"));
    let orig_dec = dir.join(format!("{label}-orig-dec.yuv"));
    let edit_dec = dir.join(format!("{label}-edit-dec.yuv"));
    fs::write(&orig_path, orig).map_err(|e| e.to_string())?;
    fs::write(&edit_path, edited).map_err(|e| e.to_string())?;
    encode_decode(&orig_path, &orig_dec, spec.width, spec.height, gop, qp)?;
    encode_decode(&edit_path, &edit_dec, spec.width, spec.height, gop, qp)?;
    let a = fs::read(&orig_dec).map_err(|e| e.to_string())?;
    let b = fs::read(&edit_dec).map_err(|e| e.to_string())?;
    let r0 = compare_yuv_to_island(&a, &b, spec, 0)?;
    let r2 = compare_yuv_to_island(&a, &b, spec, 2)?;
    println!();
    println!("=== ffmpeg re-encode  GOP={gop}  QP={qp}  ({label}) ===");
    println!(
        "two independent libx264 encodes; skip MBs are re-quantized, so this overestimates splice I"
    );
    print_role_table("threshold 0 (any luma byte)", &r0);
    println!();
    print_role_table("threshold 2 (ignore tiny quant noise)", &r2);
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let spec = args.spec;
    let orig = match load_or_synth(&args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let edited = match redact_yuv(&orig, &spec) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("redact: {e}");
            return ExitCode::FAILURE;
        }
    };

    let counts = island_counts(&spec).unwrap();
    println!(
        "clip {}×{} × {}  redact ({},{}) {}×{}  frames [{}, {})  halo {}",
        spec.width,
        spec.height,
        spec.num_frames,
        spec.rect.x,
        spec.rect.y,
        spec.rect.w,
        spec.rect.h,
        spec.frame_start,
        spec.frame_end,
        spec.halo_mbs
    );
    println!(
        "geometric I: {} gadget + {} halo = {} / {} ({:.2}%)",
        counts.gadget,
        counts.halo,
        counts.island(),
        counts.total,
        100.0 * counts.island_fraction()
    );
    println!(
        "skip post-window: {} MBs (upper bound on P-prediction leak if every later MB is dirty)",
        counts.skip_post
    );

    let pix = compare_yuv_to_island(&orig, &edited, &spec, 0).unwrap();
    println!();
    println!("=== pixel redact (no encode) ===");
    print_role_table("orig vs redacted YUV", &pix);
    if pix.outside_island_changed() != 0 || pix.halo.n_changed != 0 {
        eprintln!("warning: pixel redact leaked outside the gadget — check the rectangle");
    }

    if !ffmpeg_ok() {
        println!();
        println!("ffmpeg not found; skipping re-encode contamination. Install ffmpeg to measure GOP leak.");
        return ExitCode::SUCCESS;
    }

    let dir = match tmp_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = reencode_pass("intra", &orig, &edited, &spec, 1, args.qp, &dir) {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    }
    if args.gop != 1 {
        if let Err(e) = reencode_pass("gop", &orig, &edited, &spec, args.gop, args.qp, &dir) {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    }

    let _ = fs::remove_dir_all(&dir);
    ExitCode::SUCCESS
}
