#![allow(dead_code)]
// These crypto helpers are intentionally kept as forward-compatible utilities and
// will be enabled as more sidecar features migrate to Rust.

pub mod crypto;
pub mod krc;
pub mod netease;
pub mod qrc;

#[allow(unused_imports)]
pub use crypto::{
    AesMode, CipherOutputFormat, decrypt_aes, encrypt_aes, encrypt_rsa, from_hex, to_hex_lower,
    to_hex_upper,
};
#[allow(unused_imports)]
pub use krc::decrypt_krc;
#[allow(unused_imports)]
pub use netease::{
    EapiBody, EapiParams, EapiReqDecrypted, LinuxapiParams, WeapiParams, decrypt_eapi,
    decrypt_eapi_request, decrypt_eapi_response, encrypt_eapi, encrypt_linuxapi, encrypt_weapi,
    encrypt_weapi_rsa, generate_weapi_secret_key,
};
#[allow(unused_imports)]
pub use qrc::{decrypt_qrc, decrypt_qrc_file};
