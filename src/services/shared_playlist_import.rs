use std::{collections::HashMap, sync::Arc};

use anyhow::Context;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use crate::{providers::ProviderAdapter, types::ProviderId};

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

    if !matches!(candidate.provider.as_str(), "qq" | "netease" | "soda") {
        return Err(SharedPlaylistImportError {
            code: "NOT_IMPLEMENTED".to_owned(),
            message: format!("{} shared playlist import is not migrated yet", candidate.provider),
        }
        .into());
    }

    let adapter = deps
        .provider_adapters
        .get(&candidate.provider)
        .cloned()
        .ok_or_else(|| SharedPlaylistImportError {
            code: "UNSUPPORTED_PROVIDER".to_owned(),
            message: "unsupported shared playlist provider".to_owned(),
        })?;

    let detail = adapter.playlist_detail(&candidate.id).await?;
    let tracks = detail.tracks;
    let loaded_count = tracks.len();
    let track_ids = tracks.iter().map(|track| track.id.clone()).collect::<Vec<_>>();
    let track_count = loaded_count;

    Ok(json!({
        "provider": candidate.provider,
        "playlist": {
            "provider": candidate.provider,
            "id": if detail.id.is_empty() { candidate.id } else { detail.id },
            "name": detail.name,
            "trackCount": track_count,
            "trackIds": track_ids,
            "subscribed": false,
            "sourceUrl": candidate.source_url
        },
        "tracks": tracks,
        "trackCount": track_count,
        "loadedCount": loaded_count,
        "partial": false,
        "partialReason": ""
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
    for value in [input.url.as_deref(), input.text.as_deref()].into_iter().flatten() {
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
    let query = hash.split_once('?').map(|(_, query)| query).unwrap_or_default();
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
        provider: "qq".to_owned(),
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
        provider: "netease".to_owned(),
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
    let id = first_non_blank(&[
        Regex::new(r"gcid_([a-z0-9]+)")
            .unwrap()
            .captures(url.path())
            .and_then(|cap| cap.get(1).map(|m| m.as_str().to_owned())),
        Some(simple_hash_hex(source_url)),
    ]);
    Some(SharedPlaylistCandidate {
        provider: "kugou".to_owned(),
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

fn simple_hash_hex(value: &str) -> String {
    let mut hash: u32 = 2_166_136_261;
    for byte in value.as_bytes() {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(16_777_619);
    }
    format!("{hash:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        providers,
        types::{
            LyricPayload, PlaylistAddSongAck, PlaylistDetail, PlaylistSummary,
            ProviderLoginStatus, SongLikeAck, SongLikeCheckAck, SongUrlOptions, SongUrlResult,
            Track, TrackQualityAvailability,
        },
    };
    use async_trait::async_trait;

    fn track(provider: &str) -> Track {
        Track {
            provider: provider.to_owned(),
            id: "song-1".to_owned(),
            source_id: "song-1".to_owned(),
            media_mid: None,
            title: "Song".to_owned(),
            artists: vec!["Artist".to_owned()],
            album: String::new(),
            cover_url: String::new(),
            quality_hints: Vec::new(),
            playable_state: "unknown".to_owned(),
            duration_ms: None,
            artwork_url: None,
        }
    }

    struct MockAdapter {
        provider: String,
    }

    #[async_trait]
    impl ProviderAdapter for MockAdapter {
        fn id(&self) -> ProviderId {
            self.provider.clone()
        }

        async fn search(&self, _keyword: &str, _limit: u32) -> providers::Result<Vec<Track>> {
            Ok(Vec::new())
        }

        async fn song_url(
            &self,
            _track: &Track,
            _opts: Option<SongUrlOptions>,
        ) -> providers::Result<SongUrlResult> {
            Ok(SongUrlResult::default())
        }

        async fn track_qualities(
            &self,
            _track: &Track,
        ) -> providers::Result<TrackQualityAvailability> {
            Ok(TrackQualityAvailability::default())
        }

        async fn lyric(&self, _track: &Track) -> providers::Result<LyricPayload> {
            Ok(LyricPayload::default())
        }

        async fn playlist_list(&self) -> providers::Result<Vec<PlaylistSummary>> {
            Ok(Vec::new())
        }

        async fn playlist_detail(&self, id: &str) -> providers::Result<PlaylistDetail> {
            Ok(PlaylistDetail {
                id: id.to_owned(),
                name: "Imported".to_owned(),
                tracks: vec![track(&self.provider)],
            })
        }

        async fn login_status(&self) -> providers::Result<ProviderLoginStatus> {
            Ok(ProviderLoginStatus::default())
        }

        async fn logout(&self) -> providers::Result<()> {
            Ok(())
        }

        async fn like_song(&self, _id: &str, _liked: bool) -> providers::Result<SongLikeAck> {
            Ok(SongLikeAck::default())
        }

        async fn check_song_likes(&self, _ids: &[String]) -> providers::Result<SongLikeCheckAck> {
            Ok(SongLikeCheckAck::default())
        }

        async fn add_song_to_playlist(
            &self,
            _playlist_id: &str,
            _track_id: &str,
        ) -> providers::Result<PlaylistAddSongAck> {
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

    #[tokio::test]
    async fn imports_adapter_backed_playlist_detail() {
        let result = import_shared_playlist(
            json!({
                "url": "https://y.qq.com/n/ryqq/playlist/7697196542"
            }),
            SharedPlaylistImporterDeps {
                provider_adapters: HashMap::from([
                    (
                        "qq".to_owned(),
                        Arc::new(MockAdapter {
                            provider: "qq".to_owned(),
                        }) as Arc<dyn ProviderAdapter>,
                    ),
                ]),
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
