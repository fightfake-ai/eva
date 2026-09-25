#!/usr/bin/env python3
"""Sweep the encoder settings that docs/island-exactness-explained.md lists as untested.

For each configuration: build camera dumps, recover the camera's own reconstruction,
paint a box, publish an exact file, decode it with ffmpeg, and count how many
macroblocks outside the painted box differ from the camera's decoded picture.

Writes docs/results/exactness-matrix.csv.

Usage:
    python3 scripts/exactness_matrix.py [name ...]
"""

import csv
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FFMPEG = "/opt/homebrew/bin/ffmpeg"
JM_SRC = os.environ.get("JM_SRC", "/tmp/JM-eva-src")
SRC30 = ROOT / "docs" / "fixtures" / "sample_720p_30f.yuv"
WORK = Path("/tmp/exactness-matrix")
OUT_CSV = ROOT / "docs" / "results" / "exactness-matrix.csv"

W, H = 1280, 720
YS = W * H
CW, CH = W // 2, H // 2
FS = YS * 3 // 2
COLS, ROWS = W // 16, H // 16


# Each config: frames, refs, gop, bframes, qp, rate_kbps, paint frame, paint box in MBs.
CONFIGS = [
    dict(name="base-8f-1ref", frames=8, refs=1, paint_frame=7,
         note="baseline from the document"),
    dict(name="long-30f-1ref", frames=30, refs=1, paint_frame=15,
         note="longer clip, single GOP of 30"),
    dict(name="refs5-30f", frames=30, refs=5, paint_frame=15,
         note="five reference frames"),
    dict(name="gop8-30f-1ref", frames=30, refs=1, gop=8, paint_frame=15,
         note="four GOPs, IDR every 8 frames"),
    dict(name="bframes2-30f", frames=30, refs=3, bframes=2, paint_frame=15,
         note="two B-frames between anchors"),
    dict(name="ratectl-30f-1ref", frames=30, refs=1, rate_kbps=6000, paint_frame=15,
         note="rate-controlled capture, QP varies per frame"),
    dict(name="qp20-30f-1ref", frames=30, refs=1, qp=20, paint_frame=15,
         note="higher quality"),
    dict(name="qp34-30f-1ref", frames=30, refs=1, qp=34, paint_frame=15,
         note="lower quality"),
    dict(name="edit-large-30f", frames=30, refs=1, paint_frame=15,
         box=(20, 8, 20, 14), note="large edit, 20x14 macroblocks"),
    dict(name="edit-strip-30f", frames=30, refs=1, paint_frame=15,
         box=(5, 22, 70, 1), note="thin strip, 70x1 macroblocks"),
    dict(name="edit-two-frames-30f", frames=30, refs=1, paint_frame=15,
         paint_frames=2, note="same box painted on two consecutive frames"),
]

DEFAULTS = dict(refs=1, gop=None, bframes=0, qp=28, rate_kbps=None,
                box=(30, 12, 5, 5), paint_frames=1, note="")


def sh(cmd, log, env=None):
    with open(log, "w") as lf:
        subprocess.check_call(cmd, stdout=lf, stderr=subprocess.STDOUT, env=env, cwd=ROOT)
    return Path(log).read_text()


def glue_env(**extra):
    env = os.environ.copy()
    env["JM_SRC"] = JM_SRC
    for k in ("EVA_GLUE_PREDEC", "EVA_GLUE_EXACT", "EVA_GLUE_ENCODE_SLOTS",
              "EVA_GLUE_RESLOG", "EVA_GLUE_IPCMLOG"):
        env.pop(k, None)
    env.update({k: str(v) for k, v in extra.items()})
    return env


def decode(bitstream, out_yuv):
    subprocess.check_call([FFMPEG, "-y", "-v", "error", "-threads", "1", "-i", str(bitstream),
                           "-f", "rawvideo", "-pix_fmt", "yuv420p", str(out_yuv)])


def paint(src_yuv, dst_yuv, frames, box, paint_frame, paint_frames):
    mx0, my0, mw, mh = box
    data = bytearray(Path(src_yuv).read_bytes())
    for f in range(paint_frame, min(paint_frame + paint_frames, frames)):
        base = f * FS
        for my in range(my0, my0 + mh):
            for mx in range(mx0, mx0 + mw):
                for y in range(16):
                    i = base + (my * 16 + y) * W + mx * 16
                    data[i:i + 16] = b"\x10" * 16
                for p in (0, 1):
                    off = base + YS + p * CW * CH
                    for y in range(8):
                        i = off + (my * 8 + y) * CW + mx * 8
                        data[i:i + 8] = bytes([128]) * 8
    Path(dst_yuv).write_bytes(bytes(data))


