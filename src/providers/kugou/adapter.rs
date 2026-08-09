use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_json::Value;

use crate::{
    auth_session,
    providers::lyric::{LrcParser, UniversalLrcParser},
    providers::{
        ProviderAdapter, ProviderResult,
        error::{ProviderError, ProviderErrorCode},
    },
    types::{
        AlbumDetail, AlbumSummary, LyricPayload, PlaylistAddSongAck, PlaylistDetail,
        PlaylistSummary, ProviderId, ProviderLoginStatus, SongLikeAck, SongLikeCheckAck,
        SongUrlOptions, SongUrlResult, Track, TrackQualityAvailability, TrackQualityOption,
    },
};

use super::{
    client::KugouClient,
    lyric::KugouParser,
    map::{KugouTrackMeta, map_kugou_song},
    model::{
        KugouAddSongRequest, KugouDeleteSongRequest, KugouDeleteSongResource, KugouSongResource,
    },
};

#[derive(Clone)]
pub struct KugouAdapter {
    client: Arc<KugouClient>,
    metadata: Arc<Mutex<HashMap<String, KugouTrackMeta>>>,
}

impl KugouAdapter {
    pub fn new(client: Arc<KugouClient>) -> Self {
        Self {
            client,
            metadata: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new(Arc::new(KugouClient::new())))
    }

    fn remember(&self, track: &Track, meta: KugouTrackMeta) {
        self.metadata
            .lock()
            .unwrap()
            .insert(track.source_id.to_ascii_lowercase(), meta);
    }

    fn metadata_for(&self, id: &str) -> KugouTrackMeta {
        self.metadata
            .lock()
            .unwrap()
            .get(&id.to_ascii_lowercase())
            .cloned()
            .unwrap_or_default()
    }

    async fn playlist_tracks(
        &self,
        playlist_id: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<(Vec<Track>, u32, bool)> {
        let list_id = parse_playlist_id(playlist_id)
            .ok_or_else(|| invalid_response("invalid kugou playlist id"))?;
        let page_size = limit.clamp(1, 50);
        let first = self
            .client
            .playlist_tracks_page(list_id, 1, page_size)
            .await?;
        let first_tracks = self.map_playlist_tracks(&first);
        let total = playlist_total(&first).max(first_tracks.len() as u32);
        if total == 0 || offset >= total {
            return Ok((Vec::new(), total, false));
        }
        let start = total.saturating_sub(offset.saturating_add(page_size));
        let end = total.saturating_sub(offset);
        let mut raw_tracks = Vec::new();
        let mut position = start;
        while position < end {
            let page = position / page_size + 1;
            let page_start = (page - 1) * page_size;
            let body = if page == 1 {
                first.clone()
            } else {
                self.client
                    .playlist_tracks_page(list_id, page, page_size)
                    .await?
            };
            let tracks = self.map_playlist_tracks(&body);
            if tracks.is_empty() {
                break;
            }
            let within_page = (position - page_start) as usize;
            let take = ((end - position) as usize).min(tracks.len().saturating_sub(within_page));
            if take == 0 {
                break;
            }
            raw_tracks.extend(tracks.into_iter().skip(within_page).take(take));
            position += take as u32;
        }
        raw_tracks.reverse();
        let has_more = offset.saturating_add(raw_tracks.len() as u32) < total;
        Ok((raw_tracks, total, has_more))
    }

    fn map_playlist_tracks(&self, body: &Value) -> Vec<Track> {
        playlist_track_items(body)
            .into_iter()
            .filter_map(|raw| {
                let (track, meta) = map_kugou_song(raw);
                (!track.source_id.is_empty()).then(|| {
                    self.remember(&track, meta);
                    track
                })
            })
            .collect()
    }

    async fn favorite_playlist_id(&self) -> ProviderResult<String> {
        let body = self.client.user_collection_list().await?;
        body.standardize_playlists()
            .unwrap_or_default()
            .into_iter()
            .find(|item| {
                item.name.to_ascii_lowercase().contains("favorite")
                    || item.name.to_ascii_lowercase().contains("liked")
            })
            .map(|item| item.id)
            .ok_or_else(|| invalid_response("kugou favorite playlist not found"))
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
            .filter_map(|raw| {
                let (track, meta) = map_kugou_song(raw);
                (!track.source_id.is_empty()).then(|| {
                    self.remember(&track, meta);
                    track
                })
            })
            .collect())
    }

