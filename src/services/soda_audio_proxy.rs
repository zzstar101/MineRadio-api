use std::{collections::VecDeque, sync::Arc};

use aes::Aes128;
use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, Request, StatusCode},
    response::Response,
};
use ctr::cipher::{KeyIvInit, StreamCipher};
use futures::future::{BoxFuture, FutureExt};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::http::response::fail;

type Aes128Ctr64BE = ctr::Ctr64BE<Aes128>;

const ENCA_BYTES: &[u8] = b"enca";
const MP4A_BYTES: &[u8] = b"mp4a";
const SPADE_PREFIX: [u8; 2] = [0xfa, 0x55];
const DEFAULT_MAX_CACHE_ENTRIES: usize = 12;

#[derive(Debug)]
pub struct SodaAudioProxyRequest {
    pub target: String,
    pub request: Request<Body>,
    pub play_auth: Option<String>,
}

#[derive(Clone)]
pub struct SodaAudioProxyDeps {
    pub fetch: SodaAudioFetch,
    pub decrypt: SodaAudioDecrypt,
    pub max_cache_entries: usize,
}

pub type SodaAudioFetch =
    Arc<dyn Fn(String) -> BoxFuture<'static, anyhow::Result<SodaAudioFetchResponse>> + Send + Sync>;
pub type SodaAudioDecrypt = Arc<
    dyn Fn(Vec<u8>, String) -> BoxFuture<'static, anyhow::Result<DecryptDataResult>> + Send + Sync,
>;

pub struct SodaAudioFetchResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Default for SodaAudioProxyDeps {
    fn default() -> Self {
        let client = reqwest::Client::new();
        Self {
            fetch: Arc::new(move |target| {
                let client = client.clone();
                async move {
                    let upstream = client.get(target).send().await?;
                    let status = StatusCode::from_u16(upstream.status().as_u16())?;
                    let headers = upstream.headers().clone();
                    let body = upstream.bytes().await?.to_vec();
                    Ok(SodaAudioFetchResponse {
                        status,
                        headers,
                        body,
                    })
                }
                .boxed()
            }),
            decrypt: Arc::new(|file_data, play_auth| {
                async move { decrypt_soda_audio_data(file_data, play_auth).await }.boxed()
            }),
            max_cache_entries: DEFAULT_MAX_CACHE_ENTRIES,
        }
    }
}

#[derive(Clone)]
pub struct SodaAudioProxy {
    deps: SodaAudioProxyDeps,
    cache: Arc<Mutex<SodaAudioCache>>,
}

#[derive(Default)]
struct SodaAudioCache {
    entries: std::collections::HashMap<String, CachedSodaAudio>,
    order: VecDeque<String>,
}

#[derive(Clone)]
struct CachedSodaAudio {
    bytes: Vec<u8>,
    content_type: String,
}

enum RangeSelection {
    None,
    Invalid,
    Slice { start: usize, end: usize },
}

impl SodaAudioProxy {
    pub async fn resolve(&self, input: SodaAudioProxyRequest) -> Response {
        proxy_soda_audio(input, &self.deps, &self.cache).await
    }
}

