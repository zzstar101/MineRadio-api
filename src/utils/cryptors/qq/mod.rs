use std::time::{SystemTime, UNIX_EPOCH};

use aes_gcm::{
    Aes128Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use base64::Engine;
use hmac::{Hmac, Mac};
use rand::{Rng, RngExt};
use sha1::{Digest, Sha1};

pub mod audio;
pub mod lyric;

#[allow(unused_imports)]
pub use audio::{
    EncryptedTail, TailFormat, decrypt_file, decrypt_qq_audio, derive_qmc2_key,
    derive_qmc2_key_from_ekey, parse_encrypted_tail, qmc2_decrypt_in_place,
};
pub use lyric::{decrypt_qrc, decrypt_qrc_file};

const K0: &[u8] = b"ABCDEF1234567890";
const K1: u64 = 2_147_483_647;
const K2: u64 = 5_381;
const K3: u64 = 18_014_398_509_481_984;
const K4: u64 = 4_294_967_296;
const K5: u64 = 4_194_304;
const K6: u64 = 24 * 60 * 60 * 1000;
const K7: [usize; 8] = [23, 14, 6, 36, 16, 40, 7, 19];
const K8: [usize; 8] = [16, 1, 32, 12, 19, 27, 8, 5];
const K9: [u8; 20] = [
    89, 39, 179, 150, 218, 82, 58, 252, 177, 52, 186, 123, 120, 64, 242, 133, 143, 161, 121, 179,
];
const KA: [u8; 21] = [
    122, 63, 140, 29, 94, 155, 47, 10, 108, 77, 126, 139, 31, 58, 92, 157, 14, 43, 111, 74, 129,
];
const KB: [u8; 16] = [
    189, 48, 95, 16, 208, 255, 116, 182, 239, 84, 218, 184, 53, 181, 225, 207,
];
const KC: usize = 12;
const KD: &[u8] = b"9FF169D646A3";
const KE: u32 = 19;
const KF: u32 = 2230;
const KG: u32 = 0x9E37_79B9;

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

pub fn x4(a: &str, b: &str, c: &str, d: u64) -> (String, String) {
    xb(a, b, c, d, &mut rand::rng())
}

fn xb<R: Rng + ?Sized>(a: &str, b: &str, c: &str, d: u64, e: &mut R) -> (String, String) {
    let f = base64::engine::general_purpose::STANDARD.encode(a.as_bytes());
    let mut g = f.into_bytes();
    g.reverse();

    let mut h = <Hmac<Sha1> as Mac>::new_from_slice(KD).expect("e6");
    h.update(&g);

    let mut i = Vec::with_capacity(32);
    for _ in 0..12 {
        i.push(e.random_range(b'A'..=b'Z'));
    }
    i.extend_from_slice(&h.finalize().into_bytes());
    let j = base64::engine::general_purpose::STANDARD.encode(i);

    let k = format!("{KE}&{KF}&&{b}&{d}&{c}&");
    let l = xc(&j);
    let m = base64::engine::general_purpose::STANDARD.encode(xd(k.as_bytes(), &l, e));

    (j, m)
}

fn xc(a: &str) -> [u8; 16] {
    let b = a.as_bytes();
    let mut c = [0_u8; 16];
    c[..8].copy_from_slice(&b[..8]);
    c[8..].copy_from_slice(&b[b.len() - 8..]);
    c
}

fn xd<R: Rng + ?Sized>(a: &[u8], b: &[u8; 16], c: &mut R) -> Vec<u8> {
    let d = (8 - (a.len() + 10) % 8) % 8;
    let mut e = Vec::with_capacity(a.len() + d + 10);
    e.push((c.random::<u8>() & 0xf8) | d as u8);
    for _ in 0..d {
        e.push(c.random());
    }
    e.push(c.random());
    e.push(c.random());
    e.extend_from_slice(a);
    e.extend_from_slice(&[0; 7]);

    let mut f = Vec::with_capacity(e.len());
    let mut g = [0_u8; 8];
    let mut h = [0_u8; 8];
    for i in e.chunks_exact(8) {
        let mut j = [0_u8; 8];
        for k in 0..8 {
            j[k] = i[k] ^ g[k];
        }

        let l = xe(j, b);
        let mut m = [0_u8; 8];
        for n in 0..8 {
            m[n] = l[n] ^ h[n];
        }
        f.extend_from_slice(&m);
        g = m;
        h = j;
    }
    f
}

fn xe(a: [u8; 8], b: &[u8; 16]) -> [u8; 8] {
    let c = [
        u32::from_be_bytes(b[0..4].try_into().expect("e7")),
        u32::from_be_bytes(b[4..8].try_into().expect("e7")),
        u32::from_be_bytes(b[8..12].try_into().expect("e7")),
        u32::from_be_bytes(b[12..16].try_into().expect("e7")),
    ];
    let mut d = u32::from_be_bytes(a[0..4].try_into().expect("e8"));
    let mut e = u32::from_be_bytes(a[4..8].try_into().expect("e8"));
    let mut f = 0_u32;

    for _ in 0..16 {
        f = f.wrapping_add(KG);
        d = d.wrapping_add(
            f.wrapping_add(e) ^ c[0].wrapping_add(e << 4) ^ c[1].wrapping_add(e >> 5),
        );
        e = e.wrapping_add(
            f.wrapping_add(d) ^ c[2].wrapping_add(d << 4) ^ c[3].wrapping_add(d >> 5),
        );
    }

    let mut g = [0_u8; 8];
    g[..4].copy_from_slice(&d.to_be_bytes());
    g[4..].copy_from_slice(&e.to_be_bytes());
    g
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

//sign生成器
pub fn x9(a: &str) -> String {
    let b = hex::encode_upper(Sha1::digest(a));
    let c = b.as_bytes();

    let d: String = K7
        .into_iter()
        .filter(|&e| e < c.len())
        .map(|e| c[e] as char)
        .collect();
    let e: String = K8.into_iter().map(|f| c[f] as char).collect();

    let mut f = [0_u8; 20];
    for (g, &h) in K9.iter().enumerate() {
        let i = xf(c[g * 2]);
        let j = xf(c[g * 2 + 1]);
        f[g] = h ^ ((i << 4) | j);
    }

    let g: String = base64::engine::general_purpose::STANDARD
        .encode(f)
        .chars()
        .filter(|h| !matches!(h, '/' | '\\' | '+' | '='))
        .collect();

    format!("zzc{d}{g}{e}").to_ascii_lowercase()
}

fn xf(a: u8) -> u8 {
    match a {
        b'0'..=b'9' => a - b'0',
        b'a'..=b'f' => a - b'a' + 10,
        b'A'..=b'F' => a - b'A' + 10,
        _ => unreachable!("e9"),
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