    async fn song_url(
        &self,
        track: &Track,
        opts: Option<SongUrlOptions>,
    ) -> ProviderResult<SongUrlResult> {
        let requested_quality = opts.and_then(|value| value.quality);
        let requested = normalize_quality(requested_quality.as_deref());
        let meta = self.metadata_for(&track.source_id);
        for (hash, quality) in hash_candidates(&track.source_id, &meta, requested) {
            for response in [
                self.client
                    .song_url_h5(
                        &hash,
                        meta.album_id,
                        meta.album_audio_id,
                        quality_parameter(quality),
                    )
                    .await,
                self.client.song_url_mobile(&hash, meta.album_id).await,
                self.client
                    .song_url_web(&hash, meta.album_id, meta.album_audio_id)
                    .await,
                self.client
                    .song_url(
                        &hash,
                        meta.album_id,
                        meta.album_audio_id,
                        quality_parameter(quality),
                    )
                    .await,
            ] {
                if let Ok(body) = response {
                    if let Some(url) = play_url(&body) {
                        return Ok(SongUrlResult {
                            url: url,
                            provider: Some(ProviderId::Kugou),
                            ..Default::default()
                        });
                    }
                }
            }
        }
        Err(ProviderError {
            code: ProviderErrorCode::NoUrl,
            provider: ProviderId::Kugou,
            message: format!("kugou did not return a playable URL for {requested}"),
            retryable: false,
            action: None,
            raw_message: None,
        })
    }

