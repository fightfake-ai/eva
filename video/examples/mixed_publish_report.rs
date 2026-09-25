//! Score a mixed publish bitstream: does the island match the edit, and is everything else
//! still the camera's video?
//!
//! Inputs are three reference-decoder outputs of the same clip — the original capture stream,
//! a full re-encode of the edited clip, and the spliced stream built from
//! `scripts/eva_mixed_merge.py merge` + `scripts/jm_glue_encode.sh`.
//!
//! ```bash
//! cargo run --release -p video --example mixed_publish_report -- \
//!   --cand /tmp/eva-mixed/dec/mixed-geo.yuv \
//!   --orig /tmp/eva-mixed/dec/orig.yuv \
//!   --edited /tmp/eva-mixed/dec/edited.yuv \
//!   --slots /tmp/eva-mixed/slots-geo.txt \
//!   --gadget-slots /tmp/eva-mixed/slots-gadget.txt \
//!   --label geometric --csv docs/results/mixed-publish-720p.csv
//! ```

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const W: usize = 1280;
const H: usize = 720;

struct Frames {
    data: Vec<u8>,
}

impl Frames {
    fn load(path: &Path) -> Result<Self, String> {
        fs::read(path)
            .map(|data| Frames { data })
            .map_err(|e| format!("read {}: {e}", path.display()))
    }

    fn count(&self) -> usize {
        self.data.len() / (W * H * 3 / 2)
    }
}

/// Max and summed absolute luma difference over one macroblock.
fn mb_diff(a: &Frames, b: &Frames, frame: usize, mb: usize) -> (u8, u64) {
    let fb = W * H * 3 / 2;
    let cols = W / 16;
    let (mx, my) = (mb % cols, mb / cols);
    let base = frame * fb;
    let mut max = 0u8;
    let mut sum = 0u64;
    for row in 0..16 {
        let off = base + (my * 16 + row) * W + mx * 16;
        for i in 0..16 {
            let d = a.data[off + i].abs_diff(b.data[off + i]);
            if d > max {
                max = d;
            }
            sum += d as u64;
        }
    }
    (max, sum)
}

fn mb_mean_luma(a: &Frames, frame: usize, mb: usize) -> f64 {
    let fb = W * H * 3 / 2;
    let cols = W / 16;
    let (mx, my) = (mb % cols, mb / cols);
    let base = frame * fb;
    let mut sum = 0u64;
    for row in 0..16 {
        let off = base + (my * 16 + row) * W + mx * 16;
        for i in 0..16 {
            sum += a.data[off + i] as u64;
        }
    }
    sum as f64 / 256.0
}

fn load_slots(path: &Path) -> Result<HashSet<usize>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(text
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.trim().parse().ok())
        .collect())
}

/// A macroblock whose worst luma sample is off by more than this is a visible artifact,
/// not requantization noise.
const VISIBLE: u8 = 16;

struct Acc {
    max: u8,
    sum: u64,
    px: u64,
    mbs: usize,
    mbs_diff: usize,
    mbs_visible: usize,
}

impl Acc {
    fn new() -> Self {
        Acc {
            max: 0,
            sum: 0,
            px: 0,
            mbs: 0,
            mbs_diff: 0,
            mbs_visible: 0,
        }
    }

    fn push(&mut self, (max, sum): (u8, u64)) {
        if max > self.max {
            self.max = max;
        }
        self.sum += sum;
        self.px += 256;
        self.mbs += 1;
        if max > 0 {
            self.mbs_diff += 1;
        }
        if max > VISIBLE {
            self.mbs_visible += 1;
        }
    }

    fn mean(&self) -> f64 {
        if self.px == 0 {
            0.0
        } else {
            self.sum as f64 / self.px as f64
        }
    }
}

