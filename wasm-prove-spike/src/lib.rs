//! In-tab prove (`wasm-js`): Nova IVC + **offline** wrap.
//!
//! The editor worker (`nova_start` / `nova_step` / `nova_digest` / `nova_finish`)
//! never builds [`video::decider::DeciderEthCircuit`]. Wrap is
//! `Nova::verify` (native CycleFold) + Spartan on
//! [`video::decider::NativePrimaryCircuit`]. See `src/browser.rs` and
//! `docs/offline-decider-and-wasm-spartan.md`.
//!
//! Native Spike A (Groth16 / EVM-shaped decider) lives in `eth_spike` and is
//! **not compiled for wasm32**.

pub(crate) mod browser;
pub(crate) mod ivc_artifact;
pub use browser::{
    nova_export_vk, nova_finish, nova_start, nova_step, nova_take_ivc, nova_take_proof,
    nova_verify_full, VkExportMeta,
};

use std::marker::PhantomData;
use std::sync::Arc;

use ark_bn254::Fr;
use ark_ff::{BigInteger, PrimeField};
use video::edit::constraints::Brightness;
use video::griffin::params::GriffinParams;
use video::EditOnlyCircuit;

type Op = Brightness;

/// Native-only Groth16 proving-key cache (`spike-params.bin`).
#[cfg(not(target_arch = "wasm32"))]
mod params;
#[cfg(not(target_arch = "wasm32"))]
pub use params::{
    SpikeConfig, SpikeParams, DEFAULT_BLOCKS_PER_STEP, DEFAULT_BRIGHTNESS, DEFAULT_NUM_STEPS,
    DEFAULT_SETUP_RNG_SEED,
};

/// Native-only Nova + Groth16 `DeciderEthCircuit` (Spike A).
#[cfg(not(target_arch = "wasm32"))]
mod eth_spike;
#[cfg(not(target_arch = "wasm32"))]
pub use eth_spike::{
    run_prove, run_setup, run_spike, PhaseTimingMs, SpikeResult, SpikeSetupResult,
};

pub(crate) fn field_hex(f: &Fr) -> String {
    hex::encode(f.into_bigint().to_bytes_le())
}

pub(crate) fn spike_circuit() -> EditOnlyCircuit<Fr, Op> {
    EditOnlyCircuit {
        _e: PhantomData,
        griffin_params: Arc::new(GriffinParams::new(16, 5, 9)),
    }
}

#[cfg(feature = "wasm")]
mod wasm {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(start)]
    pub fn init_panic_hook() {
        console_error_panic_hook::set_once();
    }

    #[wasm_bindgen]
    pub fn nova_start(
        rgba: &[u8],
        width: u32,
        height: u32,
        gadget: &str,
        a: u32,
        b: u32,
        c: u32,
        d: u32,
    ) -> Result<String, JsValue> {
        crate::browser::nova_start(rgba, width, height, gadget, a, b, c, d)
            .map_err(|e| JsValue::from_str(&e))
    }

    #[wasm_bindgen]
    pub fn nova_step() -> Result<String, JsValue> {
        crate::browser::nova_step().map_err(|e| JsValue::from_str(&e))
    }

    #[wasm_bindgen]
    pub fn nova_digest() -> Result<String, JsValue> {
        crate::browser::nova_digest().map_err(|e| JsValue::from_str(&e))
    }

    #[wasm_bindgen]
    pub fn nova_finish() -> Result<String, JsValue> {
        crate::browser::nova_finish().map_err(|e| JsValue::from_str(&e))
    }

    /// Canonical `FFSP1` Spartan proof from the last successful [`nova_finish`].
    #[wasm_bindgen]
    pub fn nova_take_proof() -> Vec<u8> {
        crate::browser::nova_take_proof()
    }

    /// `FFIV1` Nova transcript from the last successful IVC check.
    #[wasm_bindgen]
    pub fn nova_take_ivc() -> Vec<u8> {
        crate::browser::nova_take_ivc()
    }

    /// Nova::verify (CycleFold included) + camera σ + Spartan + running-instance bind.
    #[wasm_bindgen]
    pub fn nova_verify_full(ivc: &[u8], vk: &[u8], proof: &[u8]) -> Result<String, JsValue> {
        crate::browser::nova_verify_full(ivc, vk, proof).map_err(|e| JsValue::from_str(&e))
    }

    /// Verify a published `FFSV1` VK against an `FFSP1` proof.
    #[wasm_bindgen]
    pub fn spartan_verify(vk: &[u8], proof: &[u8]) -> Result<bool, JsValue> {
        crate::browser::spartan_verify(vk, proof).map_err(|e| JsValue::from_str(&e))
    }

    /// Spartan2 on a ~1-constraint native circuit. Does not run Nova.
    #[wasm_bindgen]
    pub fn spartan_tiny_smoke() -> Result<String, JsValue> {
        crate::browser::spartan_tiny_smoke().map_err(|e| JsValue::from_str(&e))
    }
}
