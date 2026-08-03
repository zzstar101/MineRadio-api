use std::{collections::VecDeque, sync::Arc};

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, Request, StatusCode},
    response::Response,
};
use futures::future::{BoxFuture, FutureExt};
use tokio::sync::Mutex;

use crate::http::response::fail;
use crate::utils::cryptors::qq::audio::{self, EncryptedTail, TailFormat};

const DEFAULT_MAX_CACHE_ENTRIES: usize = 12;

// ── Request / Response types ────────────────────────────────────────────

#[derive(Debug)]
pub struct QqAudioProxyRequest {
    pub target: String,
    pub request: Request<Body>,
}

#[derive(Clone)]
pub struct QqAudioProxyDeps {
    pub fetch: QqAudioFetch,
    pub max_cache_entries: usize,
}

pub type QqAudioFetch =
    Arc<dyn Fn(String) -> BoxFuture<'static, anyhow::Result<QqAudioFetchResponse>> + Send + Sync>;

pub struct QqAudioFetchResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Default for QqAudioProxyDeps {
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
                    Ok(QqAudioFetchResponse {
                        status,
                        headers,
                        body,
                    })
                }
                .boxed()
            }),
            max_cache_entries: DEFAULT_MAX_CACHE_ENTRIES,
        }
    }
}

// ── Proxy ───────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct QqAudioProxy {
    deps: QqAudioProxyDeps,
    cache: Arc<Mutex<QqAudioCache>>,
}

#[derive(Default)]
struct QqAudioCache {
    entries: std::collections::HashMap<String, CachedQqAudio>,
    order: VecDeque<String>,
}

#[derive(Clone)]
struct CachedQqAudio {
    bytes: Vec<u8>,
    content_type: String,
}

enum RangeSelection {
    None,
    Invalid,
    Slice { start: usize, end: usize },
}

impl QqAudioProxy {
    pub async fn resolve(&self, input: QqAudioProxyRequest) -> Response {
        proxy_qq_audio(input, &self.deps, &self.cache).await
    }
}

pub fn create_qq_audio_proxy(deps: QqAudioProxyDeps) -> QqAudioProxy {
    QqAudioProxy {
        deps,
        cache: Arc::new(Mutex::new(QqAudioCache::default())),
    }
}

// ── Core logic ──────────────────────────────────────────────────────────

