#![allow(dead_code)]
// These crypto helpers are intentionally kept as forward-compatible utilities and
// will be enabled as more sidecar features migrate to Rust.

pub mod common;
pub mod kugou;
pub mod netease;
pub mod qq;
pub mod soda;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDecryptResult {
    pub data: Vec<u8>,
    pub content_type: String,
}

#[allow(unused_imports)]
pub use common::{
    AesMode, CipherOutputFormat, decrypt_aes, encrypt_aes, encrypt_rsa, from_hex, to_hex_lower,
    to_hex_upper,
};
#[allow(unused_imports)]
pub use kugou::{
    decrypt_krc, decrypt_kugou_register_payload, encrypt_kugou_register_payload,
    encrypt_kugou_register_rsa,
};
#[allow(unused_imports)]
pub use netease::{
    EapiBody, EapiParams, EapiReqDecrypted, LinuxapiParams, WeapiParams, decrypt_eapi,
    decrypt_eapi_request, decrypt_eapi_response, encrypt_eapi, encrypt_linuxapi, encrypt_weapi,
    encrypt_weapi_rsa, generate_weapi_secret_key,
};
#[allow(unused_imports)]
pub use qq::{decrypt_qq_audio, decrypt_qrc, decrypt_qrc_file};
#[allow(unused_imports)]
pub use soda::decrypt_soda_audio;
