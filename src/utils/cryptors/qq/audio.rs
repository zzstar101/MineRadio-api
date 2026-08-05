use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use base64::Engine;

use crate::api::{ApiError, ApiErrorCode, ApiResult};

use super::super::AudioDecryptResult;

const MUSICEX_MAGIC: &[u8; 8] = b"musicex\0";
const QTAG_MAGIC: &[u8; 4] = b"QTag";
const STAG_MAGIC: &[u8; 4] = b"STag";
const ENCV2_PREFIX: &[u8] = b"QQMusic EncV2,Key:";

const ENCV2_KEY1: [u8; 16] = *b"386ZJY!@#*$%^&)(";
const ENCV2_KEY2: [u8; 16] = *b"**#!(#$%&^a1cZ,T";
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
        .decode(ekey.trim_matches(char::from(0)))
        .map_err(|error| format!("invalid Base64 EKey: {error}"))?;
    derive_qmc2_key(&raw_key).ok_or_else(|| "invalid QQ Music EKey payload".to_owned())
}

pub fn derive_qmc2_key(raw_key: &[u8]) -> Option<Vec<u8>> {
    if raw_key.is_empty() {
        return None;
    }

    let decoded = if raw_key.starts_with(ENCV2_PREFIX) {
        let encv2_blob = &raw_key[ENCV2_PREFIX.len()..];
        let stage1 = decrypt_tencent_tea(encv2_blob, &ENCV2_KEY1)?;
        let stage2 = decrypt_tencent_tea(&stage1, &ENCV2_KEY2)?;
        base64::engine::general_purpose::STANDARD
            .decode(stage2)
            .ok()?
    } else {
        raw_key.to_vec()
    };

    if decoded.len() < 8 {
        return None;
    }

    let (header, body) = decoded.split_at(8);
    if body.is_empty() {
        return Some(header.to_vec());
    }

    let tea_key = derive_tea_key(header);
    if let Some(decrypted_body) = decrypt_tencent_tea(body, &tea_key) {
        let mut result = Vec::with_capacity(8 + decrypted_body.len());
        result.extend_from_slice(header);
        result.extend_from_slice(&decrypted_body);
        Some(result)
    } else {
        // The decoded blob is already the raw key (API-issued ekey).
        Some(decoded)
    }
}

fn derive_tea_key(ekey_header: &[u8]) -> [u8; 16] {
    let mut tea_key = [0_u8; 16];
    for index in 0..8 {
        tea_key[index * 2] = SIMPLE_KEY[index];
        tea_key[index * 2 + 1] = ekey_header[index];
    }
    tea_key
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
    // Layout: [pad_len(1)] [padding(0-7)] [salt(2)] [body(?)] [zero(7)].
    // Matches the tc_tea crate: FIXED_PADDING_LEN = 1 + 2 + 7 = 10.
    const SALT_LEN: usize = 2;
    const ZERO_LEN: usize = 7;

    let len = data.len();
    if len < 10 || len % 8 != 0 {
        return None;
    }

    let mut decrypted = data.to_vec();

    // Block 0 is plain TEA-ECB decryption.
    decrypted[0..8].copy_from_slice(&tea_decrypt_block(&data[..8], key));

    // Later blocks: XOR with the previously *decrypted* block, then ECB-decrypt.
    for block in 1..(len / 8) {
        let start = block * 8;
        let mut mixed = [0_u8; 8];
        for index in 0..8 {
            mixed[index] = decrypted[start + index] ^ decrypted[start - 8 + index];
        }
        decrypted[start..start + 8].copy_from_slice(&tea_decrypt_block(&mixed, key));
    }

    // Final pass: XOR each byte from index 8 onward with the original
    // ciphertext shifted by 8 (reverses the encrypt-side iv chaining).
    for index in 8..len {
        decrypted[index] ^= data[index - 8];
    }

    let pad_size = usize::from(decrypted[0] & 0b111);
    let start_loc = 1 + pad_size + SALT_LEN;
    let end_loc = len - ZERO_LEN;

    if start_loc > end_loc || !decrypted[end_loc..].iter().all(|byte| *byte == 0) {
        return None;
    }
    Some(decrypted[start_loc..end_loc].to_vec())
}

