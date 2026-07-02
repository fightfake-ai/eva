//! Edit-only IVC step circuit — proves video edits without H.264 encode constraints.
//!
//! Compared to [`crate::EditEncodeCircuit`], this circuit:
//! - Applies [`crate::edit::constraints::EditGadget`] to original macroblocks
//! - Rolls Griffin hashes of **original** pixels (`h1`) and **edited** pixels (`h2`)
//! - Does **not** use predictors, QP, DCT, quantization, or coefficient witnesses
//!
//! Intended as a smaller proof path when authenticity of the edit transformation alone
//! is sufficient (no bitstream / encode binding).

use std::marker::PhantomData;
use std::sync::Arc;

use ark_crypto_primitives::sponge::Absorb;
use ark_ff::PrimeField;
use ark_r1cs_std::{
    alloc::AllocVar,
    boolean::{AllocatedBool, Boolean},
    fields::{
        fp::{AllocatedFp, FpVar},
        FieldVar,
    },
    R1CSVar,
};
use ark_relations::r1cs::{ConstraintSystem, ConstraintSystemRef, LinearCombination, SynthesisError};
use ark_std::{end_timer, start_timer};
use folding_schemes::{
    frontend::{FCircuit, LookupArgumentRef},
    Error,
};
use rayon::prelude::*;

use crate::edit::constraints::{EditConfig, EditConfigVar, EditGadget};
use crate::encode::constraints::MatrixVar;
use crate::encode::Matrix;
use crate::griffin::{
    constraints::{GriffinCircuit, PermCircuit},
    griffin::{Griffin, Permutation},
    params::GriffinParams,
};
use crate::var::I64Var;
use crate::MB_BITS;

/// Griffin hash of original macroblock Y/U/V pixels (one partial h1 term).
pub fn hash_orig_macroblock<F: PrimeField + Absorb>(
    griffin: &Griffin<F>,
    (y, u, v): &(Matrix<u8, 16, 16>, Matrix<u8, 8, 8>, Matrix<u8, 8, 8>),
) -> F {
    griffin.hash(
        &vec![]
            .into_iter()
            .chain(y.iter())
            .chain(u.iter())
            .chain(v.iter())
            .copied()
            .collect::<Vec<_>>()
            .chunks(F::MODULUS_BIT_SIZE as usize / MB_BITS)
            .map(F::from_be_bytes_mod_order)
            .collect::<Vec<_>>(),
    )
}

/// Griffin hash of edited pixels + edit config (one partial h2 term).
pub fn hash_edited_macroblock<F: PrimeField + Absorb, E: EditGadget>(
    griffin: &Griffin<F>,
    (y, u, v): &(Matrix<u8, 16, 16>, Matrix<u8, 8, 8>, Matrix<u8, 8, 8>),
    cfg: &E::Cfg,
) -> F {
    if !cfg.should_keep_native() {
        return F::zero();
    }
    let (ey, eu, ev) = E::edit_native(y, u, v, cfg);
    let mut words = Vec::new();
    words.extend(ey.iter().copied());
    words.extend(eu.iter().copied());
    words.extend(ev.iter().copied());
    let mut field_elems = words
        .chunks(F::MODULUS_BIT_SIZE as usize / MB_BITS)
        .map(F::from_be_bytes_mod_order)
        .collect::<Vec<_>>();
    field_elems.extend(cfg.compactify::<F>());
    griffin.hash(&field_elems)
}

/// External inputs for one edit-only IVC step (no encode data).
#[derive(Clone, Debug)]
pub struct EditOnlyExternalInputs<C: Default + Clone> {
    pub blocks: Vec<(Matrix<u8, 16, 16>, Matrix<u8, 8, 8>, Matrix<u8, 8, 8>)>,
    pub edit_configs: Vec<C>,
}

