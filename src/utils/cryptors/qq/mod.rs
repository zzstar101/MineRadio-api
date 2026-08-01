use std::time::{SystemTime, UNIX_EPOCH};

use aes_gcm::{
    Aes128Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use base64::Engine;
use rand::{Rng, RngExt};
use sha1::{Digest, Sha1};

pub mod audio;
pub mod lyric;

#[allow(unused_imports)]
pub use audio::{
    EncryptedTail, TailFormat, decrypt_file, derive_qmc2_key, derive_qmc2_key_from_ekey,
    parse_encrypted_tail, qmc2_decrypt_in_place,
};
pub use lyric::{decrypt_qrc, decrypt_qrc_file};

const GUID_CHARSET: &[u8] = b"ABCDEF1234567890";
const HASH33_MASK: u64 = 2_147_483_647;
const HASH33_INIT: u64 = 5_381;
const SEARCH_ID_E_BASE: u64 = 18_014_398_509_481_984;
const SEARCH_ID_N_BASE: u64 = 4_294_967_296;
const SEARCH_ID_N_MAX: u64 = 4_194_304;
const DAY_MILLIS: u64 = 24 * 60 * 60 * 1000;
const SIGN_PART_1_INDEXES: [usize; 8] = [23, 14, 6, 36, 16, 40, 7, 19];
const SIGN_PART_2_INDEXES: [usize; 8] = [16, 1, 32, 12, 19, 27, 8, 5];
const SIGN_SCRAMBLE_VALUES: [u8; 20] = [
    89, 39, 179, 150, 218, 82, 58, 252, 177, 52, 186, 123, 120, 64, 242, 133, 143, 161, 121, 179,
];
const AG1_RESP_KEY: [u8; 21] = [
    122, 63, 140, 29, 94, 155, 47, 10, 108, 77, 126, 139, 31, 58, 92, 157, 14, 43, 111, 74, 129,
];
const AG1_REQ_KEY: [u8; 16] = [
    189, 48, 95, 16, 208, 255, 116, 182, 239, 84, 218, 184, 53, 181, 225, 207,
];
const AG1_IV_LEN: usize = 12;

pub fn encode_ag1_req(data: &str) -> Result<Vec<u8>, String> {
    let mut iv = [0_u8; AG1_IV_LEN];
    rand::rng().fill_bytes(&mut iv);

    let cipher = Aes128Gcm::new_from_slice(&AG1_REQ_KEY)
        .map_err(|_| "invalid AG1 request key".to_owned())?;
    let encrypted = cipher
        .encrypt(Nonce::from_slice(&iv), data.as_bytes())
        .map_err(|_| "failed to encrypt AG1 request".to_owned())?;

    let mut output = Vec::with_capacity(iv.len() + encrypted.len());
    output.extend_from_slice(&iv);
    output.extend_from_slice(&encrypted);
    Ok(output)
}

pub fn decode_ag1_req(data: impl AsRef<[u8]>) -> Result<String, String> {
    let data = data.as_ref();
    let (iv, encrypted) = data
        .split_at_checked(AG1_IV_LEN)
        .ok_or_else(|| "AG1 request is missing its IV".to_owned())?;

    let cipher = Aes128Gcm::new_from_slice(&AG1_REQ_KEY)
        .map_err(|_| "invalid AG1 request key".to_owned())?;
    let decrypted = cipher
        .decrypt(Nonce::from_slice(iv), encrypted)
        .map_err(|_| "failed to decrypt AG1 request".to_owned())?;

    String::from_utf8(decrypted).map_err(|_| "AG1 request is not valid UTF-8".to_owned())
}

pub fn encode_ag1_resp(data: &str) -> Vec<u8> {
    xor_ag1_resp(data.as_bytes())
}

pub fn decode_ag1_resp(data: impl AsRef<[u8]>) -> Result<String, String> {
    String::from_utf8(xor_ag1_resp(data.as_ref()))
        .map_err(|_| "AG1 response is not valid UTF-8".to_owned())
}

fn xor_ag1_resp(data: &[u8]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(index, byte)| byte ^ AG1_RESP_KEY[index % AG1_RESP_KEY.len()])
        .collect()
}

pub fn get_guid() -> String {
    let mut rng = rand::rng();
    (0..32)
        .map(|_| {
            let idx = rng.random_range(0..GUID_CHARSET.len());
            GUID_CHARSET[idx] as char
        })
        .collect()
}

