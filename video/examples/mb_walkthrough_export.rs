//! Export full 720p edit walkthrough JSON with JM syntax diff + optional ffmpeg decode layer.
//!
//! ```bash
//! cargo run --release -p video --example mb_walkthrough_export -- \
//!   --gop 1 --out docs/results/mb-walkthrough-720p-intra.json
//! ```

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use video::{
    build_mb_grid_report, ffmpeg_available, hd_center_box, load_yuv_or_fixture, mb_walkthrough_to_json,
    redact_window, HD_FRAMES, HD_HEIGHT, HD_WIDTH, IslandSpec,
};

const SOURCE: &str = "FileSamples sample_1280x720.mp4 (30 frames → YUV420p)";
const SOURCE_URL: &str = "https://filesamples.com/samples/video/mp4/sample_1280x720.mp4";

fn main() -> ExitCode {
    let mut gop = 1usize;
    let mut out = PathBuf::from("docs/results/mb-walkthrough-720p-intra.json");
    let mut skip_jm = false;
    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--gop" => {
                i += 1;
                gop = args[i].parse().unwrap_or(1);
            }
            "--out" => {
                i += 1;
                out = PathBuf::from(&args[i]);
            }
            "--skip-jm" => skip_jm = true,
            _ => {}
        }
        i += 1;
    }

    let (fs, fe) = redact_window(HD_FRAMES);
    let spec = IslandSpec {
        width: HD_WIDTH,
        height: HD_HEIGHT,
        num_frames: HD_FRAMES,
        frame_start: fs,
        frame_end: fe,
        rect: hd_center_box(),
        halo_mbs: 1,
    };

    let orig = match load_yuv_or_fixture(None, spec.width, spec.height, spec.num_frames) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let edited = match video::redact_yuv(&orig, &spec) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let dump_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/fixtures/jm-dumps-720p");
    let syntax_orig = dump_root.join("orig");
    let syntax_edit = dump_root.join("edited");

    let syntax_label: String = if skip_jm {
        "syntax layer skipped (--skip-jm)".to_string()
    } else {
        match run_jm_dumps(&orig, &edited, &spec, &syntax_orig, &syntax_edit) {
            Ok(()) => "JM pred_y_enc/coeff_y_enc/type_enc byte-wise (orig vs edited YUV)".to_string(),
            Err(e) => {
                eprintln!("JM syntax dumps failed: {e}");
                return ExitCode::FAILURE;
            }
        }
    };

    let decode_pair = if ffmpeg_available() {
        match decoded_pair(&orig, &edited, &spec, gop, 23) {
            Ok(p) => Some(p),
            Err(e) => {
                eprintln!("ffmpeg decode layer skipped: {e}");
                None
            }
        }
    } else {
        eprintln!("ffmpeg not available; decode layer omitted");
        None
    };

    let (decode_a, decode_b) = match decode_pair {
        Some((a, b)) => (Some(a), Some(b)),
        None => (None, None),
    };

    let syntax_orig_dir = if skip_jm { None } else { Some(syntax_orig.as_path()) };
    let syntax_edit_dir = if skip_jm { None } else { Some(syntax_edit.as_path()) };

    let report = match build_mb_grid_report(
        &orig,
        &edited,
        &spec,
        decode_a.as_deref(),
        decode_b.as_deref(),
        0,
        syntax_orig_dir,
        syntax_edit_dir,
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let decode_label = format!("ffmpeg GOP={gop} decode(orig) vs decode(edited), ΔY>0");
    let json = mb_walkthrough_to_json(&report, SOURCE, SOURCE_URL, &decode_label, &syntax_label);
    if let Some(parent) = out.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(e) = fs::write(&out, &json) {
        eprintln!("write {}: {e}", out.display());
        return ExitCode::FAILURE;
    }
    eprintln!("wrote {}", out.display());
    if !skip_jm {
        eprintln!("syntax dumps: {} and {}", syntax_orig.display(), syntax_edit.display());
    }
    ExitCode::SUCCESS
}

fn run_jm_dumps(
    orig: &[u8],
    edited: &[u8],
    spec: &IslandSpec,
    out_orig: &Path,
    out_edit: &Path,
) -> Result<(), String> {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/jm_syntax_dumps.sh");
    if !script.is_file() {
        return Err(format!("missing {}", script.display()));
    }
    let work = env::temp_dir().join(format!(
        "eva-jm-yuv-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&work).map_err(|e| format!("mkdir: {e}"))?;
    let orig_path = work.join("orig.yuv");
    let edit_path = work.join("edit.yuv");
    fs::write(&orig_path, orig).map_err(|e| format!("write orig: {e}"))?;
    fs::write(&edit_path, edited).map_err(|e| format!("write edit: {e}"))?;

    jm_encode_one(&script, &orig_path, out_orig, spec)?;
    jm_encode_one(&script, &edit_path, out_edit, spec)?;
    let _ = fs::remove_dir_all(&work);
    Ok(())
}

fn jm_encode_one(script: &Path, yuv: &Path, out_dir: &Path, spec: &IslandSpec) -> Result<(), String> {
    fs::create_dir_all(out_dir).map_err(|e| format!("mkdir {}: {e}", out_dir.display()))?;
    let out = Command::new(script)
        .args([
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
        ])
        .output()
        .map_err(|e| format!("spawn jm_syntax_dumps: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "jm_syntax_dumps failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    eprintln!("{}", String::from_utf8_lossy(&out.stdout));
    Ok(())
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
    let dir = env::temp_dir().join(format!("eva-walk-{tag}"));
    fs::create_dir_all(&dir).map_err(|e| format!("mkdir: {e}"))?;
    let orig_path = dir.join("orig.yuv");
    let edit_path = dir.join("edit.yuv");
    let orig_dec = dir.join("orig-dec.yuv");
    let edit_dec = dir.join("edit-dec.yuv");
    let need = yuv420_frame_bytes(spec.width, spec.height)? * spec.num_frames;
    fs::write(&orig_path, orig).map_err(|e| format!("write orig: {e}"))?;
    fs::write(&edit_path, edited).map_err(|e| format!("write edit: {e}"))?;
    encode_decode(&orig_path, &orig_dec, spec, gop, qp)?;
    encode_decode(&edit_path, &edit_dec, spec, gop, qp)?;
    let a = fs::read(&orig_dec).map_err(|e| format!("read dec: {e}"))?;
    let b = fs::read(&edit_dec).map_err(|e| format!("read dec: {e}"))?;
    let _ = fs::remove_dir_all(&dir);
    if a.len() < need || b.len() < need {
        return Err("decoded YUV too short".into());
    }
    Ok((a[..need].to_vec(), b[..need].to_vec()))
}

fn encode_decode(
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
            out.with_extension("mp4").to_str().unwrap(),
        ])
        .output()
        .map_err(|e| format!("ffmpeg enc: {e}"))?;
    if !enc.status.success() {
        return Err(format!(
            "ffmpeg encode failed: {}",
            String::from_utf8_lossy(&enc.stderr)
        ));
    }
    let dec = Command::new("ffmpeg")
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            out.with_extension("mp4").to_str().unwrap(),
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
        return Err(format!(
            "ffmpeg decode failed: {}",
            String::from_utf8_lossy(&dec.stderr)
        ));
    }
    Ok(())
}
