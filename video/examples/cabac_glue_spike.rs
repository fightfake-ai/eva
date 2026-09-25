//! CABAC glue spike (I4-only I-frame): dump → glue-encode → ffmpeg vs normal JM encode.
//!
//! Primary metric: **luma max abs diff = 0** between glue bitstream and a fresh JM encode
//! with the same I4-only settings (chroma may differ — dumps are luma-only).
//!
//! ```bash
//! dd if=docs/fixtures/sample_720p_30f.yuv of=/tmp/frame0.yuv bs=$((1280*720*3/2)) count=1
//! ./scripts/jm_syntax_dumps.sh --yuv /tmp/frame0.yuv --out docs/fixtures/glue-frame0 \
//!   --width 1280 --height 720 --frames 1 --all-intra --i4-only
//! cargo run --release -p video --example cabac_glue_spike -- \
//!   --out docs/results/cabac-glue-720p.csv
//! ```

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use video::{HD_HEIGHT, HD_WIDTH};

fn run_cmd(cmd: &mut Command) -> Result<(), String> {
    let status = cmd.status().map_err(|e| format!("{e}"))?;
    if !status.success() {
        return Err(format!("command failed: {cmd:?}"));
    }
    Ok(())
}

fn ffmpeg_luma(h264: &Path, plane: usize) -> Result<Vec<u8>, String> {
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
    let mut out_csv = PathBuf::from("docs/results/cabac-glue-720p.csv");
    let mut dumps = PathBuf::from("docs/fixtures/glue-frame0");
    let mut yuv = PathBuf::from("/tmp/frame0.yuv");
    let mut skip_encode = false;

    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                out_csv = PathBuf::from(&args[i]);
            }
            "--dumps" => {
                i += 1;
                dumps = PathBuf::from(&args[i]);
            }
            "--yuv" => {
                i += 1;
                yuv = PathBuf::from(&args[i]);
            }
            "--skip-encode" => skip_encode = true,
            _ => {}
        }
        i += 1;
    }

    if !dumps.join("pred_y_enc").exists() {
        eprintln!(
            "missing dumps at {} — run jm_syntax_dumps.sh --all-intra --i4-only first",
            dumps.display()
        );
        return ExitCode::FAILURE;
    }
    if !yuv.exists() {
        let fixture = PathBuf::from("docs/fixtures/sample_720p_30f.yuv");
        if !fixture.exists() {
            eprintln!("missing {yuv:?} and fixture");
            return ExitCode::FAILURE;
        }
        let frame_bytes = HD_WIDTH * HD_HEIGHT * 3 / 2;
        let status = Command::new("dd")
            .args([
                format!("if={}", fixture.display()),
                format!("of={}", yuv.display()),
                format!("bs={frame_bytes}"),
                "count=1".into(),
                "status=none".into(),
            ])
            .status();
        if status.map(|s| !s.success()).unwrap_or(true) {
            eprintln!("dd frame0 failed");
            return ExitCode::FAILURE;
        }
    }

    let glue_264 = PathBuf::from("docs/fixtures/glue-spike-out.264");
    let norm_264 = PathBuf::from("docs/fixtures/glue-normal-i4.264");

    if !skip_encode {
        if let Err(e) = run_cmd(
            Command::new("bash")
                .arg("scripts/jm_glue_encode.sh")
                .args([
                    "--glue-dir",
                    dumps.to_str().unwrap(),
                    "--yuv",
                    yuv.to_str().unwrap(),
                    "--out",
                    glue_264.to_str().unwrap(),
                    "--frames",
                    "1",
                    "--gop",
                    "1",
                    "--i4-only",
                ]),
        ) {
            eprintln!("glue encode: {e}");
            return ExitCode::FAILURE;
        }

        // Fresh JM encode (no EVA_GLUE_DIR) with same I4-only knobs.
        let jm = std::env::var("JM_SRC").unwrap_or_else(|_| "/tmp/JM".into());
        let lencod = PathBuf::from(&jm).join("bin/lencod.exe");
        let work = PathBuf::from("docs/fixtures/glue-normal-work");
        let _ = fs::create_dir_all(&work);
        let mut cmd = Command::new(&lencod);
        cmd.env_remove("EVA_GLUE_DIR")
            .env_remove("EVA_DUMP_DIR")
            .args([
                "-d",
                &format!("{jm}/bin/encoder.cfg"),
                "-p",
                &format!("InputFile={}", yuv.display()),
                "-p",
                &format!("SourceWidth={HD_WIDTH}"),
                "-p",
                &format!("SourceHeight={HD_HEIGHT}"),
                "-p",
                &format!("OutputWidth={HD_WIDTH}"),
                "-p",
                &format!("OutputHeight={HD_HEIGHT}"),
                "-p",
                "FramesToBeEncoded=1",
                "-p",
                "NumberBFrames=0",
                "-p",
                "ProfileIDC=100",
                "-p",
                "QPISlice=28",
                "-p",
                "QPPSlice=28",
                "-p",
                "RateControlEnable=0",
                "-p",
                "IntraPeriod=1",
                "-p",
                "IDRPeriod=1",
                "-p",
                "Transform8x8Mode=0",
                "-p",
                "DisableIntra16x16=1",
                "-p",
                &format!("OutputFile={}", norm_264.display()),
                "-p",
                &format!("ReconFile={}/rec.yuv", work.display()),
            ]);
        if let Err(e) = run_cmd(&mut cmd) {
            eprintln!("normal encode: {e}");
            return ExitCode::FAILURE;
        }
    }

    let plane = HD_WIDTH * HD_HEIGHT;
    let glue_y = match ffmpeg_luma(&glue_264, plane) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let norm_y = match ffmpeg_luma(&norm_264, plane) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let (max_luma, mean_luma) = max_mean(&glue_y, &norm_y);
    eprintln!("I4 I-frame glue vs normal encode luma: max={max_luma} mean={mean_luma:.4}");

    if let Some(parent) = out_csv.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let row = format!(
        "scenario,frames,i4_only,luma_max,luma_mean,pass\n\
         i4_iframe_roundtrip,1,true,{max_luma},{mean_luma:.6},{}\n",
        max_luma == 0
    );
    if let Err(e) = fs::write(&out_csv, row) {
        eprintln!("write csv: {e}");
        return ExitCode::FAILURE;
    }
    eprintln!("wrote {}", out_csv.display());
    if max_luma != 0 {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
