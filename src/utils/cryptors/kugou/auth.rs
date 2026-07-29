use md5::{Digest, Md5};
use rsa::Pkcs1v15Encrypt;

use super::super::common::{
    AesMode, CipherOutputFormat, decrypt_aes, encrypt_aes, parse_rsa_public_key, to_hex_lower,
};

const KUGOU_REGISTER_PUBLIC_KEY: &str = "-----BEGIN PUBLIC KEY-----\nMIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDIAG7QOELSYoIJvTFJhMpe1s/gbjDJX51HBNnEl5HXqTW6lQ7LC8jr9fWZTwusknp+sVGzwd40MwP6U5yDE27M/X1+UR4tvOGOqp94TJtQ1EPnWGWXngpeIW5GxoQGao1rmYWAu6oi1z9XkChrsUdC6DJE5E221wf/4WLFxwAtRQIDAQAB\n-----END PUBLIC KEY-----";

pub fn encrypt_kugou_register_payload(data: &str, random_key: &str) -> Result<String, String> {
    let digest = to_hex_lower(&Md5::digest(random_key.as_bytes()));
    encrypt_aes(
        data,
        AesMode::Cbc,
        &digest[..16],
        &digest[16..],
        CipherOutputFormat::Base64,
    )
}

pub fn decrypt_kugou_register_payload(
    base64_ciphertext: &str,
    random_key: &str,
) -> Result<Vec<u8>, String> {
    let digest = to_hex_lower(&Md5::digest(random_key.as_bytes()));
    decrypt_aes(
        base64_ciphertext,
        AesMode::Cbc,
        &digest[..16],
        &digest[16..],
        CipherOutputFormat::Base64,
    )
}

pub fn encrypt_kugou_register_rsa(plaintext: &str) -> Result<String, String> {
    let public_key = parse_rsa_public_key(KUGOU_REGISTER_PUBLIC_KEY)
        .map_err(|err| format!("invalid Kugou register RSA public key: {err}"))?;
    let encrypted = public_key
        .encrypt(
            &mut rsa::rand_core::OsRng,
            Pkcs1v15Encrypt,
            plaintext.as_bytes(),
        )
        .map_err(|err| format!("Kugou register RSA encryption failed: {err}"))?;
    Ok(to_hex_lower(&encrypted))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_payload_round_trips() {
        let encrypted = encrypt_kugou_register_payload("{\"uuid\":\"demo\"}", "abc123").unwrap();
        let decrypted = decrypt_kugou_register_payload(&encrypted, "abc123").unwrap();

        assert_eq!(String::from_utf8(decrypted).unwrap(), "{\"uuid\":\"demo\"}");
    }

    #[test]
    fn register_rsa_uses_a_1024_bit_block() {
        let encrypted =
            encrypt_kugou_register_rsa(r#"{"aes":"abc123","uid":0,"token":""}"#).unwrap();

        assert_eq!(encrypted.len(), 256);
    }
}
