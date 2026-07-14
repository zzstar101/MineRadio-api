use serde::Deserialize;

use crate::types::{AlbumDetail, AlbumSummary, Track};

#[derive(Deserialize)]
pub(super) struct SodaSearchResp {
    result_groups: Vec<ResultGroup>,
}

impl SodaSearchResp {
    pub fn standardize(self) -> Option<Vec<Track>> {
        let tracks: Vec<Track> = self
            .result_groups
            .into_iter()
            .find(|group| group.id == "tracks")
            .map(|group| {
                group
                    .data
                    .into_iter()
                    .filter_map(|datum| datum.entity.track.standardize())
                    .collect()
            })
            .unwrap_or_default();
        if tracks.is_empty() {
            None
        } else {
            Some(tracks)
        }
    }
}

#[derive(Deserialize)]
struct ResultGroup {
    id: String,
    data: Vec<Datum>,
}

#[derive(Deserialize)]
struct Datum {
    entity: Entity,
}

#[derive(Deserialize)]
struct Entity {
    track: SodaTrack,
}

#[derive(Deserialize)]
pub(super) struct SodaAlbumDetailResp {
    album_info: AlbumInfo,
    tracks: Vec<SodaTrack>,
}

impl SodaAlbumDetailResp {
    pub fn standardize(self) -> Option<AlbumDetail> {
        let singer = self
            .album_info
            .artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect();
        let album = self.album_info.standardize()?;
        let (track_ids, tracks): (Vec<String>, Vec<Track>) = self
            .tracks
            .into_iter()
            .filter_map(SodaTrack::standardize)
            .map(|track| (track.id.clone(), track))
            .unzip();

        if tracks.is_empty() {
            None
        } else {
            Some(AlbumDetail {
                provider: album.provider,
                id: album.id,
                name: album.name,
                singer,
                cover_url: album.cover_url,
                track_count: album.track_count,
                track_ids,
                subscribed: album.subscribed,
                tracks,
            })
        }
    }
}

#[derive(Deserialize)]
pub(super) struct SodaAlbumListResp {
    mixed_collections: Vec<MixedCollection>,
}

impl SodaAlbumListResp {
    pub fn standardize(self) -> Option<Vec<AlbumSummary>> {
        let albums = self
            .mixed_collections
            .into_iter()
            .filter_map(|collection| collection.album.standardize())
            .collect::<Vec<_>>();
        if albums.is_empty() {
            None
        } else {
            Some(albums)
        }
    }
}

#[derive(Deserialize)]
struct MixedCollection {
    album: AlbumInfo,
}

#[derive(Deserialize)]
struct AlbumInfo {
    id: Option<String>,
    name: String,
    artists: Vec<Artist>,
    count_tracks: u32,
    url_cover: SodaUrl,
    state: AlbumState,
}

impl AlbumInfo {
    fn standardize(self) -> Option<AlbumSummary> {
        self.id.map(|id| AlbumSummary {
            provider: "soda".to_owned(),
            id,
            name: self.name,
            cover_url: self.url_cover.standardize(),
            track_count: Some(self.count_tracks),
            track_ids: Vec::new(),
            subscribed: self.state.is_collected,
        })
    }
}

#[derive(Deserialize)]
pub(super) struct SodaTrack {
    id: Option<String>,
    album: TrackAlbum,
    artists: Vec<Artist>,
    duration: u64,
    name: String,
    state: TrackState,
    label_info: LabelInfo,
    bit_rates: Vec<BitRate>,
    vocal: i64,
    playable_range: PlayableRange,
}

impl SodaTrack {
    fn standardize(self) -> Option<Track> {
        self.id.map(|id| Track {
            source_id: id.clone(),
            id,
            provider: "soda".to_owned(),
            media_mid: None,
            title: self.name,
            artists: self.artists.into_iter().map(|artist| artist.name).collect(),
            album: self.album.name,
            cover_url: self.album.url_cover.standardize(),
            quality_hints: self
                .bit_rates
                .into_iter()
                .map(|bit_rate| bit_rate.quality)
                .collect(),
            playable_state: if self.label_info.only_vip_playable.unwrap_or(false) {
                "仅VIP"
            } else {
                "可播放"
            }
            .to_owned(),
            duration_ms: Some(self.duration),
            artwork_url: None,
        })
    }
}

#[derive(Deserialize)]
struct TrackAlbum {
    id: String,
    name: String,
    url_cover: SodaUrl,
    #[serde(default)]
    count_tracks: u32,
}

#[derive(Deserialize)]
struct SodaUrl {
    uri: Option<String>,
    urls: Option<Vec<String>>,
    template_prefix: Option<String>,
}

impl SodaUrl {
    fn standardize(self) -> String {
        match self.uri {
            Some(uri) => format!(
                "{}{}~{}-crop-center:256:256.webp",
                self.urls
                    .and_then(|cdn| cdn.first().cloned())
                    .unwrap_or_else(|| "https://p3-luna.douyinpic.com/img/".to_owned()),
                uri,
                self.template_prefix
                    .unwrap_or_else(|| "tplv-b829550vbb".to_owned())
            ),
            None => String::new(),
        }
    }
}

#[derive(Deserialize)]
struct Artist {
    name: String,
}

#[derive(Deserialize)]
struct BitRate {
    br: i64,
    size: i64,
    quality: String,
}

#[derive(Deserialize)]
struct PlayableRange {
    duration: i64,
    start: i64,
}

#[derive(Deserialize)]
struct LabelInfo {
    only_vip_download: Option<bool>,
    only_vip_playable: Option<bool>,
    quality_only_vip_can_download: Vec<String>,
    quality_only_vip_can_play: Vec<String>,
}

#[derive(Deserialize)]
struct Preview {
    duration: i64,
    start: Option<i64>,
    vid: String,
    bit_rates: Vec<BitRate>,
}

#[derive(Deserialize)]
struct TrackState {
    is_collected: Option<bool>,
}

#[derive(Deserialize)]
struct AlbumState {
    is_collected: Option<bool>,
}
