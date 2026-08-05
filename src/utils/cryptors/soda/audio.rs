use aes::Aes128;
use ctr::cipher::{KeyIvInit, StreamCipher};

use crate::api::{ApiError, ApiErrorCode, ApiResult};

use super::super::AudioDecryptResult;

type Aes128Ctr64BE = ctr::Ctr64BE<Aes128>;

const ENCA_BYTES: &[u8] = b"enca";
const MP4A_BYTES: &[u8] = b"mp4a";
const SPADE_PREFIX: [u8; 2] = [0xfa, 0x55];

pub fn decrypt_soda_audio(file_data: Vec<u8>, play_auth: &str) -> ApiResult<AudioDecryptResult> {
    decrypt_soda_audio_data_inner(file_data, play_auth)
}

fn concat_bytes(parts: &[&[u8]]) -> Vec<u8> {
    let total = parts.iter().map(|part| part.len()).sum();
    let mut out = Vec::with_capacity(total);
    for part in parts {
        out.extend_from_slice(part);
    }
    out
}

fn read_u32_be(data: &[u8], offset: usize) -> Option<u32> {
    data.get(offset..offset + 4)
        .map(|bytes| u32::from_be_bytes(bytes.try_into().unwrap()))
}

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    let normalized = hex.trim();
    (0..normalized.len() / 2)
        .filter_map(|index| u8::from_str_radix(&normalized[index * 2..index * 2 + 2], 16).ok())
        .collect()
}

fn index_of_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn sum_sample_sizes(sample_sizes: &[u32]) -> u32 {
    sample_sizes.iter().sum()
}

fn decrypt_aes_ctr(data: &[u8], key_bytes: &[u8], iv: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut out = data.to_vec();
    let mut cipher = Aes128Ctr64BE::new_from_slices(key_bytes, iv)?;
    cipher.apply_keystream(&mut out);
    Ok(out)
}

struct SpadeDecryptor;

impl SpadeDecryptor {
    fn bit_count(value: u32) -> u32 {
        value.count_ones()
    }

    fn decode_base36(value: u8) -> u8 {
        match value {
            b'0'..=b'9' => value - b'0',
            b'a'..=b'z' => value - b'a' + 10,
            _ => 0xff,
        }
    }

    fn decrypt_spade_inner(spade_key_bytes: &[u8]) -> Vec<u8> {
        let mut result = Vec::with_capacity(spade_key_bytes.len());
        let buff = concat_bytes(&[&SPADE_PREFIX, spade_key_bytes]);
        for (index, byte) in spade_key_bytes.iter().enumerate() {
            let raw = (*byte ^ buff[index])
                .wrapping_sub(Self::bit_count(index as u32) as u8)
                .wrapping_sub(21);
            result.push(raw);
        }
        result
    }

    fn extract_key(play_auth: &str) -> Option<String> {
        let bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, play_auth).ok()?;
        if bytes.len() < 3 {
            return None;
        }
        let padding_length = (bytes[0] ^ bytes[1] ^ bytes[2]) as isize - 48;
        if (bytes.len() as isize) < padding_length + 2 {
            return None;
        }
        let spade_end =
            normalize_js_subarray_index(bytes.len(), bytes.len() as isize - padding_length);
        let tmp_buff = if 1 > spade_end {
            Vec::new()
        } else {
            Self::decrypt_spade_inner(&bytes[1..spade_end])
        };
        if tmp_buff.is_empty() {
            return None;
        }
        let end_index = 1 + (bytes.len() as isize - padding_length - 2)
            - Self::decode_base36(tmp_buff[0]) as isize;
        let key_end = normalize_js_subarray_index(tmp_buff.len(), end_index);
        let key_bytes = if 1 > key_end {
            Vec::new()
        } else {
            tmp_buff[1..key_end].to_vec()
        };
        String::from_utf8(key_bytes).ok()
    }
}

fn normalize_js_subarray_index(length: usize, index: isize) -> usize {
    let length = length as isize;
    let normalized = if index < 0 { length + index } else { index };
    normalized.clamp(0, length) as usize
}

#[derive(Clone)]
struct Mp4Box {
    offset: usize,
    size: usize,
    data: Vec<u8>,
}

fn find_box(data: &[u8], box_type: &str, start: usize, end: usize) -> Option<Mp4Box> {
    let mut position = start;
    let end = end.min(data.len());
    while position + 8 <= end {
        let size = read_u32_be(data, position)? as usize;
        if size < 8 || position + size > data.len() {
            break;
        }
        let current_type = std::str::from_utf8(data.get(position + 4..position + 8)?).ok()?;
        if current_type == box_type {
            return Some(Mp4Box {
                offset: position,
                size,
                data: data[position + 8..position + size].to_vec(),
            });
        }
        position += size;
    }
    None
}

