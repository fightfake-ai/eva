//! Canonical bytes for a third-party `Nova::verify` (primary + CycleFold + Poseidon
//! links) plus the camera Schnorr. Not a SNARK; this is the IVC transcript.

use ark_bn254::{Fq, Fr, G1Projective as Projective};
use ark_grumpkin::Projective as GrumpkinProjective;
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use folding_schemes::ccs::r1cs::R1CS;
use folding_schemes::folding::nova::{
    CurrentInstance, CycleFoldCommittedInstance, RunningInstance, Witness,
};

pub const IVC_MAGIC: &[u8; 5] = b"FFIV1";

pub fn peek_ivc(bytes: &[u8]) -> Result<(String, usize), String> {
    if bytes.len() < 10 || &bytes[..5] != IVC_MAGIC {
        return Err("not an FFIV1 Nova transcript".into());
    }
    Ok((
        gadget_from_tag(bytes[5])?.to_string(),
        u32::from_le_bytes(bytes[6..10].try_into().map_err(|_| "truncated bps")?) as usize,
    ))
}

pub struct IvcArtifact {
    pub gadget: String,
    pub bps: usize,
    pub z_0: Vec<Fr>,
    pub z_i: Vec<Fr>,
    pub i: Fr,
    pub running: (RunningInstance<Projective>, Witness<Projective>, Vec<Fr>),
    pub incoming: (CurrentInstance<Projective>, Witness<Projective>),
    pub cyclefold: (
        CycleFoldCommittedInstance<GrumpkinProjective>,
        Witness<GrumpkinProjective>,
        Vec<Fq>,
    ),
    pub device_vk: GrumpkinProjective,
    pub sigma: (Fr, Fq),
}

fn put<T: CanonicalSerialize>(out: &mut Vec<u8>, t: &T) -> Result<(), String> {
    t.serialize_compressed(out).map_err(|e| e.to_string())
}

fn get<T: CanonicalDeserialize>(r: &mut &[u8]) -> Result<T, String> {
    T::deserialize_compressed(r).map_err(|e| e.to_string())
}

fn put_running(out: &mut Vec<u8>, u: &RunningInstance<Projective>) -> Result<(), String> {
    put(out, &u.cmE)?;
    put(out, &u.u)?;
    put(out, &u.cmQ)?;
    put(out, &u.cmW)?;
    put(out, &u.x)?;
    Ok(())
}

fn get_running(r: &mut &[u8]) -> Result<RunningInstance<Projective>, String> {
    Ok(RunningInstance {
        cmE: get(r)?,
        u: get(r)?,
        cmQ: get(r)?,
        cmW: get(r)?,
        x: get(r)?,
    })
}

fn put_current(out: &mut Vec<u8>, u: &CurrentInstance<Projective>) -> Result<(), String> {
    put(out, &u.cmE)?;
    put(out, &u.u)?;
    put(out, &u.cmQ)?;
    put(out, &u.cmW)?;
    put(out, &u.x)?;
    Ok(())
}

fn get_current(r: &mut &[u8]) -> Result<CurrentInstance<Projective>, String> {
    Ok(CurrentInstance {
        cmE: get(r)?,
        u: get(r)?,
        cmQ: get(r)?,
        cmW: get(r)?,
        x: get(r)?,
    })
}

fn put_cf(
    out: &mut Vec<u8>,
    u: &CycleFoldCommittedInstance<GrumpkinProjective>,
) -> Result<(), String> {
    put(out, &u.cmE)?;
    put(out, &u.u)?;
    put(out, &u.cmW)?;
    put(out, &u.x)?;
    Ok(())
}

fn get_cf(r: &mut &[u8]) -> Result<CycleFoldCommittedInstance<GrumpkinProjective>, String> {
    Ok(CycleFoldCommittedInstance {
        cmE: get(r)?,
        u: get(r)?,
        cmW: get(r)?,
        x: get(r)?,
    })
}

fn gadget_tag(gadget: &str) -> Result<u8, String> {
    match gadget {
        "redact" => Ok(0),
        "brightness" => Ok(1),
        other => Err(format!("unsupported gadget {other}")),
    }
}

