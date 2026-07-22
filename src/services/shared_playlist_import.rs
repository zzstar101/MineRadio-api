use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use anyhow::Context;
use md5::{Digest, Md5};
use regex::Regex;
use reqwest::{
    Client, Method,
    header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue, REFERER, USER_AGENT},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use url::Url;

use crate::{providers::ProviderAdapter, types::ProviderId};

const APPLE_MUSIC_ORIGIN: &str = "https://music.apple.com";
const ITUNES_ORIGIN: &str = "https://itunes.apple.com";
const APPLE_MUSIC_PLAYLIST_TRACK_LIMIT: usize = 500;
const KUGOU_MOBILE_ORIGIN: &str = "https://m.kugou.com";
const KUGOU_MOBILE_ALT_ORIGIN: &str = "https://m3ws.kugou.com";
const KUGOU_SIGN_SECRET: &str = "NVPh5oo715z5DIWAeQlhMDsWXXQV4hwt";
const KUGOU_ANDROID_SIGN_SECRET: &str = "OIlwieks28dk2k092lksi2UIkp";
const KUGOU_SHARED_PLAYLIST_TRACK_LIMIT: usize = 500;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SharedPlaylistCandidate {
    pub provider: String,
    pub id: String,
    #[serde(rename = "sourceUrl")]
    pub source_url: String,
}