pub fn create_soda_audio_proxy(deps: SodaAudioProxyDeps) -> SodaAudioProxy {
    SodaAudioProxy {
        deps,
        cache: Arc::new(Mutex::new(SodaAudioCache::default())),
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DecryptDataResult {
    pub data: Vec<u8>,
    pub decrypted: bool,
    pub reason: String,
}

async fn proxy_soda_audio(
    input: SodaAudioProxyRequest,
    deps: &SodaAudioProxyDeps,
    cache: &Arc<Mutex<SodaAudioCache>>,
) -> Response {
    let parsed = match parse_target_url(&input.target) {
        Ok(url) => url,
        Err(message) => return bad_request(message),
    };

    let play_auth = input.play_auth.unwrap_or_default().trim().to_owned();
    if play_auth.is_empty() {
        return bad_request("playAuth required");
    }

    match get_or_create_cached_audio(cache, deps, parsed.as_str(), &play_auth).await {
        Ok(cached) => {
            let range = parse_range(
                input
                    .request
                    .headers()
                    .get("range")
                    .and_then(|value| value.to_str().ok()),
                cached.bytes.len(),
            );
            response_for_cached_audio(&cached, range)
        }
        Err(err) => upstream_failure(err.to_string()),
    }
}

async fn get_or_create_cached_audio(
    cache: &Arc<Mutex<SodaAudioCache>>,
    deps: &SodaAudioProxyDeps,
    target: &str,
    play_auth: &str,
) -> anyhow::Result<CachedSodaAudio> {
    let cache_key = format!("{target}\n{play_auth}");
    if let Some(existing) = cache.lock().await.get_refresh(&cache_key) {
        return Ok(existing);
    }

    let upstream = (deps.fetch)(target.to_owned()).await?;
    if !upstream.status.is_success() {
        anyhow::bail!("soda audio request returned {}", upstream.status.as_u16());
    }
    let content_type = upstream
        .headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("audio/mp4")
        .to_owned();
    let decrypted = (deps.decrypt)(upstream.body, play_auth.to_owned())
        .await
        .map_err(|_| anyhow::anyhow!("soda audio decrypt failed"))?;
    if !decrypted.decrypted {
        anyhow::bail!("soda audio decrypt failed: {}", decrypted.reason);
    }

    let cached = CachedSodaAudio {
        bytes: decrypted.data,
        content_type,
    };
    if deps.max_cache_entries > 0 {
        cache
            .lock()
            .await
            .insert(cache_key, cached.clone(), deps.max_cache_entries);
    }
    Ok(cached)
}

impl SodaAudioCache {
    fn get_refresh(&mut self, key: &str) -> Option<CachedSodaAudio> {
        let item = self.entries.get(key).cloned()?;
        self.order.retain(|existing| existing != key);
        self.order.push_back(key.to_owned());
        Some(item)
    }

    fn insert(&mut self, key: String, value: CachedSodaAudio, max_entries: usize) {
        self.entries.insert(key.clone(), value);
        self.order.retain(|existing| existing != &key);
        self.order.push_back(key);
        while self.entries.len() > max_entries {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }
}

fn response_for_cached_audio(cached: &CachedSodaAudio, range: RangeSelection) -> Response {
    match range {
        RangeSelection::Invalid => {
            let mut headers = soda_audio_headers(&cached.content_type, 0, cached.bytes.len());
            headers.insert(
                "content-range",
                HeaderValue::from_str(&format!("bytes */{}", cached.bytes.len())).unwrap(),
            );
            build_response(StatusCode::RANGE_NOT_SATISFIABLE, headers, Vec::new())
        }
        RangeSelection::Slice { start, end } => {
            let body = cached.bytes[start..=end].to_vec();
            let mut headers =
                soda_audio_headers(&cached.content_type, body.len(), cached.bytes.len());
            headers.insert(
                "content-range",
                HeaderValue::from_str(&format!("bytes {start}-{end}/{}", cached.bytes.len()))
                    .unwrap(),
            );
            build_response(StatusCode::PARTIAL_CONTENT, headers, body)
        }
        RangeSelection::None => {
            let headers =
                soda_audio_headers(&cached.content_type, cached.bytes.len(), cached.bytes.len());
            build_response(StatusCode::OK, headers, cached.bytes.clone())
        }
    }
}

fn soda_audio_headers(content_type: &str, content_length: usize, _total: usize) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    headers.insert(
        "content-type",
        HeaderValue::from_str(content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("audio/mp4")),
    );
    headers.insert(
        "content-length",
        HeaderValue::from_str(&content_length.to_string()).unwrap(),
    );
    headers.insert("accept-ranges", HeaderValue::from_static("bytes"));
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    headers.insert("x-soda-audio-decrypted", HeaderValue::from_static("1"));
    headers.insert("x-soda-audio-cache", HeaderValue::from_static("hit"));
    headers
}

fn build_response(status: StatusCode, headers: HeaderMap, body: Vec<u8>) -> Response {
    Response::builder()
        .status(status)
        .body(Body::from(body))
        .map(|mut response| {
            *response.headers_mut() = headers;
            response
        })
        .unwrap_or_else(|_| upstream_failure("soda audio proxy failed"))
}

fn parse_target_url(target: &str) -> Result<url::Url, &'static str> {
    if target.trim().is_empty() {
        return Err("url required");
    }
    let url = url::Url::parse(target).map_err(|_| "invalid url")?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        _ => Err("url must use http or https"),
    }
}