fn main() -> ExitCode {
    let mut cand = PathBuf::new();
    let mut orig = PathBuf::new();
    let mut edited = PathBuf::new();
    let mut slots_path = PathBuf::new();
    let mut gadget_path: Option<PathBuf> = None;
    let mut csv: Option<PathBuf> = None;
    let mut label = "mixed".to_string();

    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let mut next = |i: &mut usize| {
            *i += 1;
            args[*i].clone()
        };
        match args[i].as_str() {
            "--cand" => cand = PathBuf::from(next(&mut i)),
            "--orig" => orig = PathBuf::from(next(&mut i)),
            "--edited" => edited = PathBuf::from(next(&mut i)),
            "--slots" => slots_path = PathBuf::from(next(&mut i)),
            "--gadget-slots" => gadget_path = Some(PathBuf::from(next(&mut i))),
            "--csv" => csv = Some(PathBuf::from(next(&mut i))),
            "--label" => label = next(&mut i),
            other => {
                eprintln!("unknown arg: {other}");
                return ExitCode::FAILURE;
            }
        }
        i += 1;
    }

    let (cand, orig, edited) = match (
        Frames::load(&cand),
        Frames::load(&orig),
        Frames::load(&edited),
    ) {
        (Ok(a), Ok(b), Ok(c)) => (a, b, c),
        (a, b, c) => {
            for r in [a.err(), b.err(), c.err()].into_iter().flatten() {
                eprintln!("{r}");
            }
            return ExitCode::FAILURE;
        }
    };
    let island = match load_slots(&slots_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let gadget = match gadget_path.as_ref().map(|p| load_slots(p)) {
        Some(Ok(v)) => v,
        Some(Err(e)) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
        None => island.clone(),
    };

    let frames = cand.count().min(orig.count()).min(edited.count());
    let mbs = (W / 16) * (H / 16);

    println!(
        "label={label} frames={frames} island_slots={} gadget_slots={}",
        island.len(),
        gadget.len()
    );
    println!(
        "{:>5} {:>7} {:>9} {:>9} {:>8} {:>9} {:>9} {:>8} {:>9} {:>9}",
        "frame",
        "island",
        "isl|edit",
        "isl_mean",
        "isl_vis",
        "keep|orig",
        "keep_mean",
        "keep_vis",
        "gad|edit",
        "gad_luma"
    );

    let mut rows = Vec::new();
    for f in 0..frames {
        let mut isl = Acc::new();
        let mut skip = Acc::new();
        let mut gad = Acc::new();
        // Every pixel outside the redact box is supposed to still be the camera's pixel,
        // whether or not the publisher had to re-prove its macroblock.
        let mut keep = Acc::new();
        let mut gad_luma_sum = 0.0;
        for mb in 0..mbs {
            let g = f * mbs + mb;
            if island.contains(&g) {
                isl.push(mb_diff(&cand, &edited, f, mb));
            } else {
                skip.push(mb_diff(&cand, &orig, f, mb));
            }
            if gadget.contains(&g) {
                gad.push(mb_diff(&cand, &edited, f, mb));
                gad_luma_sum += mb_mean_luma(&cand, f, mb);
            } else {
                keep.push(mb_diff(&cand, &orig, f, mb));
            }
        }
        let gad_luma = if gad.mbs == 0 {
            f64::NAN
        } else {
            gad_luma_sum / gad.mbs as f64
        };
        println!(
            "{f:>5} {:>7} {:>9} {:>9.3} {:>8} {:>9} {:>9.3} {:>8} {:>9} {:>9.2}",
            isl.mbs,
            isl.max,
            isl.mean(),
            isl.mbs_visible,
            keep.max,
            keep.mean(),
            keep.mbs_visible,
            gad.max,
            gad_luma
        );
        rows.push(format!(
            "{label},{f},{},{},{:.4},{},{},{},{},{:.4},{},{},{},{:.4},{},{},{:.2}",
            isl.mbs,
            isl.max,
            isl.mean(),
            isl.mbs_diff,
            isl.mbs_visible,
            skip.max,
            skip.mbs_diff,
            skip.mean(),
            skip.mbs_visible,
            keep.max,
            keep.mbs_diff,
            keep.mean(),
            keep.mbs_visible,
            gad.max,
            gad_luma
        ));
    }

    if let Some(path) = csv {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let header = "merge_mode,frame,island_mbs,island_vs_edited_max,island_vs_edited_mean,\
island_mbs_differing,island_mbs_visible,skip_vs_orig_max,skip_mbs_differing,skip_vs_orig_mean,\
skip_mbs_visible,nongadget_vs_orig_max,nongadget_mbs_differing,nongadget_vs_orig_mean,\
nongadget_mbs_visible,gadget_vs_edited_max,gadget_mean_luma\n";
        let body = rows.join("\n") + "\n";
        let text = if path.exists() && fs::read_to_string(&path).is_ok_and(|s| s.starts_with("merge_mode,"))
        {
            fs::read_to_string(&path).unwrap_or_default() + &body
        } else {
            header.to_string() + &body
        };
        if let Err(e) = fs::write(&path, text) {
            eprintln!("write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        eprintln!("wrote {}", path.display());
    }

    ExitCode::SUCCESS
}
