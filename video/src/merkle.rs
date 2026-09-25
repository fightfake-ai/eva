//! Capture Merkle trees over per-macroblock leaves.
//!
//! Protocol: camera signs `R_pix ‖ R_syn`. Prove opens island `P_i` against `R_pix`;
//! skip MBs open against both roots. Hash is SHA3-256 with domain separation
//! (`b"pix"`, `b"syn"`, `b"node"`). Not Griffin — capture is outside Nova.

use sha3::{Digest, Sha3_256};

use crate::macroblock_yuv::{MB_UV_BYTES, MB_Y_BYTES};

pub const DIGEST_LEN: usize = 32;
pub type Digest32 = [u8; DIGEST_LEN];

const EMPTY_ROOT_DST: &[u8] = b"empty-merkle";
const NODE_DST: &[u8] = b"node";
pub const PIX_DST: &[u8] = b"pix";
pub const SYN_DST: &[u8] = b"syn";

fn sha3(bytes: impl AsRef<[u8]>) -> Digest32 {
    Sha3_256::digest(bytes.as_ref()).into()
}

/// `H(domain ‖ u64_le(index) ‖ payload)`.
pub fn hash_leaf(domain: &[u8], index: u64, payload: &[u8]) -> Digest32 {
    let mut h = Sha3_256::new();
    h.update(domain);
    h.update(index.to_le_bytes());
    h.update(payload);
    h.finalize().into()
}

pub fn hash_node(left: &Digest32, right: &Digest32) -> Digest32 {
    let mut h = Sha3_256::new();
    h.update(NODE_DST);
    h.update(left);
    h.update(right);
    h.finalize().into()
}

/// One 16×16 Y + 8×8 U + 8×8 V capture leaf (`P_i`).
pub fn pixel_leaf(index: u64, y: &[u8], u: &[u8], v: &[u8]) -> Digest32 {
    debug_assert_eq!(y.len(), MB_Y_BYTES);
    debug_assert_eq!(u.len(), MB_UV_BYTES);
    debug_assert_eq!(v.len(), MB_UV_BYTES);
    let mut payload = Vec::with_capacity(MB_Y_BYTES + 2 * MB_UV_BYTES);
    payload.extend_from_slice(y);
    payload.extend_from_slice(u);
    payload.extend_from_slice(v);
    hash_leaf(PIX_DST, index, &payload)
}

/// Syntax leaf (`S_i`): predictor + quantized coeffs + `type_enc` (6 B).
pub fn syntax_leaf(
    index: u64,
    pred_y: &[u8],
    pred_u: &[u8],
    pred_v: &[u8],
    coeff_y: &[u8],
    coeff_u: &[u8],
    coeff_v: &[u8],
    type_enc: &[u8],
) -> Digest32 {
    let mut payload = Vec::with_capacity(MB_Y_BYTES * 2 + MB_UV_BYTES * 4 + type_enc.len());
    payload.extend_from_slice(pred_y);
    payload.extend_from_slice(pred_u);
    payload.extend_from_slice(pred_v);
    payload.extend_from_slice(coeff_y);
    payload.extend_from_slice(coeff_u);
    payload.extend_from_slice(coeff_v);
    payload.extend_from_slice(type_enc);
    hash_leaf(SYN_DST, index, &payload)
}

/// Binary Merkle tree, leaves padded to the next power of two with zero digests.
#[derive(Clone, Debug)]
pub struct MerkleTree {
    pub n_leaves: usize,
    layers: Vec<Vec<Digest32>>,
}

impl MerkleTree {
    pub fn from_leaves(leaves: &[Digest32]) -> Self {
        if leaves.is_empty() {
            return Self {
                n_leaves: 0,
                layers: vec![vec![sha3(EMPTY_ROOT_DST)]],
            };
        }
        let padded = leaves.len().next_power_of_two();
        let mut layer = leaves.to_vec();
        layer.resize(padded, [0u8; DIGEST_LEN]);
        let mut layers = vec![layer];
        while layers.last().unwrap().len() > 1 {
            let prev = layers.last().unwrap();
            let next = prev
                .chunks_exact(2)
                .map(|c| hash_node(&c[0], &c[1]))
                .collect();
            layers.push(next);
        }
        Self {
            n_leaves: leaves.len(),
            layers,
        }
    }

    pub fn root(&self) -> Digest32 {
        self.layers.last().unwrap()[0]
    }

