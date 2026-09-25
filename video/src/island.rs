//! Encode-skip island: which macroblocks must enter the circuit (`I`) vs copy (`J`).
//!
//! Geometric `I` is the redact box plus a Chebyshev halo of neighboring MBs on the
//! same frames. That halo is the intra-prediction / deblock ring. Inter (P-frame)
//! dependence is *not* computed here — measure it with [`crate::neighbor_impact`].

use crate::macroblock_yuv::{macroblock_xy, macroblocks_per_frame};

/// Axis-aligned pixel rectangle in one frame (`x,y` origin, size `w×h`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl PixelRect {
    pub fn x2(&self) -> usize {
        self.x.saturating_add(self.w)
    }

    pub fn y2(&self) -> usize {
        self.y.saturating_add(self.h)
    }

    /// True when origin and size are multiples of 16 (whole macroblock grid).
    pub fn is_macroblock_aligned(&self) -> bool {
        self.w > 0
            && self.h > 0
            && self.x % 16 == 0
            && self.y % 16 == 0
            && self.w % 16 == 0
            && self.h % 16 == 0
    }

    /// Expand to the smallest axis-aligned MB grid cover (privacy-safe redact snap).
    pub fn snap_outward_to_macroblocks(self) -> PixelRect {
        if self.w == 0 || self.h == 0 {
            return self;
        }
        let x = (self.x / 16) * 16;
        let y = (self.y / 16) * 16;
        let x2 = ((self.x2() + 15) / 16) * 16;
        let y2 = ((self.y2() + 15) / 16) * 16;
        PixelRect {
            x,
            y,
            w: x2.saturating_sub(x),
            h: y2.saturating_sub(y),
        }
    }

    /// Whether this rectangle overlaps the 16×16 luma block at (`mb_x`, `mb_y`).
    pub fn overlaps_macroblock(&self, mb_x: usize, mb_y: usize) -> bool {
        if self.w == 0 || self.h == 0 {
            return false;
        }
        let bx1 = mb_x * 16;
        let by1 = mb_y * 16;
        let bx2 = bx1 + 16;
        let by2 = by1 + 16;
        bx1 < self.x2() && bx2 > self.x && by1 < self.y2() && by2 > self.y
    }
}

/// Clip geometry used to classify every macroblock in a clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IslandSpec {
    pub width: usize,
    pub height: usize,
    pub num_frames: usize,
    /// Inclusive start, exclusive end — same convention as toolkit `--redact-frame-*`.
    pub frame_start: usize,
    pub frame_end: usize,
    pub rect: PixelRect,
    /// Chebyshev radius in macroblock units. `1` is the 8-neighbor ring (plus
    /// corners). `0` is gadget-only (edit-only skip; no encode halo).
    pub halo_mbs: usize,
}

impl IslandSpec {
    pub fn validate(&self) -> Result<(), String> {
        let _ = macroblocks_per_frame(self.width, self.height)?;
        if self.num_frames == 0 {
            return Err("num_frames must be positive".into());
        }
        if self.frame_start > self.frame_end {
            return Err("frame_start must be <= frame_end".into());
        }
        Ok(())
    }

    pub fn cols(&self) -> usize {
        self.width / 16
    }

    pub fn rows(&self) -> usize {
        self.height / 16
    }

    pub fn mbs_per_frame(&self) -> Result<usize, String> {
        macroblocks_per_frame(self.width, self.height)
    }

    pub fn total_mbs(&self) -> Result<usize, String> {
        Ok(self.mbs_per_frame()? * self.num_frames)
    }

    pub fn frame_end_clamped(&self) -> usize {
        self.frame_end.min(self.num_frames)
    }

    pub fn frame_start_clamped(&self) -> usize {
        self.frame_start.min(self.frame_end_clamped())
    }

    /// Decode a global Eva MB index into `(frame, mb_x, mb_y)`.
    pub fn locate(&self, global: usize) -> Result<(usize, usize, usize), String> {
        let mbs = self.mbs_per_frame()?;
        let frame = global / mbs;
        if frame >= self.num_frames {
            return Err(format!(
                "macroblock index {global} is past clip ({mbs} MBs/frame × {} frames)",
                self.num_frames
            ));
        }
        let (mb_x, mb_y) = macroblock_xy(self.width, global % mbs);
        Ok((frame, mb_x, mb_y))
    }
}

/// Role of one macroblock relative to a geometric island.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MbRole {
    /// Overlaps the redact rectangle on an edited frame. Pixels change; must encode in-circuit.
    Gadget,
    /// Neighbor of a gadget MB (Chebyshev ≤ `halo_mbs`) on the same frame. Pixels
    /// are identity; syntax may change under intra / deblock.
    Halo,
    /// Outside the halo, on a frame that contains the redact box.
    SkipInEditedFrames,
    /// Frames strictly before `frame_start`.
    SkipPre,
    /// Frames at or after `frame_end`. P-prediction from the island can leak here;
    /// this role is the *candidate set*, not a proven membership in `I`.
    SkipPost,
}