pub struct SharedPlaylistImporterDeps {
    pub provider_adapters: HashMap<ProviderId, Arc<dyn ProviderAdapter>>,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct SharedPlaylistImportError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
struct SharedPlaylistImportRequest {
    url: Option<String>,
    text: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct ExternalTrack {
    id: Option<String>,
    name: String,
    artist: Option<String>,
    artists: Vec<String>,
    album: Option<String>,
    cover: Option<String>,
    duration: Option<u64>,
}

#[derive(Clone, Debug, Default)]
struct FetchTextResponse {
    text: String,
    url: String,
}

#[derive(Clone, Debug, Default)]
struct KugouShareInfo {
    gcid: String,
    global_collection_id: String,
    uid: String,
    cover: String,
    title: String,
}

#[derive(Clone, Debug, Default)]
struct KugouPlaylistPayload {
    id: String,
    name: String,
    cover: String,
    track_count: usize,
    tracks: Vec<ExternalTrack>,
}

pub async fn import_shared_playlist(
    input: Value,
    deps: SharedPlaylistImporterDeps,
) -> anyhow::Result<Value> {
    let request: SharedPlaylistImportRequest =
        serde_json::from_value(input).context("invalid shared playlist import payload")?;
    let candidate = detect_shared_playlist(json!({
        "url": request.url,
        "text": request.text,
    }))
    .ok_or_else(|| SharedPlaylistImportError {
        code: "UNSUPPORTED_LINK".to_owned(),
        message: "unsupported shared playlist link".to_owned(),
    })?;

    if candidate.provider == "apple-music" {
        return import_apple_music_playlist(&candidate).await;
    }
    if candidate.provider == "kugou" {
        return import_kugou_playlist(&candidate).await;
    }
    if !matches!(candidate.provider.as_str(), "qq" | "netease" | "soda") {
        return Err(SharedPlaylistImportError {
            code: "NOT_IMPLEMENTED".to_owned(),
            message: format!(
                "{} shared playlist import is not migrated yet",
                candidate.provider
            ),
        }
        .into());
    }

    let adapter = candidate
        .provider
        .parse::<ProviderId>()
        .ok()
        .and_then(|pid| deps.provider_adapters.get(&pid).cloned())
        .ok_or_else(|| SharedPlaylistImportError {
            code: "UNSUPPORTED_PROVIDER".to_owned(),
            message: "unsupported shared playlist provider".to_owned(),
        })?;

    let detail = adapter.playlist_detail(&candidate.id, 0, 500).await?;
    let tracks = detail.tracks;
    let loaded_count = tracks.len();
    let track_ids = tracks
        .iter()
        .map(|track| track.id.clone())
        .collect::<Vec<_>>();
    let track_count = loaded_count;

    Ok(json!({
        "provider": candidate.provider,
        "playlist": {
            "provider": candidate.provider,
            "id": if detail.id.is_empty() { candidate.id } else { detail.id },
            "name": detail.name,
            "trackCount": track_count,
            "trackIds": track_ids,
            "collected": false,
            "sourceUrl": candidate.source_url
        },
        "tracks": tracks,
        "trackCount": track_count,
        "loadedCount": loaded_count,
        "partial": false,
        "partialReason": ""
    }))
}

async fn import_kugou_playlist(candidate: &SharedPlaylistCandidate) -> anyhow::Result<Value> {
    let source = candidate.source_url.clone();
    let info = parse_kugou_share_input(&source);
    if info.gcid.is_empty() && info.global_collection_id.is_empty() {
        return Err(SharedPlaylistImportError {
            code: "KUGOU_MISSING_ID".to_owned(),
            message: "missing Kugou playlist id".to_owned(),
        }
        .into());
    }

    let payload = match kugou_shared_playlist_full(&info).await {
        Ok(payload) => payload,
        Err(_) if !info.gcid.is_empty() => kugou_shared_playlist_from_mobile(&info).await?,
        Err(err) => return Err(err),
    };
    if payload.tracks.is_empty() {
        return Err(SharedPlaylistImportError {
            code: "KUGOU_EMPTY_PLAYLIST".to_owned(),
            message: "Kugou playlist does not contain readable tracks".to_owned(),
        }
        .into());
    }

    let tracks = payload
        .tracks
        .iter()
        .enumerate()
        .map(|(index, song)| import_only_track("kugou", song, index, &payload.cover))
        .collect::<Vec<_>>();

    Ok(json!({
        "provider": "kugou",
        "playlist": {
            "provider": "kugou",
            "id": if payload.id.is_empty() { candidate.id.clone() } else { payload.id },
            "name": if payload.name.is_empty() { "Kugou playlist".to_owned() } else { payload.name },
            "coverUrl": payload.cover,
            "trackCount": payload.track_count,
            "trackIds": tracks.iter().map(|track| track["id"].clone()).collect::<Vec<_>>(),
            "collected": false,
            "sourceUrl": source
        },
        "tracks": tracks,
        "trackCount": payload.track_count,
        "loadedCount": tracks.len(),
        "partial": payload.track_count > tracks.len(),
        "partialReason": if payload.track_count > tracks.len() {
            "Kugou shared page only exposes part of the playlist".to_owned()
        } else {
            String::new()
        }
    }))
}

async fn import_apple_music_playlist(candidate: &SharedPlaylistCandidate) -> anyhow::Result<Value> {
    let target = if candidate.source_url.trim().is_empty() {
        format!("{APPLE_MUSIC_ORIGIN}/cn/playlist/{}", candidate.id)
    } else {
        candidate.source_url.clone()
    };
    let fetched = fetch_text(&target, apple_headers(), 14_000).await?;
    let schema_text = extract_raw_html_match(
        &fetched.text,
        r#"<script[^>]+id=["']?schema:music-playlist["']?[^>]*>([\s\S]*?)</script>"#,
    );
    if schema_text.is_empty() {
        return Err(SharedPlaylistImportError {
            code: "APPLE_METADATA_UNAVAILABLE".to_owned(),
            message: "Apple Music playlist page does not expose public track metadata".to_owned(),
        }
        .into());
    }

    let schema: Map<String, Value> =
        serde_json::from_str(&schema_text).map_err(|_| SharedPlaylistImportError {
            code: "APPLE_PARSE_FAILED".to_owned(),
            message: "failed to parse Apple Music playlist metadata".to_owned(),
        })?;

    let raw_tracks = array_of(schema.get("track"))
        .into_iter()
        .take(APPLE_MUSIC_PLAYLIST_TRACK_LIMIT)
        .collect::<Vec<_>>();
    let track_ids = raw_tracks
        .iter()
        .filter_map(|track| {
            let url = apple_song_url_from_schema(track);
            let id = apple_track_id_from_url(&url);
            (!id.is_empty()).then_some(id)
        })
        .collect::<Vec<_>>();
    let lookup = apple_lookup_tracks(&track_ids).await.unwrap_or_default();
    let cover = normalize_image_url(&first_non_blank(&[
        first_image_from_unknown(schema.get("image")),
        Some(extract_meta(&fetched.text, "og:image")),
        Some(extract_meta(&fetched.text, "twitter:image")),
    ]));

    let songs = raw_tracks
        .iter()
        .enumerate()
        .filter_map(|(index, raw)| {
            let lookup_key = apple_track_id_from_url(&apple_song_url_from_schema(raw));
            normalize_apple_music_track(raw, lookup.get(&lookup_key), index)
        })
        .collect::<Vec<_>>();
    if songs.is_empty() {
        return Err(SharedPlaylistImportError {
            code: "APPLE_EMPTY_PLAYLIST".to_owned(),
            message: "Apple Music playlist does not contain readable tracks".to_owned(),
        }
        .into());
    }

    let tracks = songs
        .iter()
        .enumerate()
        .map(|(index, song)| import_only_track("apple-music", song, index, &cover))
        .collect::<Vec<_>>();
    let total = number_u64(schema.get("numTracks"))
        .unwrap_or(tracks.len() as u64)
        .max(tracks.len() as u64);

    Ok(json!({
        "provider": "apple-music",
        "playlist": {
            "provider": "apple-music",
            "id": if candidate.id.is_empty() {
                parse_apple_playlist_id(string_value(schema.get("url")).as_str())
            } else {
                candidate.id.clone()
            },
            "name": first_non_blank(&[
                Some(clean_external_text(string_value(schema.get("name")).as_str())),
                Some(extract_meta(&fetched.text, "og:title")),
                Some("Apple Music playlist".to_owned()),
            ]),
            "coverUrl": cover,
            "trackCount": total,
            "trackIds": tracks.iter().map(|track| track["id"].clone()).collect::<Vec<_>>(),
            "collected": false,
            "sourceUrl": if fetched.url.is_empty() { target } else { fetched.url }
        },
        "tracks": tracks,
        "trackCount": total,
        "loadedCount": tracks.len(),
        "partial": total as usize > tracks.len(),
        "partialReason": if total as usize > tracks.len() {
            "Apple Music shared page only exposes part of the playlist".to_owned()
        } else {
            String::new()
        }
    }))
}

pub fn detect_shared_playlist(input: Value) -> Option<SharedPlaylistCandidate> {
    let request: SharedPlaylistImportRequest = serde_json::from_value(input).ok()?;
    for raw in collect_candidates(&request) {
        let parsed = Url::parse(&raw).ok()?;
        if let Some(candidate) = detect_qq_playlist(&parsed, &raw) {
            return Some(candidate);
        }
        if let Some(candidate) = detect_netease_playlist(&parsed, &raw) {
            return Some(candidate);
        }
        if let Some(candidate) = detect_apple_music_playlist(&raw, &parsed) {
            return Some(candidate);
        }
        if let Some(candidate) = detect_qishui_playlist(&raw, &parsed) {
            return Some(candidate);
        }
        if let Some(candidate) = detect_kugou_playlist(&raw, &parsed) {
            return Some(candidate);
        }
    }
    None
}

fn collect_candidates(input: &SharedPlaylistImportRequest) -> Vec<String> {
    let mut out = Vec::new();
    for value in [input.url.as_deref(), input.text.as_deref()]
        .into_iter()
        .flatten()
    {
        let trimmed = clean_candidate(value);
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            out.push(trimmed.clone());
        }
        let re = Regex::new(r#"https?://[^\s"'<>]+"#).unwrap();
        for capture in re.find_iter(&trimmed) {
            out.push(clean_candidate(capture.as_str()));
        }
    }
    out.sort();
    out.dedup();
    out
}

fn clean_candidate(value: &str) -> String {
    value
        .trim()
        .trim_end_matches(|ch: char| {
            matches!(
                ch,
                ',' | '.' | ';' | '"' | '\'' | ')' | ']' | '>' | '，' | '。' | '；' | '！' | '？'
            )
        })
        .to_owned()
}

fn host_matches(hostname: &str, suffix: &str) -> bool {
    let host = hostname.to_lowercase();
    host == suffix || host.ends_with(&format!(".{suffix}"))
}

fn hash_search_params(url: &Url) -> Vec<(String, String)> {
    let hash = url.fragment().unwrap_or_default();
    let query = hash
        .split_once('?')
        .map(|(_, query)| query)
        .unwrap_or_default();
    url::form_urlencoded::parse(query.as_bytes())
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

fn first_non_blank(values: &[Option<String>]) -> String {
    values
        .iter()
        .flatten()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .unwrap_or_default()
        .to_owned()
}

fn detect_qq_playlist(url: &Url, source_url: &str) -> Option<SharedPlaylistCandidate> {
    if !host_matches(url.host_str().unwrap_or_default(), "y.qq.com") {
        return None;
    }
    let path = url.path();
    let id = first_non_blank(&[
        url.query_pairs()
            .find(|(key, _)| key == "id")
            .map(|(_, value)| value.into_owned()),
        url.query_pairs()
            .find(|(key, _)| key == "disstid")
            .map(|(_, value)| value.into_owned()),
        url.query_pairs()
            .find(|(key, _)| key == "tid")
            .map(|(_, value)| value.into_owned()),
        Regex::new(r"/n/ryqq/playlist/([^/?#]+)")
            .unwrap()
            .captures(path)
            .and_then(|cap| cap.get(1).map(|m| m.as_str().to_owned())),
    ]);
    if id.is_empty() {
        return None;
    }
    Some(SharedPlaylistCandidate {
        provider: ProviderId::Qq.to_string(),
        id,
        source_url: source_url.to_owned(),
    })
}

fn detect_netease_playlist(url: &Url, source_url: &str) -> Option<SharedPlaylistCandidate> {
    let host = url.host_str().unwrap_or_default();
    if !host_matches(host, "music.163.com") && !host_matches(host, "music.163.com.cn") {
        return None;
    }
    let hash_params = hash_search_params(url);
    let path = url.path();
    let id = first_non_blank(&[
        url.query_pairs()
            .find(|(key, _)| key == "id")
            .map(|(_, value)| value.into_owned()),
        hash_params
            .iter()
            .find(|(key, _)| key == "id")
            .map(|(_, value)| value.clone()),
        Regex::new(r"/playlist/(\d+)")
            .unwrap()
            .captures(path)
            .and_then(|cap| cap.get(1).map(|m| m.as_str().to_owned())),
    ]);
    if id.is_empty() {
        return None;
    }
    let playlist_hint = format!("{path}#{}", url.fragment().unwrap_or_default());
    if !Regex::new(r"(^|/)playlist(/|$)|playlist")
        .unwrap()
        .is_match(&playlist_hint)
    {
        return None;
    }
    Some(SharedPlaylistCandidate {
        provider: ProviderId::Netease.to_string(),
        id,
        source_url: source_url.to_owned(),
    })
}

fn detect_apple_music_playlist(source_url: &str, url: &Url) -> Option<SharedPlaylistCandidate> {
    let host = url.host_str().unwrap_or_default();
    if !host_matches(host, "music.apple.com") && !host_matches(host, "itunes.apple.com") {
        return None;
    }
    let id = parse_apple_playlist_id(source_url);
    if id.is_empty() {
        return None;
    }
    Some(SharedPlaylistCandidate {
        provider: "apple-music".to_owned(),
        id,
        source_url: source_url.to_owned(),
    })
}

fn detect_qishui_playlist(source_url: &str, url: &Url) -> Option<SharedPlaylistCandidate> {
    let host = url.host_str().unwrap_or_default();
    if !host_matches(host, "qishui.douyin.com") && !host_matches(host, "music.douyin.com") {
        return None;
    }
    let path = url.path();
    let id = first_non_blank(&[
        url.query_pairs()
            .find(|(key, _)| key == "playlist_id")
            .map(|(_, value)| value.into_owned()),
        Regex::new(r"/s/([^/?#]+)")
            .unwrap()
            .captures(path)
            .and_then(|cap| cap.get(1).map(|m| m.as_str().to_owned())),
        parse_qishui_playlist_id(source_url),
        Some(simple_hash_hex(source_url)),
    ]);
    Some(SharedPlaylistCandidate {
        provider: "qishui".to_owned(),
        id,
        source_url: source_url.to_owned(),
    })
}

fn detect_kugou_playlist(source_url: &str, url: &Url) -> Option<SharedPlaylistCandidate> {
    if !host_matches(url.host_str().unwrap_or_default(), "kugou.com") {
        return None;
    }
    let info = parse_kugou_share_input(source_url);
    let id = first_non_blank(&[
        (!info.global_collection_id.is_empty()).then_some(info.global_collection_id.clone()),
        (!info.gcid.is_empty()).then_some(info.gcid.clone()),
        Regex::new(r"gcid_([a-z0-9]+)")
            .unwrap()
            .captures(url.path())
            .and_then(|cap| cap.get(1).map(|m| m.as_str().to_owned())),
        Some(simple_hash_hex(source_url)),
    ]);
    Some(SharedPlaylistCandidate {
        provider: ProviderId::Kugou.to_string(),
        id,
        source_url: source_url.to_owned(),
    })
}

fn parse_apple_playlist_id(value: &str) -> String {
    Regex::new(r"\bpl\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\b")
        .unwrap()
        .find(value.trim())
        .map(|match_| match_.as_str().to_owned())
        .unwrap_or_default()
}

fn parse_qishui_playlist_id(value: &str) -> Option<String> {
    Regex::new(r"(?:playlist_id|playlist)[:=/\s]+(\d{5,})")
        .unwrap()
        .captures(value)
        .and_then(|cap| cap.get(1).map(|m| m.as_str().to_owned()))
}

fn parse_kugou_share_input(value: &str) -> KugouShareInfo {
    let raw = value.trim();
    let url_text = external_url_from_input(raw);
    let parsed = Url::parse(&url_text).ok();
    let source = parsed
        .as_ref()
        .map(Url::to_string)
        .unwrap_or_else(|| raw.to_owned());
    let gcid = Regex::new(r"gcid_([a-z0-9]+)")
        .unwrap()
        .captures(&source)
        .and_then(|captures| captures.get(1).map(|value| value.as_str().to_owned()))
        .or_else(|| {
            Regex::new(r"[?&]src_cid=(?:gcid_)?([a-z0-9]+)")
                .unwrap()
                .captures(&source)
                .and_then(|captures| captures.get(1).map(|value| value.as_str().to_owned()))
        })
        .unwrap_or_default();
    let global_collection_id =
        Regex::new(r"[?&](?:global_collection_id|global_specialid)=([a-z0-9_]+)")
            .unwrap()
            .captures(&source)
            .and_then(|captures| captures.get(1).map(|value| value.as_str().to_owned()))
            .or_else(|| {
                Regex::new(r"\bcollection_[a-z0-9_]+\b")
                    .unwrap()
                    .find(&source)
                    .map(|value| value.as_str().to_owned())
            })
            .unwrap_or_default();

    KugouShareInfo {
        gcid,
        global_collection_id,
        uid: parsed
            .as_ref()
            .and_then(|url| {
                url.query_pairs()
                    .find(|(key, _)| key == "uid")
                    .map(|(_, value)| value.into_owned())
            })
            .unwrap_or_default(),
        cover: parsed
            .as_ref()
            .and_then(|url| {
                url.query_pairs()
                    .find(|(key, _)| key == "cover")
                    .map(|(_, value)| value.into_owned())
            })
            .unwrap_or_default(),
        title: Regex::new(r#"歌单[《"]?([^》"\n]+)"#)
            .unwrap()
            .captures(raw)
            .and_then(|captures| {
                captures
                    .get(1)
                    .map(|value| clean_external_text(value.as_str()))
            })
            .unwrap_or_default(),
    }
}

fn simple_hash_hex(value: &str) -> String {
    let mut hash: u32 = 2_166_136_261;
    for byte in value.as_bytes() {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(16_777_619);
    }
    format!("{hash:x}")
}

fn md5_hex(value: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn array_of(value: Option<&Value>) -> Vec<Value> {
    value.and_then(Value::as_array).cloned().unwrap_or_default()
}

fn record(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn string_value(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(boolean)) => boolean.to_string(),
        Some(other) if !other.is_null() => other.to_string(),
        _ => String::new(),
    }
}

fn value_string(value: Option<&Value>) -> Option<String> {
    let value = string_value(value);
    if value.is_empty() { None } else { Some(value) }
}

fn number_u64(value: Option<&Value>) -> Option<u64> {
    value.and_then(Value::as_u64).or_else(|| {
        value
            .and_then(Value::as_i64)
            .and_then(|number| u64::try_from(number).ok())
    })
}

fn import_only_track(
    source: &str,
    song: &ExternalTrack,
    index: usize,
    playlist_cover: &str,
) -> Value {
    let title = clean_external_text(&song.name);
    let artists = if !song.artists.is_empty() {
        song.artists
            .iter()
            .map(|artist| clean_external_text(artist))
            .filter(|artist| !artist.is_empty())
            .collect::<Vec<_>>()
    } else {
        split_artist_names(song.artist.as_deref().unwrap_or_default())
    };
    let stable = simple_hash_hex(
        format!(
            "{}|{}|{}|{}|{}",
            source,
            song.id.as_deref().unwrap_or_default(),
            title,
            artists.join("/"),
            index
        )
        .as_str(),
    );
    let id = format!("import:{}:{}", source, song.id.clone().unwrap_or(stable));
    json!({
        "provider": "netease",
        "id": id.clone(),
        "sourceId": id,
        "title": title,
        "artists": if artists.is_empty() { vec!["Unknown Artist".to_owned()] } else { artists },
        "album": clean_external_text(song.album.as_deref().unwrap_or_default()),
        "coverUrl": normalize_image_url(song.cover.as_deref().unwrap_or(playlist_cover)),
        "durationMs": song.duration,
        "qualityHints": [],
        "playableState": "unknown"
    })
}

fn split_artist_names(value: &str) -> Vec<String> {
    let text = clean_external_text(value);
    if text.is_empty() {
        return Vec::new();
    }
    text.split(&['/', ',', '&'][..])
        .map(clean_external_text)
        .filter(|part| !part.is_empty())
        .collect()
}

fn normalize_apple_music_track(
    raw: &Value,
    lookup: Option<&Map<String, Value>>,
    index: usize,
) -> Option<ExternalTrack> {
    let item = record(raw);
    let lookup_item = lookup.cloned().unwrap_or_default();
    let audio = item
        .get("audio")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let name = clean_external_text(&first_non_blank(&[
        value_string(lookup_item.get("trackName")),
        value_string(item.get("name")),
        value_string(audio.get("name")),
    ]));
    if name.is_empty() {
        return None;
    }
    let song_url = apple_song_url_from_schema(raw);
    let id = first_non_blank(&[
        value_string(lookup_item.get("trackId")),
        Some(apple_track_id_from_url(&song_url)),
        value_string(item.get("id")),
        Some(format!("apple-{index}")),
    ]);

    Some(ExternalTrack {
        id: Some(id),
        name,
        artist: Some(clean_external_text(&first_non_blank(&[
            value_string(lookup_item.get("artistName")),
            artist_name_from_unknown(item.get("byArtist")),
        ]))),
        artists: Vec::new(),
        album: value_string(lookup_item.get("collectionName"))
            .or_else(|| value_string(item.get("inAlbum").and_then(|value| value.get("name")))),
        cover: Some(normalize_image_url(
            first_non_blank(&[
                value_string(lookup_item.get("artworkUrl100"))
                    .map(|url| url.replace("100x100bb", "600x600bb")),
                first_image_from_unknown(item.get("image")),
                first_image_from_unknown(audio.get("thumbnailUrl")),
                value_string(item.get("thumbnailUrl")),
            ])
            .as_str(),
        )),
        duration: lookup_item
            .get("trackTimeMillis")
            .and_then(Value::as_u64)
            .or_else(|| {
                let duration = first_non_blank(&[
                    value_string(item.get("duration")),
                    value_string(audio.get("duration")),
                ]);
                let millis = parse_iso_duration_ms(&duration);
                (millis > 0).then_some(millis)
            }),
    })
}

fn apple_song_url_from_schema(raw: &Value) -> String {
    let item = record(raw);
    let audio = item
        .get("audio")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let potential_action = audio
        .get("potentialAction")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let target = potential_action
        .get("target")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    first_non_blank(&[
        value_string(item.get("url")),
        value_string(audio.get("url")),
        first_string_from_unknown(target.get("actionPlatform")),
        first_string_from_unknown(target.get("url")),
    ])
}

async fn apple_lookup_tracks(
    ids: &[String],
) -> anyhow::Result<HashMap<String, Map<String, Value>>> {
    let unique = ids
        .iter()
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
        .collect::<HashSet<_>>()
        .into_iter()
        .take(APPLE_MUSIC_PLAYLIST_TRACK_LIMIT)
        .collect::<Vec<_>>();
    let client = Client::new();
    let mut out = HashMap::new();

    for chunk in unique.chunks(100) {
        let params = [
            ("id", chunk.join(",")),
            ("entity", "song".to_owned()),
            ("country", "CN".to_owned()),
        ];
        let url = format!(
            "{ITUNES_ORIGIN}/lookup?{}",
            url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(params)
                .finish()
        );
        let response = client
            .get(url)
            .headers(apple_headers())
            .send()
            .await
            .context("send apple lookup request")?;
        let data = response
            .json::<Value>()
            .await
            .context("decode apple lookup response")?;
        for item in array_of(data.get("results")) {
            let row = record(&item);
            if row.get("wrapperType").and_then(Value::as_str) != Some("track") {
                continue;
            }
            let Some(track_id) = row.get("trackId").map(|value| string_value(Some(value))) else {
                continue;
            };
            if !track_id.is_empty() {
                out.insert(track_id, row);
            }
        }
    }

    Ok(out)
}

async fn fetch_text(
    url: &str,
    headers: HeaderMap,
    timeout_ms: u64,
) -> anyhow::Result<FetchTextResponse> {
    let response = Client::new()
        .get(url)
        .headers(headers)
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .send()
        .await
        .with_context(|| format!("fetch {url}"))?;
    let final_url = response.url().to_string();
    let response = response
        .error_for_status()
        .with_context(|| format!("request {url} failed"))?;
    let text = response
        .text()
        .await
        .with_context(|| format!("read {url} body"))?;
    Ok(FetchTextResponse {
        text,
        url: final_url,
    })
}

fn apple_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126 Safari/537.36",
        ),
    );
    headers.insert(
        REFERER,
        HeaderValue::from_static("https://music.apple.com/"),
    );
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("text/html,application/json;q=0.9,*/*;q=0.8"),
    );
    headers
}

