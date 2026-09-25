//! Compare JM CABAC-glue bitstream decode to a normal JM encode (luma).
//!
//! ```bash
//! # Build I4-only single-frame dumps, then:
//! cargo run --release -p video --example glue_frame_cmp -- \
//!   --dumps docs/fixtures/glue-frame0 --h264 /tmp/glue-f0-i4.264 \
//!   --ref-h264 /tmp/norm-f0-i4.264
//! ```

use std::env;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use video::{
    decode_frame_luma, hd_center_box, load_syntax_dir, IslandSpec, SpliceDecodeMode, HD_HEIGHT,
    HD_WIDTH,
};

fn ffmpeg_luma(h264: &PathBuf, plane: usize) -> Result<Vec<u8>, String> {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(h264)
        .args(["-f", "rawvideo", "-pix_fmt", "yuv420p", "-"])
        .output()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "ffmpeg failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    if out.stdout.len() < plane {
        return Err(format!("short decode {}", out.stdout.len()));
    }
    Ok(out.stdout[..plane].to_vec())
}

fn max_mean(a: &[u8], b: &[u8]) -> (u8, f64) {
    let mut max = 0u8;
    let mut sum = 0u64;
    for (x, y) in a.iter().zip(b.iter()) {
        let d = x.abs_diff(*y);
        max = max.max(d);
        sum += d as u64;
    }
    (max, sum as f64 / a.len() as f64)
}

fn main() -> ExitCode {
    let mut dumps = PathBuf::from("docs/fixtures/glue-frame0");
    let mut h264 = PathBuf::from("/tmp/glue-f0-i4.264");
    let mut ref_h264: Option<PathBuf> = None;
    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dumps" => {
                i += 1;
                dumps = PathBuf::from(&args[i]);
            }
            "--h264" => {
                i += 1;
                h264 = PathBuf::from(&args[i]);
            }
            "--ref-h264" => {
                i += 1;
                ref_h264 = Some(PathBuf::from(&args[i]));
            }
            _ => {}
        }
        i += 1;
    }

    let syn = match load_syntax_dir(&dumps) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let n = syn.pred_y.len() / 256;
    let mbs = (HD_WIDTH / 16) * (HD_HEIGHT / 16);
    let frames = n / mbs;
    let spec = IslandSpec {
        width: HD_WIDTH,
        height: HD_HEIGHT,
        num_frames: frames.max(1),
        frame_start: 0,
        frame_end: 0,
        rect: hd_center_box(),
        halo_mbs: 0,
    };
    let plane = spec.width * spec.height;
    let mut naive = vec![0u8; plane];
    if let Err(e) = decode_frame_luma(&syn, &spec, 0, SpliceDecodeMode::Naive, &mut naive) {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    }
    let dec = match ffmpeg_luma(&h264, plane) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let (max_n, mean_n) = max_mean(&naive, &dec);
    println!("glue ffmpeg vs dump-naive: max={max_n} mean={mean_n:.3}");

    if let Some(ref_path) = ref_h264 {
        let r = match ffmpeg_luma(&ref_path, plane) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        let (max_r, mean_r) = max_mean(&r, &dec);
        println!("glue ffmpeg vs normal-encode: max={max_r} mean={mean_r:.3}");
        if max_r != 0 {
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