pub fn qmc2_decrypt_in_place(key: &[u8], buffer: &mut [u8], offset: u64) -> Result<()> {
    if key.is_empty() {
        return Err("QMC2 key is empty".to_owned());
    }
    if key.len() > 300 {
        Qmc2Rc4Crypto::new(key).decrypt(offset as usize, buffer);
    } else {
        Qmc2MapCrypto::new(key).decrypt(offset as usize, buffer);
    }
    Ok(())
}

// QMC2 ciphers ported from qmc-decoder's `qmc2.rs`:
// - key length <= 300: QMC2 Map (XOR with a scrambled key)
// - key length > 300:  QMC2 RC4 (modified RC4 stream cipher)

struct Qmc2MapCrypto {
    key: Vec<u8>,
}

impl Qmc2MapCrypto {
    fn new(key: &[u8]) -> Self {
        Self { key: key.to_vec() }
    }

    /// Rotate a key byte by its index (two same-direction shifts).
    #[inline]
    fn scramble_by_index(value: u8, index: usize) -> u8 {
        let rotation = ((index as u32).wrapping_add(4)) & 0b111;
        let left = value.wrapping_shl(rotation);
        let right = value.wrapping_shr(rotation);
        left | right
    }

    /// XOR mask for the given absolute byte offset.
    #[inline]
    fn map_l(&self, offset: usize) -> u8 {
        let mut offset_local = offset;
        if offset_local > 0x7FFF {
            offset_local %= 0x7FFF;
        }
        let index = (offset_local * offset_local + 71214) % self.key.len();
        Self::scramble_by_index(self.key[index], index)
    }

    fn decrypt(&self, offset: usize, buf: &mut [u8]) {
        for (i, byte) in buf.iter_mut().enumerate() {
            *byte ^= self.map_l(offset + i);
        }
    }
}

const FIRST_SEGMENT_SIZE: usize = 0x80;
const OTHER_SEGMENT_SIZE: usize = 0x1400;

struct Qmc2Rc4Crypto {
    /// RC4 seed box (S-box)
    s: Vec<u8>,
    /// Hash base for segment key calculation
    hash: u32,
    /// RC4 key
    rc4_key: Vec<u8>,
}

impl Qmc2Rc4Crypto {
    fn new(rc4_key: &[u8]) -> Self {
        let n = rc4_key.len();

        // QMC2 uses a variable-size S-box equal to the key length; for keys
        // longer than 255 bytes the values wrap modulo 256.
        let mut s: Vec<u8> = (0..n as u8).collect();
        if n > 256 {
            s = (0..=255u8).collect();
            s.extend((0..=255u8).cycle().take(n - 256));
        }

        // KSA (Key Scheduling Algorithm)
        let mut j = 0usize;
        for i in 0..n {
            j = (j + s[i] as usize + rc4_key[i] as usize) % n;
            s.swap(i, j);
        }

        Qmc2Rc4Crypto {
            s,
            hash: Self::calc_hash_base(rc4_key),
            rc4_key: rc4_key.to_vec(),
        }
    }

    fn calc_hash_base(data: &[u8]) -> u32 {
        let mut hash: u32 = 1;
        for &value in data.iter() {
            let value = u32::from(value);
            if value == 0 {
                continue;
            }
            let next_hash = hash.wrapping_mul(value);
            if next_hash == 0 || next_hash <= hash {
                break;
            }
            hash = next_hash;
        }
        hash
    }

    #[inline]
    fn calc_segment_key(&self, id: usize, seed: u8) -> usize {
        let dividend = f64::from(self.hash);
        let divisor = ((id + 1) * usize::from(seed)) as f64;
        let key = dividend / divisor * 100.0;
        key as u64 as usize
    }

    /// RC4 PRGA derives one byte.
    #[inline]
    fn rc4_derive(n: usize, s: &mut Vec<u8>, j: &mut usize, k: &mut usize) -> u8 {
        *j = (*j + 1) % n;
        *k = (usize::from(s[*j]) + *k) % n;
        s.swap(*j, *k);
        let index = usize::from(s[*j]) + usize::from(s[*k]);
        s[index % n]
    }

