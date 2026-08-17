//! One-off: print DeciderEthCircuit constraint count at QUICK scale.
use std::marker::PhantomData;
use std::sync::Arc;
use ark_bn254::{constraints::GVar, Fq, Fr, G1Projective as Projective};
use ark_ec::{CurveGroup, PrimeGroup};
use ark_ff::{UniformRand, Zero};
use ark_grumpkin::{constraints::GVar as GVar2, Projective as Projective2};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystem};
use folding_schemes::commitment::pedersen::Pedersen;
use folding_schemes::folding::nova::Nova;
use folding_schemes::transcript::poseidon::poseidon_test_config;
use folding_schemes::FoldingScheme;
use rand::thread_rng;
use video::decider::DeciderEthCircuit;
use video::edit::constraints::{Brightness, BrightnessCfg};
use video::griffin::params::GriffinParams;
use video::{EditOnlyCircuit, EditOnlyExternalInputs};
use ark_crypto_primitives::crh::poseidon::CRH;
use ark_crypto_primitives::crh::CRHScheme;
use ark_ec::AffineRepr;
use ark_ff::{BigInteger, PrimeField};

type NOVA = Nova<Projective, GVar, Projective2, GVar2, EditOnlyCircuit<Fr, Brightness>, Pedersen<Projective>, Pedersen<Projective2>>;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut rng = thread_rng();
    let blocks = 4usize;
    let steps = 4usize;
    let f_circuit = EditOnlyCircuit { _e: PhantomData, griffin_params: Arc::new(GriffinParams::new(16, 5, 9)) };
    let poseidon_config = poseidon_test_config();
    let block = (
        video::encode::Matrix::from_iter((0..256).map(|k| ((k * 17) % 256) as u8)),
        video::encode::Matrix::from_iter((0..64).map(|k| ((k * 5) % 256) as u8)),
        video::encode::Matrix::from_iter((0..64).map(|k| ((k * 9) % 256) as u8)),
    );
    let external = EditOnlyExternalInputs { blocks: vec![block; blocks], edit_configs: vec![BrightnessCfg(416); blocks] };
    let (pp, vp) = NOVA::preprocess(&poseidon_config, &f_circuit, &mut rng, &external)?;
    let params = (pp, vp);
    let mut nova = NOVA::init(&params, f_circuit, vec![Fr::zero(), Fr::zero()])?;
    for _ in 0..steps { nova.prove_step(&params, &external)?; }
    let sk = Fq::rand(&mut rng);
    let vk = Projective2::generator() * sk;
    let (px, py) = { let p = vk.into_affine(); p.xy().unwrap_or((Fr::zero(), Fr::zero())) };
    let sigma = {
        let r = Fq::rand(&mut rng);
        let rx = (Projective2::generator() * r).into_affine().x().unwrap_or_default();
        let e = CRH::evaluate(&poseidon_config, [rx, px, py, nova.z_i[0]])?;
        (rx, r + sk * Fq::from_le_bytes_mod_order(&e.into_bigint().to_bytes_le()))
    };
    let circuit = DeciderEthCircuit::<Projective, GVar, Projective2, GVar2>::from_nova(nova, params, vk, sigma)?;
    let cs = ConstraintSystem::<Fr>::new_ref();
    circuit.clone().generate_constraints(cs.clone())?;
    cs.finalize();
    let cs = cs.borrow().unwrap();
    println!("DeciderEth constraints={}", cs.num_constraints);
    println!("instance_vars={} witness={} committed={}", cs.num_instance_variables, cs.num_witness_variables, cs.num_committed_variables);
    Ok(())
}
