//! Print FFPB / VK / PK field sizes for local spike artifacts.
use ark_bn254::Bn254;
use ark_groth16::{ProvingKey, VerifyingKey};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use std::fs;
use std::path::PathBuf;

fn ser_len<T: CanonicalSerialize>(t: &T) -> usize {
    let mut v = Vec::new();
    t.serialize_compressed(&mut v).unwrap();
    v.len()
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let proof = fs::read(root.join("spike-proof.bin")).expect("spike-proof.bin");
    println!(
        "FFPB total: {} bytes ({:.2} MiB)",
        proof.len(),
        proof.len() as f64 / ((1 << 20) as f64)
    );

    let mut r: &[u8] = &proof[13..];
    let _z0 = Vec::<ark_bn254::Fr>::deserialize_compressed(&mut r).unwrap();
    let _h2 = ark_bn254::Fr::deserialize_compressed(&mut r).unwrap();
    let _device_vk = ark_grumpkin::Projective::deserialize_compressed(&mut r).unwrap();
    let before = r.len();
    let vk = VerifyingKey::<Bn254>::deserialize_compressed(&mut r).unwrap();
    let vk_bytes = before - r.len();
    println!(
        "VK blob in FFPB: {} bytes ({:.2} MiB)",
        vk_bytes,
        vk_bytes as f64 / ((1 << 20) as f64)
    );
    println!(
        "  gamma_abc_g1.0 (public inputs): {} pts → ~{} B",
        vk.gamma_abc_g1.0.len(),
        vk.gamma_abc_g1.0.len() * 32
    );
    println!(
        "  gamma_abc_g1.1 (committed wires): {} pts → ~{} B ({:.2} MiB)",
        vk.gamma_abc_g1.1.len(),
        vk.gamma_abc_g1.1.len() * 32,
        (vk.gamma_abc_g1.1.len() * 32) as f64 / ((1 << 20) as f64)
    );
    println!("  link_pp: l={}, t={}", vk.link_pp.l, vk.link_pp.t);
    println!(
        "  link_vk.c: {} G2 pts → ~{} B",
        vk.link_vk.c.len(),
        vk.link_vk.c.len() * 64
    );
    println!("FFPB after VK: {} bytes", r.len());

    let params = fs::read(root.join("spike-params.bin")).expect("spike-params.bin");
    println!(
        "\nFFSP params total: {} bytes ({:.2} GiB)",
        params.len(),
        params.len() as f64 / ((1 << 30) as f64)
    );
    let mut pr: &[u8] = &params[29..];
    let pk = ProvingKey::<Bn254>::deserialize_compressed(&mut pr).unwrap();
    println!("PK.vk.gamma_abc.1: {}", pk.vk.gamma_abc_g1.1.len());
    println!("PK.common.a_query: {}", pk.common.a_query.len());
    println!("PK.common.b_g1_query: {}", pk.common.b_g1_query.len());
    println!("PK.common.b_g2_query: {}", pk.common.b_g2_query.len());
    println!("PK.common.h_query: {}", pk.common.h_query.len());
    println!("PK.common.l_query: {}", pk.common.l_query.len());
    println!("PK.common.link_ek.p: {}", pk.common.link_ek.p.len());
    println!("serialized sizes:");
    println!(
        "  a_query: {:.2} MiB",
        ser_len(&pk.common.a_query) as f64 / ((1 << 20) as f64)
    );
    println!(
        "  b_g1_query: {:.2} MiB",
        ser_len(&pk.common.b_g1_query) as f64 / ((1 << 20) as f64)
    );
    println!(
        "  b_g2_query: {:.2} MiB",
        ser_len(&pk.common.b_g2_query) as f64 / ((1 << 20) as f64)
    );
    println!(
        "  h_query: {:.2} MiB",
        ser_len(&pk.common.h_query) as f64 / ((1 << 20) as f64)
    );
    println!(
        "  l_query: {:.2} MiB",
        ser_len(&pk.common.l_query) as f64 / ((1 << 20) as f64)
    );
    println!(
        "  link_ek.p: {:.2} MiB",
        ser_len(&pk.common.link_ek.p) as f64 / ((1 << 20) as f64)
    );
    println!(
        "  embedded vk: {:.2} MiB",
        ser_len(&pk.vk) as f64 / ((1 << 20) as f64)
    );
}