impl<C: Default + Clone> Default for EditOnlyExternalInputs<C> {
    fn default() -> Self {
        Self {
            blocks: vec![],
            edit_configs: vec![],
        }
    }
}

/// F-circuit that folds edit steps only (no encode).
#[derive(Clone, Debug)]
pub struct EditOnlyCircuit<F: PrimeField + Absorb, E: EditGadget> {
    pub _e: PhantomData<E>,
    pub griffin_params: Arc<GriffinParams<F>>,
}

impl<F: PrimeField + Absorb, E: EditGadget> EditOnlyCircuit<F, E> {
    fn hash_pixels<const WIDTH: usize>(pixels: &[I64Var<F>]) -> Result<Vec<FpVar<F>>, SynthesisError> {
        let s = F::from(1u32 << WIDTH);
        pixels
            .chunks(F::MODULUS_BIT_SIZE as usize / WIDTH)
            .map(|chunk| {
                let mut r = FpVar::zero();
                for v in chunk {
                    r = r * s + &v.to_fpvar();
                }
                Ok(r)
            })
            .collect()
    }

    fn process_macroblock<const DUMMY: bool>(
        cs: ConstraintSystemRef<F>,
        griffin: &GriffinCircuit<F>,
        edit_config: &E::Cfg,
        (y_block, u_block, v_block): &(Matrix<u8, 16, 16>, Matrix<u8, 8, 8>, Matrix<u8, 8, 8>),
    ) -> Result<
        (
            (Option<ark_relations::r1cs::Variable>, F),
            (Option<ark_relations::r1cs::Variable>, F),
            (Option<ark_relations::r1cs::Variable>, bool),
        ),
        SynthesisError,
    > {
        let y_block_var = MatrixVar::new_committed(cs.clone(), || Ok(y_block))?;
        let u_block_var = MatrixVar::new_committed(cs.clone(), || Ok(u_block))?;
        let v_block_var = MatrixVar::new_committed(cs.clone(), || Ok(v_block))?;
        let edit_config_var = E::CfgVar::new_witness(cs.clone(), || Ok(edit_config))?;

        let (y_edited, u_edited, v_edited) =
            E::edit_circuit(&y_block_var, &u_block_var, &v_block_var, &edit_config_var)?;

        let h1 = griffin.hash(&Self::hash_pixels::<MB_BITS>(
            &y_block_var
                .0
                .iter()
                .chain(u_block_var.0.iter())
                .chain(v_block_var.0.iter())
                .cloned()
                .collect::<Vec<_>>(),
        )?)?;

        let mut h2_input = Self::hash_pixels::<MB_BITS>(
            &y_edited
                .0
                .iter()
                .chain(u_edited.0.iter())
                .chain(v_edited.0.iter())
                .cloned()
                .collect::<Vec<_>>(),
        )?;
        h2_input.extend(edit_config_var.compactify());

        let keep = edit_config_var.should_keep_circuit()?;
        let h2 = keep.select(&griffin.hash(&h2_input)?, &FpVar::zero())?;

        let h1 = match h1 {
            FpVar::Var(h1) => (Some(h1.variable), h1.value()?),
            FpVar::Constant(h1) => (None, h1),
        };
        let h2 = match h2 {
            FpVar::Var(h2) => (Some(h2.variable), h2.value()?),
            FpVar::Constant(h2) => (None, h2),
        };
        let keep = match keep {
            Boolean::Var(keep) => (Some(keep.variable()), keep.value()?),
            Boolean::Constant(keep) => (None, keep),
        };

        let _ = DUMMY;
        Ok((h1, h2, keep))
    }

