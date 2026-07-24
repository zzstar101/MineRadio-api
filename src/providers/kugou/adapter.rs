#![allow(dead_code)]

use std::sync::Arc;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_json::Value;

use crate::{
    parsers::{
        kugou::KugouParser,
        lrc::{LrcParser, UniversalLrcParser},
    },
    providers::{ProviderAdapter, ProviderResult, error::ProviderError},
    types::{
        LyricPayload, PlaylistDetail, PlaylistSummary, ProviderId, ProviderLoginStatus,
        SongUrlOptions, SongUrlResult, Track, TrackQualityAvailability, TrackQualityOption,
    },
};

use super::{client::KugouClient, map::map_kugou_song_to_track};

#[derive(Clone, Default)]
pub struct KugouAdapter {
    client: Arc<KugouClient>,
}

impl KugouAdapter {
    pub fn new(client: Arc<KugouClient>) -> Self {
        Self { client }
    }

    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new(Arc::new(KugouClient::new())))
    }
}

#[async_trait]
impl ProviderAdapter for KugouAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Kugou
    }

    async fn search_track(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<Vec<Track>> {
        let page = offset / limit.max(1) + 1;
        let body = self.client.search(keyword, page, limit).await?;
        Ok(search_items(&body)
            .iter()
            .map(map_kugou_song_to_track)
            .filter(|track| !track.source_id.is_empty())
            .collect())
    }

    async fn song_url(
        &self,
        track: &Track,
        opts: Option<SongUrlOptions>,
    ) -> ProviderResult<SongUrlResult> {
        let requested_quality = opts.and_then(|value| value.quality);
        let quality = kugou_quality(requested_quality.as_deref());
        let album_audio_id = track
            .media_mid
            .as_deref()
            .and_then(|value| value.parse().ok())
            .unwrap_or_default();
        let body = self
            .client
            .song_url(&track.source_id, 0, album_audio_id, quality)
            .await?;
        let url = first_url(&body);
        Ok(SongUrlResult {
            url: url.clone(),
            provider: Some(ProviderId::Kugou),
            playable: Some(url.is_some()),
            quality: Some(quality.to_owned()),
            requested_quality,
            reason: url
                .is_none()
                .then(|| "kugou did not return a playable URL".to_owned()),
            ..Default::default()
        })
    }

    async fn track_qualities(&self, track: &Track) -> ProviderResult<TrackQualityAvailability> {
        let qualities = [
            ("standard", "128"),
            ("higher", "320"),
            ("lossless", "flac"),
            ("hires", "high"),
        ]
        .into_iter()
        .map(|(id, request_quality)| TrackQualityOption {
            provider: ProviderId::Kugou,
            id: id.to_owned(),
            label: id.to_owned(),
            request_quality: request_quality.to_owned(),
            source: "declared".to_owned(),
            ..Default::default()
        })
        .collect();
        Ok(TrackQualityAvailability {
            provider: ProviderId::Kugou,
            track_id: track.id.clone(),
            default_quality: Some("standard".to_owned()),
            qualities,
        })
    }

    async fn lyric(&self, track: &Track) -> ProviderResult<LyricPayload> {
        let search_resp = self.client.lyric_search(&track.source_id).await?;
        let Some(candidate) = search_resp.first_candidate() else {
            return Ok(LyricPayload {
                provider: ProviderId::Kugou,
                track_id: track.id.clone(),
                ..Default::default()
            });
        };
        let id: u64 = candidate.id.parse().unwrap_or_default();
        let access_key = candidate.access_key.as_str();
        if id == 0 || access_key.is_empty() {
            return Ok(LyricPayload {
                provider: ProviderId::Kugou,
                track_id: track.id.clone(),
                ..Default::default()
            });
        }

        // Prefer KRC (word-level lyrics), fall back to LRC
        if let Ok(body) = self.client.lyric_krc(id, access_key).await {
            if let Ok(lines) = KugouParser.decrypt_and_parse(body.content.clone()) {
                let is_word_by_word = lines.iter().any(|line| {
                    line.words
                        .as_ref()
                        .map(|words| !words.is_empty())
                        .unwrap_or(false)
                });
                return Ok(LyricPayload {
                    provider: ProviderId::Kugou,
                    track_id: track.id.clone(),
                    lines,
                    has_translation: false,
                    is_word_by_word,
                });
            }
        }

        // Fallback: plain LRC
        let body = self.client.lyric(id, access_key).await?;
        let text = decode_base64_text(&body.content).unwrap_or_default();
        Ok(LyricPayload {
            provider: ProviderId::Kugou,
            track_id: track.id.clone(),
            lines: UniversalLrcParser.parse(text).unwrap_or_default(),
            has_translation: false,
            is_word_by_word: false,
        })
    }

    async fn playlist_list(&self) -> ProviderResult<Vec<PlaylistSummary>> {
        Err(not_implemented("playlist_list"))
    }

    async fn playlist_detail(
        &self,
        _id: &str,
        _offset: u32,
        _limit: u32,
    ) -> ProviderResult<PlaylistDetail> {
        Err(not_implemented("playlist_detail"))
    }

    async fn login_status(&self) -> ProviderResult<ProviderLoginStatus> {
        Err(not_implemented("login_status"))
    }

    async fn logout(&self) -> ProviderResult<()> {
        Err(not_implemented("logout"))
    }
}

fn not_implemented(action: &str) -> ProviderError {
    ProviderError::not_implemented(ProviderId::Kugou, action)
}

fn search_items(body: &Value) -> &[Value] {
    body.get("data")
        .and_then(|value| value.get("lists"))
        .or_else(|| body.get("lists"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn first_url(body: &Value) -> Option<String> {
    body.get("url")
        .and_then(Value::as_array)
        .and_then(|urls| urls.iter().find_map(Value::as_str))
        .or_else(|| body.get("url").and_then(Value::as_str))
        .or_else(|| body.pointer("/data/url").and_then(Value::as_str))
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
}

fn decode_base64_text(value: &str) -> Option<String> {
    BASE64
        .decode(value)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

fn kugou_quality(value: Option<&str>) -> &'static str {
    match value
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "higher" | "320" => "320",
        "lossless" | "flac" => "flac",
        "hires" | "hi_res" | "high" => "high",
        _ => "128",
    }
}
