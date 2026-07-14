use ark_ff::PrimeField;
use ark_r1cs_std::{
    alloc::AllocVar,
    boolean::Boolean,
    fields::{fp::FpVar, FieldVar},
    R1CSVar,
};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};
use ndarray::Array2;
use std::fmt::Debug;
use std::{borrow::Borrow, cmp::min};

use crate::{
    encode::{constraints::MatrixVar, Matrix},
    var::I64Var,
    MB_BITS,
};

pub trait EditConfig: Default {
    fn should_keep_native(&self) -> bool {
        true
    }

    fn compactify<F: PrimeField>(&self) -> Vec<F>;
}

impl EditConfig for () {
    fn compactify<F: PrimeField>(&self) -> Vec<F> {
        vec![]
    }
}
impl EditConfig for BrightnessCfg {
    fn compactify<F: PrimeField>(&self) -> Vec<F> {
        vec![F::from(self.0)]
    }
}
impl EditConfig for MaskCfg {
    fn compactify<F: PrimeField>(&self) -> Vec<F> {
        let s = F::from(1 << (MB_BITS + 1));
        vec![]
            .into_iter()
            .chain(&self.0)
            .chain(&self.1)
            .chain(&self.2)
            .collect::<Vec<_>>()
            .chunks(F::MODULUS_BIT_SIZE as usize / (MB_BITS + 1))
            .map(|c| {
                let mut r = F::zero();
                for v in c {
                    r = r * s + F::from(v.0 as u64 * 2 + v.1 as u64);
                }
                r
            })
            .collect::<Vec<_>>()
    }
}
impl EditConfig for RemovingCfg {
    fn should_keep_native(&self) -> bool {
        self.0
    }

    fn compactify<F: PrimeField>(&self) -> Vec<F> {
        vec![F::from(self.0)]
    }
}
impl EditConfig for RedactRectCfg {
    fn compactify<F: PrimeField>(&self) -> Vec<F> {
        let mut out = vec![
            F::from(self.fill_y as u64),
            F::from(self.in_frame_range as u64),
            F::from(self.full_y as u64),
            F::from(self.full_u as u64),
            F::from(self.full_v as u64),
        ];
        if !self.full_y {
            out.extend(pack_bool_grid::<F>(&self.y_replace));
        }
        if !self.full_u {
            out.extend(pack_bool_grid::<F>(&self.u_replace));
        }
        if !self.full_v {
            out.extend(pack_bool_grid::<F>(&self.v_replace));
        }
        out
    }
}

pub trait EditConfigVar<F: PrimeField> {
    fn should_keep_circuit(&self) -> Result<Boolean<F>, SynthesisError> {
        Ok(Boolean::TRUE)
    }

    fn compactify(&self) -> Vec<FpVar<F>>;
}

impl<F: PrimeField> EditConfigVar<F> for () {
    fn compactify(&self) -> Vec<FpVar<F>> {
        vec![]
    }
}
impl<F: PrimeField> EditConfigVar<F> for BrightnessCfgVar<F> {
    fn compactify(&self) -> Vec<FpVar<F>> {
        vec![self.0.to_fpvar()]
    }
}
impl<F: PrimeField> EditConfigVar<F> for MaskCfgVar<F> {
    fn compactify(&self) -> Vec<FpVar<F>> {
        let s = F::from(1 << (MB_BITS + 1));
        vec![]
            .into_iter()
            .chain(&self.0)
            .chain(&self.1)
            .chain(&self.2)
            .collect::<Vec<_>>()
            .chunks(F::MODULUS_BIT_SIZE as usize / (MB_BITS + 1))
            .map(|c| {
                let mut r = FpVar::zero();
                for v in c {
                    r = r * s + &v.0.to_fpvar() + &v.0.to_fpvar() + FpVar::from(v.1.clone());
                }
                r
            })
            .collect::<Vec<_>>()
    }
}
impl<F: PrimeField> EditConfigVar<F> for RemovingCfgVar<F> {
    fn should_keep_circuit(&self) -> Result<Boolean<F>, SynthesisError> {
        Ok(self.0.clone())
    }