fn gadget_from_tag(tag: u8) -> Result<&'static str, String> {
    match tag {
        0 => Ok("redact"),
        1 => Ok("brightness"),
        t => Err(format!("bad gadget tag {t}")),
    }
}

pub fn encode_ivc(
    gadget: &str,
    bps: usize,
    z_0: &[Fr],
    z_i: &[Fr],
    i: Fr,
    running: &(RunningInstance<Projective>, Witness<Projective>, Vec<Fr>),
    incoming: &(CurrentInstance<Projective>, Witness<Projective>),
    cyclefold: &(
        CycleFoldCommittedInstance<GrumpkinProjective>,
        Witness<GrumpkinProjective>,
        Vec<Fq>,
    ),
    device_vk: &GrumpkinProjective,
    sigma: &(Fr, Fq),
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    out.extend_from_slice(IVC_MAGIC);
    out.push(gadget_tag(gadget)?);
    out.extend_from_slice(&(bps as u32).to_le_bytes());
    put(&mut out, &z_0.to_vec())?;
    put(&mut out, &z_i.to_vec())?;
    put(&mut out, &i)?;
    put_running(&mut out, &running.0)?;
    put(&mut out, &running.1.QW)?;
    put(&mut out, &running.2)?;
    put_current(&mut out, &incoming.0)?;
    put(&mut out, &incoming.1.QW)?;
    put_cf(&mut out, &cyclefold.0)?;
    put(&mut out, &cyclefold.1.QW)?;
    put(&mut out, &cyclefold.2)?;
    put(&mut out, device_vk)?;
    put(&mut out, &sigma.0)?;
    put(&mut out, &sigma.1)?;
    Ok(out)
}

pub fn decode_ivc(bytes: &[u8], r1cs: &R1CS<Fr>, cf_r1cs: &R1CS<Fq>) -> Result<IvcArtifact, String> {
    if bytes.len() < 10 || &bytes[..5] != IVC_MAGIC {
        return Err("not an FFIV1 Nova transcript".into());
    }
    let gadget = gadget_from_tag(bytes[5])?.to_string();
    let bps = u32::from_le_bytes(bytes[6..10].try_into().map_err(|_| "truncated bps")?) as usize;
    let mut r = &bytes[10..];
    let z_0: Vec<Fr> = get(&mut r)?;
    let z_i: Vec<Fr> = get(&mut r)?;
    let i: Fr = get(&mut r)?;
    let U_i = get_running(&mut r)?;
    let W_qw: Vec<Fr> = get(&mut r)?;
    let E: Vec<Fr> = get(&mut r)?;
    let u_i = get_current(&mut r)?;
    let w_qw: Vec<Fr> = get(&mut r)?;
    let cf_U = get_cf(&mut r)?;
    let cf_qw: Vec<Fq> = get(&mut r)?;
    let cf_E: Vec<Fq> = get(&mut r)?;
    let device_vk: GrumpkinProjective = get(&mut r)?;
    let sigma_r: Fr = get(&mut r)?;
    let sigma_s: Fq = get(&mut r)?;
    if !r.is_empty() {
        return Err(format!("trailing {} bytes in FFIV1", r.len()));
    }
    if W_qw.len() < r1cs.q {
        return Err("running witness shorter than r1cs.q".into());
    }
    if w_qw.len() < r1cs.q {
        return Err("incoming witness shorter than r1cs.q".into());
    }
    if cf_qw.len() < cf_r1cs.q {
        return Err("CycleFold witness shorter than cf_r1cs.q".into());
    }
    Ok(IvcArtifact {
        gadget,
        bps,
        z_0,
        z_i,
        i,
        running: (U_i, Witness::new(W_qw, r1cs), E),
        incoming: (u_i, Witness::new(w_qw, r1cs)),
        cyclefold: (cf_U, Witness::new(cf_qw, cf_r1cs), cf_E),
        device_vk,
        sigma: (sigma_r, sigma_s),
    })
}