    pub fn proof(&self, index: usize) -> Result<MerkleProof, String> {
        if self.n_leaves == 0 {
            return Err("empty tree has no proofs".into());
        }
        if index >= self.n_leaves {
            return Err(format!("leaf {index} out of range (n={})", self.n_leaves));
        }
        let mut siblings = Vec::new();
        let mut i = index;
        for layer in &self.layers[..self.layers.len() - 1] {
            let sib = i ^ 1;
            siblings.push(layer[sib]);
            i /= 2;
        }
        Ok(MerkleProof {
            leaf: self.layers[0][index],
            index,
            n_leaves: self.n_leaves,
            siblings,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MerkleProof {
    pub leaf: Digest32,
    pub index: usize,
    pub n_leaves: usize,
    /// Sibling hashes from the leaf toward the root.
    pub siblings: Vec<Digest32>,
}

impl MerkleProof {
    pub fn verify(&self, root: &Digest32) -> bool {
        if self.n_leaves == 0 {
            return false;
        }
        let padded = self.n_leaves.next_power_of_two();
        let expected_levels = padded.trailing_zeros() as usize;
        if self.siblings.len() != expected_levels {
            return false;
        }
        if self.index >= self.n_leaves {
            return false;
        }
        let mut acc = self.leaf;
        let mut i = self.index;
        for sib in &self.siblings {
            acc = if i % 2 == 0 {
                hash_node(&acc, sib)
            } else {
                hash_node(sib, &acc)
            };
            i /= 2;
        }
        &acc == root
    }
}

/// Build `R_pix` from Eva `orig_*_enc` streams (scan-order MBs).
pub fn capture_pixel_tree(
    orig_y: &[u8],
    orig_u: &[u8],
    orig_v: &[u8],
) -> Result<MerkleTree, String> {
    if orig_y.len() % MB_Y_BYTES != 0
        || orig_u.len() % MB_UV_BYTES != 0
        || orig_v.len() % MB_UV_BYTES != 0
    {
        return Err("orig_* lengths must be whole macroblocks".into());
    }
    let n = orig_y.len() / MB_Y_BYTES;
    if orig_u.len() / MB_UV_BYTES != n || orig_v.len() / MB_UV_BYTES != n {
        return Err("Y/U/V macroblock counts differ".into());
    }
    let leaves: Vec<Digest32> = (0..n)
        .map(|i| {
            pixel_leaf(
                i as u64,
                &orig_y[i * MB_Y_BYTES..(i + 1) * MB_Y_BYTES],
                &orig_u[i * MB_UV_BYTES..(i + 1) * MB_UV_BYTES],
                &orig_v[i * MB_UV_BYTES..(i + 1) * MB_UV_BYTES],
            )
        })
        .collect();
    Ok(MerkleTree::from_leaves(&leaves))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proof_roundtrip() {
        let leaves: Vec<Digest32> = (0..5u64).map(|i| hash_leaf(b"t", i, &[i as u8])).collect();
        let tree = MerkleTree::from_leaves(&leaves);
        let root = tree.root();
        for i in 0..5 {
            let p = tree.proof(i).unwrap();
            assert!(p.verify(&root));
            let mut bad = p.clone();
            bad.leaf[0] ^= 1;
            assert!(!bad.verify(&root));
        }
        assert!(tree.proof(5).is_err());
    }

    #[test]
    fn empty_tree_is_stable() {
        let a = MerkleTree::from_leaves(&[]);
        let b = MerkleTree::from_leaves(&[]);
        assert_eq!(a.root(), b.root());
        assert_ne!(a.root(), MerkleTree::from_leaves(&[[1u8; 32]]).root());
    }

    #[test]
    fn pixel_tree_opens() {
        let y = vec![7u8; MB_Y_BYTES * 3];
        let u = vec![8u8; MB_UV_BYTES * 3];
        let v = vec![9u8; MB_UV_BYTES * 3];
        let tree = capture_pixel_tree(&y, &u, &v).unwrap();
        let p = tree.proof(1).unwrap();
        assert!(p.verify(&tree.root()));
        assert_eq!(
            p.leaf,
            pixel_leaf(
                1,
                &y[MB_Y_BYTES..2 * MB_Y_BYTES],
                &u[MB_UV_BYTES..2 * MB_UV_BYTES],
                &v[MB_UV_BYTES..2 * MB_UV_BYTES]
            )
        );
    }
}
