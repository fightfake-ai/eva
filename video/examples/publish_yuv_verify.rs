//! Close the island-only loop: statement YUV ↔ publish decode, with a small edit witness.
//!
//! Protocol shape (definition B from the exact-publish work):
//! - Camera public: decode of the signed H.264 → `R_pix` (Merkle over macroblock YUV).
//! - Statement public: decode of the exact publish → `R_stmt`.
//! - Edit witness: geometric paint island only (here a 5×5 MB box).
//! - Cascade / DF ring live in the publish file; they are *not* part of the SNARK edit.
//!
//! ```bash
//! # After building last-frame exact publish + ffmpeg decode:
//! cargo run --release -p video --example publish_yuv_verify -- \
//!   /tmp/glue-8f-ref1-src.yuv \
//!   /tmp/exact-f7-ff.yuv \
//!   /tmp/exact-f7-ipcm.txt \
//!   1280 720 8 \
//!   480 192 80 80 7 8 \
//!   16 160 96
//! ```
//!
//! Args: signed.yuv  statement.yuv  ipcm.txt  W H frames \
//!       paint_x paint_y paint_w paint_h frame_start frame_end \
//!       [fill_y fill_u fill_v]

use std::collections::HashSet;
use std::env;
use std::fs;
use std::process::ExitCode;

use video::macroblock_yuv::{
    macroblock_xy, macroblocks_per_frame, macroblocks_to_yuv420, yuv420_frame_bytes,
    yuv420_to_macroblocks, MB_UV_BYTES, MB_Y_BYTES,
};
use video::{
    capture_pixel_tree, native_redact_edit_macroblocks, IslandSpec, PixelRect,
};

fn usage() -> &'static str {
    "Usage: publish_yuv_verify <signed.yuv> <statement.yuv> <ipcm.txt> \\\n\
     \t<width> <height> <num_frames> \\\n\
     \t<paint_x> <paint_y> <paint_w> <paint_h> <frame_start> <frame_end> \\\n\
     \t[fill_y fill_u fill_v]\n\
     \n\
     statement.yuv must be the ffmpeg decode of the exact publish (definition B).\n\
     ipcm.txt lines: `frame mx my` from EVA_GLUE_IPCMLOG."
}

fn parse_usize(s: &str, name: &str) -> Result<usize, String> {
    s.parse().map_err(|_| format!("invalid {name}: {s}"))
}

fn parse_u8(s: &str, name: &str) -> Result<u8, String> {
    s.parse().map_err(|_| format!("invalid {name}: {s}"))
}

fn hex32(d: &[u8; 32]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

fn load_ipcm(path: &str) -> Result<HashSet<(usize, usize, usize)>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    let mut out = HashSet::new();
    for (lineno, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let f = parts
            .next()
            .ok_or_else(|| format!("{path}:{}: missing frame", lineno + 1))?;
        let mx = parts
            .next()
            .ok_or_else(|| format!("{path}:{}: missing mx", lineno + 1))?;
        let my = parts
            .next()
            .ok_or_else(|| format!("{path}:{}: missing my", lineno + 1))?;
        out.insert((
            parse_usize(f, "frame")?,
            parse_usize(mx, "mx")?,
            parse_usize(my, "my")?,
        ));
    }
    Ok(out)
}

fn mb_planes_eq(
    a: &[u8],
    b: &[u8],
    width: usize,
    height: usize,
    frame: usize,
    mx: usize,
    my: usize,
) -> bool {
    let fs = width * height * 3 / 2;
    let ys = width * height;
    let base = frame * fs;
    let y0 = my * 16;
    let x0 = mx * 16;
    for row in 0..16 {
        let i = base + (y0 + row) * width + x0;
        if a[i..i + 16] != b[i..i + 16] {
            return false;
        }
    }
    let cw = width / 2;
    let ch = height / 2;
    for plane in 0..2 {
        let off = base + ys + plane * cw * ch;
        for row in 0..8 {
            let i = off + (my * 8 + row) * cw + mx * 8;
            if a[i..i + 8] != b[i..i + 8] {
                return false;
            }
        }
    }
    true
}

fn mb_matches_fill(
    yuv: &[u8],
    width: usize,
    height: usize,
    frame: usize,
    mx: usize,
    my: usize,
    fill_y: u8,
    fill_u: u8,
    fill_v: u8,
) -> bool {
    let fs = width * height * 3 / 2;
    let ys = width * height;
    let base = frame * fs;
    let y0 = my * 16;
    let x0 = mx * 16;
    for row in 0..16 {
        let i = base + (y0 + row) * width + x0;
        if yuv[i..i + 16].iter().any(|&p| p != fill_y) {
            return false;
        }
    }
    let cw = width / 2;
    let ch = height / 2;
    for row in 0..8 {
        let u = base + ys + (my * 8 + row) * cw + mx * 8;
        let v = base + ys + cw * ch + (my * 8 + row) * cw + mx * 8;
        if yuv[u..u + 8].iter().any(|&p| p != fill_u) {
            return false;
        }
        if yuv[v..v + 8].iter().any(|&p| p != fill_v) {
            return false;
        }
    }
    true
}