fn extract_raw_html_match(html: &str, pattern: &str) -> String {
    Regex::new(pattern)
        .unwrap()
        .captures(html)
        .and_then(|captures| {
            captures
                .get(1)
                .map(|value| value.as_str().trim().to_owned())
        })
        .unwrap_or_default()
}

fn extract_meta(html: &str, name: &str) -> String {
    let escaped = regex::escape(name);
    let pattern = format!(
        r#"<meta[^>]+(?:name|property|itemprop)=["']{escaped}["'][^>]+content=["']([^"']+)["']"#
    );
    clean_external_text(&extract_raw_html_match(html, &pattern))
}

fn clean_external_text(value: &str) -> String {
    decode_html_entities(
        &Regex::new(r"<[^>]+>")
            .unwrap()
            .replace_all(value, "")
            .replace("&nbsp;", " "),
    )
    .replace("\\u002F", "/")
    .replace("\\/", "/")
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ")
    .trim()
    .to_owned()
}

fn decode_html_entities(value: &str) -> String {
    Regex::new(r"&(#x?[0-9a-fA-F]+|[a-zA-Z]+);")
        .unwrap()
        .replace_all(value, |captures: &regex::Captures| {
            let entity = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
            match entity.to_ascii_lowercase().as_str() {
                "amp" => "&".to_owned(),
                "lt" => "<".to_owned(),
                "gt" => ">".to_owned(),
                "quot" => "\"".to_owned(),
                "apos" => "'".to_owned(),
                "nbsp" => " ".to_owned(),
                lower if lower.starts_with("#x") => u32::from_str_radix(&lower[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
                    .map(|ch| ch.to_string())
                    .unwrap_or_default(),
                lower if lower.starts_with('#') => lower[1..]
                    .parse::<u32>()
                    .ok()
                    .and_then(char::from_u32)
                    .map(|ch| ch.to_string())
                    .unwrap_or_default(),
                _ => captures
                    .get(0)
                    .map(|m| m.as_str())
                    .unwrap_or_default()
                    .to_owned(),
            }
        })
        .to_string()
}

fn normalize_image_url(raw: &str) -> String {
    let mut text = clean_external_text(raw);
    if text.is_empty() {
        return String::new();
    }
    if text.starts_with("//") {
        text = format!("https:{text}");
    }
    if text.starts_with("http://") {
        text = text.replacen("http://", "https://", 1);
    }
    if text.starts_with("https://") {
        text
    } else {
        String::new()
    }
}

fn first_image_from_unknown(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Array(items)) => items
            .iter()
            .find_map(|item| first_image_from_unknown(Some(item))),
        Some(Value::Object(map)) => {
            for key in ["url", "contentUrl", "thumbnailUrl"] {
                if let Some(text) = map.get(key).and_then(Value::as_str) {
                    if !text.trim().is_empty() {
                        return Some(text.to_owned());
                    }
                }
            }
            first_string_from_unknown(map.get("url_list"))
        }
        _ => None,
    }
}

