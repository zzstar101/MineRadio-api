use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use base64::Engine;

const MUSICEX_MAGIC: &[u8; 8] = b"musicex\0";
const QTAG_MAGIC: &[u8; 4] = b"QTag";
const STAG_MAGIC: &[u8; 4] = b"STag";
const ENCV2_PREFIX: &[u8] = b"QQMusic EncV2,Key:";
const ENCV2_KEY1: [u8; 16] = [
    0x33, 0x38, 0x36, 0x5a, 0x4a, 0x59, 0x21, 0x40, 0x23, 0x2a, 0x24, 0x25, 0x5e, 0x26, 0x29, 0x28,
];
const ENCV2_KEY2: [u8; 16] = [
    0x2a, 0x24, 0x25, 0x5e, 0x26, 0x29, 0x28, 0x23, 0x40, 0x21, 0x33, 0x38, 0x36, 0x5a, 0x4a, 0x59,
];
const SIMPLE_KEY: [u8; 8] = [0x69, 0x56, 0x46, 0x38, 0x2b, 0x20, 0x15, 0x0b];

pub type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedTail {
    pub format: TailFormat,
    pub song_mid: String,
    pub filename: String,
    pub audio_size: u64,
    pub ekey: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TailFormat {
    MusicEx,
    Legacy,
}

pub fn parse_encrypted_tail(path: &Path) -> Result<Option<EncryptedTail>> {
    let mut file =
        File::open(path).map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    let file_size = file
        .metadata()
        .map_err(|error| format!("cannot read {} metadata: {error}", path.display()))?
        .len();
    if file_size < 8 {
        return Ok(None);
    }

    file.seek(SeekFrom::End(-8))
        .map_err(|error| format!("cannot seek {}: {error}", path.display()))?;
    let mut tail8 = [0_u8; 8];
    file.read_exact(&mut tail8)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if tail8 == *MUSICEX_MAGIC {
        return parse_musicex_tail(&mut file, file_size);
    }

    let tail4: [u8; 4] = tail8[4..].try_into().expect("tail is four bytes");
    if tail4 == *QTAG_MAGIC || tail4 == *STAG_MAGIC {
        return parse_legacy_tail(&mut file, file_size, tail4);
    }
    Ok(None)
}

fn parse_musicex_tail(file: &mut File, file_size: u64) -> Result<Option<EncryptedTail>> {
    if file_size < 192 {
        return Ok(None);
    }
    file.seek(SeekFrom::End(-16))
        .map_err(|error| format!("cannot seek MusicEx tail: {error}"))?;
    let mut size_bytes = [0_u8; 4];
    file.read_exact(&mut size_bytes)
        .map_err(|error| format!("cannot read MusicEx tail size: {error}"))?;
    let tail_size = u32::from_le_bytes(size_bytes) as u64;
    if !(17..=4096).contains(&tail_size) || tail_size > file_size {
        return Ok(None);
    }

    file.seek(SeekFrom::End(-(tail_size as i64)))
        .map_err(|error| format!("cannot seek MusicEx metadata: {error}"))?;
    let mut tail = vec![0_u8; tail_size as usize];
    file.read_exact(&mut tail)
        .map_err(|error| format!("cannot read MusicEx metadata: {error}"))?;
    if !tail.ends_with(MUSICEX_MAGIC) {
        return Ok(None);
    }

    for (song_range, filename_range, audio_size) in [
        (12..72, 72..168, file_size - tail_size),
        (28..88, 88..184, file_size.saturating_sub(tail_size + 16)),
    ] {
        if filename_range.end > tail.len() || audio_size == 0 {
            continue;
        }
        let song_mid = decode_utf16_field(&tail[song_range]);
        let filename = decode_utf16_field(&tail[filename_range]);
        if song_mid.starts_with("00") && filename.contains('.') {
            return Ok(Some(EncryptedTail {
                format: TailFormat::MusicEx,
                song_mid,
                filename,
                audio_size,
                ekey: None,
            }));
        }
    }
    Ok(None)
}

fn parse_legacy_tail(
    file: &mut File,
    file_size: u64,
    tag: [u8; 4],
) -> Result<Option<EncryptedTail>> {
    file.seek(SeekFrom::End(-8))
        .map_err(|error| format!("cannot seek legacy tail: {error}"))?;
    let mut length_bytes = [0_u8; 4];
    file.read_exact(&mut length_bytes)
        .map_err(|error| format!("cannot read legacy EKey length: {error}"))?;
    let ekey_len = u32::from_le_bytes(length_bytes) as u64;
    if ekey_len == 0 || ekey_len > 4096 || ekey_len + 8 > file_size {
        return Ok(None);
    }
    let audio_size = file_size - ekey_len - 8;
    if audio_size == 0 {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(audio_size))
        .map_err(|error| format!("cannot seek legacy EKey: {error}"))?;
    let mut data = vec![0_u8; ekey_len as usize];
    file.read_exact(&mut data)
        .map_err(|error| format!("cannot read legacy EKey: {error}"))?;
    let (song_mid, ekey) = if tag == *QTAG_MAGIC {
        let mut fields = data.splitn(2, |byte| *byte == b',');
        let song_mid = String::from_utf8_lossy(fields.next().unwrap_or_default()).into_owned();
        let ekey = String::from_utf8_lossy(fields.next().unwrap_or_default()).into_owned();
        (song_mid, ekey)
    } else {
        (String::new(), String::from_utf8_lossy(&data).into_owned())
    };
    if ekey.is_empty() {
        return Ok(None);
    }
    Ok(Some(EncryptedTail {
        format: TailFormat::Legacy,
        song_mid,
        filename: String::new(),
        audio_size,
        ekey: Some(ekey),
    }))
}

fn decode_utf16_field(data: &[u8]) -> String {
    let units = data
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]));
    String::from_utf16_lossy(&units.collect::<Vec<_>>())
        .trim_end_matches('\0')
        .trim()
        .to_owned()
}

