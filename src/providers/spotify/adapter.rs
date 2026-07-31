use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Method;
use serde_json::{Value, json};

use crate::{
    providers::{ProviderAdapter, ProviderResult},
    types::{
        AlbumDetail, AlbumSummary, LyricPayload, PlaylistAddSongAck, PlaylistDetail,
        PlaylistSummary, ProviderId, ProviderLoginStatus, SongLikeAck, SongLikeCheckAck,
        SongUrlOptions, SongUrlResult, Track, TrackQualityAvailability,
    },
};

use super::{
    client::SpotifyClient,
    map::{self, LIKED_PLAYLIST_ID},
};

#[derive(Clone)]
pub struct SpotifyAdapter {
    client: Arc<SpotifyClient>,
}

impl SpotifyAdapter {
    pub fn new(client: Arc<SpotifyClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ProviderAdapter for SpotifyAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Spotify
    }

    async fn search_track(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<Vec<Track>> {
        let body = self
            .client
            .get(
                "search",
                &[
                    ("q", keyword.to_owned()),
                    ("type", "track".to_owned()),
                    ("market", self.client.market().to_owned()),
                    ("offset", offset.to_string()),
                    ("limit", limit.clamp(1, 50).to_string()),
                ],
                false,
            )
            .await?;
        Ok(items(&body, "/tracks/items")
            .filter_map(|value| map::track(value, None))
            .collect())
    }

    async fn song_url(
        &self,
        track: &Track,
        opts: Option<SongUrlOptions>,
    ) -> ProviderResult<SongUrlResult> {
        let requested_quality = opts.and_then(|value| value.quality);
        let resolved = self
            .client
            .resolve_audio(&track.source_id, requested_quality.as_deref())
            .await?;
        let quality = resolved.format;
        Ok(SongUrlResult {
            url: format!(
                "/providers/spotify/audio-proxy?id={}&quality={}",
                urlencoding::encode(&track.source_id),
                urlencoding::encode(quality.id)
            ),
            proxied: true,
            provider: Some(ProviderId::Spotify),
            trial: Some(false),
            ..Default::default()
        })
    }

    async fn track_qualities(&self, track: &Track) -> ProviderResult<TrackQualityAvailability> {
        let qualities = self.client.available_qualities(&track.source_id).await?;
        Ok(TrackQualityAvailability {
            provider: ProviderId::Spotify,
            track_id: track.id.clone(),
            default_quality: qualities
                .first()
                .map(|quality| quality.request_quality.clone()),
            qualities,
        })
    }

    async fn lyric(&self, track: &Track) -> ProviderResult<LyricPayload> {
        Ok(LyricPayload {
            provider: ProviderId::Spotify,
            track_id: track.id.clone(),
            lines: Vec::new(),
            has_translation: false,
            is_word_by_word: false,
        })
    }

    async fn playlist_list(&self) -> ProviderResult<Vec<PlaylistSummary>> {
        let liked = self
            .client
            .get(
                "me/tracks",
                &[
                    ("limit", "1".to_owned()),
                    ("market", self.client.market().to_owned()),
                ],
                true,
            )
            .await?;
        let liked_total = liked
            .get("total")
            .and_then(Value::as_u64)
            .unwrap_or_default()
            .min(u32::MAX as u64) as u32;
        let cover = items(&liked, "/items")
            .next()
            .and_then(|value| value.get("track"))
            .and_then(|value| map::track(value, None))
            .map(|track| track.cover_url)
            .unwrap_or_default();
        let body = self
            .client
            .get(
                "me/playlists",
                &[("limit", "50".to_owned()), ("offset", "0".to_owned())],
                true,
            )
            .await?;
        let mut playlists = vec![map::liked_playlist(liked_total, cover)];
        playlists.extend(items(&body, "/items").filter_map(map::playlist));
        Ok(playlists)
    }

    async fn playlist_detail(
        &self,
        id: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<PlaylistDetail> {
        let limit = limit.clamp(1, 50);
        let (path, body, name, collected) =
            if id.is_empty() || id == "liked" || id == LIKED_PLAYLIST_ID {
                let body = self
                    .client
                    .get(
                        "me/tracks",
                        &[
                            ("limit", limit.to_string()),
                            ("offset", offset.to_string()),
                            ("market", self.client.market().to_owned()),
                        ],
                        true,
                    )
                    .await?;
                ("liked", body, "Spotify Liked Songs".to_owned(), Some(true))
            } else {
                let path = format!("playlists/{id}/items");
                let body = self
                    .client
                    .get(
                        &path,
                        &[
                            ("limit", limit.to_string()),
                            ("offset", offset.to_string()),
                            ("market", self.client.market().to_owned()),
                        ],
                        true,
                    )
                    .await?;
                ("playlist", body, String::new(), None)
            };
        let tracks: Vec<Track> = items(&body, "/items")
            .filter_map(|entry| {
                map::track(
                    entry
                        .get("track")
                        .or_else(|| entry.get("item"))
                        .unwrap_or(entry),
                    None,
                )
            })
            .collect();
        let total = body
            .get("total")
            .and_then(Value::as_u64)
            .unwrap_or(tracks.len() as u64)
            .min(u32::MAX as u64) as u32;
        Ok(PlaylistDetail {
            provider: ProviderId::Spotify,
            id: if path == "liked" {
                LIKED_PLAYLIST_ID.to_owned()
            } else {
                id.to_owned()
            },
            name,
            cover_url: tracks
                .first()
                .map(|track| track.cover_url.clone())
                .unwrap_or_default(),
            track_count: Some(total),
            track_ids: tracks.iter().map(|track| track.id.clone()).collect(),
            collected,
            tracks,
            has_more: body
                .get("next")
                .and_then(Value::as_str)
                .map(|value| !value.is_empty()),
        })
    }

    async fn login_status(&self) -> ProviderResult<ProviderLoginStatus> {
        let body = self.client.get("me", &[], true).await?;
        let premium = body
            .get("product")
            .and_then(Value::as_str)
            .is_some_and(|value| value.eq_ignore_ascii_case("premium"));
        Ok(ProviderLoginStatus {
            provider: ProviderId::Spotify,
            logged_in: true,
            nickname: Some(map::string(&body, "display_name")).filter(|value| !value.is_empty()),
            user_id: Some(map::string(&body, "id")).filter(|value| !value.is_empty()),
            avatar_url: Some(map::image(body.get("images"))).filter(|value| !value.is_empty()),
            is_vip: Some(premium),
            vip_label: Some(if premium {
                "Premium".to_owned()
            } else {
                "Free".to_owned()
            }),
            ..Default::default()
        })
    }

    async fn logout(&self) -> ProviderResult<()> {
        self.client.logout().await;
        Ok(())
    }

    async fn like_song(&self, id: &str, liked: bool) -> ProviderResult<SongLikeAck> {
        self.client
            .request(
                if liked { Method::PUT } else { Method::DELETE },
                "me/library",
                &[("uris", format!("spotify:track:{id}"))],
                None,
                true,
            )
            .await?;
        Ok(SongLikeAck {
            provider: ProviderId::Spotify,
            id: id.to_owned(),
            liked,
            code: Some(0),
        })
    }

    async fn check_song_likes(&self, ids: &[String]) -> ProviderResult<SongLikeCheckAck> {
        let ids = ids
            .iter()
            .filter(|id| !id.trim().is_empty())
            .take(40)
            .cloned()
            .collect::<Vec<_>>();
        if ids.is_empty() {
            return Ok(SongLikeCheckAck {
                provider: ProviderId::Spotify,
                ids,
                liked: Default::default(),
            });
        }
        let uris = ids
            .iter()
            .map(|id| format!("spotify:track:{id}"))
            .collect::<Vec<_>>()
            .join(",");
        let body = self
            .client
            .get("me/library/contains", &[("uris", uris)], true)
            .await?;
        let values = body.as_array().cloned().unwrap_or_default();
        Ok(SongLikeCheckAck {
            provider: ProviderId::Spotify,
            liked: ids
                .iter()
                .enumerate()
                .map(|(index, id)| {
                    (
                        id.clone(),
                        values.get(index).and_then(Value::as_bool).unwrap_or(false),
                    )
                })
                .collect(),
            ids,
        })
    }

    async fn update_song_in_playlist(
        &self,
        playlist_id: &str,
        track_id: &str,
        adding: bool,
    ) -> ProviderResult<PlaylistAddSongAck> {
        let path = format!("playlists/{playlist_id}/items");
        let method = if adding { Method::POST } else { Method::DELETE };
        self.client
            .request(
                method,
                &path,
                &[],
                Some(json!({ "uris": [format!("spotify:track:{track_id}")] })),
                true,
            )
            .await?;
        Ok(PlaylistAddSongAck {
            provider: ProviderId::Spotify,
            playlist_id: playlist_id.to_owned(),
            track_id: track_id.to_owned(),
            success: true,
            code: Some(0),
        })
    }

    async fn album_list(&self) -> ProviderResult<Vec<AlbumSummary>> {
        let body = self
            .client
            .get(
                "me/albums",
                &[
                    ("limit", "50".to_owned()),
                    ("offset", "0".to_owned()),
                    ("market", self.client.market().to_owned()),
                ],
                true,
            )
            .await?;
        Ok(items(&body, "/items")
            .filter_map(|item| map::album(item.get("album").unwrap_or(item)))
            .collect())
    }

    async fn album_detail(&self, id: &str, offset: u32, limit: u32) -> ProviderResult<AlbumDetail> {
        let body = self
            .client
            .get(
                &format!("albums/{id}"),
                &[("market", self.client.market().to_owned())],
                false,
            )
            .await?;
        let album = map::album(&body).unwrap_or(AlbumSummary {
            provider: ProviderId::Spotify,
            id: id.to_owned(),
            ..Default::default()
        });
        let tracks = items(&body, "/tracks/items")
            .skip(offset as usize)
            .take(limit.clamp(1, 50) as usize)
            .filter_map(|track| map::track(track, Some(&body)))
            .collect::<Vec<_>>();
        Ok(AlbumDetail {
            provider: ProviderId::Spotify,
            id: album.id,
            name: album.name,
            artists: album.artists,
            cover_url: album.cover_url,
            track_count: album.track_count,
            track_ids: tracks.iter().map(|track| track.id.clone()).collect(),
            collected: None,
            tracks,
            has_more: body
                .pointer("/tracks/next")
                .and_then(Value::as_str)
                .map(|value| !value.is_empty()),
        })
    }
}

fn items<'a>(body: &'a Value, pointer: &str) -> impl Iterator<Item = &'a Value> {
    body.pointer(pointer)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}