fn first_string_from_unknown(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Array(items)) => items
            .iter()
            .find_map(|item| first_string_from_unknown(Some(item))),
        Some(Value::Object(map)) => first_non_blank(&[
            value_string(map.get("url")),
            value_string(map.get("href")),
            value_string(map.get("@id")),
        ])
        .into(),
        _ => None,
    }
}

fn artist_name_from_unknown(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => Some(clean_external_text(text)),
        Some(Value::Array(items)) => {
            let names = items
                .iter()
                .filter_map(|item| artist_name_from_unknown(Some(item)))
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            (!names.is_empty()).then_some(names.join(" / "))
        }
        Some(Value::Object(map)) => Some(clean_external_text(
            first_non_blank(&[
                value_string(map.get("name")),
                value_string(map.get("artistName")),
                value_string(map.get("nickname")),
            ])
            .as_str(),
        )),
        _ => None,
    }
}

fn parse_iso_duration_ms(value: &str) -> u64 {
    let captures = Regex::new(r"^P(?:T)?(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?$")
        .unwrap()
        .captures(value.trim());
    let Some(captures) = captures else {
        return 0;
    };
    let hours = captures
        .get(1)
        .and_then(|value| value.as_str().parse::<u64>().ok())
        .unwrap_or(0);
    let minutes = captures
        .get(2)
        .and_then(|value| value.as_str().parse::<u64>().ok())
        .unwrap_or(0);
    let seconds = captures
        .get(3)
        .and_then(|value| value.as_str().parse::<u64>().ok())
        .unwrap_or(0);
    (hours * 3600 + minutes * 60 + seconds) * 1000
}