pub fn derive_qmc2_key_from_ekey(ekey: &str) -> Result<Vec<u8>> {
    let raw_key = base64::engine::general_purpose::STANDARD
        .decode(ekey.trim())
        .map_err(|error| format!("invalid Base64 EKey: {error}"))?;
    derive_qmc2_key(&raw_key).ok_or_else(|| "invalid QQ Music EKey payload".to_owned())
}

pub fn derive_qmc2_key(raw_key: &[u8]) -> Option<Vec<u8>> {
    let raw_key = if raw_key.starts_with(ENCV2_PREFIX) {
        decrypt_encv2(raw_key)?
    } else {
        raw_key.to_vec()
    };
    if raw_key.len() < 8 {
        return None;
    }
    let mut tea_key = [0_u8; 16];
    for index in 0..8 {
        tea_key[index * 2] = SIMPLE_KEY[index];
        tea_key[index * 2 + 1] = raw_key[index];
    }
    let decrypted = decrypt_tencent_tea(&raw_key[8..], &tea_key)?;
    let mut result = raw_key[..8].to_vec();
    result.extend(decrypted);
    Some(result)
}

fn decrypt_encv2(raw_key: &[u8]) -> Option<Vec<u8>> {
    let first = decrypt_tencent_tea(&raw_key[ENCV2_PREFIX.len()..], &ENCV2_KEY1)?;
    let second = decrypt_tencent_tea(&first, &ENCV2_KEY2)?;
    base64::engine::general_purpose::STANDARD
        .decode(second)
        .ok()
}