    async fn track_qualities(&self, track: &Track) -> ProviderResult<TrackQualityAvailability> {
        let meta = self.metadata_for(&track.source_id);
        let qualities = [
            ("jymaster", !meta.res_hash.is_empty()),
            ("hires", !meta.res_hash.is_empty()),
            ("lossless", !meta.sq_hash.is_empty()),
            ("exhigh", !meta.hq_hash.is_empty()),
            ("standard", true),
        ]
        .into_iter()
        .filter(|(_, available)| *available)
        .map(|(id, _)| TrackQualityOption {
            provider: ProviderId::Kugou,
            id: id.to_owned(),
            label: id.to_owned(),
            request_quality: id.to_owned(),
            level: Some(id.to_owned()),
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
            return Ok(empty_lyric(track));
        };
        let id: u64 = candidate.id.parse().unwrap_or_default();
        if id == 0 || candidate.access_key.is_empty() {
            return Ok(empty_lyric(track));
        }
        if let Ok(body) = self.client.lyric_krc(id, &candidate.access_key).await {
            if let Ok(lines) = KugouParser.decrypt_and_parse(body.content) {
                let is_word_by_word = lines
                    .iter()
                    .any(|line| line.words.as_ref().is_some_and(|words| !words.is_empty()));
                return Ok(LyricPayload {
                    provider: ProviderId::Kugou,
                    track_id: track.id.clone(),
                    lines,
                    has_translation: false,
                    is_word_by_word,
                });
            }
        }
        let body = self.client.lyric(id, &candidate.access_key).await?;
        let text = BASE64
            .decode(body.content)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .unwrap_or_default();
        Ok(LyricPayload {
            provider: ProviderId::Kugou,
            track_id: track.id.clone(),
            lines: UniversalLrcParser.parse(text).unwrap_or_default(),
            has_translation: false,
            is_word_by_word: false,
        })
    }

    async fn playlist_list(&self) -> ProviderResult<Vec<PlaylistSummary>> {
        self.client
            .user_collection_list()
            .await?
            .standardize_playlists()
            .ok_or_else(|| no_result("playlist_list"))
    }

    async fn playlist_detail(
        &self,
        id: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<PlaylistDetail> {
        let (tracks, total, has_more) = self.playlist_tracks(id, offset, limit).await?;
        Ok(PlaylistDetail {
            provider: ProviderId::Kugou,
            id: id.to_owned(),
            name: String::new(),
            cover_url: tracks
                .first()
                .map(|track| track.cover_url.clone())
                .unwrap_or_default(),
            track_count: Some(total),
            track_ids: tracks.iter().map(|track| track.id.clone()).collect(),
            collected: None,
            tracks,
            has_more: Some(has_more),
        })
    }

    async fn album_list(&self) -> ProviderResult<Vec<AlbumSummary>> {
        self.client
            .user_collection_list()
            .await?
            .standardize_albums()
            .ok_or_else(|| no_result("album_list"))
    }

    async fn album_detail(&self, id: &str, offset: u32, limit: u32) -> ProviderResult<AlbumDetail> {
        let detail_body = self.client.album_detail(id).await?;
        let page_size = limit.clamp(1, 50);
        let page = offset / page_size + 1;
        let page_offset = offset % page_size;
        let request_size = (page_size + page_offset).min(50);
        let songs_body = self.client.album_songs(id, page, request_size).await?;
        let raw_tracks = album_song_items(&songs_body);
        let tracks = raw_tracks
            .into_iter()
            .filter_map(|raw| {
                let (track, meta) = map_kugou_song(raw);
                (!track.source_id.is_empty()).then(|| {
                    self.remember(&track, meta);
                    track
                })
            })
            .skip(page_offset as usize)
            .take(page_size as usize)
            .collect::<Vec<_>>();
        let album = album_info(&detail_body, id)
            .or_else(|| album_song_items(&songs_body).into_iter().next());
        let first_track = tracks.first();
        let track_count = album_total(&songs_body)
            .or_else(|| (!tracks.is_empty()).then_some(tracks.len() as u32 + offset));
        let album_id = album
            .map(|value| string(value, &["album_id", "AlbumID"]))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| id.to_owned());
        let name = album
            .map(|value| string(value, &["album_name", "AlbumName", "name"]))
            .filter(|value| !value.is_empty())
            .or_else(|| first_track.map(|track| track.album.clone()))
            .unwrap_or_default();
        let artists = album
            .map(album_artists)
            .filter(|artists| !artists.is_empty())
            .or_else(|| first_track.map(|track| track.artists.clone()))
            .unwrap_or_default();
        let cover_url = album
            .map(album_cover_url)
            .filter(|value| !value.is_empty())
            .or_else(|| first_track.map(|track| track.cover_url.clone()))
            .unwrap_or_default();
        let has_more = track_count.map(|total| offset + (tracks.len() as u32) < total);
        Ok(AlbumDetail {
            provider: ProviderId::Kugou,
            id: album_id,
            name,
            artists,
            cover_url,
            track_count,
            track_ids: tracks.iter().map(|track| track.id.clone()).collect(),
            collected: None,
            tracks,
            has_more,
        })
    }

    async fn login_status(&self) -> ProviderResult<ProviderLoginStatus> {
        let (_, auth) = self.client.current_auth().await;
        Ok(ProviderLoginStatus {
            provider: ProviderId::Kugou,
            logged_in: auth.logged_in,
            nickname: (!auth.nickname.is_empty()).then_some(auth.nickname),
            user_id: (!auth.user_id.is_empty()).then_some(auth.user_id),
            avatar_url: (!auth.avatar_url.is_empty()).then_some(auth.avatar_url),
            ..Default::default()
        })
    }

    async fn logout(&self) -> ProviderResult<()> {
        auth_session::clear_runtime_provider_cookie(&ProviderId::Kugou).await;
        Ok(())
    }

    async fn like_song(&self, id: &str, liked: bool) -> ProviderResult<SongLikeAck> {
        let playlist_id = self.favorite_playlist_id().await?;
        self.update_song_in_playlist(&playlist_id, id, liked)
            .await?;
        Ok(SongLikeAck {
            provider: ProviderId::Kugou,
            id: id.to_owned(),
            liked,
            code: Some(0),
        })
    }

    async fn check_song_likes(&self, ids: &[String]) -> ProviderResult<SongLikeCheckAck> {
        let playlist_id = self.favorite_playlist_id().await?;
        let wanted = ids
            .iter()
            .map(|id| id.to_ascii_lowercase())
            .collect::<std::collections::HashSet<_>>();
        let mut liked = HashMap::new();
        let mut offset = 0;
        while offset < 300 && liked.len() < wanted.len() {
            let (tracks, total, has_more) = self.playlist_tracks(&playlist_id, offset, 50).await?;
            for track in tracks {
                if wanted.contains(&track.source_id.to_ascii_lowercase()) {
                    liked.insert(track.id, true);
                }
            }
            if !has_more || offset.saturating_add(50) >= total {
                break;
            }
            offset += 50;
        }
        for id in ids {
            liked.entry(id.clone()).or_insert(false);
        }
        Ok(SongLikeCheckAck {
            provider: ProviderId::Kugou,
            ids: ids.to_vec(),
            liked,
        })
    }

    async fn update_song_in_playlist(
        &self,
        playlist_id: &str,
        track_id: &str,
        adding: bool,
    ) -> ProviderResult<PlaylistAddSongAck> {
        let list_id = parse_playlist_id(playlist_id)
            .ok_or_else(|| invalid_response("invalid kugou playlist id"))?;
        let (_, auth) = self.client.current_auth().await;
        if !auth.playback_ready() {
            return Err(login_required());
        }
        let meta = self.metadata_for(track_id);
        if adding {
            let payload = KugouAddSongRequest {
                userid: auth.user_id.parse().unwrap_or_default(),
                token: &auth.token,
                listid: list_id,
                list_ver: 0,
                r#type: 0,
                slow_upload: 1,
                scene: "false;null",
                data: vec![KugouSongResource {
                    number: 1,
                    name: &meta.title,
                    hash: track_id,
                    size: 0,
                    sort: 0,
                    timelen: meta.duration_ms,
                    bitrate: 0,
                    album_id: meta.album_id,
                    mixsongid: meta.album_audio_id,
                }],
            };
            self.client.add_song_to_playlist(&payload).await?;
        } else {
            let file_id = self
                .find_file_id(playlist_id, track_id)
                .await?
                .ok_or_else(|| invalid_response("kugou song not in playlist"))?;
            let payload = KugouDeleteSongRequest {
                listid: list_id,
                userid: auth.user_id.parse().unwrap_or_default(),
                token: &auth.token,
                r#type: 0,
                list_ver: 0,
                data: vec![KugouDeleteSongResource { fileid: file_id }],
            };
            self.client.delete_song_from_playlist(&payload).await?;
        }
        Ok(PlaylistAddSongAck {
            provider: ProviderId::Kugou,
            playlist_id: playlist_id.to_owned(),
            track_id: track_id.to_owned(),
            success: true,
            code: Some(0),
        })
    }
}

impl KugouAdapter {
    async fn find_file_id(&self, playlist_id: &str, track_id: &str) -> ProviderResult<Option<u64>> {
        let list_id = parse_playlist_id(playlist_id)
            .ok_or_else(|| invalid_response("invalid kugou playlist id"))?;
        for page in 1..=6 {
            let body = self.client.playlist_tracks_page(list_id, page, 50).await?;
            for item in playlist_track_items(&body) {
                let hash = string(item, &["FileHash", "hash", "Hash"]);
                if hash.eq_ignore_ascii_case(track_id) {
                    return Ok(number(item, &["fileid", "file_id"]));
                }
            }
        }
        Ok(None)
    }
}

fn album_song_items(body: &Value) -> Vec<&Value> {
    if let Some(items) = body.as_array() {
        return items.iter().collect();
    }
    for path in [
        "/data/info",
        "/data/list",
        "/data/lists",
        "/data/songs",
        "/data/album_audio",
        "/info",
        "/list",
        "/lists",
        "/songs",
    ] {
        if let Some(items) = body.pointer(path).and_then(Value::as_array) {
            return items.iter().collect();
        }
    }
    Vec::new()
}

fn album_info<'a>(body: &'a Value, id: &str) -> Option<&'a Value> {
    let data = body.get("data").unwrap_or(body);
    if let Some(items) = data.as_array() {
        return items
            .iter()
            .find(|item| string(item, &["album_id", "AlbumID"]) == id)
            .or_else(|| items.first());
    }
    ["album", "albums", "info"]
        .into_iter()
        .find_map(|key| data.get(key))
        .and_then(|value| {
            value
                .as_array()
                .and_then(|items| {
                    items
                        .iter()
                        .find(|item| string(item, &["album_id", "AlbumID"]) == id)
                        .or_else(|| items.first())
                })
                .or_else(|| value.is_object().then_some(value))
        })
        .or_else(|| data.is_object().then_some(data))
}