fn apple_track_id_from_url(value: &str) -> String {
    [
        r"/song/[^/?#]+/(\d{5,})",
        r"[?&]i=(\d{5,})",
        r"/(\d{5,})(?:[?#]|$)",
    ]
    .iter()
    .find_map(|pattern| {
        Regex::new(pattern)
            .unwrap()
            .captures(value)
            .and_then(|captures| captures.get(1).map(|value| value.as_str().to_owned()))
    })
    .unwrap_or_default()
}

async fn kugou_shared_playlist_full(info: &KugouShareInfo) -> anyhow::Result<KugouPlaylistPayload> {
    let mut global_collection_id = info.global_collection_id.trim().to_owned();
    if global_collection_id.is_empty() && !info.gcid.is_empty() {
        global_collection_id = kugou_decode_gcid(&info.gcid).await?;
    }
    if global_collection_id.is_empty() {
        anyhow::bail!("no kugou collection id");
    }
    let list_info = kugou_collection_info(&global_collection_id).await?;
    let total = number_u64(
        list_info
            .get("songcount")
            .or_else(|| list_info.get("count"))
            .or_else(|| list_info.get("total")),
    )
    .unwrap_or(KUGOU_SHARED_PLAYLIST_TRACK_LIMIT as u64) as usize;
    let songs = kugou_collection_songs(&global_collection_id, total).await?;
    let payload =
        normalize_kugou_collection_playlist(&global_collection_id, &list_info, &songs, info);
    if payload.tracks.is_empty() {
        anyhow::bail!("empty kugou collection");
    }
    Ok(payload)
}

async fn kugou_shared_playlist_from_mobile(
    info: &KugouShareInfo,
) -> anyhow::Result<KugouPlaylistPayload> {
    let mobile_url = kugou_mobile_songlist_url(info);
    if mobile_url.is_empty() {
        anyhow::bail!("missing kugou mobile url");
    }
    let fetched = fetch_text(&mobile_url, kugou_mobile_headers(), 12_000).await?;
    let json_text = extract_window_output_json(&fetched.text);
    if json_text.is_empty() {
        anyhow::bail!("missing kugou h5 data");
    }
    let data = serde_json::from_str::<Value>(&json_text).context("parse kugou h5 payload")?;
    Ok(normalize_kugou_h5_playlist(&data, info))
}

fn kugou_mobile_songlist_url(info: &KugouShareInfo) -> String {
    let gcid = info.gcid.trim_start_matches("gcid_");
    if gcid.is_empty() {
        return String::new();
    }
    let mut url = Url::parse(&format!("{KUGOU_MOBILE_ORIGIN}/songlist/gcid_{gcid}/")).unwrap();
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("iszlist", "1");
        pairs.append_pair("src_cid", gcid);
        if !info.uid.is_empty() {
            pairs.append_pair("uid", &info.uid);
        }
        if !info.cover.is_empty() {
            pairs.append_pair("cover", &info.cover);
        }
        pairs.append_pair("chl", "weibo");
    }
    url.to_string()
}

async fn kugou_decode_gcid(gcid: &str) -> anyhow::Result<String> {
    let id = if gcid.to_ascii_lowercase().starts_with("gcid_") {
        gcid.to_owned()
    } else {
        format!("gcid_{gcid}")
    };
    let params = "dfid=-&appid=1005&mid=0&clientver=20109&clienttime=640612895&uuid=-";
    let body = json!({
        "ret_info": 1,
        "data": [{ "id": id, "id_type": 2 }]
    })
    .to_string();
    let signature = kugou_signature_from_query(params, "android", Some(&body));
    let url =
        format!("https://t.kugou.com/v1/songlist/batch_decode?{params}&signature={signature}");
    let response = fetch_json(
        &url,
        Method::POST,
        kugou_batch_decode_headers(),
        Some(body),
        12_000,
    )
    .await?;
    let data = normalize_kugou_api_json(&response);
    let list = array_of(data.get("list"));
    let first = list.first().map(record).unwrap_or_default();
    Ok(first_non_blank(&[
        value_string(first.get("global_collection_id")),
        value_string(first.get("global_specialid")),
    ]))
}

async fn kugou_collection_info(global_collection_id: &str) -> anyhow::Result<Map<String, Value>> {
    let params = format!(
        "appid=1058&specialid=0&global_specialid={}&format=jsonp&srcappid=2919&clientver=20000&clienttime=1586163242519&mid=1586163242519&uuid=1586163242519&dfid=-",
        urlencoding::encode(global_collection_id)
    );
    let signature = kugou_signature_from_query(&params, "web", None);
    let url =
        format!("https://mobiles.kugou.com/api/v5/special/info_v2?{params}&signature={signature}");
    let response = fetch_json(
        &url,
        Method::GET,
        kugou_collection_info_headers(),
        None,
        12_000,
    )
    .await?;
    Ok(normalize_kugou_api_json(&response))
}

