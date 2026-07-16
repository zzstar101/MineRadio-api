use serde::Deserialize;

use crate::types::{AlbumDetail, AlbumSummary, SongUrlOptions, SongUrlResult, Track};

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

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct SodaSongUrlResp {
    result: SodaSongUrlResult,
}

impl SodaSongUrlResp {
    pub fn standardize(self, opt: SongUrlOptions) -> Option<SongUrlResult> {
        let target = opt.quality.unwrap_or(String::new());
        let (a, b) = match target.as_str() {
            "jymaster" => ("spatial", "录音室音质"),
            "hires" => ("hi_res", "超清全景声"),
            "lossless" => ("highest", "无损音质"),
            "exhigh" => ("higher", "极高音质"),
            "standard" | _ => ("medium", "标准音质"),
        };

        let list = self.result.data;
        let play_info = list
            .play_info_list
            .iter()
            .find(|item| item.quality == a)
            .or_else(|| list.play_info_list.first())?;
        let play_url = (!play_info.main_play_url.is_empty())
            .then_some(play_info.main_play_url.as_str())
            .or_else(|| {
                (!play_info.backup_play_url.is_empty())
                    .then_some(play_info.backup_play_url.as_str())
            })?;

        Some(SongUrlResult {
            url: Some(format!(
                "/providers/soda/audio-proxy?url={}&playAuth={}",
                urlencoding::encode(play_url),
                urlencoding::encode(&play_info.play_auth)
            )),
            proxied: true,
            provider: Some("soda".to_owned()),
            trial: None,
            playable: Some(true),
            level: Some(play_info.quality.clone()),
            quality: Some(b.to_owned()),
            br: Some(play_info.bitrate),
            expires_at: Some(play_info.url_expire.to_string()),
            ..Default::default()
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SodaSongUrlResult {
    data: SodaSongUrlData,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SodaSongUrlData {
    play_info_list: Vec<SodaSongUrlList>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SodaSongUrlList {
    bitrate: u32,

    quality: String,

    play_auth: String,

    main_play_url: String,

    backup_play_url: String,

    url_expire: i64,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn song_url_standardize_uses_requested_quality_and_backup_url() {
        let response = serde_json::from_value::<SodaSongUrlResp>(json!({
            "Result": {
                "Data": {
                    "PlayInfoList": [
                        {
                            "Bitrate": 320000,
                            "Quality": "higher",
                            "PlayAuth": "auth",
                            "MainPlayUrl": "",
                            "BackupPlayUrl": "https://cdn.example.com/song.m4a",
                            "UrlExpire": 123
                        }
                    ]
                }
            }
        }))
        .unwrap();

        let result = response
            .standardize(SongUrlOptions {
                quality: Some("exhigh".to_owned()),
            })
            .unwrap();

        assert_eq!(result.level.as_deref(), Some("higher"));
        assert_eq!(result.br, Some(320000));
        assert_eq!(result.expires_at.as_deref(), Some("123"));
        assert!(
            result
                .url
                .as_deref()
                .is_some_and(|url| url.contains("https%3A%2F%2Fcdn.example.com%2Fsong.m4a"))
        );
    }
}