    /// Encrypt/decrypt the first segment (offset < 0x80).
    fn encode_first_segment(&self, offset: usize, buf: &mut [u8]) {
        let n = self.rc4_key.len();
        let mut offset = offset;
        for byte in buf.iter_mut() {
            let key1 = self.rc4_key[offset % n];
            let key2 = self.calc_segment_key(offset, key1);
            *byte ^= self.rc4_key[key2 % n];
            offset += 1;
        }
    }

    /// Encrypt/decrypt any segment at or beyond 0x80.
    fn encode_other_segment(&self, offset: usize, buf: &mut [u8]) {
        let seg_id = offset / OTHER_SEGMENT_SIZE;
        let seg_id_small = seg_id & 0x1FF;

        let mut discard_count = self.calc_segment_key(seg_id, self.rc4_key[seg_id_small]) & 0x1FF;
        discard_count += offset % OTHER_SEGMENT_SIZE;

        let n = self.rc4_key.len();
        let mut s = self.s.clone();
        let mut j = 0usize;
        let mut k = 0usize;
        for _ in 0..discard_count {
            Self::rc4_derive(n, &mut s, &mut j, &mut k);
        }

        for byte in buf.iter_mut() {
            *byte ^= Self::rc4_derive(n, &mut s, &mut j, &mut k);
        }
    }

    fn decrypt(&self, offset: usize, buf: &mut [u8]) {
        let mut offset = offset;
        let mut len = buf.len();
        let mut i = 0usize;

        // First segment has a different algorithm.
        if offset < FIRST_SEGMENT_SIZE {
            let len_processed = std::cmp::min(len, FIRST_SEGMENT_SIZE - offset);
            self.encode_first_segment(offset, &mut buf[i..i + len_processed]);
            i += len_processed;
            len -= len_processed;
            offset += len_processed;
        }

        // Align to the segment boundary.
        let to_align = offset % OTHER_SEGMENT_SIZE;
        if to_align != 0 {
            let len_processed = std::cmp::min(len, OTHER_SEGMENT_SIZE - to_align);
            self.encode_other_segment(offset, &mut buf[i..i + len_processed]);
            i += len_processed;
            len -= len_processed;
            offset += len_processed;
        }

        // Process full segments.
        while len > OTHER_SEGMENT_SIZE {
            self.encode_other_segment(offset, &mut buf[i..i + OTHER_SEGMENT_SIZE]);
            i += OTHER_SEGMENT_SIZE;
            len -= OTHER_SEGMENT_SIZE;
            offset += OTHER_SEGMENT_SIZE;
        }

        // Remaining bytes.
        if len > 0 {
            self.encode_other_segment(offset, &mut buf[i..i + len]);
        }
    }
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
            format!(
                "qq encrypted file ({:?}) has no embedded EKey; pass it explicitly",
                metadata.format
            )
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
        assert_eq!(fnv1a64(&one_pass), 0x2698_05be_c3cc_a2a8);
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
    fn parse_ekey_derives_test_key() {
        // Reference vector from qmc-decoder's test_parse_ekey.
        let ekey = "VGhpcyBpcyBHFWEh4cjZ1Vi7rJ56XeoPlqGM1sxBGPg7mt89umKclFBr9iqfmFdS";
        assert_eq!(
            std::str::from_utf8(&derive_qmc2_key_from_ekey(ekey).unwrap()).unwrap(),
            "This is a test key for test purpose :D"
        );
    }

    #[test]
    fn derive_tea_key_interleaves_simple_key_and_header() {
        // Reference vector from qmc-decoder's test_derive_tea_key.
        let ekey_header = [0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8];
        assert_eq!(
            derive_tea_key(&ekey_header),
            [
                0x69, 0xf1, 0x56, 0xf2, 0x46, 0xf3, 0x38, 0xf4, 0x2b, 0xf5, 0x20, 0xf6, 0x15, 0xf7,
                0x0b, 0xf8,
            ]
        );
    }