fn decrypt_soda_audio_data_inner(
    file_data: Vec<u8>,
    play_auth: &str,
) -> ApiResult<AudioDecryptResult> {
    let Some(hex_key) = SpadeDecryptor::extract_key(play_auth) else {
        return not_decrypted(file_data, "playAuth key extraction failed");
    };
    if hex_key.is_empty() {
        return not_decrypted(file_data, "playAuth key extraction failed");
    }

    let Some(moov) = find_box(&file_data, "moov", 0, file_data.len()) else {
        return not_decrypted(file_data, "moov box not found");
    };
    let mut senc = find_box(&file_data, "senc", moov.offset + 8, moov.offset + moov.size);
    let Some(trak) = find_box(&file_data, "trak", moov.offset + 8, moov.offset + moov.size) else {
        return not_decrypted(file_data, "trak box not found");
    };
    let Some(mdia) = find_box(&file_data, "mdia", trak.offset + 8, trak.offset + trak.size) else {
        return not_decrypted(file_data, "mdia box not found");
    };
    let Some(minf) = find_box(&file_data, "minf", mdia.offset + 8, mdia.offset + mdia.size) else {
        return not_decrypted(file_data, "minf box not found");
    };
    let Some(stbl) = find_box(&file_data, "stbl", minf.offset + 8, minf.offset + minf.size) else {
        return not_decrypted(file_data, "stbl box not found");
    };
    let Some(stsz) = find_box(&file_data, "stsz", stbl.offset + 8, stbl.offset + stbl.size) else {
        return not_decrypted(file_data, "stsz box not found");
    };
    let Some(mdat) = find_box(&file_data, "mdat", 0, file_data.len()) else {
        return not_decrypted(file_data, "mdat box not found");
    };
    let mdat_payload_size = (mdat.size - 8) as u32;

    if stsz.data.len() < 12 {
        return not_decrypted(file_data, "stsz box is truncated");
    }
    let sample_size_fixed = read_u32_be(&stsz.data, 4).unwrap_or(0);
    let sample_count = read_u32_be(&stsz.data, 8).unwrap_or(0);
    if sample_size_fixed != 0 && sample_size_fixed.saturating_mul(sample_count) != mdat_payload_size
    {
        return not_decrypted(file_data, "sample size table does not match mdat payload");
    }
    if sample_size_fixed == 0 && stsz.data.len() < 12 + sample_count as usize * 4 {
        return not_decrypted(file_data, "stsz sample table is truncated");
    }
    let sample_sizes = if sample_size_fixed != 0 {
        vec![sample_size_fixed; sample_count as usize]
    } else {
        (0..sample_count as usize)
            .filter_map(|index| read_u32_be(&stsz.data, 12 + index * 4))
            .collect::<Vec<_>>()
    };
    if sample_size_fixed == 0 && sum_sample_sizes(&sample_sizes) != mdat_payload_size {
        return not_decrypted(file_data, "sample size table does not match mdat payload");
    }

    if senc.is_none() {
        senc = find_box(&file_data, "senc", stbl.offset + 8, stbl.offset + stbl.size);
    }
    let Some(senc) = senc else {
        return not_decrypted(file_data, "senc box not found");
    };
    if senc.data.len() < 8 {
        return not_decrypted(file_data, "senc box is truncated");
    }
    let senc_flags = read_u32_be(&senc.data, 0).unwrap_or(0) & 0x00ff_ffff;
    let senc_sample_count = read_u32_be(&senc.data, 4).unwrap_or(0);
    if (senc_flags & 0x02) != 0 {
        return not_decrypted(
            file_data,
            "soda audio subsample encryption is not supported",
        );
    }

    let mut ivs = Vec::new();
    let mut senc_ptr = 8;
    for _ in 0..senc_sample_count {
        if senc_ptr + 8 > senc.data.len() {
            return not_decrypted(file_data, "senc IV table is truncated");
        }
        let mut iv = Vec::with_capacity(16);
        iv.extend_from_slice(&senc.data[senc_ptr..senc_ptr + 8]);
        iv.extend_from_slice(&[0; 8]);
        ivs.push(iv);
        senc_ptr += 8;
    }

    let key_bytes = hex_to_bytes(&hex_key);
    let mut decrypted_mdat = Vec::new();
    let mut read_ptr = mdat.offset + 8;
    for (index, sample_size) in sample_sizes.iter().enumerate() {
        let sample_size = *sample_size as usize;
        let Some(sample) = file_data.get(read_ptr..read_ptr + sample_size) else {
            return not_decrypted(file_data, "sample size table does not match mdat payload");
        };
        if let Some(iv) = ivs.get(index) {
            match decrypt_aes_ctr(sample, &key_bytes, iv) {
                Ok(decrypted) => decrypted_mdat.extend_from_slice(&decrypted),
                Err(_) => return not_decrypted(file_data, "soda audio decrypt failed"),
            }
        } else {
            decrypted_mdat.extend_from_slice(sample);
        }
        read_ptr += sample_size;
    }

    if decrypted_mdat.len() != mdat.size - 8 {
        return not_decrypted(file_data, "sample size table does not match mdat payload");
    }
    let mut output = file_data;
    output[mdat.offset + 8..mdat.offset + 8 + decrypted_mdat.len()]
        .copy_from_slice(&decrypted_mdat);

    if let Some(stsd) = find_box(&output, "stsd", stbl.offset + 8, stbl.offset + stbl.size) {
        let original_stsd = &output[stsd.offset..stsd.offset + stsd.size];
        if let Some(enca_index) = index_of_bytes(original_stsd, ENCA_BYTES) {
            output[stsd.offset + enca_index..stsd.offset + enca_index + 4]
                .copy_from_slice(MP4A_BYTES);
        }
    }

    Ok(AudioDecryptResult {
        data: output,
        content_type: "audio/mp4".to_owned(),
    })
}

fn not_decrypted(_data: Vec<u8>, reason: &str) -> ApiResult<AudioDecryptResult> {
    let code = if reason == "soda audio subsample encryption is not supported" {
        ApiErrorCode::Unavailable
    } else {
        ApiErrorCode::BadRequest
    };
    Err(ApiError::new(code, reason))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_input_returns_bad_request() {
        let error = decrypt_soda_audio(Vec::new(), "invalid play auth").unwrap_err();

        assert_eq!(error.code, ApiErrorCode::BadRequest);
    }
}
