//! Bit-oriented Keccak-f[1600] in R1CS (Boolean gadgets).
//!
//! This is the unoptimized baseline the island-only cost model needs: one constraint per
//! XOR/AND on bits. No lookups. Matches what "Keccak inside Nova with SHA3-256 Merkle trees"
//! would cost if openings go in-circuit.

use ark_ff::PrimeField;
use ark_r1cs_std::prelude::*;
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

const LANES: usize = 25;
const LANE_BITS: usize = 64;
const ROUNDS: usize = 24;

/// Round constants for Keccak-f[1600] (64-bit, little-endian lane convention).
const RC: [u64; ROUNDS] = [
    0x0000_0000_0000_0001,
    0x0000_0000_0000_8082,
    0x8000_0000_0000_808A,
    0x8000_0000_8000_8000,
    0x0000_0000_0000_808B,
    0x0000_0000_8000_0001,
    0x8000_0000_8000_8081,
    0x8000_0000_0000_8009,
    0x0000_0000_0000_008A,
    0x0000_0000_0000_0088,
    0x0000_0000_8000_8009,
    0x0000_0000_8000_000A,
    0x0000_0000_8000_808B,
    0x8000_0000_0000_008B,
    0x8000_0000_0000_8089,
    0x8000_0000_0000_8003,
    0x8000_0000_0000_8002,
    0x8000_0000_0000_0080,
    0x0000_0000_0000_800A,
    0x8000_0000_8000_000A,
    0x8000_0000_8000_8081,
    0x8000_0000_0000_8080,
    0x0000_0000_8000_0001,
    0x8000_0000_8000_8008,
];

/// ρ offsets for lanes 0..24 (x + 5*y).
const RHO: [u32; LANES] = [
    0, 1, 62, 28, 27, 36, 44, 6, 55, 20, 3, 10, 43, 25, 39, 41, 45, 15, 21, 8, 18, 2, 61, 56, 14,
];

#[inline]
fn idx(x: usize, y: usize) -> usize {
    x + 5 * y
}

/// Native Keccak-f[1600] on 25 little-endian `u64` lanes.
pub fn keccak_f_native(state: &mut [u64; LANES]) {
    for round in 0..ROUNDS {
        // θ
        let mut c = [0u64; 5];
        for x in 0..5 {
            c[x] = state[idx(x, 0)]
                ^ state[idx(x, 1)]
                ^ state[idx(x, 2)]
                ^ state[idx(x, 3)]
                ^ state[idx(x, 4)];
        }
        let mut d = [0u64; 5];
        for x in 0..5 {
            d[x] = c[(x + 4) % 5] ^ c[(x + 1) % 5].rotate_left(1);
        }
        for y in 0..5 {
            for x in 0..5 {
                state[idx(x, y)] ^= d[x];
            }
        }

        // ρ + π into `b`
        let mut b = [0u64; LANES];
        for y in 0..5 {
            for x in 0..5 {
                let lane = state[idx(x, y)].rotate_left(RHO[idx(x, y)]);
                b[idx(y, (2 * x + 3 * y) % 5)] = lane;
            }
        }

        // χ
        for y in 0..5 {
            for x in 0..5 {
                state[idx(x, y)] =
                    b[idx(x, y)] ^ ((!b[idx((x + 1) % 5, y)]) & b[idx((x + 2) % 5, y)]);
            }
        }

        // ι
        state[0] ^= RC[round];
    }
}

type LaneVar<F> = [Boolean<F>; LANE_BITS];

fn lane_from_u64<F: PrimeField>(cs: ConstraintSystemRef<F>, v: u64) -> Result<LaneVar<F>, SynthesisError> {
    let mut bits = Vec::with_capacity(LANE_BITS);
    for i in 0..LANE_BITS {
        bits.push(Boolean::new_witness(cs.clone(), || Ok(((v >> i) & 1) == 1))?);
    }
    Ok(bits.try_into().unwrap())
}

fn lane_to_u64<F: PrimeField>(lane: &LaneVar<F>) -> Result<u64, SynthesisError> {
    let mut v = 0u64;
    for (i, b) in lane.iter().enumerate() {
        if b.value()? {
            v |= 1u64 << i;
        }
    }
    Ok(v)
}