def mb_eq(a, b, f, mx, my):
    base = f * FS
    y0, x0 = my * 16, mx * 16
    for y in range(16):
        i = base + (y0 + y) * W + x0
        if a[i:i + 16] != b[i:i + 16]:
            return False
    for p in (0, 1):
        off = base + YS + p * CW * CH
        for y in range(8):
            i = off + (my * 8 + y) * CW + mx * 8
            if a[i:i + 8] != b[i:i + 8]:
                return False
    return True


def run_config(cfg):
    c = dict(DEFAULTS)
    c.update(cfg)
    name = c["name"]
    frames = c["frames"]
    d = WORK / name
    d.mkdir(parents=True, exist_ok=True)

    src = d / "src.yuv"
    if not src.exists():
        Path(src).write_bytes(SRC30.read_bytes()[: frames * FS])

    dumps = d / "cam"
    dump_args = ["bash", "scripts/jm_syntax_dumps.sh", "--yuv", str(src), "--out", str(dumps),
                 "--width", str(W), "--height", str(H), "--frames", str(frames),
                 "--qp", str(c["qp"]), "--refs", str(c["refs"]), "--df-off"]
    if c["gop"]:
        dump_args += ["--gop", str(c["gop"])]
    if c["bframes"]:
        dump_args += ["--bframes", str(c["bframes"])]
    if c["rate_kbps"]:
        dump_args += ["--rate-kbps", str(c["rate_kbps"])]

    result = dict(name=name, frames=frames, refs=c["refs"], gop=c["gop"] or "one",
                  bframes=c["bframes"], qp=c["qp"],
                  rate_kbps=c["rate_kbps"] or "constant QP",
                  edit_mbs=c["box"][2] * c["box"][3] * c["paint_frames"],
                  note=c["note"])

    try:
        sh(dump_args, d / "dumps.log")
    except subprocess.CalledProcessError:
        result.update(status="camera encode failed", detail=(d / "dumps.log").name)
        return result

    cam_264 = dumps / "source.264"
    result["camera_bytes"] = cam_264.stat().st_size

    # The camera's own reconstruction, from injecting its syntax with no edit.
    blank = d / "blank.yuv"
    if not blank.exists():
        blank.write_bytes(bytes(frames * FS))
    pre = d / "pre.yuv"
    if pre.exists():
        pre.unlink()  # the glue appends frames, so a re-run must start empty
    glue_args = ["bash", "scripts/jm_glue_encode.sh", "--glue-dir", str(dumps),
                 "--yuv", str(blank), "--out", str(d / "predec.264"),
                 "--frames", str(frames), "--qp", str(c["qp"]),
                 "--refs", str(c["refs"]), "--df-off"]
    if c["gop"]:
        glue_args += ["--gop", str(c["gop"])]
    if c["bframes"]:
        glue_args += ["--bframes", str(c["bframes"])]
    if c["rate_kbps"]:
        glue_args += ["--rate-kbps", str(c["rate_kbps"])]
    try:
        sh(glue_args[:-0] if False else glue_args, d / "predec.log",
           env=glue_env(EVA_GLUE_PREDEC=str(pre)))
    except subprocess.CalledProcessError:
        result.update(status="syntax replay failed")
        return result

    # Control: replaying the camera's syntax must reproduce its file byte for byte.
    result["control_identical"] = (
        (d / "predec.264").read_bytes() == cam_264.read_bytes()
    )

    painted = d / "painted.yuv"
    paint(pre, painted, frames, c["box"], c["paint_frame"], c["paint_frames"])

    ipcm = d / "raw.txt"
    pub = d / "published.264"
    pub_args = ["bash", "scripts/jm_glue_encode.sh", "--glue-dir", str(dumps),
                "--yuv", str(painted), "--out", str(pub),
                "--frames", str(frames), "--qp", str(c["qp"]),
                "--refs", str(c["refs"]), "--df-off"]
    if c["gop"]:
        pub_args += ["--gop", str(c["gop"])]
    if c["bframes"]:
        pub_args += ["--bframes", str(c["bframes"])]
    if c["rate_kbps"]:
        pub_args += ["--rate-kbps", str(c["rate_kbps"])]
    try:
        text = sh(pub_args, d / "publish.log",
                  env=glue_env(EVA_GLUE_EXACT=1, EVA_GLUE_IPCMLOG=str(ipcm)))
    except subprocess.CalledProcessError:
        result.update(status="publish failed")
        return result

    m = re.search(r"exact IPCM macroblocks (\d+)", text)
    result["raw_blocks"] = int(m.group(1)) if m else -1
    result["published_bytes"] = pub.stat().st_size
    result["size_ratio"] = round(result["published_bytes"] / result["camera_bytes"], 2)

    cam_yuv = d / "camera-decode.yuv"
    pub_yuv = d / "published-decode.yuv"
    decode(cam_264, cam_yuv)
    decode(pub, pub_yuv)
    cam = cam_yuv.read_bytes()
    dec = pub_yuv.read_bytes()
    if len(cam) != frames * FS or len(dec) != frames * FS:
        result.update(status=f"decode length mismatch {len(cam)} {len(dec)}")
        return result

    mx0, my0, mw, mh = c["box"]
    painted_set = {
        (f, mx, my)
        for f in range(c["paint_frame"], min(c["paint_frame"] + c["paint_frames"], frames))
        for my in range(my0, my0 + mh)
        for mx in range(mx0, mx0 + mw)
    }
    outside = differ = 0
    frames_differ = set()
    for f in range(frames):
        for my in range(ROWS):
            for mx in range(COLS):
                if (f, mx, my) in painted_set:
                    continue
                outside += 1
                if not mb_eq(dec, cam, f, mx, my):
                    differ += 1
                    frames_differ.add(f)

    # The published file must also reproduce the edited picture the editor asked for.
    result["decode_is_target"] = dec == painted.read_bytes()
    result["outside_edit_mbs"] = outside
    result["outside_edit_differ"] = differ
    result["frames_with_difference"] = ",".join(str(f) for f in sorted(frames_differ)) or "-"
    result["raw_per_frame"] = ";".join(
        f"{f}:{n}" for f, n in sorted(
            {fr: sum(1 for line in ipcm.read_text().splitlines()
                     if line.split() and int(line.split()[0]) == fr)
             for fr in {int(line.split()[0]) for line in ipcm.read_text().splitlines()
                        if line.split()}}.items()
        )
    )
    result["status"] = "EXACT outside the edit" if differ == 0 else f"{differ} blocks differ"
    return result


