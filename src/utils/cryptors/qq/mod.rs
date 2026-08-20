use std::time::{SystemTime, UNIX_EPOCH};

use aes_gcm::{
    Aes128Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use rand::{Rng, RngExt};

pub mod audio;
pub mod lyric;

#[allow(unused_imports)]
pub use audio::{
    EncryptedTail, TailFormat, decrypt_file, decrypt_qq_audio, derive_qmc2_key,
    derive_qmc2_key_from_ekey, parse_encrypted_tail, qmc2_decrypt_in_place,
};
pub use lyric::{decrypt_qrc, decrypt_qrc_file};

use crate::sidecar_log;

const K0: &[u8] = b"ABCDEF1234567890";
const K1: u64 = 2_147_483_647;
const K2: u64 = 5_381;
const K3: u64 = 18_014_398_509_481_984;
const K4: u64 = 4_294_967_296;
const K5: u64 = 4_194_304;
const K6: u64 = 24 * 60 * 60 * 1000;
const KA: [u8; 21] = [
    122, 63, 140, 29, 94, 155, 47, 10, 108, 77, 126, 139, 31, 58, 92, 157, 14, 43, 111, 74, 129,
];
const KB: [u8; 16] = [
    189, 48, 95, 16, 208, 255, 116, 182, 239, 84, 218, 184, 53, 181, 225, 207,
];
const KC: usize = 12;

pub fn x0(a: impl AsRef<[u8]>) -> Result<Vec<u8>, String> {
    let mut b = [0_u8; KC];
    rand::rng().fill_bytes(&mut b);

    let c = Aes128Gcm::new_from_slice(&KB).map_err(|_| "e0".to_owned())?;
    let d = c
        .encrypt(Nonce::from_slice(&b), a.as_ref())
        .map_err(|_| "e1".to_owned())?;

    let mut e = Vec::with_capacity(b.len() + d.len());
    e.extend_from_slice(&b);
    e.extend_from_slice(&d);
    Ok(e)
}

pub fn x1(a: impl AsRef<[u8]>) -> Result<String, String> {
    let a = a.as_ref();
    let (b, c) = a.split_at_checked(KC).ok_or_else(|| "e2".to_owned())?;

    let d = Aes128Gcm::new_from_slice(&KB).map_err(|_| "e0".to_owned())?;
    let e = d
        .decrypt(Nonce::from_slice(b), c)
        .map_err(|_| "e3".to_owned())?;

    String::from_utf8(e).map_err(|_| "e4".to_owned())
}

pub fn x2(a: &str) -> Vec<u8> {
    xa(a.as_bytes())
}

pub fn x3(a: impl AsRef<[u8]>) -> Result<String, String> {
    String::from_utf8(xa(a.as_ref())).map_err(|_| "e5".to_owned())
}

pub fn x4(a: &str, d: u64) -> (String, String) {
    match crate::utils::cryptors::csigner::real_x4(a, d) {
        Ok((j, m)) => (j, m),
        Err(err) => {
            sidecar_log::spawn_runtime_log(serde_json::json!(format!(
                "csigner x4 签名失败: {err}"
            )));
            (String::new(), String::new())
        }
    }
}

pub fn x4_fix_identity(uin: &str, guid: &str) {
    if let Err(err) = crate::utils::cryptors::csigner::set_x4_identity(uin, guid) {
        sidecar_log::spawn_runtime_log(serde_json::json!(format!("csigner x4 签名失败: {err}")));
    }
}

fn xa(a: &[u8]) -> Vec<u8> {
    a.iter()
        .enumerate()
        .map(|(b, c)| c ^ KA[b % KA.len()])
        .collect()
}

pub fn x5() -> String {
    let mut a = rand::rng();
    (0..32)
        .map(|_| {
            let b = a.random_range(0..K0.len());
            K0[b] as char
        })
        .collect()
}

pub fn x6(a: &str) -> u64 {
    let mut b = K2;
    for c in a.chars() {
        b = b.wrapping_mul(33).wrapping_add(c as u64);
    }
    b & K1
}

pub fn x7(a: &str) -> u64 {
    a.bytes()
        .fold(K2, |b, c| b.wrapping_add(b << 5).wrapping_add(c as u64))
        & K1
}

pub fn x8() -> String {
    let mut a = rand::rng();
    let b = a.random_range(1_u64..=20);
    let c = b * K3;
    let d = a.random_range(0_u64..=K5) * K4;
    let e = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        % K6 as u128) as u64;
    (c + d + e).to_string()
}

pub fn x9(a: &str) -> String {
    match crate::utils::cryptors::csigner::real_x9(a) {
        Ok(sign) => sign,
        Err(err) => {
            sidecar_log::spawn_runtime_log(serde_json::json!(format!(
                "csigner x9 签名失败: {err}"
            )));
            String::new()
        }
    }
}

pub fn xj(mut a: u64) -> String {
    let mut bytes = Vec::new();

    while a > 0 {
        bytes.push((a & 0xff) as u8);
        a >>= 8;
    }

    bytes.reverse();

    String::from_utf8(bytes).unwrap()
}