fn tea_decrypt_block(block: &[u8], key: &[u8; 16]) -> [u8; 8] {
    let mut v0 = u32::from_be_bytes(block[..4].try_into().expect("TEA block has four bytes"));
    let mut v1 = u32::from_be_bytes(block[4..].try_into().expect("TEA block has four bytes"));
    let k0 = u32::from_be_bytes(key[..4].try_into().expect("TEA key has four bytes"));
    let k1 = u32::from_be_bytes(key[4..8].try_into().expect("TEA key has four bytes"));
    let k2 = u32::from_be_bytes(key[8..12].try_into().expect("TEA key has four bytes"));
    let k3 = u32::from_be_bytes(key[12..].try_into().expect("TEA key has four bytes"));
    let delta = 0x9e37_79b9_u32;
    let mut sum = delta.wrapping_mul(16);
    for _ in 0..16 {
        v1 = v1.wrapping_sub(
            ((v0 << 4).wrapping_add(k2)) ^ v0.wrapping_add(sum) ^ ((v0 >> 5).wrapping_add(k3)),
        );
        v0 = v0.wrapping_sub(
            ((v1 << 4).wrapping_add(k0)) ^ v1.wrapping_add(sum) ^ ((v1 >> 5).wrapping_add(k1)),
        );
        sum = sum.wrapping_sub(delta);
    }
    let mut result = [0_u8; 8];
    result[..4].copy_from_slice(&v0.to_be_bytes());
    result[4..].copy_from_slice(&v1.to_be_bytes());
    result
}

fn decrypt_tencent_tea(data: &[u8], key: &[u8; 16]) -> Option<Vec<u8>> {
    if data.len() < 16 || !data.len().is_multiple_of(8) {
        return None;
    }
    let mut current_block = tea_decrypt_block(&data[..8], key);
    let padding = (current_block[0] & 0x07) as usize;
    let output_len = data.len().checked_sub(1 + padding + 2 + 7)?;
    if output_len == 0 {
        return None;
    }
    let mut previous_cipher = [0_u8; 8];
    let mut current_cipher: [u8; 8] = data[..8].try_into().expect("TEA block has eight bytes");
    let mut input_position = 8;
    let mut block_position = 1 + padding;

    let next_block = |current_block: &mut [u8; 8],
                      previous_cipher: &mut [u8; 8],
                      current_cipher: &mut [u8; 8],
                      input_position: &mut usize| {
        if *input_position + 8 > data.len() {
            return None;
        }
        let next_cipher: [u8; 8] = data[*input_position..*input_position + 8].try_into().ok()?;
        let mut mixed = [0_u8; 8];
        for index in 0..8 {
            mixed[index] = current_block[index] ^ next_cipher[index];
        }
        let decoded = tea_decrypt_block(&mixed, key);
        *previous_cipher = *current_cipher;
        *current_cipher = next_cipher;
        *current_block = decoded;
        *input_position += 8;
        Some(())
    };

    for _ in 0..2 {
        if block_position < 8 {
            block_position += 1;
        } else {
            next_block(
                &mut current_block,
                &mut previous_cipher,
                &mut current_cipher,
                &mut input_position,
            )?;
            block_position = 0;
        }
    }

    let mut output = Vec::with_capacity(output_len);
    while output.len() < output_len {
        if block_position < 8 {
            output.push(current_block[block_position] ^ previous_cipher[block_position]);
            block_position += 1;
        } else {
            next_block(
                &mut current_block,
                &mut previous_cipher,
                &mut current_cipher,
                &mut input_position,
            )?;
            block_position = 0;
        }
    }
    Some(output)
}

pub fn qmc2_decrypt_in_place(key: &[u8], buffer: &mut [u8], offset: u64) -> Result<()> {
    if key.is_empty() {
        return Err("QMC2 key is empty".to_owned());
    }
    if key.len() > 300 {
        rc4_decrypt(key, buffer, offset);
    } else {
        map_decrypt(key, buffer, offset);
    }
    Ok(())
}

fn map_decrypt(key: &[u8], buffer: &mut [u8], offset: u64) {
    for (index, byte) in buffer.iter_mut().enumerate() {
        *byte ^= map_mask(key, offset + index as u64);
    }
}

fn map_mask(key: &[u8], mut offset: u64) -> u8 {
    if offset > 0x7fff {
        offset %= 0x7fff;
    }
    let index = ((offset as u128 * offset as u128 + 71_214) % key.len() as u128) as usize;
    let shift = (index + 4) % 8;
    // QMC2 defines this as two same-direction shifts, not a bit rotation.
    (((key[index] as u16) << shift) | ((key[index] as u16) >> shift)) as u8
}