fn global_mb_index(width: usize, height: usize, frame: usize, mx: usize, my: usize) -> usize {
    let cols = width / 16;
    let mbs = macroblocks_per_frame(width, height).unwrap();
    frame * mbs + my * cols + mx
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 12 || args.len() > 15 {
        eprintln!("{}", usage());
        return ExitCode::FAILURE;
    }

    let run = (|| -> Result<(), String> {
        let signed_path = &args[0];
        let stmt_path = &args[1];
        let ipcm_path = &args[2];
        let width = parse_usize(&args[3], "width")?;
        let height = parse_usize(&args[4], "height")?;
        let num_frames = parse_usize(&args[5], "num_frames")?;
        let paint_x = parse_usize(&args[6], "paint_x")?;
        let paint_y = parse_usize(&args[7], "paint_y")?;
        let paint_w = parse_usize(&args[8], "paint_w")?;
        let paint_h = parse_usize(&args[9], "paint_h")?;
        let frame_start = parse_usize(&args[10], "frame_start")?;
        let frame_end = parse_usize(&args[11], "frame_end")?;
        let fill_y = if args.len() > 12 {
            parse_u8(&args[12], "fill_y")?
        } else {
            16
        };
        let fill_u = if args.len() > 13 {
            parse_u8(&args[13], "fill_u")?
        } else {
            160
        };
        let fill_v = if args.len() > 14 {
            parse_u8(&args[14], "fill_v")?
        } else {
            96
        };

        let frame_bytes = yuv420_frame_bytes(width, height)?;
        let need = frame_bytes * num_frames;
        let signed = fs::read(signed_path).map_err(|e| format!("read {signed_path}: {e}"))?;
        let statement = fs::read(stmt_path).map_err(|e| format!("read {stmt_path}: {e}"))?;
        if signed.len() != need || statement.len() != need {
            return Err(format!(
                "YUV size mismatch: signed={} statement={} need={need}",
                signed.len(),
                statement.len()
            ));
        }

        let spec = IslandSpec {
            width,
            height,
            num_frames,
            frame_start,
            frame_end,
            rect: PixelRect {
                x: paint_x,
                y: paint_y,
                w: paint_w,
                h: paint_h,
            },
            halo_mbs: 0, // gadget-only: edit witness, not encode halo
        };
        spec.validate()?;
        if !spec.rect.is_macroblock_aligned() {
            return Err("paint rectangle must be macroblock-aligned".into());
        }

        let cols = width / 16;
        let rows = height / 16;
        let mbs_per_frame = cols * rows;
        let mut paint: HashSet<(usize, usize, usize)> = HashSet::new();
        for f in frame_start..frame_end.min(num_frames) {
            let mx0 = paint_x / 16;
            let my0 = paint_y / 16;
            let mx1 = (paint_x + paint_w) / 16;
            let my1 = (paint_y + paint_h) / 16;
            for my in my0..my1 {
                for mx in mx0..mx1 {
                    paint.insert((f, mx, my));
                }
            }
        }
        let ipcm = load_ipcm(ipcm_path)?;
        let cascade: HashSet<_> = ipcm.difference(&paint).cloned().collect();

        println!("=== publish_yuv_verify (definition B) ===");
        println!("signed     {signed_path}");
        println!("statement  {stmt_path}");
        println!("frames {num_frames}  size {width}x{height}  mbs/frame {mbs_per_frame}");
        println!(
            "paint island {} MBs  cascade {}  total IPCM {}",
            paint.len(),
            cascade.len(),
            ipcm.len()
        );

        // 1) Frames before the edit: statement == signed on every MB.
        let mut pre_edit_ok = 0usize;
        let mut pre_edit_bad = 0usize;
        for f in 0..frame_start.min(num_frames) {
            for my in 0..rows {
                for mx in 0..cols {
                    if mb_planes_eq(&signed, &statement, width, height, f, mx, my) {
                        pre_edit_ok += 1;
                    } else {
                        pre_edit_bad += 1;
                    }
                }
            }
        }
        println!(
            "pre-edit frames [0,{frame_start}): identical MBs {pre_edit_ok}  differ {pre_edit_bad}"
        );
        if pre_edit_bad != 0 {
            return Err("statement diverges from signed before the edit frame".into());
        }

        // 2) Paint MBs in the statement show the fill (edit witness).
        let mut paint_ok = 0usize;
        let mut paint_bad = 0usize;
        for &(f, mx, my) in &paint {
            if mb_matches_fill(
                &statement, width, height, f, mx, my, fill_y, fill_u, fill_v,
            ) {
                paint_ok += 1;
            } else {
                paint_bad += 1;
            }
        }
        println!("paint fill in statement: ok {paint_ok}  bad {paint_bad}");
        if paint_bad != 0 {
            return Err("statement paint MBs do not match fill witness".into());
        }

        // 3) On the edit frame(s): classify every MB.
        let mut inject_ok = 0usize;
        let mut cascade_diff = 0usize;
        let mut ring = 0usize;
        for f in frame_start..frame_end.min(num_frames) {
            for my in 0..rows {
                for mx in 0..cols {
                    let key = (f, mx, my);
                    if paint.contains(&key) {
                        continue;
                    }
                    let eq = mb_planes_eq(&signed, &statement, width, height, f, mx, my);
                    if eq {
                        inject_ok += 1;
                    } else if cascade.contains(&key) {
                        cascade_diff += 1;
                    } else {
                        ring += 1;
                    }
                }
            }
        }
        println!(
            "edit-frame non-paint: inject≡signed {inject_ok}  cascade≠signed {cascade_diff}  ring {ring}"
        );

        // 4) Merkle public claims.
        let (sy, su, sv) = yuv420_to_macroblocks(&signed, width, height, num_frames)?;
        let (ty, tu, tv) = yuv420_to_macroblocks(&statement, width, height, num_frames)?;
        let pix_tree = capture_pixel_tree(&sy, &su, &sv)?;
        let stmt_tree = capture_pixel_tree(&ty, &tu, &tv)?;
        let r_pix = pix_tree.root();
        let r_stmt = stmt_tree.root();
        println!("R_pix  {}", hex32(&r_pix));
        println!("R_stmt {}", hex32(&r_stmt));
        if r_pix == r_stmt {
            return Err("R_pix == R_stmt; expected an edit".into());
        }

        // 5) Open every paint leaf against both roots (SNARK-shaped witness).
        let mut openings_ok = 0usize;
        for &(f, mx, my) in &paint {
            let g = global_mb_index(width, height, f, mx, my);
            let p_orig = pix_tree.proof(g)?;
            let p_stmt = stmt_tree.proof(g)?;
            if !p_orig.verify(&r_pix) || !p_stmt.verify(&r_stmt) {
                return Err(format!("Merkle opening failed at global MB {g} ({f},{mx},{my})"));
            }
            // Leaf payloads differ: edit witness.
            let oy = &sy[g * MB_Y_BYTES..(g + 1) * MB_Y_BYTES];
            let tyb = &ty[g * MB_Y_BYTES..(g + 1) * MB_Y_BYTES];
            if oy == tyb {
                return Err(format!("paint MB {g} unchanged in statement"));
            }
            openings_ok += 1;
        }
        println!(
            "Merkle openings: {openings_ok}/{} paint MBs open under R_pix and R_stmt",
            paint.len()
        );

        // 6) Outside paint: statement leaf equals signed leaf iff MB matched above.
        let mut outside_same_leaf = 0usize;
        let mut outside_diff_leaf = 0usize;
        for g in 0..(mbs_per_frame * num_frames) {
            let frame = g / mbs_per_frame;
            let (mx, my) = macroblock_xy(width, g % mbs_per_frame);
            if paint.contains(&(frame, mx, my)) {
                continue;
            }
            let same = sy[g * MB_Y_BYTES..(g + 1) * MB_Y_BYTES]
                == ty[g * MB_Y_BYTES..(g + 1) * MB_Y_BYTES]
                && su[g * MB_UV_BYTES..(g + 1) * MB_UV_BYTES]
                    == tu[g * MB_UV_BYTES..(g + 1) * MB_UV_BYTES]
                && sv[g * MB_UV_BYTES..(g + 1) * MB_UV_BYTES]
                    == tv[g * MB_UV_BYTES..(g + 1) * MB_UV_BYTES];
            if same {
                outside_same_leaf += 1;
            } else {
                outside_diff_leaf += 1;
            }
        }
        println!(
            "outside-paint leaf equality: same {outside_same_leaf}  differ {outside_diff_leaf} (cascade+ring)"
        );

        // 7) Native RedactRect gadget on signed pixels → paint MBs must match statement.
        //    RedactRect fills chroma with 128; skip this check for other fills.
        if fill_u == 128 && fill_v == 128 {
            let (ey, eu, ev) = native_redact_edit_macroblocks(
                &sy,
                &su,
                &sv,
                width,
                height,
                num_frames,
                paint_x,
                paint_y,
                paint_w,
                paint_h,
                frame_start,
                frame_end,
                fill_y,
            )?;
            let gadget_yuv = macroblocks_to_yuv420(&ey, &eu, &ev, width, height, num_frames)?;
            let mut gadget_paint_ok = 0usize;
            let mut gadget_paint_bad = 0usize;
            for &(f, mx, my) in &paint {
                if mb_planes_eq(&gadget_yuv, &statement, width, height, f, mx, my) {
                    gadget_paint_ok += 1;
                } else {
                    gadget_paint_bad += 1;
                }
            }
            println!(
                "RedactRect native ↔ statement paint: ok {gadget_paint_ok}  bad {gadget_paint_bad}"
            );
            if gadget_paint_bad != 0 {
                return Err("native RedactRect paint MBs diverge from statement".into());
            }
        } else {
            println!(
                "RedactRect native check skipped (fill UV {fill_u}/{fill_v} ≠ 128/128)"
            );
        }

        println!(
            "PASS — statement is publish decode; edit witness is {} MBs; cascade stays outside the SNARK",
            paint.len()
        );
        Ok(())
    })();

    match run {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("FAIL: {e}");
            ExitCode::FAILURE
        }
    }
}