fn bad_request(message: impl Into<String>) -> Response {
    fail(StatusCode::BAD_REQUEST, "BAD_REQUEST", message)
}

fn upstream_failure(message: impl Into<String>) -> Response {
    fail(StatusCode::BAD_GATEWAY, "SODA_AUDIO_PROXY", message)
}

fn parse_range(range_header: Option<&str>, total_length: usize) -> RangeSelection {
    let Some(range_header) = range_header else {
        return RangeSelection::None;
    };
    let Ok(re) = regex::Regex::new(r"(?i)^bytes=(\d*)-(\d*)$") else {
        return RangeSelection::Invalid;
    };
    let Some(captures) = re.captures(range_header.trim()) else {
        return RangeSelection::Invalid;
    };
    let start_raw = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
    let end_raw = captures.get(2).map(|m| m.as_str()).unwrap_or_default();
    if start_raw.is_empty() && end_raw.is_empty() {
        return RangeSelection::Invalid;
    }

    if start_raw.is_empty() {
        let Ok(suffix_length) = end_raw.parse::<usize>() else {
            return RangeSelection::Invalid;
        };
        if suffix_length == 0 {
            return RangeSelection::Invalid;
        }
        let start = total_length.saturating_sub(suffix_length);
        if start >= total_length {
            return RangeSelection::Invalid;
        }
        return RangeSelection::Slice {
            start,
            end: total_length - 1,
        };
    }

    let Ok(start) = start_raw.parse::<usize>() else {
        return RangeSelection::Invalid;
    };
    let end = end_raw
        .parse::<usize>()
        .ok()
        .filter(|end| *end < total_length)
        .unwrap_or_else(|| total_length.saturating_sub(1));
    if start >= total_length || end < start {
        return RangeSelection::Invalid;
    }
    RangeSelection::Slice { start, end }
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

#[cfg(test)]
pub fn decode_soda_spade_bytes_for_test(spade_key_bytes: &[u8]) -> Vec<u8> {
    SpadeDecryptor::decrypt_spade_inner(spade_key_bytes)
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

pub async fn decrypt_soda_audio_data(
    file_data: Vec<u8>,
    play_auth: String,
) -> anyhow::Result<DecryptDataResult> {
    Ok(decrypt_soda_audio_data_inner(file_data, &play_auth))
}

fn decrypt_soda_audio_data_inner(file_data: Vec<u8>, play_auth: &str) -> DecryptDataResult {
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

    DecryptDataResult {
        data: output,
        decrypted: true,
        reason: "decrypted".to_owned(),
    }
}

fn not_decrypted(data: Vec<u8>, reason: &str) -> DecryptDataResult {
    DecryptDataResult {
        data,
        decrypted: false,
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use std::sync::Mutex as StdMutex;

    fn bytes(values: &[u8]) -> Vec<u8> {
        values.to_vec()
    }

    fn u32_bytes(value: u32) -> Vec<u8> {
        value.to_be_bytes().to_vec()
    }

    fn mp4_box(kind: &str, payload: &[u8]) -> Vec<u8> {
        let mut out = u32_bytes(payload.len() as u32 + 8);
        out.extend_from_slice(kind.as_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn request() -> Request<Body> {
        Request::builder()
            .uri("http://127.0.0.1/providers/soda/audio-proxy")
            .body(Body::empty())
            .unwrap()
    }

    fn range_request(range: &str) -> Request<Body> {
        Request::builder()
            .uri("http://127.0.0.1/providers/soda/audio-proxy")
            .header("range", range)
            .body(Body::empty())
            .unwrap()
    }

    async fn response_text(response: Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[test]
    fn spade_decode_wraps_underflow() {
        assert_eq!(decode_soda_spade_bytes_for_test(&[0xee])[0], 0xff);
    }

    #[tokio::test]
    async fn rejects_missing_play_auth() {
        let service = create_soda_audio_proxy(SodaAudioProxyDeps::default());
        let response = service
            .resolve(SodaAudioProxyRequest {
                target: "https://media.example.test/song.m4a".to_owned(),
                play_auth: None,
                request: request(),
            })
            .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(response_text(response).await.contains("playAuth required"));
    }

    #[tokio::test]
    async fn rejects_subsample_encryption() {
        let play_auth = "AHBg".to_owned();
        let mut senc_payload = u32_bytes(0x00000002);
        senc_payload.extend_from_slice(&u32_bytes(1));
        senc_payload.extend_from_slice(&bytes(&[0, 1, 2, 3, 4, 5, 6, 7]));
        senc_payload.extend_from_slice(&0_u16.to_be_bytes());
        senc_payload.extend_from_slice(&0_u16.to_be_bytes());
        senc_payload.extend_from_slice(&u32_bytes(4));
        let senc = mp4_box("senc", &senc_payload);
        let mut stsz_payload = u32_bytes(0);
        stsz_payload.extend_from_slice(&u32_bytes(1));
        stsz_payload.extend_from_slice(&u32_bytes(4));
        let stsz = mp4_box("stsz", &stsz_payload);
        let stbl = mp4_box("stbl", &stsz);
        let minf = mp4_box("minf", &stbl);
        let mdia = mp4_box("mdia", &minf);
        let trak = mp4_box("trak", &mdia);
        let moov = mp4_box("moov", &concat_bytes(&[&senc, &trak]));
        let file_data = concat_bytes(&[&moov, &mp4_box("mdat", &bytes(&[1, 2, 3, 4]))]);

        let result = decrypt_soda_audio_data(file_data.clone(), play_auth)
            .await
            .unwrap();
        assert_eq!(
            result,
            DecryptDataResult {
                data: file_data,
                decrypted: false,
                reason: "soda audio subsample encryption is not supported".to_owned()
            }
        );
    }

    #[tokio::test]
    async fn caches_decrypted_bytes_after_first_request() {
        let fetch_calls = Arc::new(StdMutex::new(Vec::<String>::new()));
        let decrypt_calls = Arc::new(StdMutex::new(Vec::<String>::new()));
        let fetch_calls_for_dep = Arc::clone(&fetch_calls);
        let decrypt_calls_for_dep = Arc::clone(&decrypt_calls);
        let service = create_soda_audio_proxy(SodaAudioProxyDeps {
            fetch: Arc::new(move |target| {
                let fetch_calls = Arc::clone(&fetch_calls_for_dep);
                async move {
                    fetch_calls.lock().unwrap().push(target);
                    let mut headers = HeaderMap::new();
                    headers.insert("content-type", HeaderValue::from_static("audio/mp4"));
                    Ok(SodaAudioFetchResponse {
                        status: StatusCode::OK,
                        headers,
                        body: b"upstream-bytes".to_vec(),
                    })
                }
                .boxed()
            }),
            decrypt: Arc::new(move |_bytes, play_auth| {
                let decrypt_calls = Arc::clone(&decrypt_calls_for_dep);
                async move {
                    decrypt_calls.lock().unwrap().push(play_auth);
                    Ok(DecryptDataResult {
                        data: b"abcdefghij".to_vec(),
                        decrypted: true,
                        reason: "decrypted".to_owned(),
                    })
                }
                .boxed()
            }),
            max_cache_entries: DEFAULT_MAX_CACHE_ENTRIES,
        });

        let target = "https://media.example.test/cache-song.m4a";
        let play_auth = "play-auth-cache";
        let first = service
            .resolve(SodaAudioProxyRequest {
                target: target.to_owned(),
                play_auth: Some(play_auth.to_owned()),
                request: request(),
            })
            .await;
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(response_text(first).await, "abcdefghij");

        let second = service
            .resolve(SodaAudioProxyRequest {
                target: target.to_owned(),
                play_auth: Some(play_auth.to_owned()),
                request: request(),
            })
            .await;
        assert_eq!(second.status(), StatusCode::OK);
        assert_eq!(second.headers().get("x-soda-audio-cache").unwrap(), "hit");
        assert_eq!(second.headers().get("accept-ranges").unwrap(), "bytes");
        assert_eq!(response_text(second).await, "abcdefghij");
        assert_eq!(fetch_calls.lock().unwrap().as_slice(), &[target.to_owned()]);
        assert_eq!(
            decrypt_calls.lock().unwrap().as_slice(),
            &[play_auth.to_owned()]
        );
    }

    #[tokio::test]
    async fn serves_range_requests_from_cached_decrypted_bytes() {
        let fetch_calls = Arc::new(StdMutex::new(Vec::<String>::new()));
        let fetch_calls_for_dep = Arc::clone(&fetch_calls);
        let service = create_soda_audio_proxy(SodaAudioProxyDeps {
            fetch: Arc::new(move |target| {
                let fetch_calls = Arc::clone(&fetch_calls_for_dep);
                async move {
                    fetch_calls.lock().unwrap().push(target);
                    let mut headers = HeaderMap::new();
                    headers.insert("content-type", HeaderValue::from_static("audio/mp4"));
                    Ok(SodaAudioFetchResponse {
                        status: StatusCode::OK,
                        headers,
                        body: b"upstream-bytes".to_vec(),
                    })
                }
                .boxed()
            }),
            decrypt: Arc::new(|_bytes, _play_auth| {
                async move {
                    Ok(DecryptDataResult {
                        data: b"abcdefghij".to_vec(),
                        decrypted: true,
                        reason: "decrypted".to_owned(),
                    })
                }
                .boxed()
            }),
            max_cache_entries: DEFAULT_MAX_CACHE_ENTRIES,
        });

        let target = "https://media.example.test/range-song.m4a";
        let play_auth = "play-auth-range";
        let warmup = service
            .resolve(SodaAudioProxyRequest {
                target: target.to_owned(),
                play_auth: Some(play_auth.to_owned()),
                request: request(),
            })
            .await;
        assert_eq!(warmup.status(), StatusCode::OK);

        let ranged = service
            .resolve(SodaAudioProxyRequest {
                target: target.to_owned(),
                play_auth: Some(play_auth.to_owned()),
                request: range_request("bytes=2-5"),
            })
            .await;
        assert_eq!(ranged.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            ranged.headers().get("content-range").unwrap(),
            "bytes 2-5/10"
        );
        assert_eq!(ranged.headers().get("content-length").unwrap(), "4");
        assert_eq!(ranged.headers().get("accept-ranges").unwrap(), "bytes");
        assert_eq!(ranged.headers().get("x-soda-audio-cache").unwrap(), "hit");
        assert_eq!(response_text(ranged).await, "cdef");
        assert_eq!(fetch_calls.lock().unwrap().as_slice(), &[target.to_owned()]);
    }

    #[tokio::test]
    async fn evicts_old_decrypted_entries_when_cache_entry_limit_is_reached() {
        let fetch_calls = Arc::new(StdMutex::new(Vec::<String>::new()));
        let fetch_calls_for_dep = Arc::clone(&fetch_calls);
        let service = create_soda_audio_proxy(SodaAudioProxyDeps {
            fetch: Arc::new(move |target| {
                let fetch_calls = Arc::clone(&fetch_calls_for_dep);
                async move {
                    fetch_calls.lock().unwrap().push(target.clone());
                    let mut headers = HeaderMap::new();
                    headers.insert("content-type", HeaderValue::from_static("audio/mp4"));
                    Ok(SodaAudioFetchResponse {
                        status: StatusCode::OK,
                        headers,
                        body: format!("upstream:{target}").into_bytes(),
                    })
                }
                .boxed()
            }),
            decrypt: Arc::new(|bytes, _play_auth| {
                async move {
                    Ok(DecryptDataResult {
                        data: bytes,
                        decrypted: true,
                        reason: "decrypted".to_owned(),
                    })
                }
                .boxed()
            }),
            max_cache_entries: 1,
        });

        let first_target = "https://media.example.test/first.m4a";
        let second_target = "https://media.example.test/second.m4a";
        assert_eq!(
            service
                .resolve(SodaAudioProxyRequest {
                    target: first_target.to_owned(),
                    play_auth: Some("auth-1".to_owned()),
                    request: request(),
                })
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            service
                .resolve(SodaAudioProxyRequest {
                    target: second_target.to_owned(),
                    play_auth: Some("auth-2".to_owned()),
                    request: request(),
                })
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            service
                .resolve(SodaAudioProxyRequest {
                    target: first_target.to_owned(),
                    play_auth: Some("auth-1".to_owned()),
                    request: request(),
                })
                .await
                .status(),
            StatusCode::OK
        );

        assert_eq!(
            fetch_calls.lock().unwrap().as_slice(),
            &[
                first_target.to_owned(),
                second_target.to_owned(),
                first_target.to_owned()
            ]
        );
    }
}