    fn compactify(&self) -> Vec<FpVar<F>> {
        vec![FpVar::from(self.0.clone())]
    }
}
impl<F: PrimeField> EditConfigVar<F> for RedactRectCfgVar<F> {
    fn compactify(&self) -> Vec<FpVar<F>> {
        let mut out = vec![
            FpVar::constant(F::from(self.fill_y as u64)),
            FpVar::from(self.in_frame_range.clone()),
            FpVar::from(self.full_y.clone()),
            FpVar::from(self.full_u.clone()),
            FpVar::from(self.full_v.clone()),
        ];
        match &self.full_y {
            Boolean::Constant(true) => {}
            _ => out.extend(pack_bool_grid_var(&self.y_replace)),
        }
        match &self.full_u {
            Boolean::Constant(true) => {}
            _ => out.extend(pack_bool_grid_var(&self.u_replace)),
        }
        match &self.full_v {
            Boolean::Constant(true) => {}
            _ => out.extend(pack_bool_grid_var(&self.v_replace)),
        }
        out
    }
}

pub trait EditGadget: Clone + Debug + Default {
    type Cfg: Sync + Debug + Clone + EditConfig;
    type CfgVar<F: PrimeField>: AllocVar<Self::Cfg, F> + EditConfigVar<F>;

    fn edit_native(
        y: &Matrix<u8, 16, 16>,
        u: &Matrix<u8, 8, 8>,
        v: &Matrix<u8, 8, 8>,
        cfg: &Self::Cfg,
    ) -> (Matrix<u8, 16, 16>, Matrix<u8, 8, 8>, Matrix<u8, 8, 8>);

    fn edit_circuit<F: PrimeField>(
        y: &MatrixVar<I64Var<F>, 16, 16>,
        u: &MatrixVar<I64Var<F>, 8, 8>,
        v: &MatrixVar<I64Var<F>, 8, 8>,
        cfg: &Self::CfgVar<F>,
    ) -> Result<
        (
            MatrixVar<I64Var<F>, 16, 16>,
            MatrixVar<I64Var<F>, 8, 8>,
            MatrixVar<I64Var<F>, 8, 8>,
        ),
        SynthesisError,
    >;

    fn result_has_constant_encoding() -> (bool, bool, bool);
}

#[derive(Clone, Debug, Default)]
pub struct NoOp {}

impl EditGadget for NoOp {
    type Cfg = ();
    type CfgVar<F: PrimeField> = ();

    fn edit_native(
        y: &Matrix<u8, 16, 16>,
        u: &Matrix<u8, 8, 8>,
        v: &Matrix<u8, 8, 8>,
        _cfg: &Self::Cfg,
    ) -> (Matrix<u8, 16, 16>, Matrix<u8, 8, 8>, Matrix<u8, 8, 8>) {
        (y.clone(), u.clone(), v.clone())
    }

    fn edit_circuit<F: PrimeField>(
        y: &MatrixVar<I64Var<F>, 16, 16>,
        u: &MatrixVar<I64Var<F>, 8, 8>,
        v: &MatrixVar<I64Var<F>, 8, 8>,
        _cfg: &Self::CfgVar<F>,
    ) -> Result<
        (
            MatrixVar<I64Var<F>, 16, 16>,
            MatrixVar<I64Var<F>, 8, 8>,
            MatrixVar<I64Var<F>, 8, 8>,
        ),
        SynthesisError,
    > {
        Ok((y.clone(), u.clone(), v.clone()))
    }

    fn result_has_constant_encoding() -> (bool, bool, bool) {
        (false, false, false)
    }
}

#[derive(Clone, Debug, Default)]
pub struct InvertColor {}

impl EditGadget for InvertColor {
    type Cfg = ();
    type CfgVar<F: PrimeField> = ();

    fn edit_native(
        y: &Matrix<u8, 16, 16>,
        u: &Matrix<u8, 8, 8>,
        v: &Matrix<u8, 8, 8>,
        _cfg: &Self::Cfg,
    ) -> (Matrix<u8, 16, 16>, Matrix<u8, 8, 8>, Matrix<u8, 8, 8>) {
        (y.invert_color(), u.invert_color(), v.invert_color())
    }

    fn edit_circuit<F: PrimeField>(
        y: &MatrixVar<I64Var<F>, 16, 16>,
        u: &MatrixVar<I64Var<F>, 8, 8>,
        v: &MatrixVar<I64Var<F>, 8, 8>,
        _cfg: &Self::CfgVar<F>,
    ) -> Result<
        (
            MatrixVar<I64Var<F>, 16, 16>,
            MatrixVar<I64Var<F>, 8, 8>,
            MatrixVar<I64Var<F>, 8, 8>,
        ),
        SynthesisError,
    > {
        Ok((y.invert_color()?, u.invert_color()?, v.invert_color()?))
    }

