use serde::Deserialize;

use crate::types::{
    AlbumDetail, AlbumSummary, PlaylistDetail, PlaylistSummary, SongUrlOptions, SongUrlResult,
    Track, TrackQualityAvailability, TrackQualityOption,
};

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
pub(super) struct SodaPlaylistListResp {
    playlists: Vec<SodaPlaylistListList>,
}

impl SodaPlaylistListResp {
    pub fn standardize(self) -> Option<Vec<PlaylistSummary>> {
        let res: Vec<PlaylistSummary> = self
            .playlists
            .into_iter()
            .map(|p| PlaylistSummary {
                provider: "soda".to_owned(),
                id: p.id,
                name: p.title,
                cover_url: p
                    .url_cover
                    .map(|u| u.standardize())
                    .unwrap_or("".to_string()),
                track_count: p.count_tracks,
                track_ids: vec![],
                collected: Some(true),
            })
            .collect();
        if res.is_empty() { None } else { Some(res) }
    }
}

#[derive(Deserialize)]
struct SodaPlaylistListList {
    id: String,

    title: String,
    //为什么会有缺封面的呀,汽水你这家伙
    url_cover: Option<SodaUrl>,

    count_tracks: Option<u32>,
}

#[derive(Deserialize)]
pub(super) struct SodaPLaylistDetailResp {
    //next_cursor: Option<String>,
    playlist: Playlist,

    media_resources: Vec<MediaResource>,
}

impl SodaPLaylistDetailResp {
    pub fn standardize(self) -> Option<PlaylistDetail> {
        let p = self.playlist;
        let tracks: Vec<Track> = self
            .media_resources
            .into_iter()
            .map(|m| m.entity.track_wrapper.track.standardize())
            .collect();
        if tracks.is_empty() {
            None
        } else {
            Some(PlaylistDetail {
                provider: "soda".to_owned(),
                id: p.id,
                name: p.title,
                cover_url: p.url_cover.standardize(),
                track_count: p.count_tracks,
                track_ids: tracks.iter().map(|t| t.id.clone()).collect(),
                collected: p.state.and_then(|s| s.is_collected),
                tracks,
            })
        }
    }
}

#[derive(Deserialize)]
struct MediaResource {
    entity: Entity,
}

#[derive(Deserialize)]
struct Entity {
    track_wrapper: TrackWrapper,
}

#[derive(Deserialize)]
struct TrackWrapper {
    track: SodaTrack,
}

/* #[derive(Deserialize)]
struct Owner {
    id: String,

    nickname: String,

    medium_avatar_url: AvatarUrl,
}

#[derive(Deserialize)]
struct AvatarUrl {
    urls: Vec<String>,

    need_complete_url: bool,
}*/

#[derive(Deserialize)]
struct Playlist {
    id: String,

    title: String,

    url_cover: SodaUrl,

    count_tracks: Option<u32>,

    //owner: Owner,
    state: Option<State>,
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
    url_cover: SodaUrl,
    state: Option<State>,
}

impl SodaAlbumListInfo {
    fn standardize(self) -> AlbumSummary {
        let id = self.id;
        AlbumSummary {
            provider: "soda".to_owned(),
            id,
            artists: self
                .artists
                .into_iter()
                .map(|a| a.name.unwrap_or_default())
                .collect(),
            name: self.name,
            cover_url: self.url_cover.standardize(),
            track_count: Some(self.count_tracks),
            track_ids: Vec::new(),
            collected: self.state.and_then(|s| s.is_collected),
        }
    }
}

#[derive(Deserialize)]
pub(super) struct SodaAlbumDetailResp {
    album_info: SodaAlbumListInfo,
    tracks: Vec<SodaTrack>,
}

impl SodaAlbumDetailResp {
    pub fn standardize(self) -> AlbumDetail {
        let artists = self
            .album_info
            .artists
            .iter()
            .map(|artist| artist.name.clone().unwrap_or_default())
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
            artists,
            cover_url: album.cover_url,
            track_count: album.track_count,
            track_ids,
            collected: album.collected,
            tracks,
        }
    }
}