    fn fold_step_hashes(
        _cs: ConstraintSystemRef<F>,
        griffin: &GriffinCircuit<F>,
        z_i: &[FpVar<F>],
        h1s: Vec<FpVar<F>>,
        h2s: Vec<FpVar<F>>,
        keeps: Vec<Boolean<F>>,
    ) -> Result<Vec<FpVar<F>>, SynthesisError> {
        let mut h1_chain = h1s;
        h1_chain.push(z_i[0].clone());
        let h1 = griffin.hash(&h1_chain)?;

        let h2 = {
            let mut sum_cs = ConstraintSystemRef::None;
            let mut value = 0u16;
            let mut constant = 0u16;
            let mut new_lc = vec![];

            for keep in keeps {
                let keep_cs = keep.cs();
                match keep {
                    Boolean::Constant(c) => constant += c as u16,
                    Boolean::Var(variable) => {
                        sum_cs = sum_cs.or(keep_cs);
                        value += variable.value().unwrap() as u16;
                        new_lc.push((F::one(), variable.variable()));
                    }
                }
            }

            let any_kept = if sum_cs.is_none() {
                FpVar::constant(F::from(constant))
            } else {
                FpVar::Var(AllocatedFp::new(
                    Some(F::from(constant + value)),
                    sum_cs.new_lc(if sum_cs.should_construct_matrices() {
                        LinearCombination(new_lc) + (F::from(constant), ark_relations::r1cs::Variable::One)
                    } else {
                        LinearCombination::new()
                    })?,
                    sum_cs,
                ))
            };

            let mut h2_chain = h2s;
            h2_chain.push(z_i[1].clone());
            any_kept.is_zero()?.select(&z_i[1], &griffin.hash(&h2_chain)?)?
        };

        Ok(vec![h1, h2])
    }
}

impl<F: PrimeField + Absorb, E: EditGadget> FCircuit<F> for EditOnlyCircuit<F, E> {
    type Params = Self;
    type ExternalInputs = EditOnlyExternalInputs<E::Cfg>;

    fn new(params: Self::Params) -> Self {
        params
    }

    fn state_len(&self) -> usize {
        2
    }

