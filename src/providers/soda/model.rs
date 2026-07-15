use serde::Deserialize;

use crate::types::{AlbumDetail, AlbumSummary, Track};

#[derive(Deserialize)]
pub(super) struct SodaSearchResp {
    result_groups: Vec<SodaSearchGroup>,
}

impl SodaSearchResp {
    pub fn standardize(self) -> Vec<Track> {
        self.result_groups
            .into_iter()
            .find(|group| group.id == "tracks")
            .map(|group| {
                group
                    .data
                    .into_iter()
                    .map(|data| data.entity.track.standardize())
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[derive(Deserialize)]
struct SodaSearchGroup {
    id: String,
    data: Vec<SodaSearchData>,
}

#[derive(Deserialize)]
struct SodaSearchData {
    entity: SodaSearchEntity,
}

#[derive(Deserialize)]
struct SodaSearchEntity {
    track: SodaTrack,
}

#[derive(Deserialize)]
pub(super) struct SodaAlbumDetailResp {
    album_info: SodaAlbumListInfo,
    tracks: Vec<SodaTrack>,
}

impl SodaAlbumDetailResp {
    pub fn standardize(self) -> AlbumDetail {
        let singer = self
            .album_info
            .artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect();
        let album = self.album_info.standardize();
        let (track_ids, tracks): (Vec<String>, Vec<Track>) = self
            .tracks
            .into_iter()
            .map(|track| {
                let track = track.standardize();
                (track.id.clone(), track)
            })
            .unzip();

        AlbumDetail {
            provider: album.provider,
            id: album.id,
            name: album.name,
            singer,
            cover_url: album.cover_url,
            track_count: album.track_count,
            track_ids,
            subscribed: album.subscribed,
            tracks,
        }
    }
}

#[derive(Deserialize)]
pub(super) struct SodaAlbumListResp {
    mixed_collections: Vec<SodaAlbumListData>,
}

impl SodaAlbumListResp {
    pub fn standardize(self) -> Vec<AlbumSummary> {
        self.mixed_collections
            .into_iter()
            .map(|collection| collection.album.standardize())
            .collect()
    }
}

#[derive(Deserialize)]
struct SodaAlbumListData {
    album: SodaAlbumListInfo,
}

#[derive(Deserialize)]
struct SodaAlbumListInfo {
    id: String,
    name: String,
    artists: Vec<Artist>,
    count_tracks: u32,
    url_cover: Url,
    state: State,
}

impl SodaAlbumListInfo {
    fn standardize(self) -> AlbumSummary {
        let id = self.id;
        AlbumSummary {
            provider: "soda".to_owned(),
            id,
            name: self.name,
            cover_url: self.url_cover.standardize(),
            track_count: Some(self.count_tracks),
            track_ids: Vec::new(),
            subscribed: self.state.is_collected,
        }
    }
}

#[derive(Deserialize)]
struct TrackAlbum {
    name: String,
    url_cover: Url,
}

//reuseable
#[derive(Deserialize)]
struct Url {
    uri: Option<String>,
    urls: Option<Vec<String>>,
    template_prefix: Option<String>,
}

impl Url {
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
pub(super) struct SodaTrack {
    id: String,
    album: TrackAlbum,
    artists: Vec<Artist>,
    duration: u64,
    name: String,
    //state: TrackState,
    label_info: LabelInfo,
    bit_rates: Vec<BitRate>,
}

#[derive(Deserialize)]
struct BitRate {
    quality: String,
}

#[derive(Deserialize)]
struct LabelInfo {
    only_vip_playable: Option<bool>,
}

impl SodaTrack {
    fn standardize(self) -> Track {
        let id = self.id;
        Track {
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
        }
    }
}

#[derive(Deserialize)]
struct State {
    is_collected: Option<bool>,
}

#[derive(Deserialize)]
struct Artist {
    name: String,
}