fn album_total(body: &Value) -> Option<u32> {
    [
        "/data/total",
        "/data/total_count",
        "/data/count",
        "/total",
        "/total_count",
        "/count",
    ]
    .into_iter()
    .find_map(|path| body.pointer(path).and_then(value_u32))
}

fn album_artists(album: &Value) -> Vec<String> {
    if let Some(authors) = album.get("authors").and_then(Value::as_array) {
        let artists = authors
            .iter()
            .map(|author| string(author, &["author_name", "name", "AuthorName"]))
            .filter(|artist| !artist.is_empty())
            .collect::<Vec<_>>();
        if !artists.is_empty() {
            return artists;
        }
    }
    split_artist_names(&string(
        album,
        &["author_name", "authors", "artist", "singer"],
    ))
}

fn album_cover_url(album: &Value) -> String {
    string(
        album,
        &["sizable_cover", "album_img", "album_image", "cover", "img"],
    )
    .replace("{size}", "400")
    .replace("{width}", "400")
    .replace("{height}", "400")
}

fn split_artist_names(value: &str) -> Vec<String> {
    value
        .split([',', '/', '&', ';'])
        .map(str::trim)
        .filter(|artist| !artist.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn value_u32(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .or_else(|| value.as_str()?.parse().ok())
}

fn search_items(body: &Value) -> impl Iterator<Item = &Value> {
    body.pointer("/data/lists")
        .or_else(|| body.get("lists"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn playlist_track_items(body: &Value) -> Vec<&Value> {
    let data = body.get("data").unwrap_or(body);
    let chunk = data
        .get("info")
        .or_else(|| data.get("songs"))
        .or_else(|| data.get("lists"))
        .or_else(|| data.get("file"));
    chunk
        .and_then(Value::as_array)
        .map(|items| items.iter().collect())
        .or_else(|| {
            chunk
                .and_then(|value| value.get("file"))
                .and_then(Value::as_array)
                .map(|items| items.iter().collect())
        })
        .unwrap_or_default()
}

fn playlist_total(body: &Value) -> u32 {
    body.pointer("/data/count")
        .and_then(Value::as_u64)
        .unwrap_or_default()
        .min(u32::MAX as u64) as u32
}

fn string(item: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| item.get(*key))
        .and_then(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .or_else(|| value.as_u64().map(|value| value.to_string()))
        })
        .unwrap_or_default()
}

fn number(item: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| item.get(*key))
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        })
}