fn xor_lane<F: PrimeField>(a: &LaneVar<F>, b: &LaneVar<F>) -> LaneVar<F> {
    let mut out = Vec::with_capacity(LANE_BITS);
    for i in 0..LANE_BITS {
        out.push(&a[i] ^ &b[i]);
    }
    out.try_into().unwrap()
}

fn and_lane<F: PrimeField>(a: &LaneVar<F>, b: &LaneVar<F>) -> LaneVar<F> {
    let mut out = Vec::with_capacity(LANE_BITS);
    for i in 0..LANE_BITS {
        out.push(&a[i] & &b[i]);
    }
    out.try_into().unwrap()
}

fn not_lane<F: PrimeField>(a: &LaneVar<F>) -> LaneVar<F> {
    let mut out = Vec::with_capacity(LANE_BITS);
    for i in 0..LANE_BITS {
        out.push((!&a[i]).clone());
    }
    out.try_into().unwrap()
}

fn rotl_lane<F: PrimeField>(a: &LaneVar<F>, n: u32) -> LaneVar<F> {
    let n = (n as usize) % LANE_BITS;
    let mut out = Vec::with_capacity(LANE_BITS);
    for i in 0..LANE_BITS {
        out.push(a[(i + LANE_BITS - n) % LANE_BITS].clone());
    }
    out.try_into().unwrap()
}

fn xor_const_lane<F: PrimeField>(a: &LaneVar<F>, c: u64) -> LaneVar<F> {
    let mut out = Vec::with_capacity(LANE_BITS);
    for i in 0..LANE_BITS {
        if ((c >> i) & 1) == 1 {
            out.push((!&a[i]).clone());
        } else {
            out.push(a[i].clone());
        }
    }
    out.try_into().unwrap()
}

/// Keccak-f[1600] on Boolean lanes. Returns the output state.
pub fn keccak_f_circuit<F: PrimeField>(
    state: &[LaneVar<F>; LANES],
) -> Result<[LaneVar<F>; LANES], SynthesisError> {
    let mut state = state.clone();
    for round in 0..ROUNDS {
        // θ
        let mut c: [LaneVar<F>; 5] = std::array::from_fn(|_| {
            [(); LANE_BITS].map(|_| Boolean::constant(false))
        });
        for x in 0..5 {
            let mut acc = state[idx(x, 0)].clone();
            for y in 1..5 {
                acc = xor_lane(&acc, &state[idx(x, y)]);
            }
            c[x] = acc;
        }
        let mut d: [LaneVar<F>; 5] = std::array::from_fn(|_| {
            [(); LANE_BITS].map(|_| Boolean::constant(false))
        });
        for x in 0..5 {
            let rotated = rotl_lane(&c[(x + 1) % 5], 1);
            d[x] = xor_lane(&c[(x + 4) % 5], &rotated);
        }
        for y in 0..5 {
            for x in 0..5 {
                state[idx(x, y)] = xor_lane(&state[idx(x, y)], &d[x]);
            }
        }

        // ρ + π
        let mut b: [LaneVar<F>; LANES] = std::array::from_fn(|_| {
            [(); LANE_BITS].map(|_| Boolean::constant(false))
        });
        for y in 0..5 {
            for x in 0..5 {
                let rotated = rotl_lane(&state[idx(x, y)], RHO[idx(x, y)]);
                b[idx(y, (2 * x + 3 * y) % 5)] = rotated;
            }
        }

        // χ
        for y in 0..5 {
            for x in 0..5 {
                let not_b = not_lane(&b[idx((x + 1) % 5, y)]);
                let t = and_lane(&not_b, &b[idx((x + 2) % 5, y)]);
                state[idx(x, y)] = xor_lane(&b[idx(x, y)], &t);
            }
        }

        // ι
        state[0] = xor_const_lane(&state[0], RC[round]);
    }
    Ok(state)
}

