//! Dummy 64×16 brightness: prove, then `nova_verify_full` (Nova + CycleFold + σ + Spartan).

use wasm_prove_spike::{
    nova_export_vk, nova_finish, nova_start, nova_step, nova_take_ivc, nova_take_proof,
    nova_verify_full,
};

fn dummy_rgba(w: usize, h: usize) -> Vec<u8> {
    let mut rgba = vec![0u8; w * h * 4];
    for i in 0..(w * h) {
        let o = i * 4;
        rgba[o] = (i % 251) as u8;
        rgba[o + 1] = ((i * 3) % 251) as u8;
        rgba[o + 2] = 90;
        rgba[o + 3] = 255;
    }
    rgba
}

#[test]
fn nova_verify_full_dummy_brightness() {
    let w = 64u32;
    let h = 16u32;
    let rgba = dummy_rgba(w as usize, h as usize);
    let info: serde_json::Value =
        serde_json::from_str(&nova_start(&rgba, w, h, "brightness", 1024, 0, 0, 0).unwrap()).unwrap();
    let steps = info["steps"].as_u64().expect("steps") as usize;
    assert_eq!(steps, 1, "64×16 at bps=4 is one fold");

    for _ in 0..steps {
        nova_step().unwrap();
    }

    let (vk_bytes, meta) = nova_export_vk().unwrap();
    assert_eq!(meta.gadget, "brightness");
    assert_eq!(meta.bps, 4);

    let done: serde_json::Value = serde_json::from_str(&nova_finish().unwrap()).unwrap();
    assert_eq!(done["proof_system"], "nova-offline-spartan", "{done}");
    assert_eq!(done["ivc_verified"], true);
    assert_eq!(done["verified"], true);

    let ivc = nova_take_ivc();
    let proof = nova_take_proof();
    assert!(ivc.starts_with(b"FFIV1"), "IVC magic");
    assert!(proof.starts_with(b"FFSP1"), "proof magic");

    let report: serde_json::Value =
        serde_json::from_str(&nova_verify_full(&ivc, &vk_bytes, &proof).unwrap()).unwrap();
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["nova_ivc"], true);
    assert_eq!(report["cyclefold"], true);
    assert_eq!(report["camera_sigma"], true);
    assert_eq!(report["spartan"], true);
    assert_eq!(report["spartan_binds_running"], true);

    if let Ok(path) = std::env::var("DUMP_FFPROOF") {
        let bundle = serde_json::json!({
            "format": "org.fightfake.edit-bundle.v1",
            "simulated": false,
            "complete": true,
            "circuit_id": meta.circuit_id,
            "vk": { "path": format!("/verify/vk/{}.bin", meta.circuit_id), "sha256": meta.vk_sha256 },
            "created": "1970-01-01T00:00:00Z",
            "source": "dummy-64x16.png",
            "edited": "dummy-64x16-edited.png",
            "claim": {
                "gadget": "brightness",
                "h1": done["h1"],
                "h2": done["h2"],
                "view": { "w": 64, "h": 16 },
                "steps": 1,
                "gadgets": [{ "gadget": "brightness", "scale": 1024 }]
            },
            "checks": {
                "nova_ivc": "recheckable",
                "cyclefold": "recheckable",
                "camera_sigma": "recheckable",
                "spartan": "recheckable"
            },
            "note": "Dummy 64×16 native round-trip for /verify.",
            "proof_system": "nova-offline-spartan",
            "assertion": done,
            "spartan_proof_b64": b64(&proof),
            "nova_ivc_b64": b64(&ivc),
        });
        std::fs::write(&path, serde_json::to_string_pretty(&bundle).unwrap() + "\n").unwrap();
        eprintln!("wrote {path}");
    }
}

fn b64(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let a = chunk[0] as u32;
        let b = chunk.get(1).copied().unwrap_or(0) as u32;
        let c = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (a << 16) | (b << 8) | c;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
