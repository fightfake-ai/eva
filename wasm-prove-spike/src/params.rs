//! Offline Groth16 proving-key cache for prove-only wasm runs.
//!
//! Nova preprocess still runs at prove time (fast, fits in wasm memory).
//! Groth16 trusted setup is exported here and loaded from `spike-params.bin`.

use ark_bn254::Bn254;
use ark_groth16::ProvingKey;
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};

use crate::SpikeConfig;

const MAGIC: [u8; 4] = *b"FFSP";
const VERSION: u8 = 1;

/// Cached Groth16 proving key for the toy decider circuit (fixed `SpikeConfig` shape).
#[derive(Clone, Debug)]
pub struct SpikeParams {
    pub config: SpikeConfig,
    pub groth16_pk: ProvingKey<Bn254>,
}

impl SpikeParams {
    pub fn matches_config(&self, config: &SpikeConfig) -> bool {
        self.config.blocks_per_step == config.blocks_per_step
            && self.config.num_steps == config.num_steps
            && self.config.brightness_scale == config.brightness_scale
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        out.push(VERSION);
        out.extend_from_slice(&(self.config.blocks_per_step as u64).to_le_bytes());
        out.extend_from_slice(&(self.config.num_steps as u64).to_le_bytes());
        out.extend_from_slice(&(self.config.brightness_scale as u64).to_le_bytes());
        self.groth16_pk
            .serialize_compressed(&mut out)
            .map_err(|e| format!("serialize groth16 pk: {e}"))?;
        Ok(out)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 29 || bytes[..4] != MAGIC {
            return Err("not a spike params file (expected FFSP header)".into());
        }
        if bytes[4] != VERSION {
            return Err(format!(
                "unsupported spike params version {} (expected {VERSION})",
                bytes[4]
            ));
        }
        let blocks_per_step = u64::from_le_bytes(bytes[5..13].try_into().unwrap()) as usize;
        let num_steps = u64::from_le_bytes(bytes[13..21].try_into().unwrap()) as usize;
        let brightness_scale = u64::from_le_bytes(bytes[21..29].try_into().unwrap()) as u16;
        let mut r: &[u8] = &bytes[29..];
        let groth16_pk =
            ProvingKey::<Bn254>::deserialize_compressed(&mut r).map_err(|e| format!("deserialize groth16 pk: {e}"))?;
        Ok(Self {
            config: SpikeConfig {
                blocks_per_step,
                num_steps,
                brightness_scale,
                rng_seed: 0, // prove-time seed comes from the caller
            },
            groth16_pk,
        })
    }

    pub fn looks_like_params(bytes: &[u8]) -> bool {
        bytes.len() >= 5 && bytes[..4] == MAGIC
    }
}