/// Allocate a full Keccak state as witnesses and run one Keccak-f. Returns (constraints, output).
pub fn measure_keccak_f_constraints<F: PrimeField>(
    cs: ConstraintSystemRef<F>,
    input: &[u64; LANES],
) -> Result<(usize, [u64; LANES]), SynthesisError> {
    let before = cs.num_constraints();
    let mut lanes = Vec::with_capacity(LANES);
    for &v in input {
        lanes.push(lane_from_u64(cs.clone(), v)?);
    }
    let after_alloc = cs.num_constraints();
    let state: [LaneVar<F>; LANES] = lanes.try_into().unwrap();
    let out = keccak_f_circuit(&state)?;
    let after = cs.num_constraints();
    let mut native = [0u64; LANES];
    for i in 0..LANES {
        native[i] = lane_to_u64(&out[i])?;
    }
    // Allocation of 1600 Boolean witnesses is free in arkworks (no constraints);
    // report only the permutation itself.
    let _ = (before, after_alloc);
    Ok((after - before, native))
}

/// SHA3-256 rate in bytes.
pub const SHA3_256_RATE: usize = 136;

/// How many Keccak-f permutations SHA3-256 needs to absorb `msg_len` bytes (incl. pad10*1).
pub fn sha3_256_permutations_for_msg(msg_len: usize) -> usize {
    // SHA3 appends 0x06 then pad with zeros and a final 0x80 in the last byte of the block.
    // Equivalent: padded length is the least multiple of rate that is > msg_len
    // (at least one padding byte).
    let padded = ((msg_len + 1).div_ceil(SHA3_256_RATE)) * SHA3_256_RATE;
    padded / SHA3_256_RATE
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_bn254::Fr;
    use ark_relations::r1cs::ConstraintSystem;
    use sha3::{Digest, Sha3_256};

    #[test]
    fn keccak_f_matches_sha3_empty() {
        // SHA3-256("") = known; empty message → one padded block → one Keccak-f from zero state
        // with rate XOR of padding.
        let mut state = [0u64; LANES];
        // Absorb empty with SHA3 padding into rate (first 17 lanes = 136 bytes).
        // pad: first byte 0x06, last byte of block 0x80.
        let mut block = [0u8; SHA3_256_RATE];
        block[0] = 0x06;
        block[SHA3_256_RATE - 1] |= 0x80;
        for (i, chunk) in block.chunks_exact(8).enumerate() {
            let mut lane = [0u8; 8];
            lane.copy_from_slice(chunk);
            state[i] ^= u64::from_le_bytes(lane);
        }
        keccak_f_native(&mut state);
        let mut out = [0u8; 32];
        for i in 0..4 {
            out[i * 8..(i + 1) * 8].copy_from_slice(&state[i].to_le_bytes());
        }
        let expected = Sha3_256::digest([]);
        assert_eq!(out, expected.as_slice());
    }

    #[test]
    fn circuit_matches_native_and_counts() {
        let mut input = [0u64; LANES];
        for i in 0..LANES {
            input[i] = (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        }
        let mut expect = input;
        keccak_f_native(&mut expect);

        let cs = ConstraintSystem::<Fr>::new_ref();
        let (n, got) = measure_keccak_f_constraints(cs.clone(), &input).unwrap();
        assert_eq!(got, expect);
        assert!(cs.is_satisfied().unwrap());
        // Sanity: bit-oriented Keccak-f is tens of thousands of constraints, not hundreds.
        assert!(n > 50_000, "unexpectedly cheap: {n}");
        assert!(n < 300_000, "unexpectedly expensive: {n}");
        // Anchors docs/results/hash-r1cs-cost.csv (155,200 on ark BN254 Boolean gadgets).
        assert_eq!(n, 155_200, "Keccak-f constraint count changed: {n}");
    }

    #[test]
    fn sha3_perm_count_matches_intuition() {
        assert_eq!(sha3_256_permutations_for_msg(0), 1);
        assert_eq!(sha3_256_permutations_for_msg(67), 1); // Merkle node
        assert_eq!(sha3_256_permutations_for_msg(135), 1);
        assert_eq!(sha3_256_permutations_for_msg(136), 2);
        assert_eq!(sha3_256_permutations_for_msg(395), 3); // pixel leaf-ish
    }
}