async fn kugou_collection_songs(
    global_collection_id: &str,
    total: usize,
) -> anyhow::Result<Vec<Value>> {
    let mut tracks = Vec::new();
    let mut page = 1;
    let mut remaining = total.min(KUGOU_SHARED_PLAYLIST_TRACK_LIMIT);

    while remaining > 0 {
        let limit = remaining.min(300);
        let params = format!(
            "appid=1058&global_specialid={}&specialid=0&plat=0&version=8000&page={page}&pagesize={limit}&srcappid=2919&clientver=20000&clienttime=1586163263991&mid=1586163263991&uuid=1586163263991&dfid=-",
            urlencoding::encode(global_collection_id)
        );
        let signature = kugou_signature_from_query(&params, "web", None);
        let url = format!(
            "https://mobiles.kugou.com/api/v5/special/song_v2?{params}&signature={signature}"
        );
        let response = fetch_json(
            &url,
            Method::GET,
            kugou_collection_song_headers(),
            None,
            12_000,
        )
        .await?;
        let body = normalize_kugou_api_json(&response);
        let songs = {
            let info = array_of(body.get("info"));
            if !info.is_empty() {
                info
            } else {
                let songs = array_of(body.get("songs"));
                if !songs.is_empty() {
                    songs
                } else {
                    array_of(body.get("list"))
                }
            }
        };
        if songs.is_empty() {
            break;
        }
        remaining = remaining.saturating_sub(songs.len());
        let len = songs.len();
        tracks.extend(songs);
        if len < limit {
            break;
        }
        page += 1;
    }

    tracks.truncate(KUGOU_SHARED_PLAYLIST_TRACK_LIMIT);
    Ok(tracks)
}

fn normalize_kugou_collection_playlist(
    global_collection_id: &str,
    info: &Map<String, Value>,
    raw_songs: &[Value],
    fallback: &KugouShareInfo,
) -> KugouPlaylistPayload {
    let tracks = raw_songs
        .iter()
        .take(KUGOU_SHARED_PLAYLIST_TRACK_LIMIT)
        .filter_map(normalize_kugou_shared_song)
        .collect::<Vec<_>>();
    let track_count = number_u64(
        info.get("songcount")
            .or_else(|| info.get("count"))
            .or_else(|| info.get("total")),
    )
    .unwrap_or(tracks.len() as u64) as usize;

    KugouPlaylistPayload {
        id: format!(
            "kugou:{}",
            if global_collection_id.is_empty() {
                format!("gcid_{}", fallback.gcid)
            } else {
                global_collection_id.to_owned()
            }
        ),
        name: clean_external_text(&first_non_blank(&[
            value_string(info.get("specialname")),
            value_string(info.get("name")),
            (!fallback.title.is_empty()).then_some(fallback.title.clone()),
            Some("Kugou playlist".to_owned()),
        ])),
        cover: kugou_cover_url(&first_non_blank(&[
            value_string(info.get("imgurl")),
            value_string(info.get("pic")),
            (!fallback.cover.is_empty()).then_some(fallback.cover.clone()),
        ])),
        track_count,
        tracks,
    }
}

fn normalize_kugou_h5_playlist(data: &Value, fallback: &KugouShareInfo) -> KugouPlaylistPayload {
    let root = record(data);
    let info = root
        .get("info")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let list_info = info
        .get("listinfo")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let raw_songs = array_of(info.get("songs"));
    let tracks = raw_songs
        .iter()
        .filter_map(normalize_kugou_shared_song)
        .collect::<Vec<_>>();
    let track_count = number_u64(list_info.get("count").or_else(|| info.get("count")))
        .unwrap_or(tracks.len() as u64) as usize;

    KugouPlaylistPayload {
        id: format!("kugou:gcid_{}", fallback.gcid),
        name: clean_external_text(&first_non_blank(&[
            value_string(list_info.get("name")),
            (!fallback.title.is_empty()).then_some(fallback.title.clone()),
            Some("Kugou playlist".to_owned()),
        ])),
        cover: kugou_cover_url(&first_non_blank(&[
            value_string(list_info.get("pic")),
            (!fallback.cover.is_empty()).then_some(fallback.cover.clone()),
        ])),
        track_count,
        tracks,
    }
}

fn normalize_kugou_shared_song(raw: &Value) -> Option<ExternalTrack> {
    let song = record(raw);
    let name_text = clean_external_text(&first_non_blank(&[
        value_string(song.get("name")),
        value_string(song.get("songname")),
        value_string(song.get("fileName")),
        value_string(song.get("filename")),
        value_string(song.get("SongName")),
    ]));
    let mut artist = String::new();
    let mut title = name_text.clone();
    if let Some(split_index) = name_text.find(" - ") {
        if split_index > 0 {
            artist = name_text[..split_index].trim().to_owned();
            title = name_text[(split_index + 3)..].trim().to_owned();
        }
    }
    if artist.is_empty() {
        artist = artist_name_from_unknown(song.get("singerinfo")).unwrap_or_default();
    }
    if artist.is_empty() {
        artist = first_non_blank(&[
            value_string(song.get("singerName")),
            value_string(song.get("author_name")),
            value_string(song.get("singername")),
            value_string(song.get("SingerName")),
        ]);
    }
    if title.is_empty() {
        return None;
    }
    let trans_param = song
        .get("trans_param")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let album_info = song
        .get("albuminfo")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mid = first_non_blank(&[
        value_string(song.get("mixsongid")),
        value_string(song.get("add_mixsongid")),
        value_string(song.get("EMixSongID")),
        value_string(song.get("MixSongID")),
        value_string(song.get("album_audio_id")),
        value_string(song.get("audio_id")),
    ]);
    let hash = first_non_blank(&[
        value_string(song.get("hash")),
        value_string(song.get("FileHash")),
    ]);
    let duration = number_u64(song.get("timelen").or_else(|| song.get("timeLength")))
        .or_else(|| number_u64(song.get("duration")).map(|value| value * 1000));

    Some(ExternalTrack {
        id: Some(first_non_blank(&[
            (!mid.is_empty()).then_some(mid),
            (!hash.is_empty()).then_some(hash),
            Some(simple_hash_hex(&format!("{title}|{artist}"))),
        ])),
        name: title,
        artist: Some(clean_external_text(&artist)),
        artists: Vec::new(),
        album: Some(clean_external_text(&first_non_blank(&[
            value_string(song.get("remark")),
            value_string(song.get("albumName")),
            value_string(song.get("AlbumName")),
            value_string(album_info.get("name")),
        ]))),
        cover: Some(kugou_cover_url(&first_non_blank(&[
            value_string(song.get("cover")),
            value_string(song.get("imgUrl")),
            value_string(song.get("Image")),
            value_string(trans_param.get("union_cover")),
        ]))),
        duration,
    })
}

fn kugou_cover_url(raw: &str) -> String {
    normalize_image_url(&raw.replace("{size}", "480"))
}