    fn result_has_constant_encoding() -> (bool, bool, bool) {
        (false, false, false)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Grayscale {}

impl EditGadget for Grayscale {
    type Cfg = ();
    type CfgVar<F: PrimeField> = ();

    fn edit_native(
        y: &Matrix<u8, 16, 16>,
        _u: &Matrix<u8, 8, 8>,
        _v: &Matrix<u8, 8, 8>,
        _cfg: &Self::Cfg,
    ) -> (Matrix<u8, 16, 16>, Matrix<u8, 8, 8>, Matrix<u8, 8, 8>) {
        (
            y.clone(),
            Matrix::from_vec(vec![128; 64]),
            Matrix::from_vec(vec![128; 64]),
        )
    }

    fn edit_circuit<F: PrimeField>(
        y: &MatrixVar<I64Var<F>, 16, 16>,
        _u: &MatrixVar<I64Var<F>, 8, 8>,
        _v: &MatrixVar<I64Var<F>, 8, 8>,
        _cfg: &Self::CfgVar<F>,
    ) -> Result<
        (
            MatrixVar<I64Var<F>, 16, 16>,
            MatrixVar<I64Var<F>, 8, 8>,
            MatrixVar<I64Var<F>, 8, 8>,
        ),
        SynthesisError,
    > {
        Ok((
            y.clone(),
            MatrixVar::new_constant(ConstraintSystemRef::None, Matrix::from_vec(vec![128u8; 64]))?,
            MatrixVar::new_constant(ConstraintSystemRef::None, Matrix::from_vec(vec![128u8; 64]))?,
        ))
    }

    fn result_has_constant_encoding() -> (bool, bool, bool) {
        (false, true, true)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Brightness {}

#[derive(Clone, Debug, Default)]
pub struct BrightnessCfg(pub u16);

pub struct BrightnessCfgVar<F: PrimeField>(pub I64Var<F>);

impl<F: PrimeField> AllocVar<BrightnessCfg, F> for BrightnessCfgVar<F> {
    fn new_variable<T: Borrow<BrightnessCfg>>(
        cs: impl Into<ark_relations::r1cs::Namespace<F>>,
        f: impl FnOnce() -> Result<T, SynthesisError>,
        mode: ark_r1cs_std::prelude::AllocationMode,
    ) -> Result<Self, SynthesisError> {
        let cs = cs.into().cs();
        let cfg = f()?;
        let cfg = cfg.borrow();
        Ok(Self(I64Var::new_variable(cs, || Ok(cfg.0), mode)?))
    }
}

impl EditGadget for Brightness {
    type Cfg = BrightnessCfg;
    type CfgVar<F: PrimeField> = BrightnessCfgVar<F>;

    fn edit_native(
        y: &Matrix<u8, 16, 16>,
        u: &Matrix<u8, 8, 8>,
        v: &Matrix<u8, 8, 8>,
        cfg: &Self::Cfg,
    ) -> (Matrix<u8, 16, 16>, Matrix<u8, 8, 8>, Matrix<u8, 8, 8>) {
        (
            y.iter()
                .map(|&v| min(255, (v as u64 * cfg.0 as u64) >> 8) as u8)
                .collect(),
            u.clone(),
            v.clone(),
        )
    }

    fn edit_circuit<F: PrimeField>(
        y: &MatrixVar<I64Var<F>, 16, 16>,
        u: &MatrixVar<I64Var<F>, 8, 8>,
        v: &MatrixVar<I64Var<F>, 8, 8>,
        cfg: &Self::CfgVar<F>,
    ) -> Result<
        (
            MatrixVar<I64Var<F>, 16, 16>,
            MatrixVar<I64Var<F>, 8, 8>,
            MatrixVar<I64Var<F>, 8, 8>,
        ),
        SynthesisError,
    > {
        Ok((
            y.iter()
                .map(|v| {
                    let (p, q, r) = {
                        let cs = v.cs().or(cfg.0.cs());
                        let v = v.value()? * cfg.0.value()?;
                        let p = v >> 16;
                        let q = (v >> 8) & ((1 << 8) - 1);
                        let r = v & ((1 << 8) - 1);
                        (
                            I64Var::new_committed(cs.clone(), || Ok(p))?,
                            I64Var::new_committed(cs.clone(), || Ok(q))?,
                            I64Var::new_committed(cs, || Ok(r))?,
                        )
                    };
                    v.mul_equals(&cfg.0, &(&p * (1 << 16) + &q * (1 << 8) + r))?;

                    p.is_zero()?.select(&q, &I64Var::constant(255))
                })
                .collect::<Result<_, _>>()?,
            u.clone(),
            v.clone(),
        ))
    }

    fn result_has_constant_encoding() -> (bool, bool, bool) {
        (false, false, false)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Masking {}

impl Masking {
    pub fn mask_native<const M: usize, const N: usize>(
        data: &Matrix<u8, M, N>,
        mask: &Array2<(u8, bool)>,
    ) -> Matrix<u8, M, N> {
        let mut masked = data.clone();
        for i in 0..M {
            for j in 0..N {
                let &(v, b) = &mask[[i, j]];
                if b {
                    masked[(i, j)] = v;
                }
            }
        }
        masked
    }

    pub fn mask_circuit<F: PrimeField, const M: usize, const N: usize>(
        data: &MatrixVar<I64Var<F>, M, N>,
        mask: &Array2<(I64Var<F>, Boolean<F>)>,
    ) -> Result<MatrixVar<I64Var<F>, M, N>, SynthesisError> {
        let mut masked = data.clone();
        for i in 0..M {
            for j in 0..N {
                let (v, b) = &mask[[i, j]];
                masked[(i, j)] = b.select(&masked[(i, j)], &v)?;
            }
        }
        Ok(masked)
    }
}

impl EditGadget for Masking {
    type Cfg = MaskCfg;
    type CfgVar<F: PrimeField> = MaskCfgVar<F>;

    fn edit_native(
        y: &Matrix<u8, 16, 16>,
        u: &Matrix<u8, 8, 8>,
        v: &Matrix<u8, 8, 8>,
        cfg: &Self::Cfg,
    ) -> (Matrix<u8, 16, 16>, Matrix<u8, 8, 8>, Matrix<u8, 8, 8>) {
        (
            Self::mask_native(y, &cfg.0),
            Self::mask_native(u, &cfg.1),
            Self::mask_native(v, &cfg.2),
        )
    }

    fn edit_circuit<F: PrimeField>(
        y: &MatrixVar<I64Var<F>, 16, 16>,
        u: &MatrixVar<I64Var<F>, 8, 8>,
        v: &MatrixVar<I64Var<F>, 8, 8>,
        cfg: &Self::CfgVar<F>,
    ) -> Result<
        (
            MatrixVar<I64Var<F>, 16, 16>,
            MatrixVar<I64Var<F>, 8, 8>,
            MatrixVar<I64Var<F>, 8, 8>,
        ),
        SynthesisError,
    > {
        Ok((
            Self::mask_circuit(y, &cfg.0)?,
            Self::mask_circuit(u, &cfg.1)?,
            Self::mask_circuit(v, &cfg.2)?,
        ))
    }

    fn result_has_constant_encoding() -> (bool, bool, bool) {
        (false, false, false)
    }
}

#[derive(Clone, Debug)]
pub struct MaskCfg(
    pub Array2<(u8, bool)>,
    pub Array2<(u8, bool)>,
    pub Array2<(u8, bool)>,
);

impl Default for MaskCfg {
    fn default() -> Self {
        Self(
            Array2::from_elem((16, 16), (0, false)),
            Array2::from_elem((8, 8), (0, false)),
            Array2::from_elem((8, 8), (0, false)),
        )
    }
}

#[derive(Clone)]
pub struct MaskCfgVar<F: PrimeField>(
    pub Array2<(I64Var<F>, Boolean<F>)>,
    pub Array2<(I64Var<F>, Boolean<F>)>,
    pub Array2<(I64Var<F>, Boolean<F>)>,
);

impl<F: PrimeField> AllocVar<MaskCfg, F> for MaskCfgVar<F> {
    fn new_variable<T: Borrow<MaskCfg>>(
        cs: impl Into<ark_relations::r1cs::Namespace<F>>,
        f: impl FnOnce() -> Result<T, SynthesisError>,
        mode: ark_r1cs_std::prelude::AllocationMode,
    ) -> Result<Self, SynthesisError> {
        let cs = cs.into().cs();
        let mask = f()?;
        let mask = mask.borrow();
        let mut y_var = Array2::from_elem((16, 16), (I64Var::<F>::zero(), Boolean::FALSE));
        let mut u_var = Array2::from_elem((8, 8), (I64Var::<F>::zero(), Boolean::FALSE));
        let mut v_var = Array2::from_elem((8, 8), (I64Var::<F>::zero(), Boolean::FALSE));
        for i in 0..16 {
            for j in 0..16 {
                y_var[[i, j]] = (
                    I64Var::<F>::new_committed(cs.clone(), || Ok(mask.0[[i, j]].0))?,
                    Boolean::new_variable(cs.clone(), || Ok(mask.0[[i, j]].1), mode)?,
                );
            }
        }
        for i in 0..8 {
            for j in 0..8 {
                u_var[[i, j]] = (
                    I64Var::<F>::new_committed(cs.clone(), || Ok(mask.1[[i, j]].0))?,
                    Boolean::new_variable(cs.clone(), || Ok(mask.1[[i, j]].1), mode)?,
                );
                v_var[[i, j]] = (
                    I64Var::<F>::new_committed(cs.clone(), || Ok(mask.2[[i, j]].0))?,
                    Boolean::new_variable(cs.clone(), || Ok(mask.2[[i, j]].1), mode)?,
                );
            }
        }
        Ok(Self(y_var, u_var, v_var))
    }
}

impl<F: PrimeField, const M: usize, const N: usize> MatrixVar<I64Var<F>, M, N> {
    pub fn invert_color(&self) -> Result<MatrixVar<I64Var<F>, M, N>, SynthesisError> {
        let mut inverted = self.clone();
        for i in 0..M {
            for j in 0..N {
                inverted[(i, j)] = I64Var::constant(255) - &inverted[(i, j)];
            }
        }
        Ok(inverted)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Removing {}

impl Removing {
    pub fn remove_native<const M: usize, const N: usize>(
        data: &Matrix<u8, M, N>,
        cfg: &bool,
    ) -> Matrix<u8, M, N> {
        let mut cropped = data.clone();
        for i in 0..M {
            for j in 0..N {
                if !cfg {
                    cropped[(i, j)] = 0;
                }
            }
        }
        cropped
    }

    pub fn remove_circuit<F: PrimeField, const M: usize, const N: usize>(
        data: &MatrixVar<I64Var<F>, M, N>,
        cfg: &Boolean<F>,
    ) -> Result<MatrixVar<I64Var<F>, M, N>, SynthesisError> {
        let mut cropped = data.clone();
        for i in 0..M {
            for j in 0..N {
                cropped[(i, j)] = cfg.select(&cropped[(i, j)], &I64Var::zero())?;
            }
        }
        Ok(cropped)
    }
}

#[derive(Clone, Debug, Default)]
pub struct RemovingCfg(pub bool);

#[derive(Clone)]
pub struct RemovingCfgVar<F: PrimeField>(Boolean<F>);

impl<F: PrimeField> AllocVar<RemovingCfg, F> for RemovingCfgVar<F> {
    fn new_variable<T: Borrow<RemovingCfg>>(
        cs: impl Into<ark_relations::r1cs::Namespace<F>>,
        f: impl FnOnce() -> Result<T, SynthesisError>,
        mode: ark_r1cs_std::prelude::AllocationMode,
    ) -> Result<Self, SynthesisError> {
        let cs = cs.into().cs();
        let cfg = f()?;
        let cfg = cfg.borrow();
        Ok(Self(Boolean::new_variable(cs.clone(), || Ok(cfg.0), mode)?))
    }
}

impl EditGadget for Removing {
    type Cfg = RemovingCfg;
    type CfgVar<F: PrimeField> = RemovingCfgVar<F>;

    fn edit_native(
        y: &Matrix<u8, 16, 16>,
        u: &Matrix<u8, 8, 8>,
        v: &Matrix<u8, 8, 8>,
        cfg: &Self::Cfg,
    ) -> (Matrix<u8, 16, 16>, Matrix<u8, 8, 8>, Matrix<u8, 8, 8>) {
        (
            Self::remove_native(y, &cfg.0),
            Self::remove_native(u, &cfg.0),
            Self::remove_native(v, &cfg.0),
        )
    }

    fn edit_circuit<F: PrimeField>(
        y: &MatrixVar<I64Var<F>, 16, 16>,
        u: &MatrixVar<I64Var<F>, 8, 8>,
        v: &MatrixVar<I64Var<F>, 8, 8>,
        cfg: &Self::CfgVar<F>,
    ) -> Result<
        (
            MatrixVar<I64Var<F>, 16, 16>,
            MatrixVar<I64Var<F>, 8, 8>,
            MatrixVar<I64Var<F>, 8, 8>,
        ),
        SynthesisError,
    > {
        Ok((
            Self::remove_circuit(y, &cfg.0)?,
            Self::remove_circuit(u, &cfg.0)?,
            Self::remove_circuit(v, &cfg.0)?,
        ))
    }

    fn result_has_constant_encoding() -> (bool, bool, bool) {
        (false, false, false)
    }
}

/// Pack a row-major bool grid into field elements (one bit per bool).
fn pack_bool_grid<F: PrimeField>(grid: &Array2<bool>) -> Vec<F> {
    let bits = grid.iter().copied().collect::<Vec<_>>();
    pack_bits::<F>(&bits)
}

fn pack_bits<F: PrimeField>(bits: &[bool]) -> Vec<F> {
    // Field-native doubling rather than `1u64 << i`: chunks can be up to
    // `MODULUS_BIT_SIZE` (~254 for BN254 Fr) bits wide, which overflows a
    // native u64/u128 shift long before the field itself would.
    let chunk_bits = F::MODULUS_BIT_SIZE as usize;
    bits.chunks(chunk_bits)
        .map(|chunk| {
            let mut r = F::zero();
            let mut power = F::one();
            for &b in chunk {
                if b {
                    r += power;
                }
                power.double_in_place();
            }
            r
        })
        .collect()
}

fn pack_bool_grid_var<F: PrimeField>(grid: &Array2<Boolean<F>>) -> Vec<FpVar<F>> {
    let bits: Vec<_> = grid.iter().cloned().collect();
    let chunk_bits = F::MODULUS_BIT_SIZE as usize;
    bits.chunks(chunk_bits)
        .map(|chunk| {
            let mut r = FpVar::zero();
            let mut power = F::one();
            for b in chunk {
                r += FpVar::from(b.clone()) * FpVar::constant(power);
                power.double_in_place();
            }
            r
        })
        .collect()
}

/// Rectangle redaction: replace pixels inside a fixed axis-aligned box (per macroblock).
///
/// Config is built from the global rectangle plus macroblock origin. When the whole
/// 16×16 / 8×8 plane lies inside the box, a single boolean replaces the plane
/// (like [`Removing`]); otherwise only edge pixels carry per-pixel replace flags.
#[derive(Clone, Debug)]
pub struct RedactRectCfg {
    pub in_frame_range: bool,
    pub fill_y: u8,
    pub full_y: bool,
    pub full_u: bool,
    pub full_v: bool,
    pub y_replace: Array2<bool>,
    pub u_replace: Array2<bool>,
    pub v_replace: Array2<bool>,
}

impl Default for RedactRectCfg {
    fn default() -> Self {
        Self {
            in_frame_range: false,
            fill_y: 0,
            full_y: false,
            full_u: false,
            full_v: false,
            y_replace: Array2::from_elem((16, 16), false),
            u_replace: Array2::from_elem((8, 8), false),
            v_replace: Array2::from_elem((8, 8), false),
        }
    }
}

impl RedactRectCfg {
    /// Build config for one macroblock from a clamped pixel rectangle and frame gate.
    pub fn from_rectangle(
        origin_x: usize,
        origin_y: usize,
        in_frame_range: bool,
        x1: usize,
        y1: usize,
        x2: usize,
        y2: usize,
        fill_y: u8,
    ) -> Self {
        let pixel_in_box =
            |px: usize, py: usize| in_frame_range && px >= x1 && px < x2 && py >= y1 && py < y2;

        let y_replace = Array2::from_shape_fn((16, 16), |(m, n)| {
            pixel_in_box(origin_x + m, origin_y + n)
        });
        let u_replace = Array2::from_shape_fn((8, 8), |(m, n)| {
            pixel_in_box(origin_x + m * 2, origin_y + n * 2)
        });
        let v_replace = u_replace.clone();

        let full_y = y_replace.iter().all(|&b| b);
        let full_u = u_replace.iter().all(|&b| b);
        let full_v = v_replace.iter().all(|&b| b);

        Self {
            in_frame_range,
            fill_y,
            full_y,
            full_u,
            full_v,
            y_replace,
            u_replace,
            v_replace,
        }
    }
}

#[derive(Clone)]
pub struct RedactRectCfgVar<F: PrimeField> {
    pub in_frame_range: Boolean<F>,
    pub fill_y: u8,
    pub full_y: Boolean<F>,
    pub full_u: Boolean<F>,
    pub full_v: Boolean<F>,
    pub y_replace: Array2<Boolean<F>>,
    pub u_replace: Array2<Boolean<F>>,
    pub v_replace: Array2<Boolean<F>>,
}

impl<F: PrimeField> AllocVar<RedactRectCfg, F> for RedactRectCfgVar<F> {
    fn new_variable<T: Borrow<RedactRectCfg>>(
        cs: impl Into<ark_relations::r1cs::Namespace<F>>,
        f: impl FnOnce() -> Result<T, SynthesisError>,
        mode: ark_r1cs_std::prelude::AllocationMode,
    ) -> Result<Self, SynthesisError> {
        let cs = cs.into().cs();
        let cfg = f()?.borrow().clone();

        let bool_grid = |values: &Array2<bool>, rows: usize, cols: usize| {
            let mut grid = Array2::from_elem((rows, cols), Boolean::FALSE);
            for i in 0..rows {
                for j in 0..cols {
                    grid[[i, j]] =
                        Boolean::new_variable(cs.clone(), || Ok(values[[i, j]]), mode)?;
                }
            }
            Ok(grid)
        };

        // NOTE: `y_replace`/`u_replace`/`v_replace` are *always* allocated as
        // real circuit variables here, even when the native `full_y`/`full_u`/
        // `full_v` flag is true (in which case the grid is all-`false` and
        // logically redundant, since `redact_plane_circuit` ignores `partial`
        // once `full` selects the fill value). Skipping the allocation in that
        // case — e.g. using bare `Boolean::FALSE` Rust-level constants instead
        // of witnessed variables — would make the number of constraint-system
        // variables (and hence the whole R1CS shape) depend on the *value* of
        // the witness rather than being fixed ahead of time, which breaks
        // Nova/IVC folding across steps whose macroblocks don't all share the
        // same full/partial pattern (the folding scheme reuses one fixed
        // step-circuit topology for every step). `full_y` above is itself
        // always allocated in `mode` (never a bare Rust `Boolean::Constant`
        // when synthesized through `EditOnlyCircuit`, since configs are
        // allocated with `new_witness`), so `compactify()`'s
        // `Boolean::Constant(true)` fast path never actually triggers on this
        // path either — the grid is always packed into the hash, so there is
        // no allocation to skip for real savings here anyway.
        Ok(Self {
            in_frame_range: Boolean::new_variable(cs.clone(), || Ok(cfg.in_frame_range), mode)?,
            fill_y: cfg.fill_y,
            full_y: Boolean::new_variable(cs.clone(), || Ok(cfg.full_y), mode)?,
            full_u: Boolean::new_variable(cs.clone(), || Ok(cfg.full_u), mode)?,
            full_v: Boolean::new_variable(cs.clone(), || Ok(cfg.full_v), mode)?,
            y_replace: bool_grid(&cfg.y_replace, 16, 16)?,
            u_replace: bool_grid(&cfg.u_replace, 8, 8)?,
            v_replace: bool_grid(&cfg.v_replace, 8, 8)?,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct RedactRect {}

impl RedactRect {
    pub fn redact_plane_native<const M: usize, const N: usize>(
        data: &Matrix<u8, M, N>,
        full: bool,
        partial: &Array2<bool>,
        fill: u8,
    ) -> Matrix<u8, M, N> {
        let mut out = data.clone();
        if full {
            for i in 0..M {
                for j in 0..N {
                    out[(i, j)] = fill;
                }
            }
        } else {
            for i in 0..M {
                for j in 0..N {
                    if partial[[i, j]] {
                        out[(i, j)] = fill;
                    }
                }
            }
        }
        out
    }

    pub fn redact_plane_circuit<F: PrimeField, const M: usize, const N: usize>(
        data: &MatrixVar<I64Var<F>, M, N>,
        full: &Boolean<F>,
        partial: &Array2<Boolean<F>>,
        fill: u8,
    ) -> Result<MatrixVar<I64Var<F>, M, N>, SynthesisError> {
        let fill_var = I64Var::constant(fill as i64);
        let mut out = data.clone();
        match full {
            Boolean::Constant(true) => {
                for i in 0..M {
                    for j in 0..N {
                        out[(i, j)] = fill_var.clone();
                    }
                }
            }
            Boolean::Constant(false) => {
                for i in 0..M {
                    for j in 0..N {
                        out[(i, j)] = partial[[i, j]].select(&fill_var, &out[(i, j)])?;
                    }
                }
            }
            _ => {
                for i in 0..M {
                    for j in 0..N {
                        let replaced = partial[[i, j]].select(&fill_var, &out[(i, j)])?;
                        out[(i, j)] = full.select(&fill_var, &replaced)?;
                    }
                }
            }
        }
        Ok(out)
    }
}

impl EditGadget for RedactRect {
    type Cfg = RedactRectCfg;
    type CfgVar<F: PrimeField> = RedactRectCfgVar<F>;

    fn edit_native(
        y: &Matrix<u8, 16, 16>,
        u: &Matrix<u8, 8, 8>,
        v: &Matrix<u8, 8, 8>,
        cfg: &Self::Cfg,
    ) -> (Matrix<u8, 16, 16>, Matrix<u8, 8, 8>, Matrix<u8, 8, 8>) {
        let y_active = cfg.in_frame_range && cfg.full_y;
        let u_active = cfg.in_frame_range && cfg.full_u;
        let v_active = cfg.in_frame_range && cfg.full_v;
        (
            Self::redact_plane_native(y, y_active, &cfg.y_replace, cfg.fill_y),
            Self::redact_plane_native(u, u_active, &cfg.u_replace, 128),
            Self::redact_plane_native(v, v_active, &cfg.v_replace, 128),
        )
    }

    fn edit_circuit<F: PrimeField>(
        y: &MatrixVar<I64Var<F>, 16, 16>,
        u: &MatrixVar<I64Var<F>, 8, 8>,
        v: &MatrixVar<I64Var<F>, 8, 8>,
        cfg: &Self::CfgVar<F>,
    ) -> Result<
        (
            MatrixVar<I64Var<F>, 16, 16>,
            MatrixVar<I64Var<F>, 8, 8>,
            MatrixVar<I64Var<F>, 8, 8>,
        ),
        SynthesisError,
    > {
        let y_full = &cfg.in_frame_range & &cfg.full_y;
        let u_full = &cfg.in_frame_range & &cfg.full_u;
        let v_full = &cfg.in_frame_range & &cfg.full_v;
        Ok((
            Self::redact_plane_circuit(y, &y_full, &cfg.y_replace, cfg.fill_y)?,
            Self::redact_plane_circuit(u, &u_full, &cfg.u_replace, 128)?,
            Self::redact_plane_circuit(v, &v_full, &cfg.v_replace, 128)?,
        ))
    }

    fn result_has_constant_encoding() -> (bool, bool, bool) {
        (false, false, false)
    }
}

#[cfg(test)]
pub mod tests {
    use ark_bn254::Fr;
    use ark_relations::r1cs::ConstraintSystem;
    use rand::{thread_rng, Rng};
    use rayon::prelude::*;

    use super::*;

    #[test]
    fn test_bright() {
        (0..=65535).into_par_iter().for_each(|i| {
            let rng = &mut thread_rng();
            let y = Matrix::from_iter((0..16 * 16).map(|_| rng.gen_range(0..=255)));
            let u = Matrix::from_iter((0..8 * 8).map(|_| rng.gen_range(0..=255)));
            let v = Matrix::from_iter((0..8 * 8).map(|_| rng.gen_range(0..=255)));

            let cs = ConstraintSystem::<Fr>::new_ref();
            let y_var = MatrixVar::new_witness(cs.clone(), || Ok(y.clone())).unwrap();
            let u_var = MatrixVar::new_witness(cs.clone(), || Ok(u.clone())).unwrap();
            let v_var = MatrixVar::new_witness(cs.clone(), || Ok(v.clone())).unwrap();
            let scale = BrightnessCfg(i);
            let (yy, uu, vv) = Brightness::edit_native(&y, &u, &v, &scale);
            let scale_var = BrightnessCfgVar::new_witness(cs.clone(), || Ok(scale)).unwrap();
            let (yy_var, uu_var, vv_var) =
                Brightness::edit_circuit(&y_var, &u_var, &v_var, &scale_var).unwrap();

            assert_eq!(yy, yy_var.value().unwrap().to_u8());
            assert_eq!(uu, uu_var.value().unwrap().to_u8());
            assert_eq!(vv, vv_var.value().unwrap().to_u8());
        });
    }

    #[test]
    fn test_redact_rect_matches_masking() {
        let rng = &mut thread_rng();
        let y = Matrix::from_iter((0..16 * 16).map(|_| rng.gen_range(0..=255)));
        let u = Matrix::from_iter((0..8 * 8).map(|_| rng.gen_range(0..=255)));
        let v = Matrix::from_iter((0..8 * 8).map(|_| rng.gen_range(0..=255)));

        let cfg = RedactRectCfg::from_rectangle(48, 80, true, 52, 84, 100, 120, 0);
        let (yy, uu, vv) = RedactRect::edit_native(&y, &u, &v, &cfg);

        let cs = ConstraintSystem::<Fr>::new_ref();
        let y_var = MatrixVar::new_witness(cs.clone(), || Ok(y.clone())).unwrap();
        let u_var = MatrixVar::new_witness(cs.clone(), || Ok(u.clone())).unwrap();
        let v_var = MatrixVar::new_witness(cs.clone(), || Ok(v.clone())).unwrap();
        let cfg_var = RedactRectCfgVar::new_witness(cs.clone(), || Ok(cfg)).unwrap();
        let (yy_var, uu_var, vv_var) =
            RedactRect::edit_circuit(&y_var, &u_var, &v_var, &cfg_var).unwrap();

        assert_eq!(yy, yy_var.value().unwrap().to_u8());
        assert_eq!(uu, uu_var.value().unwrap().to_u8());
        assert_eq!(vv, vv_var.value().unwrap().to_u8());
    }
}
