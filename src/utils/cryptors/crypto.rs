use aes::Aes128;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyInit, KeyIvInit, block_padding::Pkcs7};
use anyhow::{Context, anyhow};
use base64::{Engine as _, engine::general_purpose};
use rsa::{BigUint, RsaPublicKey, pkcs8::DecodePublicKey, traits::PublicKeyParts};

const RSA_BLOCK_SIZE: usize = 128;

type Aes128CbcEnc = cbc::Encryptor<Aes128>;
type Aes128CbcDec = cbc::Decryptor<Aes128>;
type Aes128EcbEnc = ecb::Encryptor<Aes128>;
type Aes128EcbDec = ecb::Decryptor<Aes128>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AesMode {
    Cbc,
    Ecb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CipherOutputFormat {
    Base64,
    Hex,
}

pub fn encrypt_aes(
    text: &str,
    mode: AesMode,
    key: &str,
    iv: &str,
    format: CipherOutputFormat,
) -> anyhow::Result<String> {
    let encrypted = match mode {
        AesMode::Cbc => encrypt_cbc(text.as_bytes(), key.as_bytes(), iv.as_bytes())?,
        AesMode::Ecb => encrypt_ecb(text.as_bytes(), key.as_bytes())?,
    };

    Ok(match format {
        CipherOutputFormat::Hex => to_hex_upper(&encrypted),
        CipherOutputFormat::Base64 => general_purpose::STANDARD.encode(encrypted),
    })
}

pub fn decrypt_aes(
    ciphertext: &str,
    mode: AesMode,
    key: &str,
    iv: &str,
    format: CipherOutputFormat,
) -> anyhow::Result<Vec<u8>> {
    let encrypted = match format {
        CipherOutputFormat::Hex => from_hex(ciphertext)?,
        CipherOutputFormat::Base64 => general_purpose::STANDARD
            .decode(ciphertext)
            .context("invalid base64 ciphertext")?,
    };

    match mode {
        AesMode::Cbc => decrypt_cbc(&encrypted, key.as_bytes(), iv.as_bytes()),
        AesMode::Ecb => decrypt_ecb(&encrypted, key.as_bytes()),
    }
}

pub fn encrypt_rsa(plaintext: &str, public_key: &str) -> anyhow::Result<String> {
    let public_key = parse_rsa_public_key(public_key)?;
    let mut padded = [0u8; RSA_BLOCK_SIZE];
    let bytes = plaintext.as_bytes();
    if bytes.len() > RSA_BLOCK_SIZE {
        return Err(anyhow!("rsa plaintext is longer than block size"));
    }

    padded[RSA_BLOCK_SIZE - bytes.len()..].copy_from_slice(bytes);
    let message = BigUint::from_bytes_be(&padded);
    let encrypted = message.modpow(public_key.e(), public_key.n());
    let mut encrypted_bytes = encrypted.to_bytes_be();
    let key_size = public_key.size();
    if encrypted_bytes.len() > key_size {
        return Err(anyhow!("rsa encrypted block is longer than key size"));
    }

    let mut output = vec![0u8; key_size];
    output[key_size - encrypted_bytes.len()..].copy_from_slice(&encrypted_bytes);
    encrypted_bytes.fill(0);
    Ok(to_hex_lower(&output))
}

pub fn to_hex_upper(bytes: &[u8]) -> String {
    encode_hex(bytes, b"0123456789ABCDEF")
}

pub fn to_hex_lower(bytes: &[u8]) -> String {
    encode_hex(bytes, b"0123456789abcdef")
}

pub fn from_hex(text: &str) -> anyhow::Result<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return Err(anyhow!("hex ciphertext length must be even"));
    }

    let mut output = Vec::with_capacity(text.len() / 2);
    for chunk in text.as_bytes().chunks_exact(2) {
        let high = hex_value(chunk[0])?;
        let low = hex_value(chunk[1])?;
        output.push((high << 4) | low);
    }
    Ok(output)
}