pub fn hash33(s: &str) -> u64 {
    let mut h = HASH33_INIT;
    for c in s.chars() {
        h = h.wrapping_mul(33).wrapping_add(c as u64);
    }
    h & HASH33_MASK
}

pub fn gtk_from_pskey(input: &str) -> u64 {
    input.bytes().fold(HASH33_INIT, |hash, byte| {
        hash.wrapping_add(hash << 5).wrapping_add(byte as u64)
    }) & HASH33_MASK
}

pub fn get_search_id() -> String {
    let mut rng = rand::rng();
    let e = rng.random_range(1_u64..=20);
    let t = e * SEARCH_ID_E_BASE;
    let n = rng.random_range(0_u64..=SEARCH_ID_N_MAX) * SEARCH_ID_N_BASE;
    let r = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        % DAY_MILLIS as u128) as u64;
    (t + n + r).to_string()
}

//sign生成器
pub fn sign(payload: &str) -> String {
    let hash = hex::encode_upper(Sha1::digest(payload));
    let hash_bytes = hash.as_bytes();

    let part1: String = SIGN_PART_1_INDEXES
        .into_iter()
        .filter(|&idx| idx < hash_bytes.len())
        .map(|idx| hash_bytes[idx] as char)
        .collect();
    let part2: String = SIGN_PART_2_INDEXES
        .into_iter()
        .map(|idx| hash_bytes[idx] as char)
        .collect();

    let mut scrambled = [0_u8; 20];
    for (i, &value) in SIGN_SCRAMBLE_VALUES.iter().enumerate() {
        let hi = decode_hex_nibble(hash_bytes[i * 2]);
        let lo = decode_hex_nibble(hash_bytes[i * 2 + 1]);
        scrambled[i] = value ^ ((hi << 4) | lo);
    }

    let b64_part: String = base64::engine::general_purpose::STANDARD
        .encode(scrambled)
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | '+' | '='))
        .collect();

    format!("zzc{part1}{b64_part}{part2}").to_ascii_lowercase()
}

fn decode_hex_nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        b'A'..=b'F' => value - b'A' + 10,
        _ => unreachable!("sha1 hex only contains [0-9a-fA-F]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_has_expected_format() {
        let guid = get_guid();
        assert_eq!(guid.len(), 32);
        assert!(guid.chars().all(|c| matches!(c, 'A'..='F' | '0'..='9')));
    }

    #[test]
    fn hash33_matches_known_values() {
        assert_eq!(hash33(""), 5_381);
        assert_eq!(hash33("a"), 177_670);
        assert_eq!(hash33("abc"), 193_485_963);
        assert_eq!(hash33("腾讯"), 6_989_618);
        assert_eq!(hash33("hello"), 261_238_937);
    }

    #[test]
    fn search_id_is_numeric_and_in_expected_range() {
        let search_id = get_search_id();
        assert!(!search_id.is_empty());
        assert!(search_id.chars().all(|c| c.is_ascii_digit()));

        let value: u64 = search_id.parse().expect("search_id should parse to u64");
        let min = SEARCH_ID_E_BASE;
        let max = (20 * SEARCH_ID_E_BASE) + (SEARCH_ID_N_MAX * SEARCH_ID_N_BASE) + (DAY_MILLIS - 1);
        assert!(value >= min);
        assert!(value <= max);
    }

    #[test]
    fn qq_sign_matches_known_value() {
        let body = serde_json::json!({
            "foo": "bar",
            "num": 1
        });
        assert_eq!(
            sign(&serde_json::to_string(&body).expect("压缩失败")),
            "zzcf3ea51dcp3xdwnxisjgufsk0znclehf2t85bc1d3d4"
        );
    }

    #[test]
    fn ag1_req_round_trip() {
        let encoded = encode_ag1_req("AG1 request").expect("AG1 request should encrypt");

        assert_eq!(encoded.len(), AG1_IV_LEN + "AG1 request".len() + 16);
        assert_eq!(decode_ag1_req(&encoded), Ok("AG1 request".to_owned()));
    }

    #[test]
    fn ag1_req_rejects_tampered_data() {
        let mut encoded = encode_ag1_req("AG1 request").expect("AG1 request should encrypt");
        *encoded.last_mut().expect("encrypted request has a tag") ^= 1;

        assert!(decode_ag1_req(encoded).is_err());
    }

    #[test]
    fn ag1_resp_round_trip() {
        let encoded = encode_ag1_resp("AG1 response");

        assert_eq!(decode_ag1_resp(&encoded), Ok("AG1 response".to_owned()));
    }
}