fn parse_playlist_id(id: &str) -> Option<u64> {
    if let Ok(id) = id.parse() {
        return Some(id);
    }
    let parts = id
        .strip_prefix("collection_")?
        .split('_')
        .collect::<Vec<_>>();
    parts.get(2)?.parse().ok()
}

fn normalize_quality(value: Option<&str>) -> &'static str {
    match value.unwrap_or_default().to_ascii_lowercase().as_str() {
        "jymaster" | "viper_tape" => "jymaster",
        "hires" | "hi_res" => "hires",
        "lossless" | "flac" => "lossless",
        "exhigh" | "higher" | "320" => "exhigh",
        _ => "standard",
    }
}

fn quality_parameter(quality: &str) -> &'static str {
    match quality {
        "jymaster" => "viper_tape",
        "hires" => "hires",
        "lossless" => "flac",
        "exhigh" => "320",
        _ => "128",
    }
}

fn hash_candidates<'a>(
    default_hash: &'a str,
    meta: &'a KugouTrackMeta,
    requested: &'a str,
) -> Vec<(&'a str, &'a str)> {
    let chain = [
        ("jymaster", meta.res_hash.as_str()),
        ("hires", meta.res_hash.as_str()),
        ("lossless", meta.sq_hash.as_str()),
        ("exhigh", meta.hq_hash.as_str()),
        ("standard", default_hash),
    ];
    let start = chain
        .iter()
        .position(|(level, _)| *level == requested)
        .unwrap_or(chain.len() - 1);
    chain[start..]
        .iter()
        .filter_map(|(level, hash)| (!hash.is_empty()).then_some((*hash, *level)))
        .collect()
}