async fn fetch_json(
    url: &str,
    method: Method,
    headers: HeaderMap,
    body: Option<String>,
    timeout_ms: u64,
) -> anyhow::Result<Value> {
    let client = Client::new();
    let mut request = client
        .request(method, url)
        .headers(headers)
        .timeout(std::time::Duration::from_millis(timeout_ms));
    if let Some(body) = body {
        request = request.body(body);
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("fetch json {url}"))?
        .error_for_status()
        .with_context(|| format!("request {url} failed"))?;
    let text = response
        .text()
        .await
        .with_context(|| format!("read {url} body"))?;
    let normalized = Regex::new(r"^callback\d*\(")
        .unwrap()
        .replace(&text.trim(), "")
        .to_string();
    let normalized = normalized
        .strip_suffix(')')
        .unwrap_or(&normalized)
        .to_owned();
    serde_json::from_str(&normalized).with_context(|| format!("parse json from {url}"))
}

fn normalize_kugou_api_json(data: &Value) -> Map<String, Value> {
    let object = record(data);
    object
        .get("data")
        .and_then(Value::as_object)
        .cloned()
        .or_else(|| object.get("info").and_then(Value::as_object).cloned())
        .unwrap_or(object)
}

fn kugou_signature_from_query(query: &str, platform: &str, body: Option<&str>) -> String {
    let secret = if platform == "android" {
        KUGOU_ANDROID_SIGN_SECRET
    } else {
        KUGOU_SIGN_SECRET
    };
    let params = query
        .split('&')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let mut sorted = params;
    sorted.sort_unstable();
    let payload = format!(
        "{}{}{}{}",
        secret,
        sorted.join(""),
        body.unwrap_or_default(),
        secret
    );
    md5_hex(&payload)
}

fn kugou_headers(extra: &[(&str, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(REFERER, HeaderValue::from_static("https://m.kugou.com/"));
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126 Safari/537.36",
        ),
    );
    for (key, value) in extra {
        if let Ok(name) = reqwest::header::HeaderName::from_bytes(key.as_bytes()) {
            if let Ok(value) = HeaderValue::from_str(value) {
                headers.insert(name, value);
            }
        }
    }
    headers
}

fn kugou_mobile_headers() -> HeaderMap {
    kugou_headers(&[
        ("Origin", KUGOU_MOBILE_ORIGIN),
        (
            "User-Agent",
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 Mobile/15E148 Safari/604.1",
        ),
    ])
}

fn kugou_batch_decode_headers() -> HeaderMap {
    let mut headers = kugou_headers(&[
        ("Referer", KUGOU_MOBILE_ORIGIN),
        ("Origin", KUGOU_MOBILE_ORIGIN),
        (
            "User-Agent",
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 Mobile/15E148 Safari/604.1",
        ),
    ]);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers
}

fn kugou_collection_info_headers() -> HeaderMap {
    kugou_headers(&[
        ("mid", "1586163242519"),
        ("Referer", "https://m3ws.kugou.com/share/index.php"),
        ("Origin", KUGOU_MOBILE_ALT_ORIGIN),
        ("dfid", "-"),
        ("clienttime", "1586163242519"),
        (
            "User-Agent",
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 Mobile/15E148 Safari/604.1",
        ),
    ])
}

fn kugou_collection_song_headers() -> HeaderMap {
    kugou_headers(&[
        ("mid", "1586163263991"),
        ("Referer", "https://m3ws.kugou.com/share/index.php"),
        ("Origin", KUGOU_MOBILE_ALT_ORIGIN),
        ("dfid", "-"),
        ("clienttime", "1586163263991"),
        (
            "User-Agent",
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 Mobile/15E148 Safari/604.1",
        ),
    ])
}

fn extract_window_output_json(html: &str) -> String {
    let marker = "window.$output";
    let Some(marker_index) = html.find(marker) else {
        return String::new();
    };
    let Some(equals_index) = html[marker_index..].find('=') else {
        return String::new();
    };
    extract_balanced_json(html, marker_index + equals_index + 1)
}

fn extract_balanced_json(text: &str, start: usize) -> String {
    let bytes = text.as_bytes();
    let mut index = start;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    if index >= bytes.len() || bytes[index] != b'{' {
        return String::new();
    }
    let mut depth = 0usize;
    let mut in_string = false;
    let mut quote = b'"';
    let mut escaped = false;
    for i in index..bytes.len() {
        let ch = bytes[i];
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == b'\\' {
                escaped = true;
            } else if ch == quote {
                in_string = false;
            }
            continue;
        }
        if ch == b'"' || ch == b'\'' {
            in_string = true;
            quote = ch;
            continue;
        }
        if ch == b'{' {
            depth += 1;
        } else if ch == b'}' {
            depth -= 1;
            if depth == 0 {
                return text[index..=i].to_owned();
            }
        }
    }
    String::new()
}