async fn proxy_qq_audio(
    input: QqAudioProxyRequest,
    deps: &QqAudioProxyDeps,
    cache: &Arc<Mutex<QqAudioCache>>,
) -> Response {
    let parsed = match parse_target_url(&input.target) {
        Ok(url) => url,
        Err(message) => return bad_request(message),
    };

    let target = parsed.as_str();
    match get_or_create_cached_audio(cache, deps, target).await {
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
    cache: &Arc<Mutex<QqAudioCache>>,
    deps: &QqAudioProxyDeps,
    target: &str,
) -> anyhow::Result<CachedQqAudio> {
    let cache_key = target.to_owned();
    if let Some(existing) = cache.lock().await.get_refresh(&cache_key) {
        return Ok(existing);
    }

    let upstream = (deps.fetch)(target.to_owned()).await?;
    if !upstream.status.is_success() {
        anyhow::bail!("qq audio request returned {}", upstream.status.as_u16());
    }

    let encrypted_body = upstream.body;
    let upstream_content_type = upstream
        .headers
        .get("content-type")
        .and_then(|v| v.to_str().ok());

    let decrypted = decrypt_qq_audio_data(encrypted_body, target, upstream_content_type, None)?;

    let cached = CachedQqAudio {
        bytes: decrypted.data,
        content_type: decrypted.content_type,
    };

    if deps.max_cache_entries > 0 {
        cache
            .lock()
            .await
            .insert(cache_key, cached.clone(), deps.max_cache_entries);
    }
    Ok(cached)
}

// ── Cache ───────────────────────────────────────────────────────────────

impl QqAudioCache {
    fn get_refresh(&mut self, key: &str) -> Option<CachedQqAudio> {
        let item = self.entries.get(key).cloned()?;
        self.order.retain(|existing| existing != key);
        self.order.push_back(key.to_owned());
        Some(item)
    }

    fn insert(&mut self, key: String, value: CachedQqAudio, max_entries: usize) {
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

// ── Decrypt ─────────────────────────────────────────────────────────────

struct QqDecryptResult {
    data: Vec<u8>,
    content_type: String,
}

/// Attempt to decrypt QQ Music encrypted audio data (MGG/MFLAC/MNAC).
///
/// If `explicit_ekey` is provided it takes priority; otherwise the EKey is
/// parsed from the file tail. If the data is not encrypted (no recognizable
/// tail) and no explicit EKey was given, it is passed through unchanged.
fn decrypt_qq_audio_data(
    mut data: Vec<u8>,
    target_url: &str,
    upstream_content_type: Option<&str>,
    explicit_ekey: Option<&str>,
) -> anyhow::Result<QqDecryptResult> {
    // Resolve EKey: explicit > embedded tail > error (if encrypted tail found but no EKey)
    let (ekey_str, audio_size) =
        if let Some(ekey) = explicit_ekey.map(|s| s.trim()).filter(|s| !s.is_empty()) {
            // Explicit EKey provided — still need to find audio_size from tail if possible
            let audio_size = parse_encrypted_tail_from_bytes(&data)
                .map(|t| t.audio_size as usize)
                .unwrap_or(data.len());
            (ekey.to_owned(), audio_size)
        } else {
            match parse_encrypted_tail_from_bytes(&data) {
                Some(tail) => {
                    let ekey = tail.ekey.ok_or_else(|| {
                        anyhow::anyhow!(
                            "qq encrypted file ({:?}) has no embedded EKey — pass it explicitly",
                            tail.format
                        )
                    })?;
                    (ekey, tail.audio_size as usize)
                }
                None => {
                    // Not encrypted — pass through
                    return Ok(QqDecryptResult {
                        content_type: upstream_content_type.unwrap_or("audio/mpeg").to_owned(),
                        data,
                    });
                }
            }
        };

    if audio_size == 0 || audio_size > data.len() {
        anyhow::bail!("qq encrypted audio has invalid audio size ({audio_size})");
    }

    let key = audio::derive_qmc2_key_from_ekey(&ekey_str)
        .map_err(|e| anyhow::anyhow!("qq EKey derivation failed: {e}"))?;

    audio::qmc2_decrypt_in_place(&key, &mut data[..audio_size], 0)
        .map_err(|e| anyhow::anyhow!("qq audio decrypt failed: {e}"))?;

    // Truncate to just the decrypted audio portion (strip the tail)
    data.truncate(audio_size);

    let content_type = detect_content_type(target_url, &data);

    Ok(QqDecryptResult {
        data,
        content_type: content_type.to_owned(),
    })
}

// ── In-memory tail parsing (adapted from audio::parse_encrypted_tail) ───

const MUSICEX_MAGIC: &[u8; 8] = b"musicex\0";
const QTAG_MAGIC: &[u8; 4] = b"QTag";
const STAG_MAGIC: &[u8; 4] = b"STag";

fn parse_encrypted_tail_from_bytes(data: &[u8]) -> Option<EncryptedTail> {
    if data.len() < 8 {
        return None;
    }

    let tail8: &[u8; 8] = data[data.len() - 8..].try_into().ok()?;
    if tail8 == MUSICEX_MAGIC {
        return parse_musicex_tail_from_bytes(data);
    }

    let tail4: &[u8; 4] = tail8[4..].try_into().ok()?;
    if *tail4 == *QTAG_MAGIC || *tail4 == *STAG_MAGIC {
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
    if !tail.ends_with(MUSICEX_MAGIC) {
        return None;
    }

    for (song_range, filename_range, audio_size) in [
        (12..72, 72..168, file_size - tail_size),
        (28..88, 88..184, file_size.saturating_sub(tail_size + 16)),
    ] {
        if filename_range.end > tail.len() || audio_size == 0 {
            continue;
        }
        let song_mid = decode_utf16_field(tail.get(song_range)?);
        let filename = decode_utf16_field(tail.get(filename_range)?);
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
    let (song_mid, ekey) = if tag == *QTAG_MAGIC {
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

fn decode_utf16_field(data: &[u8]) -> String {
    let units = data
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]));
    String::from_utf16_lossy(&units.collect::<Vec<_>>())
        .trim_end_matches('\0')
        .trim()
        .to_owned()
}

// ── Content-type detection ──────────────────────────────────────────────

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

// ── Response helpers ────────────────────────────────────────────────────

fn response_for_cached_audio(cached: &CachedQqAudio, range: RangeSelection) -> Response {
    match range {
        RangeSelection::Invalid => {
            let mut headers = qq_audio_headers(&cached.content_type, 0, cached.bytes.len(), false);
            headers.insert(
                "content-range",
                HeaderValue::from_str(&format!("bytes */{}", cached.bytes.len())).unwrap(),
            );
            build_response(StatusCode::RANGE_NOT_SATISFIABLE, headers, Vec::new())
        }
        RangeSelection::Slice { start, end } => {
            let body = cached.bytes[start..=end].to_vec();
            let mut headers =
                qq_audio_headers(&cached.content_type, body.len(), cached.bytes.len(), true);
            headers.insert(
                "content-range",
                HeaderValue::from_str(&format!("bytes {start}-{end}/{}", cached.bytes.len()))
                    .unwrap(),
            );
            build_response(StatusCode::PARTIAL_CONTENT, headers, body)
        }
        RangeSelection::None => {
            let headers = qq_audio_headers(
                &cached.content_type,
                cached.bytes.len(),
                cached.bytes.len(),
                true,
            );
            build_response(StatusCode::OK, headers, cached.bytes.clone())
        }
    }
}

fn qq_audio_headers(
    content_type: &str,
    content_length: usize,
    _total: usize,
    cache_hit: bool,
) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    headers.insert(
        "content-type",
        HeaderValue::from_str(content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("audio/mpeg")),
    );
    headers.insert(
        "content-length",
        HeaderValue::from_str(&content_length.to_string()).unwrap(),
    );
    headers.insert("accept-ranges", HeaderValue::from_static("bytes"));
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    headers.insert("x-qq-audio-decrypted", HeaderValue::from_static("1"));
    if cache_hit {
        headers.insert("x-qq-audio-cache", HeaderValue::from_static("hit"));
    }
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
        .unwrap_or_else(|_| upstream_failure("qq audio proxy failed"))
}

// ── Shared helpers ──────────────────────────────────────────────────────

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
    fail(StatusCode::BAD_GATEWAY, "QQ_AUDIO_PROXY", message)
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

// ── File-level convenience ──────────────────────────────────────────────

/// Decrypt a QQ Music encrypted audio file (mgg/mflac/mnac) from `input_path`
/// and write the result to `output_path`. Returns the content type of the decrypted audio.
#[allow(dead_code)]
pub fn decrypt_qq_audio_file(
    input_path: &std::path::Path,
    output_path: &std::path::Path,
) -> anyhow::Result<String> {
    let data = std::fs::read(input_path)?;
    let filename = input_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let fake_url = format!("https://media.example.test/{filename}");
    let result = decrypt_qq_audio_data(data, &fake_url, None, None)?;
    std::fs::write(output_path, &result.data)?;
    Ok(result.content_type)
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::cryptors::qq::audio::{TailFormat, qmc2_decrypt_in_place};
    use axum::body::to_bytes;
    use std::sync::Mutex as StdMutex;

    fn request() -> Request<Body> {
        Request::builder()
            .uri("http://127.0.0.1/providers/qq/audio-proxy")
            .body(Body::empty())
            .unwrap()
    }

    fn range_request(range: &str) -> Request<Body> {
        Request::builder()
            .uri("http://127.0.0.1/providers/qq/audio-proxy")
            .header("range", range)
            .body(Body::empty())
            .unwrap()
    }

    async fn response_text(response: Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    /// Build a minimal QTag-encrypted payload for testing.
    fn build_qtag_encrypted_payload(decrypted: &[u8], ekey: &str, song_mid: &str) -> Vec<u8> {
        let key = audio::derive_qmc2_key_from_ekey(ekey).unwrap();
        let mut encrypted = decrypted.to_vec();
        qmc2_decrypt_in_place(&key, &mut encrypted, 0).unwrap(); // encrypt (XOR is symmetric)

        let mut payload = encrypted;
        payload.extend_from_slice(format!("{song_mid},").as_bytes());
        payload.extend_from_slice(ekey.as_bytes());
        let ekey_len = song_mid.len() + 1 + ekey.len();
        payload.extend_from_slice(&(ekey_len as u32).to_le_bytes());
        payload.extend_from_slice(QTAG_MAGIC);
        payload
    }

    /// Build a minimal STag-encrypted payload for testing.
    fn build_stag_encrypted_payload(decrypted: &[u8], ekey: &str) -> Vec<u8> {
        let key = audio::derive_qmc2_key_from_ekey(ekey).unwrap();
        let mut encrypted = decrypted.to_vec();
        qmc2_decrypt_in_place(&key, &mut encrypted, 0).unwrap();

        let mut payload = encrypted;
        payload.extend_from_slice(ekey.as_bytes());
        let ekey_len = ekey.len();
        payload.extend_from_slice(&(ekey_len as u32).to_le_bytes());
        payload.extend_from_slice(STAG_MAGIC);
        payload
    }

    // ── decrypt_qq_audio_data tests ──────────────────────────────────

    #[test]
    fn decrypt_qtag_file_round_trip() {
        let original = b"OggS\x00\x02\x00\x00\x00\x00\x00\x00\x00\x00rest-of-audio-data";
        let ekey = "MTIzNDU2NzhQWhkuzlyHosmotu2+kFP0";
        let payload = build_qtag_encrypted_payload(original, ekey, "001test");

        let result =
            decrypt_qq_audio_data(payload.clone(), "https://example.test/song.mgg", None, None)
                .unwrap();

        assert_eq!(result.data, original.as_slice());
        assert_eq!(result.content_type, "audio/ogg");
    }

    #[test]
    fn decrypt_stag_file_round_trip() {
        let original = b"fLaC\x00\x00\x00\x00rest-of-flac-data";
        let ekey = "MTIzNDU2NzhQWhkuzlyHosmotu2+kFP0";
        let payload = build_stag_encrypted_payload(original, ekey);

        let result = decrypt_qq_audio_data(
            payload.clone(),
            "https://example.test/song.mflac",
            None,
            None,
        )
        .unwrap();

        assert_eq!(result.data, original.as_slice());
        assert_eq!(result.content_type, "audio/flac");
    }

    #[test]
    fn passthrough_unencrypted_data() {
        let data = b"\xff\xfb\x90\x00plain-mp3-data".to_vec();

        let result =
            decrypt_qq_audio_data(data.clone(), "https://example.test/song.mp3", None, None)
                .unwrap();

        assert_eq!(result.data, data);
        assert_eq!(result.content_type, "audio/mpeg");
    }

    #[test]
    fn parses_mflac_from_url_extension() {
        let original = b"fLaC\x00\x00\x00\x00flac-data-here";
        let ekey = "MTIzNDU2NzhQWhkuzlyHosmotu2+kFP0";
        let payload = build_qtag_encrypted_payload(original, ekey, "001abc");

        let result = decrypt_qq_audio_data(
            payload.clone(),
            "https://example.test/song.mflac",
            None,
            None,
        )
        .unwrap();

        assert_eq!(result.content_type, "audio/flac");
    }

    #[test]
    fn detects_ogg_from_magic_bytes() {
        let ogg_header = b"OggS\x00\x02\x00\x00\x00\x00\x00\x00\x00\x00more-data";
        let ekey = "MTIzNDU2NzhQWhkuzlyHosmotu2+kFP0";
        let payload = build_qtag_encrypted_payload(ogg_header, ekey, "001test");

        let result = decrypt_qq_audio_data(
            payload,
            "https://example.test/stream", // no recognizable extension
            None,
            None,
        )
        .unwrap();

        assert_eq!(result.content_type, "audio/ogg");
    }

    // ── Cache tests ─────────────────────────────────────────────────

    #[tokio::test]
    async fn caches_decrypted_bytes_and_hits_on_second_request() {
        let fetch_calls = Arc::new(StdMutex::new(Vec::<String>::new()));
        let fetch_calls_for_dep = Arc::clone(&fetch_calls);

        let original = b"OggS\x00\x02\x00\x00\x00\x00\x00\x00\x00\x00rest-of-audio-data";
        let ekey = "MTIzNDU2NzhQWhkuzlyHosmotu2+kFP0";
        let encrypted_payload = build_qtag_encrypted_payload(original, ekey, "001cache");
        let encrypted_payload_for_dep = encrypted_payload.clone();

        let service = create_qq_audio_proxy(QqAudioProxyDeps {
            fetch: Arc::new(move |target| {
                let fetch_calls = Arc::clone(&fetch_calls_for_dep);
                let encrypted_payload = encrypted_payload_for_dep.clone();
                async move {
                    fetch_calls.lock().unwrap().push(target);
                    let mut headers = HeaderMap::new();
                    headers.insert(
                        "content-type",
                        HeaderValue::from_static("application/octet-stream"),
                    );
                    Ok(QqAudioFetchResponse {
                        status: StatusCode::OK,
                        headers,
                        body: encrypted_payload,
                    })
                }
                .boxed()
            }),
            max_cache_entries: DEFAULT_MAX_CACHE_ENTRIES,
        });

        let target = "https://media.example.test/cache-song.mgg";
        let first = service
            .resolve(QqAudioProxyRequest {
                target: target.to_owned(),
                request: request(),
            })
            .await;
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(
            response_text(first).await,
            String::from_utf8_lossy(original)
        );

        let second = service
            .resolve(QqAudioProxyRequest {
                target: target.to_owned(),
                request: request(),
            })
            .await;
        assert_eq!(second.status(), StatusCode::OK);
        assert_eq!(second.headers().get("x-qq-audio-cache").unwrap(), "hit");
        assert_eq!(
            response_text(second).await,
            String::from_utf8_lossy(original)
        );
        assert_eq!(fetch_calls.lock().unwrap().as_slice(), &[target.to_owned()]);
    }

    #[tokio::test]
    async fn serves_range_requests_from_cache() {
        let original = b"0123456789ABCDEFGHIJ";
        let ekey = "MTIzNDU2NzhQWhkuzlyHosmotu2+kFP0";
        let encrypted_payload = build_qtag_encrypted_payload(original, ekey, "001range");

        let fetch_calls = Arc::new(StdMutex::new(Vec::<String>::new()));
        let fetch_calls_for_dep = Arc::clone(&fetch_calls);
        let encrypted_payload_for_dep = encrypted_payload.clone();

        let service = create_qq_audio_proxy(QqAudioProxyDeps {
            fetch: Arc::new(move |target| {
                let fetch_calls = Arc::clone(&fetch_calls_for_dep);
                let encrypted_payload = encrypted_payload_for_dep.clone();
                async move {
                    fetch_calls.lock().unwrap().push(target);
                    let mut headers = HeaderMap::new();
                    headers.insert(
                        "content-type",
                        HeaderValue::from_static("application/octet-stream"),
                    );
                    Ok(QqAudioFetchResponse {
                        status: StatusCode::OK,
                        headers,
                        body: encrypted_payload,
                    })
                }
                .boxed()
            }),
            max_cache_entries: DEFAULT_MAX_CACHE_ENTRIES,
        });

        let target = "https://media.example.test/range-song.mgg";

        // Warm cache
        let warmup = service
            .resolve(QqAudioProxyRequest {
                target: target.to_owned(),
                request: request(),
            })
            .await;
        assert_eq!(warmup.status(), StatusCode::OK);

        // Range request
        let ranged = service
            .resolve(QqAudioProxyRequest {
                target: target.to_owned(),
                request: range_request("bytes=4-9"),
            })
            .await;
        assert_eq!(ranged.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            ranged.headers().get("content-range").unwrap(),
            &format!("bytes 4-9/{}", original.len())
        );
        assert_eq!(ranged.headers().get("content-length").unwrap(), "6");
        assert_eq!(response_text(ranged).await, "456789");
        assert_eq!(fetch_calls.lock().unwrap().as_slice(), &[target.to_owned()]);
    }

    #[tokio::test]
    async fn evicts_old_entries_when_cache_limit_is_reached() {
        let fetch_calls = Arc::new(StdMutex::new(Vec::<String>::new()));
        let fetch_calls_for_dep = Arc::clone(&fetch_calls);

        let original = b"plain-test-data";
        let ekey = "MTIzNDU2NzhQWhkuzlyHosmotu2+kFP0";

        let service = create_qq_audio_proxy(QqAudioProxyDeps {
            fetch: Arc::new(move |target| {
                let fetch_calls = Arc::clone(&fetch_calls_for_dep);
                let encrypted = build_qtag_encrypted_payload(original, ekey, "001evict");
                async move {
                    fetch_calls.lock().unwrap().push(target);
                    let mut headers = HeaderMap::new();
                    headers.insert(
                        "content-type",
                        HeaderValue::from_static("application/octet-stream"),
                    );
                    Ok(QqAudioFetchResponse {
                        status: StatusCode::OK,
                        headers,
                        body: encrypted,
                    })
                }
                .boxed()
            }),
            max_cache_entries: 1,
        });

        let first_target = "https://media.example.test/first.mgg";
        let second_target = "https://media.example.test/second.mgg";

        assert_eq!(
            service
                .resolve(QqAudioProxyRequest {
                    target: first_target.to_owned(),
                    request: request(),
                })
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            service
                .resolve(QqAudioProxyRequest {
                    target: second_target.to_owned(),
                    request: request(),
                })
                .await
                .status(),
            StatusCode::OK
        );
        // First target evicted — should be fetched again
        assert_eq!(
            service
                .resolve(QqAudioProxyRequest {
                    target: first_target.to_owned(),
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

    #[tokio::test]
    async fn rejects_invalid_url() {
        let service = create_qq_audio_proxy(QqAudioProxyDeps::default());
        let response = service
            .resolve(QqAudioProxyRequest {
                target: "".to_owned(),
                request: request(),
            })
            .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(response_text(response).await.contains("url required"));
    }

    // ── Tail parsing tests ───────────────────────────────────────────

    #[test]
    fn parse_legacy_qtag_tail_from_bytes() {
        let original = b"audio-data-here";
        let ekey = "MTIzNDU2NzhQWhkuzlyHosmotu2+kFP0";
        let payload = build_qtag_encrypted_payload(original, ekey, "001xd0HI0X9GNq");

        let tail = parse_encrypted_tail_from_bytes(&payload).unwrap();
        assert_eq!(tail.format, TailFormat::Legacy);
        assert_eq!(tail.song_mid, "001xd0HI0X9GNq");
        assert_eq!(tail.ekey.as_deref(), Some(ekey));
        assert_eq!(tail.audio_size as usize, original.len());
    }

    #[test]
    fn parse_legacy_stag_tail_from_bytes() {
        let original = b"flac-audio-data";
        let ekey = "MTIzNDU2NzhQWhkuzlyHosmotu2+kFP0";
        let payload = build_stag_encrypted_payload(original, ekey);

        let tail = parse_encrypted_tail_from_bytes(&payload).unwrap();
        assert_eq!(tail.format, TailFormat::Legacy);
        assert_eq!(tail.ekey.as_deref(), Some(ekey));
    }

    #[test]
    fn non_encrypted_data_returns_none() {
        let data = b"\xff\xfb\x90\x00plain-mp3-data";
        assert!(parse_encrypted_tail_from_bytes(data).is_none());
    }

    #[test]
    fn short_data_returns_none() {
        assert!(parse_encrypted_tail_from_bytes(b"short").is_none());
    }

    // ── Content-type tests ───────────────────────────────────────────

    #[test]
    fn content_type_from_mgg_extension() {
        assert_eq!(
            content_type_from_extension("https://example.test/song.mgg"),
            Some("audio/ogg")
        );
    }

    #[test]
    fn content_type_from_mflac_extension() {
        assert_eq!(
            content_type_from_extension("https://example.test/song.mflac"),
            Some("audio/flac")
        );
    }

    #[test]
    fn content_type_from_mnac_extension() {
        assert_eq!(
            content_type_from_extension("https://example.test/song.mnac"),
            Some("audio/nac")
        );
    }

    #[test]
    fn content_type_ignores_query_string() {
        assert_eq!(
            content_type_from_extension("https://example.test/song.mgg?token=abc"),
            Some("audio/ogg")
        );
    }

    #[test]
    fn content_type_from_ogg_magic() {
        let data = b"OggS\x00\x02\x00\x00\x00\x00\x00\x00\x00\x00";
        assert_eq!(content_type_from_magic(data), "audio/ogg");
    }

    #[test]
    fn content_type_from_flac_magic() {
        let data = b"fLaC\x00\x00\x00\x00rest";
        assert_eq!(content_type_from_magic(data), "audio/flac");
    }

    #[test]
    fn content_type_defaults_to_audio_mpeg() {
        let data = b"\x00\x00\x00\x00unknown";
        assert_eq!(content_type_from_magic(data), "audio/mpeg");
    }

    // ── Real-file integration test ──────────────────────────────────
    //
    // Set the QQ_DECRYPT_TEST_FILE environment variable to the path of a
    // real .mgg/.mflac file, then run:
    //
    //   $env:QQ_DECRYPT_TEST_FILE="D:\music\song.mgg"; cargo test decrypt_real_encrypted_file -- --ignored --nocapture
    //
    // Optionally pass an explicit EKey:
    //
    //   $env:QQ_DECRYPT_EKEY="MTIzNDU2NzhQWhkuzlyHosmotu2+kFP0"; cargo test decrypt_real_encrypted_file -- --ignored --nocapture
    //
    // The explicit EKey takes priority over the embedded tail EKey.
    // The decrypted output is written next to the source file with a
    // .decrypted extension plus the detected format suffix.

    #[test]
    #[ignore = "set QQ_DECRYPT_TEST_FILE to a .mgg/.mflac path, then run with --ignored"]
    fn decrypt_real_encrypted_file() {
        let input_path = std::env::var("QQ_DECRYPT_TEST_FILE").expect(
            "QQ_DECRYPT_TEST_FILE env var is not set — export it pointing to a .mgg/.mflac file",
        );

        let input = std::path::Path::new(&input_path);
        assert!(
            input.exists(),
            "QQ_DECRYPT_TEST_FILE does not exist: {input_path}"
        );

        let explicit_ekey = std::env::var("QQ_DECRYPT_EKEY")
            .ok()
            .filter(|v| !v.trim().is_empty());

        let data = std::fs::read(input).expect("failed to read input file");
        let filename = input
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        eprintln!(
            "input  : {input_path}  ({:.2} MiB)",
            data.len() as f64 / 1_048_576.0
        );

        if let Some(ref ekey) = explicit_ekey {
            eprintln!("ekey   : {ekey}  (explicit via QQ_DECRYPT_EKEY)");
        } else if let Some(tail) = parse_encrypted_tail_from_bytes(&data) {
            eprintln!(
                "tail   : format={:?}  song_mid={}  audio_size={}  ekey={:?}",
                tail.format,
                tail.song_mid,
                tail.audio_size,
                tail.ekey.as_deref().unwrap_or("<none>"),
            );
        } else {
            eprintln!("tail   : not found (treating as non-encrypted)");
        }

        let fake_url = format!("https://media.example.test/{filename}");
        let result = decrypt_qq_audio_data(data, &fake_url, None, explicit_ekey.as_deref())
            .expect("decrypt failed");

        assert!(!result.data.is_empty(), "decrypted data is empty");

        let out_ext = match result.content_type.as_str() {
            "audio/ogg" => ".ogg",
            "audio/flac" => ".flac",
            "audio/mp4" => ".m4a",
            _ => ".mp3",
        };
        let output_path = input.with_file_name(format!(
            "{}.decrypted{out_ext}",
            input.file_stem().unwrap().to_str().unwrap()
        ));

        std::fs::write(&output_path, &result.data).expect("failed to write output file");

        eprintln!(
            "output : {}  ({:.2} MiB)  content_type={}",
            output_path.display(),
            result.data.len() as f64 / 1_048_576.0,
            result.content_type,
        );
    }
}