fn play_url(body: &Value) -> Option<String> {
    [
        "/url/0",
        "/url",
        "/play_url",
        "/data/url/0",
        "/data/url",
        "/data/play_url",
        "/data/play_backup_url",
        "/data/backupUrl",
        "/backup_url",
        "/backupUrl",
    ]
    .into_iter()
    .find_map(|pointer| body.pointer(pointer).and_then(Value::as_str))
    .map(|url| url.replace("\\/", "/"))
    .filter(|url| !url.is_empty())
}

fn empty_lyric(track: &Track) -> LyricPayload {
    LyricPayload {
        provider: ProviderId::Kugou,
        track_id: track.id.clone(),
        ..Default::default()
    }
}

fn no_result(action: &str) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::NoResult,
        provider: ProviderId::Qq,
        message: format!("{} no result", action),
        retryable: false,
        action: Some(action.to_string()),
        raw_message: None,
    }
}

fn invalid_response(message: &str) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::InvalidResponse,
        provider: ProviderId::Kugou,
        message: message.to_owned(),
        retryable: false,
        action: None,
        raw_message: None,
    }
}
fn login_required() -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::LoginRequired,
        provider: ProviderId::Kugou,
        message: "kugou login with userid and token required".to_owned(),
        retryable: false,
        action: Some("login".to_owned()),
        raw_message: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_playlist_id, playlist_track_items, quality_parameter};
    use serde_json::json;

    #[test]
    fn parses_global_collection_playlist_id() {
        assert_eq!(parse_playlist_id("collection_1_2_345_6"), Some(345));
        assert_eq!(parse_playlist_id("345"), Some(345));
    }

    #[test]
    fn maps_quality_levels_to_kugou_request_values() {
        assert_eq!(quality_parameter("jymaster"), "viper_tape");
        assert_eq!(quality_parameter("lossless"), "flac");
        assert_eq!(quality_parameter("exhigh"), "320");
    }

    #[test]
    fn reads_tracks_from_the_nested_file_response_shape() {
        let body = json!({ "data": { "info": { "file": [{ "hash": "a" }] } } });
        assert_eq!(playlist_track_items(&body).len(), 1);
    }
}