#[derive(Deserialize)]
struct TrackAlbum {
    name: String,
    url_cover: Option<SodaUrl>,
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

#[derive(Deserialize)]
pub(super) struct SodaTrackV2Resp {
    lyric: Lyric,
    track: SodaTrack,
    track_player: TrackPlayer,
}

impl SodaTrackV2Resp {
    pub fn standardize_lyric(self) -> (String, Option<String>, String) {
        (
            self.lyric.content,
            self.lyric.translations.map(|t| t.cn),
            self.track.id,
        )
    }
    pub fn standardize_track_qualities(self) -> Option<TrackQualityAvailability> {
        Some(TrackQualityAvailability {
            provider: "soda".to_owned(),
            track_id: self.track.id.clone(),
            default_quality: Some("standard".to_string()),
            qualities: self.track.standardize_quality()?,
        })
    }
    pub fn get_songurl(self) -> String {
        self.track_player.url_player_info
    }
    pub fn is_collected(self) -> Option<bool> {
        self.track.state.and_then(|s| s.is_collected)
    }
}

#[derive(Deserialize)]
struct Lyric {
    content: String,
    translations: Option<Cn>,
}

#[derive(Deserialize)]
struct Cn {
    cn: String,
}

#[derive(Deserialize)]
struct TrackPlayer {
    url_player_info: String,
}

//reuseable
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
pub(super) struct SodaTrack {
    id: String,
    album: TrackAlbum,
    artists: Vec<Artist>,
    duration: u64,
    name: String,
    state: Option<State>,
    label_info: LabelInfo,
    bit_rates: Vec<BitRate>,
}

impl SodaTrack {
    pub fn standardize(self) -> Track {
        let id = self.id;
        Track {
            source_id: id.clone(),
            id,
            provider: "soda".to_owned(),
            media_mid: None,
            title: self.name,
            artists: self
                .artists
                .into_iter()
                .map(|artist| artist.name.unwrap_or_default())
                .collect(),
            album: self.album.name,
            cover_url: self
                .album
                .url_cover
                .map(|u| u.standardize())
                .unwrap_or_default(),
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
    pub fn standardize_quality(self) -> Option<Vec<TrackQualityOption>> {
        let s: Vec<TrackQualityOption> = self
            .bit_rates
            .into_iter()
            .map(|b| {
                let raw_quality = b.quality;
                let (level, label) = match raw_quality.as_str() {
                    "spatial" => ("jymaster", "录音室音质"),
                    "hi_res" => ("hires", "超清全景声"),
                    "highest" => ("lossless", "无损音质"),
                    "higher" => ("exhigh", "极高音质"),
                    "medium" | _ => ("standard", "标准音质"),
                };
                let (level, label) = (level.to_string(), label.to_string());
                let br = b.br;
                let size = b.size;
                TrackQualityOption {
                    provider: "soda".to_owned(),
                    id: level.to_owned(),
                    label,
                    detail: Some(
                        if self.label_info.only_vip_playable.unwrap_or(false) {
                            "仅VIP"
                        } else {
                            "可播放"
                        }
                        .to_owned(),
                    ),
                    request_quality: level.to_owned(),
                    level: Some(level.to_owned()),
                    r#type: Some(raw_quality),
                    br: Some(br),
                    size: Some(size),
                    source: "declared".to_owned(),
                    ..Default::default()
                }
            })
            .collect();
        if s.is_empty() { None } else { Some(s) }
    }
}

#[derive(Deserialize)]
struct BitRate {
    br: u32,
    size: u64,
    quality: String,
}

#[derive(Deserialize)]
struct LabelInfo {
    only_vip_playable: Option<bool>,
}

#[derive(Deserialize)]
struct State {
    is_collected: Option<bool>,
}

#[derive(Deserialize)]
struct Artist {
    //抖音创作原声是没有作者的哈基汽水
    name: Option<String>,
}