    fn step_native(
        &self,
        _i: usize,
        z_i: Vec<F>,
        external_inputs: &Self::ExternalInputs,
    ) -> Result<Vec<F>, Error> {
        let Self::ExternalInputs { blocks, edit_configs } = external_inputs;
        let griffin = Griffin::new(&self.griffin_params);

        let mut h1s = blocks
            .par_iter()
            .map(|(y, u, v)| {
                griffin.hash(
                    &vec![]
                        .into_iter()
                        .chain(y.iter())
                        .chain(u.iter())
                        .chain(v.iter())
                        .copied()
                        .collect::<Vec<_>>()
                        .chunks(F::MODULUS_BIT_SIZE as usize / MB_BITS)
                        .map(F::from_be_bytes_mod_order)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        h1s.push(z_i[0]);

        let mut h2s = blocks
            .par_iter()
            .zip(edit_configs.par_iter())
            .map(|((y, u, v), cfg)| {
                if !cfg.should_keep_native() {
                    return F::zero();
                }
                let (ey, eu, ev) = E::edit_native(y, u, v, cfg);
                let mut words = Vec::new();
                words.extend(ey.iter().copied());
                words.extend(eu.iter().copied());
                words.extend(ev.iter().copied());
                let mut field_elems = words
                    .chunks(F::MODULUS_BIT_SIZE as usize / MB_BITS)
                    .map(F::from_be_bytes_mod_order)
                    .collect::<Vec<_>>();
                field_elems.extend(cfg.compactify::<F>());
                griffin.hash(&field_elems)
            })
            .collect::<Vec<_>>();
        h2s.push(z_i[1]);

        let h1 = griffin.hash(&h1s);
        let h2 = if edit_configs.iter().any(|c| c.should_keep_native()) {
            griffin.hash(&h2s)
        } else {
            z_i[1]
        };

        Ok(vec![h1, h2])
    }

    fn generate_step_constraints(
        &self,
        cs: ConstraintSystemRef<F>,
        la: LookupArgumentRef<F>,
        _i: usize,
        z_i: Vec<FpVar<F>>,
        external_inputs: &Self::ExternalInputs,
    ) -> Result<Vec<FpVar<F>>, SynthesisError> {
        let Self::ExternalInputs { blocks, edit_configs } = external_inputs;
        let griffin = GriffinCircuit::new(&self.griffin_params);

        // Table is unused (no encode lookups) but must be non-empty for Nova's
        // `build_histo` driver; committed pixel / edit intermediates stay in 0..255.
        la.set_table((0u32..(1 << MB_BITS)).map(F::from).collect());

        let mode = cs.borrow().unwrap().mode;
        let (w0, q0, lc0) = {
            let cs = cs.borrow().unwrap();
            (
                cs.num_witness_variables,
                cs.num_committed_variables,
                cs.num_linear_combinations,
            )
        };

        let (w_off, q_off, lc_off, n_constraints) = {
            let cs = ConstraintSystemRef::new(ConstraintSystem::new_offset(0, 0, 0));
            cs.set_mode(mode);
            let _ = Self::process_macroblock::<true>(cs.clone(), &griffin, &edit_configs[0], &blocks[0])?;
            let inner = cs.into_inner().unwrap();
            (
                inner.num_witness_variables,
                inner.num_committed_variables,
                inner.num_linear_combinations,
                inner.num_constraints,
            )
        };

        let timer = start_timer!(|| "Edit-only: synthesize macroblocks");
        let partial = blocks
            .par_iter()
            .zip(edit_configs.par_iter())
            .enumerate()
            .map(|(j, (block, edit_config))| {
                let cs = ConstraintSystemRef::new(ConstraintSystem::new_offset(
                    w0 + w_off * j,
                    q0 + q_off * j,
                    lc0 + lc_off * j,
                ));
                cs.set_mode(mode);

                let (h1, h2, keep) =
                    Self::process_macroblock::<false>(cs.clone(), &griffin, edit_config, block)?;

                let inner = cs.into_inner().unwrap();
                Ok((
                    (
                        inner.witness_assignment,
                        inner.committed_assignment,
                        inner.lc_map,
                        inner.a_constraints,
                        inner.b_constraints,
                        inner.c_constraints,
                    ),
                    h1,
                    h2,
                    keep,
                ))
            })
            .collect::<Result<Vec<_>, SynthesisError>>()?;
        end_timer!(timer);

        {
            let cs = cs.borrow_mut().unwrap();
            cs.num_committed_variables += q_off * blocks.len();
            cs.num_witness_variables += w_off * blocks.len();
            cs.num_constraints += n_constraints * blocks.len();
            cs.num_linear_combinations += lc_off * blocks.len();
            cs.committed_assignment.reserve(q_off * blocks.len());
            cs.witness_assignment.reserve(w_off * blocks.len());
        }

        let mut h1s = vec![];
        let mut h2s = vec![];
        let mut keeps = vec![];
        for (partial_cs, h1, h2, keep) in partial {
            {
                let cs = cs.borrow_mut().unwrap();
                cs.witness_assignment.extend_from_slice(&partial_cs.0);
                cs.committed_assignment.extend_from_slice(&partial_cs.1);
                cs.lc_map.extend(partial_cs.2);
                cs.a_constraints.extend_from_slice(&partial_cs.3);
                cs.b_constraints.extend_from_slice(&partial_cs.4);
                cs.c_constraints.extend_from_slice(&partial_cs.5);
            }
            h1s.push(match h1.0 {
                Some(var) => FpVar::Var(AllocatedFp::new(Some(h1.1), var, cs.clone())),
                None => FpVar::Constant(h1.1),
            });
            h2s.push(match h2.0 {
                Some(var) => FpVar::Var(AllocatedFp::new(Some(h2.1), var, cs.clone())),
                None => FpVar::Constant(h2.1),
            });
            keeps.push(match keep.0 {
                Some(var) => Boolean::Var(AllocatedBool::new(Some(keep.1), var, cs.clone())),
                None => Boolean::Constant(keep.1),
            });
        }

        Self::fold_step_hashes(cs, &griffin, &z_i, h1s, h2s, keeps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_bn254::{constraints::GVar, Fr, G1Projective as Projective};
    use ark_grumpkin::{constraints::GVar as GVar2, Projective as Projective2};
    use ark_ff::UniformRand;
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::Zero;
    use folding_schemes::folding::nova::circuits::AugmentedFCircuit;
    use folding_schemes::frontend::LookupArgument;
    use folding_schemes::transcript::poseidon::poseidon_test_config;
    use rand::{thread_rng, Rng};

    use crate::edit::constraints::{Brightness, BrightnessCfg};

    fn sample_block(rng: &mut impl Rng) -> (Matrix<u8, 16, 16>, Matrix<u8, 8, 8>, Matrix<u8, 8, 8>) {
        (
            Matrix::from_iter((0..256).map(|_| rng.gen_range(0..=255u8))),
            Matrix::from_iter((0..64).map(|_| rng.gen_range(0..=255u8))),
            Matrix::from_iter((0..64).map(|_| rng.gen_range(0..=255u8))),
        )
    }

    #[test]
    fn edit_only_native_matches_constraints() {
        let rng = &mut thread_rng();
        let blocks_per_step = 4usize;
        let circuit = EditOnlyCircuit::<Fr, Brightness> {
            _e: PhantomData,
            griffin_params: Arc::new(GriffinParams::new(16, 5, 9)),
        };
        let inputs = EditOnlyExternalInputs {
            blocks: (0..blocks_per_step).map(|_| sample_block(rng)).collect(),
            edit_configs: vec![BrightnessCfg(416); blocks_per_step],
        };

        let z0 = vec![Fr::zero(), Fr::zero()];
        let z1_native = circuit.step_native(0, z0.clone(), &inputs).unwrap();

        let cs = ConstraintSystem::<Fr>::new_ref();
        let la = LookupArgument::new_ref();
        let z0_var = z0.iter().map(|&x| FpVar::constant(x)).collect::<Vec<_>>();
        let z1_var = circuit
            .generate_step_constraints(cs.clone(), la.clone(), 0, z0_var, &inputs)
            .unwrap();

        assert_eq!(z1_native, z1_var.iter().map(|v| v.value().unwrap()).collect::<Vec<_>>());
        assert!(cs.is_satisfied().unwrap(), "step constraints unsatisfied");

        la.build_histo(cs.clone()).unwrap();
        la.generate_lookup_constraints(cs.clone(), Fr::rand(rng))
            .unwrap();
        cs.finalize();
        assert!(cs.is_satisfied().unwrap());
    }

    #[test]
    fn edit_only_augmented_circuit_satisfies() {
        let rng = &mut thread_rng();
        let blocks_per_step = 4usize;
        let f_circuit = EditOnlyCircuit::<Fr, Brightness> {
            _e: PhantomData,
            griffin_params: Arc::new(GriffinParams::new(16, 5, 9)),
        };
        let inputs = EditOnlyExternalInputs {
            blocks: (0..blocks_per_step).map(|_| sample_block(rng)).collect(),
            edit_configs: vec![BrightnessCfg(416); blocks_per_step],
        };

        let cs = ConstraintSystem::<Fr>::new_ref();
        let la = LookupArgument::new_ref();
        AugmentedFCircuit::<Projective, Projective2, GVar2, _>::empty(
            &poseidon_test_config(),
            la.clone(),
            &f_circuit,
            &inputs,
        )
        .run(cs.clone())
        .unwrap();
        la.build_histo(cs.clone()).unwrap();
        la.generate_lookup_constraints(cs.clone(), Fr::rand(rng))
            .unwrap();
        cs.finalize();
        assert!(cs.is_satisfied().unwrap());
    }
}