impl MbRole {
    /// Geometric island: gadget ∪ halo. These indices enter Nova.
    pub fn in_island(self) -> bool {
        matches!(self, MbRole::Gadget | MbRole::Halo)
    }
}

/// Counts for one [`IslandSpec`]. `island = gadget + halo`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IslandCounts {
    pub gadget: usize,
    pub halo: usize,
    pub skip_in_edited_frames: usize,
    pub skip_pre: usize,
    pub skip_post: usize,
    pub total: usize,
}

impl IslandCounts {
    pub fn island(&self) -> usize {
        self.gadget + self.halo
    }

    pub fn skip(&self) -> usize {
        self.skip_in_edited_frames + self.skip_pre + self.skip_post
    }

    /// Fraction of MBs that would enter the circuit. Paper-facing number.
    pub fn island_fraction(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.island() as f64 / self.total as f64
        }
    }
}

fn chebyshev(ax: usize, ay: usize, bx: usize, by: usize) -> usize {
    ax.abs_diff(bx).max(ay.abs_diff(by))
}

fn is_gadget_mb(spec: &IslandSpec, frame: usize, mb_x: usize, mb_y: usize) -> bool {
    let f0 = spec.frame_start_clamped();
    let f1 = spec.frame_end_clamped();
    frame >= f0 && frame < f1 && spec.rect.overlaps_macroblock(mb_x, mb_y)
}

/// True if (`mb_x`, `mb_y`) is within `halo_mbs` of some gadget MB in `frame`.
fn is_halo_mb(spec: &IslandSpec, frame: usize, mb_x: usize, mb_y: usize) -> bool {
    if spec.halo_mbs == 0 {
        return false;
    }
    if is_gadget_mb(spec, frame, mb_x, mb_y) {
        return false;
    }
    let cols = spec.cols();
    let rows = spec.rows();
    let r = spec.halo_mbs;
    let x0 = mb_x.saturating_sub(r);
    let x1 = (mb_x + r).min(cols.saturating_sub(1));
    let y0 = mb_y.saturating_sub(r);
    let y1 = (mb_y + r).min(rows.saturating_sub(1));
    for gy in y0..=y1 {
        for gx in x0..=x1 {
            if chebyshev(mb_x, mb_y, gx, gy) == 0 || chebyshev(mb_x, mb_y, gx, gy) > r {
                continue;
            }
            if is_gadget_mb(spec, frame, gx, gy) {
                return true;
            }
        }
    }
    false
}

/// Classify one global macroblock index.
pub fn mb_role(spec: &IslandSpec, global: usize) -> Result<MbRole, String> {
    spec.validate()?;
    let (frame, mb_x, mb_y) = spec.locate(global)?;
    let f0 = spec.frame_start_clamped();
    let f1 = spec.frame_end_clamped();
    if frame < f0 {
        return Ok(MbRole::SkipPre);
    }
    if frame >= f1 {
        return Ok(MbRole::SkipPost);
    }
    if is_gadget_mb(spec, frame, mb_x, mb_y) {
        return Ok(MbRole::Gadget);
    }
    if is_halo_mb(spec, frame, mb_x, mb_y) {
        return Ok(MbRole::Halo);
    }
    Ok(MbRole::SkipInEditedFrames)
}

/// Global indices in `I` (gadget ∪ halo), scan order: frame-major, then row-major MB.
pub fn island_indices(spec: &IslandSpec) -> Result<Vec<usize>, String> {
    spec.validate()?;
    let total = spec.total_mbs()?;
    let mut out = Vec::new();
    for g in 0..total {
        if mb_role(spec, g)?.in_island() {
            out.push(g);
        }
    }
    Ok(out)
}