    #[test]
    fn simple_key_matches_reference() {
        // Reference vector from qmc-decoder's test_simple_make_key.
        assert_eq!(SIMPLE_KEY, [0x69, 0x56, 0x46, 0x38, 0x2b, 0x20, 0x15, 0x0b]);
    }

    #[test]
    fn map_cipher_matches_reference_vector() {
        // Reference vector from qmc-decoder's test_map_l (pins the 71214 constant).
        let key: [u8; 16] = [
            0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x4B, 0x4C, 0x4D, 0x4E,
            0x4F, 0x50,
        ];
        let mut data = [0u8; 16];
        Qmc2MapCrypto::new(&key).decrypt(0, &mut data);
        assert_eq!(
            data,
            [
                0x3F, 0x8A, 0xC1, 0x49, 0x3F, 0x49, 0xC1, 0x8A, 0x3F, 0x8A, 0xC1, 0x49, 0x3F, 0x49,
                0xC1, 0x8A
            ]
        );
    }

    #[test]
    fn map_cipher_matches_reference_vector_at_boundary() {
        // Reference vector from qmc-decoder's test_map_l_boundary.
        let key: [u8; 16] = [
            0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x4B, 0x4C, 0x4D, 0x4E,
            0x4F, 0x50,
        ];
        let mut data = [0u8; 16];
        Qmc2MapCrypto::new(&key).decrypt(0x7FFF - 8, &mut data);
        assert_eq!(
            data,
            [
                0x8A, 0x3F, 0x8A, 0xC1, 0x49, 0x3F, 0x49, 0xC1, 0x8A, 0x8A, 0xC1, 0x49, 0x3F, 0x49,
                0xC1, 0x8A
            ]
        );
    }

    #[test]
    fn rc4_hash_base_matches_reference() {
        // Reference vectors from qmc-decoder's test_rc4_hash_base.
        assert_eq!(Qmc2Rc4Crypto::calc_hash_base(&[1u8, 99]), 1);
        assert_eq!(Qmc2Rc4Crypto::calc_hash_base(&[0xff; 16]), 0xfc05fc01);
    }