fn rc4_decrypt(key: &[u8], buffer: &mut [u8], mut offset: u64) {
    const FIRST_SEGMENT_SIZE: u64 = 128;
    const SEGMENT_SIZE: u64 = 5120;
    let key_len = key.len();
    let mut box_state: Vec<u8> = (0..key_len).map(|index| index as u8).collect();
    let mut cursor = 0_usize;
    for index in 0..key_len {
        cursor = (cursor + box_state[index] as usize + key[index] as usize) % key_len;
        box_state.swap(index, cursor);
    }
    let hash = rc4_hash(key);
    let mut processed = 0_usize;

    if offset < FIRST_SEGMENT_SIZE {
        let length = (FIRST_SEGMENT_SIZE - offset).min(buffer.len() as u64) as usize;
        for index in 0..length {
            buffer[index] ^= key[rc4_segment_skip(key, hash, offset + index as u64)];
        }
        processed += length;
        offset += length as u64;
    }
    while processed < buffer.len() {
        let segment_offset = offset % SEGMENT_SIZE;
        let length =
            (SEGMENT_SIZE - segment_offset).min((buffer.len() - processed) as u64) as usize;
        let mut box_copy = box_state.clone();
        let mut cursor_a = 0_usize;
        let mut cursor_b = 0_usize;
        let skip = segment_offset as usize + rc4_segment_skip(key, hash, offset / SEGMENT_SIZE);
        for index in 0..skip + length {
            cursor_a = (cursor_a + 1) % key_len;
            cursor_b = (box_copy[cursor_a] as usize + cursor_b) % key_len;
            box_copy.swap(cursor_a, cursor_b);
            if index >= skip {
                let mask =
                    box_copy[(box_copy[cursor_a] as usize + box_copy[cursor_b] as usize) % key_len];
                buffer[processed + index - skip] ^= mask;
            }
        }
        processed += length;
        offset += length as u64;
    }
}

fn rc4_hash(key: &[u8]) -> u32 {
    let mut result = 1_u32;
    for &value in key {
        if value == 0 {
            continue;
        }
        let next = result.wrapping_mul(value as u32);
        if next == 0 || next <= result {
            break;
        }
        result = next;
    }
    result
}

fn rc4_segment_skip(key: &[u8], hash: u32, segment_id: u64) -> usize {
    let seed = key[(segment_id % key.len() as u64) as usize];
    if seed == 0 {
        return 0;
    }
    (((hash as f64 / ((segment_id + 1) as f64 * seed as f64) * 100.0) as u64) % key.len() as u64)
        as usize
}