fn external_url_from_input(value: &str) -> String {
    let raw = value.trim();
    Regex::new(r#"https?://[^\s"'<>]+"#)
        .unwrap()
        .find(raw)
        .map(|value| clean_candidate(value.as_str()))
        .unwrap_or_else(|| clean_candidate(raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        providers,
        types::{
            LyricPayload, PlayableState, PlaylistAddSongAck, PlaylistDetail, PlaylistSummary,
            ProviderLoginStatus, SongLikeAck, SongLikeCheckAck, SongUrlOptions, SongUrlResult,
            Track, TrackQualityAvailability,
        },
    };
    use async_trait::async_trait;

    fn track(provider: ProviderId) -> Track {
        Track {
            provider,
            id: "song-1".to_owned(),
            source_id: "song-1".to_owned(),
            media_mid: None,
            title: "Song".to_owned(),
            artists: vec!["Artist".to_owned()],
            album: String::new(),
            cover_url: String::new(),
            quality_hints: Vec::new(),
            playable_state: PlayableState::Unknown,
            duration_ms: None,
            artwork_url: None,
        }
    }

    struct MockAdapter {
        provider: ProviderId,
    }

    #[async_trait]
    impl ProviderAdapter for MockAdapter {
        fn id(&self) -> ProviderId {
            self.provider
        }

        async fn search(
            &self,
            _keyword: &str,
            _limit: u32,
        ) -> providers::ProviderResult<Vec<Track>> {
            Ok(Vec::new())
        }

        async fn song_url(
            &self,
            _track: &Track,
            _opts: Option<SongUrlOptions>,
        ) -> providers::ProviderResult<SongUrlResult> {
            Ok(SongUrlResult::default())
        }

        async fn track_qualities(
            &self,
            _track: &Track,
        ) -> providers::ProviderResult<TrackQualityAvailability> {
            Ok(TrackQualityAvailability::default())
        }

        async fn lyric(&self, _track: &Track) -> providers::ProviderResult<LyricPayload> {
            Ok(LyricPayload::default())
        }

        async fn playlist_list(&self) -> providers::ProviderResult<Vec<PlaylistSummary>> {
            Ok(Vec::new())
        }

        async fn playlist_detail(&self, id: &str) -> providers::ProviderResult<PlaylistDetail> {
            Ok(PlaylistDetail {
                provider: self.provider.clone(),
                id: id.to_owned(),
                name: "Imported".to_owned(),
                cover_url: String::new(),
                track_count: None,
                track_ids: Vec::new(),
                collected: Some(false),
                tracks: vec![track(self.provider)],
            })
        }

        async fn login_status(&self) -> providers::ProviderResult<ProviderLoginStatus> {
            Ok(ProviderLoginStatus::default())
        }

        async fn logout(&self) -> providers::ProviderResult<()> {
            Ok(())
        }

        async fn like_song(
            &self,
            _id: &str,
            _liked: bool,
        ) -> providers::ProviderResult<SongLikeAck> {
            Ok(SongLikeAck::default())
        }

        async fn check_song_likes(
            &self,
            _ids: &[String],
        ) -> providers::ProviderResult<SongLikeCheckAck> {
            Ok(SongLikeCheckAck::default())
        }

        async fn add_song_to_playlist(
            &self,
            _playlist_id: &str,
            _track_id: &str,
        ) -> providers::ProviderResult<PlaylistAddSongAck> {
            Ok(PlaylistAddSongAck::default())
        }
    }

    #[test]
    fn detects_i2_qq_playlist_share_urls() {
        let candidate = detect_shared_playlist(json!({
            "text": "https://i2.y.qq.com/n3/other/pages/details/playlist.html?id=7167576049&hosteuin="
        }))
        .unwrap();

        assert_eq!(
            candidate,
            SharedPlaylistCandidate {
                provider: "qq".to_owned(),
                id: "7167576049".to_owned(),
                source_url: "https://i2.y.qq.com/n3/other/pages/details/playlist.html?id=7167576049&hosteuin=".to_owned(),
            }
        );
    }

    #[test]
    fn detects_ryqq_playlist_urls() {
        let candidate = detect_shared_playlist(json!({
            "url": "https://y.qq.com/n/ryqq/playlist/7697196542"
        }))
        .unwrap();

        assert_eq!(candidate.provider, "qq");
        assert_eq!(candidate.id, "7697196542");
    }

    #[test]
    fn detects_netease_playlist_inside_share_text() {
        let candidate = detect_shared_playlist(json!({
            "text": "share https://music.163.com/#/playlist?id=12345"
        }))
        .unwrap();

        assert_eq!(candidate.provider, "netease");
        assert_eq!(candidate.id, "12345");
    }

    #[test]
    fn detects_apple_music_playlist_urls() {
        let candidate = detect_shared_playlist(json!({
            "text": "https://music.apple.com/cn/playlist/demo/pl.3950454ced8c45a3b0cc693c2a7db97b"
        }))
        .unwrap();

        assert_eq!(candidate.provider, "apple-music");
        assert_eq!(candidate.id, "pl.3950454ced8c45a3b0cc693c2a7db97b");
    }

    #[test]
    fn detects_qishui_short_links() {
        let candidate = detect_shared_playlist(json!({
            "text": "https://qishui.douyin.com/s/iCdLprn7/"
        }))
        .unwrap();

        assert_eq!(candidate.provider, "qishui");
        assert_eq!(candidate.id, "iCdLprn7");
    }

    #[test]
    fn detects_kugou_gcid_links() {
        let candidate = detect_shared_playlist(json!({
            "text": "https://m.kugou.com/songlist/gcid_3z106tadezl7z03a/?src_cid=3z106tadezl7z03a"
        }))
        .unwrap();

        assert_eq!(candidate.provider, "kugou");
        assert_eq!(candidate.id, "3z106tadezl7z03a");
    }

    #[test]
    fn normalizes_apple_music_track_from_schema_and_lookup() {
        let raw = json!({
            "name": "Track Name",
            "url": "https://music.apple.com/cn/song/demo/123456789",
            "duration": "PT3M20S",
            "byArtist": { "name": "Schema Artist" },
            "inAlbum": { "name": "Schema Album" }
        });
        let lookup = json!({
            "trackId": 123456789,
            "trackName": "Lookup Name",
            "artistName": "Lookup Artist",
            "collectionName": "Lookup Album",
            "artworkUrl100": "https://is1-ssl.mzstatic.com/image/thumb/demo/100x100bb.jpg",
            "trackTimeMillis": 200000
        });

        let track = normalize_apple_music_track(&raw, lookup.as_object(), 0).unwrap();
        assert_eq!(track.id.as_deref(), Some("123456789"));
        assert_eq!(track.name, "Lookup Name");
        assert_eq!(track.artist.as_deref(), Some("Lookup Artist"));
        assert_eq!(track.album.as_deref(), Some("Lookup Album"));
        assert_eq!(track.duration, Some(200000));
        assert_eq!(
            track.cover.as_deref(),
            Some("https://is1-ssl.mzstatic.com/image/thumb/demo/600x600bb.jpg")
        );
    }

    #[test]
    fn imports_external_track_as_netease_placeholder_track() {
        let track = import_only_track(
            "apple-music",
            &ExternalTrack {
                id: Some("123".to_owned()),
                name: "Song".to_owned(),
                artist: Some("Alice / Bob".to_owned()),
                album: Some("Album".to_owned()),
                cover: Some("http://img.example/cover.jpg".to_owned()),
                duration: Some(180000),
                ..Default::default()
            },
            0,
            "",
        );

        assert_eq!(track["provider"], "netease");
        assert_eq!(track["sourceId"], "import:apple-music:123");
        assert_eq!(track["artists"][0], "Alice");
        assert_eq!(track["coverUrl"], "https://img.example/cover.jpg");
        assert_eq!(track["durationMs"], 180000);
    }

    #[test]
    fn parses_kugou_share_input_from_mobile_url() {
        let info = parse_kugou_share_input(
            "https://m.kugou.com/songlist/gcid_3z106tadezl7z03a/?src_cid=3z106tadezl7z03a&uid=42&cover=https%3A%2F%2Fimg.example%2Fa.jpg",
        );
        assert_eq!(info.gcid, "3z106tadezl7z03a");
        assert_eq!(info.uid, "42");
        assert_eq!(info.cover, "https://img.example/a.jpg");
    }

    #[test]
    fn kugou_signature_matches_sorted_query_digest() {
        let signature = kugou_signature_from_query("b=2&a=1", "web", None);
        assert_eq!(
            signature,
            md5_hex(&format!("{KUGOU_SIGN_SECRET}a=1b=2{KUGOU_SIGN_SECRET}"))
        );
    }

    #[test]
    fn extracts_window_output_json_object() {
        let html = r#"<script>window.$output = {"info":{"listinfo":{"name":"demo"}}};</script>"#;
        let json = extract_window_output_json(html);
        assert_eq!(json, r#"{"info":{"listinfo":{"name":"demo"}}}"#);
    }

    #[tokio::test]
    async fn imports_adapter_backed_playlist_detail() {
        let result = import_shared_playlist(
            json!({
                "url": "https://y.qq.com/n/ryqq/playlist/7697196542"
            }),
            SharedPlaylistImporterDeps {
                provider_adapters: HashMap::from([(
                    ProviderId::Qq,
                    Arc::new(MockAdapter {
                        provider: ProviderId::Qq,
                    }) as Arc<dyn ProviderAdapter>,
                )]),
            },
        )
        .await
        .unwrap();

        assert_eq!(result["provider"], "qq");
        assert_eq!(result["playlist"]["id"], "7697196542");
        assert_eq!(result["loadedCount"], 1);
        assert_eq!(result["tracks"][0]["provider"], "qq");
    }

    #[tokio::test]
    async fn rejects_unsupported_links() {
        let err = import_shared_playlist(
            json!({
                "text": "https://example.com/playlist/1"
            }),
            SharedPlaylistImporterDeps {
                provider_adapters: HashMap::new(),
            },
        )
        .await
        .unwrap_err();

        let err = err.downcast::<SharedPlaylistImportError>().unwrap();
        assert_eq!(err.code, "UNSUPPORTED_LINK");
    }
}
