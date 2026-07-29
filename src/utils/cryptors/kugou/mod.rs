pub mod auth;
pub mod lyric;

pub use auth::{
    decrypt_kugou_register_payload, encrypt_kugou_register_payload, encrypt_kugou_register_rsa,
};
pub use lyric::decrypt_krc;
