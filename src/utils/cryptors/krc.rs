use std::io::Read;

use anyhow::{Context, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use flate2::read::{DeflateDecoder, ZlibDecoder};

const KRC_KEY: &[u8] = &[
    0x40, 0x47, 0x61, 0x77, 0x5e, 0x32, 0x74, 0x47, 0x51, 0x36, 0x31, 0x2d, 0xce, 0xd2, 0x6e, 0x69,
];

pub fn decrypt_krc(encoded: &str) -> Result<String, String> {
    let clean: String = encoded
        .chars()
        .filter(|char| !char.is_whitespace())
        .collect();
    let decoded = BASE64
        .decode(&clean)
        .with_context(|| format!("failed to base64 decode krc payload, len={}", clean.len()))
        .map_err(|err| err.to_string())?;
    if decoded.len() <= 4 {
        return Err(format!("decoded krc data too short: {} bytes", decoded.len()));
    }

    let mut data = decoded[4..].to_vec();
    xor_krc_key(&mut data);
    let inflated = inflate_krc_payload(&data).map_err(|err| err.to_string())?;
    let skip = if inflated.starts_with(&[0xEF, 0xBB, 0xBF]) {
        3
    } else {
        1
    };
    if inflated.len() <= skip {
        return Err(format!(
            "inflated krc data too short after header skip({skip}): {} bytes",
            inflated.len()
        ));
    }

    String::from_utf8(inflated[skip..].to_vec())
        .context("krc payload is not utf-8")
        .map_err(|err| err.to_string())
}

fn xor_krc_key(data: &mut [u8]) {
    for (index, byte) in data.iter_mut().enumerate() {
        *byte ^= KRC_KEY[index % KRC_KEY.len()];
    }
}

fn inflate_krc_payload(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut zlib_output = Vec::new();
    if ZlibDecoder::new(data).read_to_end(&mut zlib_output).is_ok() && !zlib_output.is_empty() {
        return Ok(zlib_output);
    }

    let mut deflate_output = Vec::new();
    DeflateDecoder::new(data)
        .read_to_end(&mut deflate_output)
        .with_context(|| {
            format!(
                "failed to inflate krc payload (xor_head={})",
                hex_head(data)
            )
        })?;
    if deflate_output.is_empty() {
        return Err(anyhow!(
            "inflating krc payload produced empty output (xor_head={})",
            hex_head(data)
        ));
    }
    Ok(deflate_output)
}

fn hex_head(data: &[u8]) -> String {
    data.iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::{Compression, write::ZlibEncoder};

    use super::*;

    #[test]
    fn krc_decrypt_decodes_zlib_payload() {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"\0[00:00.00]hello").unwrap();
        let mut compressed = encoder.finish().unwrap();
        xor_krc_key(&mut compressed);

        let mut payload = b"krc1".to_vec();
        payload.extend_from_slice(&compressed);
        let encoded = BASE64.encode(payload);

        assert_eq!(decrypt_krc(&encoded).unwrap(), "[00:00.00]hello");
    }
}