pub fn decrypt_file(
    input: &Path,
    output: &Path,
    supplied_ekey: Option<&str>,
) -> Result<EncryptedTail> {
    let metadata = parse_encrypted_tail(input)?
        .ok_or_else(|| "unsupported or malformed QQ Music encrypted file".to_owned())?;
    let ekey = supplied_ekey
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| metadata.ekey.clone())
        .ok_or_else(|| {
            "this MusicEx file has no embedded EKey; provide --ekey or --ekey-file".to_owned()
        })?;
    let key = derive_qmc2_key_from_ekey(&ekey)?;
    let mut source =
        File::open(input).map_err(|error| format!("cannot open {}: {error}", input.display()))?;
    let mut target = File::create(output)
        .map_err(|error| format!("cannot create {}: {error}", output.display()))?;
    let mut offset = 0_u64;
    let mut chunk = vec![0_u8; 1024 * 1024];
    while offset < metadata.audio_size {
        let requested = (metadata.audio_size - offset).min(chunk.len() as u64) as usize;
        source
            .read_exact(&mut chunk[..requested])
            .map_err(|error| format!("encrypted audio ended early at {offset}: {error}"))?;
        qmc2_decrypt_in_place(&key, &mut chunk[..requested], offset)?;
        target
            .write_all(&chunk[..requested])
            .map_err(|error| format!("cannot write {}: {error}", output.display()))?;
        offset += requested as u64;
    }
    target
        .flush()
        .map_err(|error| format!("cannot flush {}: {error}", output.display()))?;
    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_path(suffix: &str) -> std::path::PathBuf {
        let sequence = TEST_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "mgg-test-{}-{sequence}{suffix}",
            std::process::id()
        ))
    }

    fn fnv1a64(data: &[u8]) -> u64 {
        data.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    }

    #[test]
    fn map_cipher_is_independent_of_chunk_boundaries() {
        let key: Vec<u8> = (1..32).collect();
        let mut one_pass: Vec<u8> = (0..80_000).map(|index| (index * 17) as u8).collect();
        let mut chunked = one_pass.clone();
        qmc2_decrypt_in_place(&key, &mut one_pass, 0).unwrap();
        for (start, end) in [(0, 9), (9, 0x8004), (0x8004, 42_000), (42_000, 80_000)] {
            qmc2_decrypt_in_place(&key, &mut chunked[start..end], start as u64).unwrap();
        }
        assert_eq!(one_pass, chunked);
        assert_eq!(fnv1a64(&one_pass), 0xd163_d700_3386_7fcb);
    }

    #[test]
    fn rc4_cipher_is_independent_of_chunk_boundaries() {
        let key: Vec<u8> = (0..301).map(|index| (index * 37) as u8).collect();
        let mut one_pass: Vec<u8> = (0..20_000).map(|index| (index * 11) as u8).collect();
        let mut chunked = one_pass.clone();
        qmc2_decrypt_in_place(&key, &mut one_pass, 0).unwrap();
        for (start, end) in [
            (0, 17),
            (17, 128),
            (128, 5121),
            (5121, 12_013),
            (12_013, 20_000),
        ] {
            qmc2_decrypt_in_place(&key, &mut chunked[start..end], start as u64).unwrap();
        }
        assert_eq!(one_pass, chunked);
        assert_eq!(fnv1a64(&one_pass), 0x3581_72b7_549d_958d);
    }

    #[test]
    fn map_cipher_is_symmetric() {
        let key: Vec<u8> = (1..32).collect();
        let original: Vec<u8> = (0..40_000).map(|index| (index * 3) as u8).collect();
        let mut encrypted = original.clone();
        qmc2_decrypt_in_place(&key, &mut encrypted, 0).unwrap();
        qmc2_decrypt_in_place(&key, &mut encrypted, 0).unwrap();
        assert_eq!(original, encrypted);
    }

    #[test]
    fn rejects_short_raw_key() {
        assert!(derive_qmc2_key(b"short").is_none());
    }

    #[test]
    fn derives_key_from_python_implementation_vector() {
        let ekey = "MTIzNDU2NzhQWhkuzlyHosmotu2+kFP0";
        assert_eq!(derive_qmc2_key_from_ekey(ekey).unwrap(), b"12345678abcdef");
    }

    #[test]
    fn decrypts_legacy_qtag_file_end_to_end() {
        let source_path = test_path(".mgg");
        let output_path = test_path(".ogg");
        let original: Vec<u8> = (0..8192).map(|index| (index * 29) as u8).collect();
        let mut encrypted = original.clone();
        qmc2_decrypt_in_place(b"12345678abcdef", &mut encrypted, 0).unwrap();
        let ekey = b"MTIzNDU2NzhQWhkuzlyHosmotu2+kFP0";
        let mut input = encrypted;
        input.extend_from_slice(b"001song,");
        input.extend_from_slice(ekey);
        input.extend_from_slice(&(8_u32 + ekey.len() as u32).to_le_bytes());
        input.extend_from_slice(QTAG_MAGIC);
        fs::write(&source_path, input).unwrap();

        let metadata = decrypt_file(&source_path, &output_path, None).unwrap();

        assert_eq!(metadata.format, TailFormat::Legacy);
        assert_eq!(metadata.song_mid, "001song");
        assert_eq!(fs::read(&output_path).unwrap(), original);
        fs::remove_file(source_path).unwrap();
        fs::remove_file(output_path).unwrap();
    }

    #[test]
    fn utf16_fields_decode_like_musicex_metadata() {
        let mut field = vec![0_u8; 60];
        let value = "001xd0HI0X9GNq".encode_utf16().collect::<Vec<_>>();
        for (index, unit) in value.iter().enumerate() {
            field[index * 2..index * 2 + 2].copy_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode_utf16_field(&field), "001xd0HI0X9GNq");
    }
}