    #[test]
    fn rc4_first_segment_matches_reference() {
        // Reference vector from qmc-decoder's test_rc4_first_segment.
        let mut rc4_key = [0u8; 255];
        for (i, p) in rc4_key.iter_mut().enumerate() {
            *p = i as u8;
        }
        let mut data = [0u8; 16];
        Qmc2Rc4Crypto::new(&rc4_key).decrypt(0, &mut data);
        assert_eq!(data, [0, 50, 16, 8, 5, 3, 2, 1, 1, 1, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn tc_tea_decrypts_reference_vector() {
        // Known-good vector from the tc_tea crate.
        const GOOD_ENCRYPTED_DATA: [u8; 24] = [
            0x91, 0x09, 0x51, 0x62, 0xe3, 0xf5, 0xb6, 0xdc, 0x6b, 0x41, 0x4b, 0x50, 0xd1, 0xa5,
            0xb8, 0x4e, 0xc5, 0x0d, 0x0c, 0x1b, 0x11, 0x96, 0xfd, 0x3c,
        ];
        const KEY: [u8; 16] = *b"12345678ABCDEFGH";
        assert_eq!(
            decrypt_tencent_tea(&GOOD_ENCRYPTED_DATA, &KEY).unwrap(),
            vec![1u8, 2, 3, 4, 5, 6, 7, 8]
        );
    }

    #[test]
    fn empty_body_ekey_uses_header_as_key() {
        let ekey = base64::engine::general_purpose::STANDARD.encode(b"ABCDEFGH");
        assert_eq!(derive_qmc2_key_from_ekey(&ekey).unwrap(), b"ABCDEFGH");
    }

    #[test]
    fn short_body_ekey_falls_back_to_raw_key() {
        // 8-byte header + 9-byte body: the body is too short for TEA, so the
        // whole decoded blob is used as the raw key (API-issued ekey).
        let mut blob = b"HEADER00".to_vec();
        blob.extend_from_slice(b"123456789");
        let ekey = base64::engine::general_purpose::STANDARD.encode(&blob);
        assert_eq!(derive_qmc2_key_from_ekey(&ekey).unwrap(), blob);
    }

    #[test]
    fn null_bytes_in_ekey_are_stripped() {
        let ekey = "MTIzNDU2NzhQWhkuzlyHosmotu2+kFP0\u{0}\u{0}";
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

/// Attempt to decrypt QQ Music encrypted audio data (MGG/MFLAC/MNAC).
///
/// If `explicit_ekey` is provided it takes priority; otherwise an embedded
/// EKey is used. Unencrypted input is passed through unchanged.
pub fn decrypt_qq_audio(
    mut data: Vec<u8>,
    explicit_ekey: Option<&str>,
) -> ApiResult<AudioDecryptResult> {
    // Resolve EKey: explicit > embedded tail > error (if encrypted tail found but no EKey)
    let (ekey_str, audio_size) =
        if let Some(ekey) = explicit_ekey.map(|s| s.trim()).filter(|s| !s.is_empty()) {
            // The tail still determines the audio portion when available.
            let audio_size = parse_encrypted_tail_from_bytes(&data)
                .map(|t| t.audio_size as usize)
                .unwrap_or(data.len());
            (ekey.to_owned(), audio_size)
        } else {
            match parse_encrypted_tail_from_bytes(&data) {
                Some(tail) => {
                    let ekey = tail.ekey.ok_or_else(|| {
                        ApiError::new(
                            ApiErrorCode::BadRequest,
                            format!(
                                "qq encrypted file ({:?}) has no embedded EKey; pass it explicitly",
                                tail.format,
                            ),
                        )
                    })?;
                    (ekey, tail.audio_size as usize)
                }
                None => {
                    // Unencrypted input is passed through.
                    return Ok(AudioDecryptResult {
                        content_type: detect_content_type("", &data).to_owned(),
                        data,
                    });
                }
            }
        };

    if audio_size == 0 || audio_size > data.len() {
        return Err(ApiError::new(
            ApiErrorCode::BadRequest,
            format!("qq encrypted audio has invalid audio size ({audio_size})"),
        ));
    }

    let key = derive_qmc2_key_from_ekey(&ekey_str).map_err(|error| {
        ApiError::new(
            ApiErrorCode::BadRequest,
            format!("qq EKey derivation failed: {error}"),
        )
    })?;

    qmc2_decrypt_in_place(&key, &mut data[..audio_size], 0).map_err(|error| {
        ApiError::new(
            ApiErrorCode::Internal,
            format!("qq audio decrypt failed: {error}"),
        )
    })?;

    // Truncate to just the decrypted audio portion (strip the tail)
    data.truncate(audio_size);

    let content_type = detect_content_type("", &data);

    Ok(AudioDecryptResult {
        data,
        content_type: content_type.to_owned(),
    })
}

// In-memory encrypted-tail parsing.

const MEMORY_MUSICEX_MAGIC: &[u8; 8] = b"musicex\0";
const MEMORY_QTAG_MAGIC: &[u8; 4] = b"QTag";
const MEMORY_STAG_MAGIC: &[u8; 4] = b"STag";

fn parse_encrypted_tail_from_bytes(data: &[u8]) -> Option<EncryptedTail> {
    if data.len() < 8 {
        return None;
    }

    let tail8: &[u8; 8] = data[data.len() - 8..].try_into().ok()?;
    if tail8 == MEMORY_MUSICEX_MAGIC {
        return parse_musicex_tail_from_bytes(data);
    }

    let tail4: &[u8; 4] = tail8[4..].try_into().ok()?;
    if *tail4 == *MEMORY_QTAG_MAGIC || *tail4 == *MEMORY_STAG_MAGIC {
        return parse_legacy_tail_from_bytes(data, *tail4);
    }
    None
}

fn parse_musicex_tail_from_bytes(data: &[u8]) -> Option<EncryptedTail> {
    let file_size = data.len() as u64;
    if file_size < 192 {
        return None;
    }
    let tail_size =
        u32::from_le_bytes(data[data.len() - 16..data.len() - 12].try_into().ok()?) as u64;
    if !(17..=4096).contains(&tail_size) || tail_size > file_size {
        return None;
    }
    let tail_start = data.len().checked_sub(tail_size as usize)?;
    let tail = data.get(tail_start..)?;
    if !tail.ends_with(MEMORY_MUSICEX_MAGIC) {
        return None;
    }

    for (song_range, filename_range, audio_size) in [
        (12..72, 72..168, file_size - tail_size),
        (28..88, 88..184, file_size.saturating_sub(tail_size + 16)),
    ] {
        if filename_range.end > tail.len() || audio_size == 0 {
            continue;
        }
        let song_mid = decode_memory_utf16_field(tail.get(song_range)?);
        let filename = decode_memory_utf16_field(tail.get(filename_range)?);
        if song_mid.starts_with("00") && filename.contains('.') {
            return Some(EncryptedTail {
                format: TailFormat::MusicEx,
                song_mid,
                filename,
                audio_size,
                ekey: None,
            });
        }
    }
    None
}

fn parse_legacy_tail_from_bytes(data: &[u8], tag: [u8; 4]) -> Option<EncryptedTail> {
    let file_size = data.len() as u64;
    let ekey_len = u32::from_le_bytes(data[data.len() - 8..data.len() - 4].try_into().ok()?) as u64;
    if ekey_len == 0 || ekey_len > 4096 || ekey_len + 8 > file_size {
        return None;
    }
    let audio_size = file_size - ekey_len - 8;
    if audio_size == 0 {
        return None;
    }
    let ekey_data = data.get(audio_size as usize..data.len() - 8)?;
    let (song_mid, ekey) = if tag == *MEMORY_QTAG_MAGIC {
        let mut fields = ekey_data.splitn(2, |byte| *byte == b',');
        let song_mid = String::from_utf8_lossy(fields.next().unwrap_or_default()).into_owned();
        let ekey = String::from_utf8_lossy(fields.next().unwrap_or_default()).into_owned();
        (song_mid, ekey)
    } else {
        (
            String::new(),
            String::from_utf8_lossy(ekey_data).into_owned(),
        )
    };
    if ekey.is_empty() {
        return None;
    }
    Some(EncryptedTail {
        format: TailFormat::Legacy,
        song_mid,
        filename: String::new(),
        audio_size,
        ekey: Some(ekey),
    })
}

fn decode_memory_utf16_field(data: &[u8]) -> String {
    let units = data
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]));
    String::from_utf16_lossy(&units.collect::<Vec<_>>())
        .trim_end_matches('\0')
        .trim()
        .to_owned()
}

// Content-type detection.

fn detect_content_type(url: &str, data: &[u8]) -> &'static str {
    // Try URL extension first
    if let Some(ct) = content_type_from_extension(url) {
        return ct;
    }
    // Fall back to magic bytes
    content_type_from_magic(data)
}

fn content_type_from_extension(url: &str) -> Option<&'static str> {
    let path = url.split('?').next().unwrap_or(url);
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        "mgg" | "ogg" | "opus" => Some("audio/ogg"),
        "mflac" | "flac" => Some("audio/flac"),
        "mnac" | "nac" => Some("audio/nac"),
        "mp3" => Some("audio/mpeg"),
        "m4a" => Some("audio/mp4"),
        _ => None,
    }
}

fn content_type_from_magic(data: &[u8]) -> &'static str {
    if data.len() < 4 {
        return "audio/mpeg";
    }
    match &data[..4] {
        b"OggS" => "audio/ogg",
        b"fLaC" => "audio/flac",
        b"ID3\x04" | &[0xff, 0xfb, ..] | &[0xff, 0xf3, ..] | &[0xff, 0xf2, ..] => "audio/mpeg",
        _ if data.starts_with(b"\x00\x00\x00") && data.get(4..8) == Some(b"ftyp") => "audio/mp4",
        _ => "audio/mpeg",
    }
}