fn encrypt_cbc(plaintext: &[u8], key: &[u8], iv: &[u8]) -> anyhow::Result<Vec<u8>> {
    ensure_aes_key(key)?;
    ensure_aes_iv(iv)?;
    let mut output = vec![0u8; plaintext.len() + 16];
    let encrypted = Aes128CbcEnc::new(key.into(), iv.into())
        .encrypt_padded_b2b_mut::<Pkcs7>(plaintext, &mut output)
        .map_err(|err| anyhow!("aes cbc encrypt failed: {err}"))?;
    Ok(encrypted.to_vec())
}

fn decrypt_cbc(ciphertext: &[u8], key: &[u8], iv: &[u8]) -> anyhow::Result<Vec<u8>> {
    ensure_aes_key(key)?;
    ensure_aes_iv(iv)?;
    let mut output = vec![0u8; ciphertext.len()];
    let decrypted = Aes128CbcDec::new(key.into(), iv.into())
        .decrypt_padded_b2b_mut::<Pkcs7>(ciphertext, &mut output)
        .map_err(|err| anyhow!("aes cbc decrypt failed: {err}"))?;
    Ok(decrypted.to_vec())
}

fn encrypt_ecb(plaintext: &[u8], key: &[u8]) -> anyhow::Result<Vec<u8>> {
    ensure_aes_key(key)?;
    let mut output = vec![0u8; plaintext.len() + 16];
    let encrypted = Aes128EcbEnc::new(key.into())
        .encrypt_padded_b2b_mut::<Pkcs7>(plaintext, &mut output)
        .map_err(|err| anyhow!("aes ecb encrypt failed: {err}"))?;
    Ok(encrypted.to_vec())
}

fn decrypt_ecb(ciphertext: &[u8], key: &[u8]) -> anyhow::Result<Vec<u8>> {
    ensure_aes_key(key)?;
    let mut output = vec![0u8; ciphertext.len()];
    let decrypted = Aes128EcbDec::new(key.into())
        .decrypt_padded_b2b_mut::<Pkcs7>(ciphertext, &mut output)
        .map_err(|err| anyhow!("aes ecb decrypt failed: {err}"))?;
    Ok(decrypted.to_vec())
}

fn parse_rsa_public_key(public_key: &str) -> anyhow::Result<RsaPublicKey> {
    if let Ok(parsed) = RsaPublicKey::from_public_key_pem(public_key) {
        return Ok(parsed);
    }

    let base64_body: String = public_key
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("-----"))
        .collect();
    let der = general_purpose::STANDARD
        .decode(base64_body)
        .context("invalid rsa public key")?;
    RsaPublicKey::from_public_key_der(&der).context("invalid rsa public key")
}

fn ensure_aes_key(key: &[u8]) -> anyhow::Result<()> {
    if key.len() == 16 {
        Ok(())
    } else {
        Err(anyhow!("aes-128 key must be 16 bytes"))
    }
}

fn ensure_aes_iv(iv: &[u8]) -> anyhow::Result<()> {
    if iv.len() == 16 {
        Ok(())
    } else {
        Err(anyhow!("aes cbc iv must be 16 bytes"))
    }
}

fn encode_hex(bytes: &[u8], alphabet: &[u8; 16]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(alphabet[(byte >> 4) as usize] as char);
        output.push(alphabet[(byte & 0x0f) as usize] as char);
    }
    output
}

fn hex_value(byte: u8) -> anyhow::Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(anyhow!("invalid hex character")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aes_cbc_round_trips_base64() {
        let encrypted = encrypt_aes(
            r#"{"hello":"world"}"#,
            AesMode::Cbc,
            "0CoJUm6Qyw8W8jud",
            "0102030405060708",
            CipherOutputFormat::Base64,
        )
        .unwrap();
        let decrypted = decrypt_aes(
            &encrypted,
            AesMode::Cbc,
            "0CoJUm6Qyw8W8jud",
            "0102030405060708",
            CipherOutputFormat::Base64,
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(decrypted).unwrap(),
            r#"{"hello":"world"}"#
        );
    }
}
