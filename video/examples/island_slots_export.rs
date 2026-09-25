//! Write the island macroblock index list that `scripts/eva_mixed_merge.py merge` consumes.
//!
//! `geometric` is gadget ∪ Chebyshev halo on the edited frames. `pmv` additionally takes the
//! motion-vector fixpoint from the *original capture* dumps, which is the only side the
//! publisher is allowed to look at when deciding what must be re-proved.
//!
//! ```bash
//! cargo run --release -p video --example island_slots_export -- \
//!   --dumps docs/fixtures/glue-30f-normal --mode pmv --halo 1 \
//!   --out /tmp/slots-pmv.txt
//! ```

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use video::{
    hd_center_box, island_indices, load_mv_dir, pmv_closure, redact_window, IslandSpec, HD_FRAMES,
    HD_HEIGHT, HD_WIDTH,
};

fn main() -> ExitCode {
    let mut dumps = PathBuf::from("docs/fixtures/glue-30f-normal");
    let mut out = PathBuf::from("/tmp/island-slots.txt");
    let mut mode = "geometric".to_string();
    let mut halo = 1usize;
    let mut frames = HD_FRAMES;
    let mut window: Option<(usize, usize)> = None;

    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dumps" => {
                i += 1;
                dumps = PathBuf::from(&args[i]);
            }
            "--out" => {
                i += 1;
                out = PathBuf::from(&args[i]);
            }
            "--mode" => {
                i += 1;
                mode = args[i].clone();
            }
            "--halo" => {
                i += 1;
                halo = args[i].parse().unwrap_or(1);
            }
            "--frames" => {
                i += 1;
                frames = args[i].parse().unwrap_or(HD_FRAMES);
            }
            "--window" => {
                window = Some((
                    args[i + 1].parse().unwrap_or(0),
                    args[i + 2].parse().unwrap_or(0),
                ));
                i += 2;
            }
            other => {
                eprintln!("unknown arg: {other}");
                return ExitCode::FAILURE;
            }
        }
        i += 1;
    }

    let (fs_start, fs_end) = window.unwrap_or_else(|| redact_window(frames));
    let spec = IslandSpec {
        width: HD_WIDTH,
        height: HD_HEIGHT,
        num_frames: frames,
        frame_start: fs_start,
        frame_end: fs_end,
        rect: hd_center_box(),
        halo_mbs: halo,
    };

    let geo = match island_indices(&spec) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("island_indices: {e}");
            return ExitCode::FAILURE;
        }
    };
    let total = spec.total_mbs().unwrap();

    let mut slots: Vec<usize> = match mode.as_str() {
        "geometric" => geo.clone(),
        "pmv" => {
            let type_enc = match fs::read(dumps.join("type_enc")) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("read type_enc: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let mv = match load_mv_dir(&dumps, total) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("load mv: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match pmv_closure(&spec, &mv, &type_enc[..total * 6]) {
                Ok((set, counts)) => {
                    eprintln!(
                        "closure: geometric={} closure={} added_post={} added_edited={} added_pre={}",
                        counts.geometric,
                        counts.closure,
                        counts.added_post,
                        counts.added_edited_outside_halo,
                        counts.added_pre
                    );
                    set.into_iter().collect()
                }
                Err(e) => {
                    eprintln!("pmv_closure: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        other => {
            eprintln!("unknown --mode {other} (geometric|pmv)");
            return ExitCode::FAILURE;
        }
    };
    slots.sort_unstable();

    let mut text = format!(
        "# mode={mode} halo={halo} frames={frames} window={fs_start}..{fs_end} \
         slots={} total={total} geometric={}\n",
        slots.len(),
        geo.len()
    );
    for g in &slots {
        text.push_str(&format!("{g}\n"));
    }
    if let Some(parent) = out.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(e) = fs::write(&out, text) {
        eprintln!("write {}: {e}", out.display());
        return ExitCode::FAILURE;
    }
    eprintln!(
        "{}: {} slots of {total} ({:.2}%)",
        out.display(),
        slots.len(),
        100.0 * slots.len() as f64 / total as f64
    );
    ExitCode::SUCCESS
}