pub fn island_counts(spec: &IslandSpec) -> Result<IslandCounts, String> {
    spec.validate()?;
    let total = spec.total_mbs()?;
    let mut c = IslandCounts {
        total,
        ..Default::default()
    };
    for g in 0..total {
        match mb_role(spec, g)? {
            MbRole::Gadget => c.gadget += 1,
            MbRole::Halo => c.halo += 1,
            MbRole::SkipInEditedFrames => c.skip_in_edited_frames += 1,
            MbRole::SkipPre => c.skip_pre += 1,
            MbRole::SkipPost => c.skip_post += 1,
        }
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_3x2() -> IslandSpec {
        // 48×32 → 3 cols × 2 rows = 6 MBs/frame. Two frames.
        IslandSpec {
            width: 48,
            height: 32,
            num_frames: 2,
            frame_start: 0,
            frame_end: 1,
            rect: PixelRect {
                x: 16,
                y: 0,
                w: 16,
                h: 16,
            },
            halo_mbs: 1,
        }
    }

    #[test]
    fn gadget_is_the_overlapping_mb() {
        let spec = spec_3x2();
        // Frame 0: MB (1,0) is the box; neighbors (0,0),(2,0),(1,1) plus corners (0,1),(2,1).
        assert_eq!(mb_role(&spec, 1).unwrap(), MbRole::Gadget);
        assert_eq!(mb_role(&spec, 0).unwrap(), MbRole::Halo);
        assert_eq!(mb_role(&spec, 2).unwrap(), MbRole::Halo);
        assert_eq!(mb_role(&spec, 4).unwrap(), MbRole::Halo); // (1,1)
        assert_eq!(mb_role(&spec, 3).unwrap(), MbRole::Halo); // (0,1) corner
        assert_eq!(mb_role(&spec, 5).unwrap(), MbRole::Halo); // (2,1)
                                                              // Frame 1 is after the window.
        assert_eq!(mb_role(&spec, 6).unwrap(), MbRole::SkipPost);
        let c = island_counts(&spec).unwrap();
        assert_eq!(c.gadget, 1);
        assert_eq!(c.halo, 5);
        assert_eq!(c.island(), 6);
        assert_eq!(c.skip_post, 6);
        assert_eq!(c.total, 12);
    }

    #[test]
    fn halo_zero_is_gadget_only() {
        let mut spec = spec_3x2();
        spec.halo_mbs = 0;
        let c = island_counts(&spec).unwrap();
        assert_eq!(c.gadget, 1);
        assert_eq!(c.halo, 0);
        assert_eq!(mb_role(&spec, 0).unwrap(), MbRole::SkipInEditedFrames);
    }

    #[test]
    fn partial_edge_mb_counts_as_gadget() {
        let spec = IslandSpec {
            width: 32,
            height: 32,
            num_frames: 1,
            frame_start: 0,
            frame_end: 1,
            rect: PixelRect {
                x: 8,
                y: 8,
                w: 16,
                h: 16,
            },
            halo_mbs: 0,
        };
        // 16×16 box at (8,8) straddles the 16-pixel grid → all four MBs.
        let c = island_counts(&spec).unwrap();
        assert_eq!(c.gadget, 4);
        assert_eq!(c.halo, 0);
    }

    #[test]
    fn cif_box_plus_ring_counts() {
        // Foreman CIF 352×288, box 96,80,176,144 on frames [2,4), halo 1.
        let spec = IslandSpec {
            width: 352,
            height: 288,
            num_frames: 10,
            frame_start: 2,
            frame_end: 4,
            rect: PixelRect {
                x: 96,
                y: 80,
                w: 176,
                h: 144,
            },
            halo_mbs: 1,
        };
        let c = island_counts(&spec).unwrap();
        // MBs: x 6..16 inclusive (11), y 5..13 inclusive (9) → 99 gadget / frame × 2 frames.
        assert_eq!(c.gadget, 99 * 2);
        // Outer ring: x 5..17 (13), y 4..14 (11) → 143; halo = 143-99 = 44 / frame.
        assert_eq!(c.halo, 44 * 2);
        assert_eq!(c.skip_pre, 396 * 2);
        assert_eq!(c.skip_post, 396 * 6);
        assert_eq!(c.total, 3960);
        assert_eq!(island_indices(&spec).unwrap().len(), c.island());
    }

    #[test]
    fn hd_center_snap_is_mb_aligned() {
        let raw = PixelRect {
            x: 320,
            y: 180,
            w: 640,
            h: 360,
        };
        let snapped = raw.snap_outward_to_macroblocks();
        assert!(snapped.is_macroblock_aligned());
        assert_eq!(snapped.x, 320);
        assert_eq!(snapped.y, 176);
        assert_eq!(snapped.w, 640);
        assert_eq!(snapped.h, 368);
        assert!(!raw.is_macroblock_aligned());
    }

    #[test]
    fn pre_window_is_skip_pre() {
        let spec = IslandSpec {
            width: 16,
            height: 16,
            num_frames: 3,
            frame_start: 1,
            frame_end: 2,
            rect: PixelRect {
                x: 0,
                y: 0,
                w: 16,
                h: 16,
            },
            halo_mbs: 1,
        };
        assert_eq!(mb_role(&spec, 0).unwrap(), MbRole::SkipPre);
        assert_eq!(mb_role(&spec, 1).unwrap(), MbRole::Gadget);
        assert_eq!(mb_role(&spec, 2).unwrap(), MbRole::SkipPost);
    }
}