def main():
    WORK.mkdir(parents=True, exist_ok=True)
    wanted = sys.argv[1:]
    rows = []
    for cfg in CONFIGS:
        if wanted and cfg["name"] not in wanted:
            continue
        print(f"=== {cfg['name']} …", flush=True)
        try:
            r = run_config(cfg)
        except Exception as e:  # keep the sweep going; report the failure
            r = dict(name=cfg["name"], status=f"error: {e}")
        rows.append(r)
        print("   ", json.dumps({k: v for k, v in r.items() if k != "note"}), flush=True)

    fields = ["name", "frames", "refs", "gop", "bframes", "qp", "rate_kbps", "edit_mbs",
              "raw_blocks", "camera_bytes", "published_bytes", "size_ratio",
              "control_identical", "decode_is_target", "outside_edit_mbs",
              "outside_edit_differ", "frames_with_difference", "raw_per_frame",
              "status", "note"]
    OUT_CSV.parent.mkdir(parents=True, exist_ok=True)

    # Merge with anything already measured, so the sweep can run in batches.
    merged = {}
    if OUT_CSV.exists():
        with open(OUT_CSV, newline="") as fh:
            for old in csv.DictReader(fh):
                merged[old["name"]] = old
    for r in rows:
        merged[r["name"]] = r
    order = [c["name"] for c in CONFIGS]
    with open(OUT_CSV, "w", newline="") as fh:
        wr = csv.DictWriter(fh, fieldnames=fields, extrasaction="ignore")
        wr.writeheader()
        for name in order:
            if name in merged:
                wr.writerow(merged[name])
    print("wrote", OUT_CSV)
    return 0


if __name__ == "__main__":
    sys.exit(main())
