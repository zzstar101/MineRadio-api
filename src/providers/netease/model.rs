use serde::Deserialize;

use crate::types::{AlbumDetail, AlbumSummary, PlayableState, PlaylistSummary, ProviderId, Track};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseLyricResp {
    //lrc歌词
    pub(super) lrc: NeteaseLyric,
    //逐字歌词
    pub(super) yrc: NeteaseLyric,
    //lrc翻译歌词
    pub(super) tlyric: NeteaseLyric,
}

/// lyric/v1 wraps everything under a top-level `lrc` key.
/// Converted to [`NeteaseLyricResp`] for a unified model.
#[derive(Deserialize)]
pub(super) struct NeteaseLyricV1Resp {
    lrc: NeteaseLyricV1Inner,
}

#[derive(Deserialize)]
struct NeteaseLyricV1Inner {
    #[serde(default)]
    lyric: String,
    #[serde(default)]
    tlyric: Option<NeteaseLyric>,
    #[serde(default)]
    yrc: Option<NeteaseLyric>,
}

impl From<NeteaseLyricV1Resp> for NeteaseLyricResp {
    fn from(v1: NeteaseLyricV1Resp) -> Self {
        let inner = v1.lrc;
        Self {
            lrc: NeteaseLyric {
                lyric: if inner.lyric.is_empty() {
                    None
                } else {
                    Some(inner.lyric)
                },
            },
            tlyric: inner.tlyric.unwrap_or(NeteaseLyric { lyric: None }),
            yrc: inner.yrc.unwrap_or(NeteaseLyric { lyric: None }),
        }
    }
}

#[derive(Deserialize)]
pub struct NeteaseLyric {
    pub(super) lyric: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NeteaseAlbumListResp {
    data: Vec<Album>,
    has_more: bool,
}

impl NeteaseAlbumListResp {
    pub(super) fn standardize(self) -> Option<Vec<AlbumSummary>> {
        if self.data.is_empty() {
            return None;
        }
        let v: Vec<AlbumSummary> = self
            .data
            .into_iter()
            .map(|a| AlbumSummary {
                provider: ProviderId::Netease,
                id: a.id.to_string(),
                name: a.name,
                artists: a.artists.into_iter().map(|a| a.name).collect(),
                cover_url: a.pic_url,
                track_count: a.size,
                track_ids: vec![],
                collected: Some(true),
            })
            .collect();
        if v.is_empty() { None } else { Some(v) }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NeteaseAlbumDetailResp {
    songs: Vec<Song>,
    album: Album,
}

impl NeteaseAlbumDetailResp {
    pub(super) fn standardize(self) -> AlbumDetail {
        let a = self.album;
        let mut track_ids = Vec::new();
        let tracks: Vec<Track> = self
            .songs
            .into_iter()
            .map(|t| {
                track_ids.push(t.id.to_string());
                Track {
                    id: t.id.to_string(),
                    provider: ProviderId::Netease,
                    source_id: t.id.to_string(),
                    media_mid: None,
                    title: t.name,
                    artists: t.ar.into_iter().map(|a| a.name).collect(),
                    album: a.name.clone(),
                    cover_url: a.pic_url.clone(),
                    quality_hints: vec!["standard".to_owned()],
                    duration_ms: t.dt,
                    playable_state: get_playable(t.fee),
                    artwork_url: None,
                }
            })
            .collect();
        AlbumDetail {
            provider: ProviderId::Netease,
            id: a.id.to_string(),
            name: a.name,
            artists: a.artists.into_iter().map(|a| a.name).collect(),
            cover_url: a.pic_url,
            track_count: a.size,
            track_ids,
            collected: None,
            has_more: None,
            tracks,
        }
    }
}

// ── 搜索响应模型（参考 netease-qq-music-api）──

/// `/api/v1/search/album/get` 专辑搜索响应
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NeteaseSearchAlbumResp {
    result: NeteaseSearchAlbumData,
}

#[derive(Deserialize)]
struct NeteaseSearchAlbumData {
    albums: Vec<NeteaseSearchAlbum>,
    #[serde(rename = "albumCount")]
    album_count: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NeteaseSearchAlbum {
    id: u64,
    name: String,
    pic_url: String,
    artist: NeteaseSearchAlbumArtist,
}

#[derive(Deserialize)]
struct NeteaseSearchAlbumArtist {
    id: u64,
    name: String,
}

impl NeteaseSearchAlbumResp {
    pub(super) fn standardize(self) -> Option<Vec<AlbumSummary>> {
        if self.result.albums.is_empty() {
            return None;
        }
        let v: Vec<AlbumSummary> = self
            .result
            .albums
            .into_iter()
            .map(|a| AlbumSummary {
                provider: ProviderId::Netease,
                id: a.id.to_string(),
                name: a.name,
                artists: vec![a.artist.name],
                cover_url: a.pic_url,
                track_count: None,
                track_ids: vec![],
                collected: None,
            })
            .collect();
        if v.is_empty() { None } else { Some(v) }
    }
}

/// `/api/v1/search/playlist/get` 歌单搜索响应
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NeteaseSearchPlaylistResp {
    result: NeteaseSearchPlaylistData,
}

#[derive(Deserialize)]
struct NeteaseSearchPlaylistData {
    playlists: Vec<NeteaseSearchPlaylist>,
}

#[derive(Deserialize)]
struct NeteaseSearchPlaylist {
    id: u64,
    name: String,
    #[serde(rename = "coverImgUrl")]
    cover_img_url: String,
}

impl NeteaseSearchPlaylistResp {
    pub(super) fn standardize(self) -> Option<Vec<PlaylistSummary>> {
        if self.result.playlists.is_empty() {
            return None;
        }
        let v: Vec<PlaylistSummary> = self
            .result
            .playlists
            .into_iter()
            .map(|p| PlaylistSummary {
                provider: ProviderId::Netease,
                id: p.id.to_string(),
                name: p.name,
                cover_url: p.cover_img_url,
                track_count: None,
                track_ids: vec![],
                collected: None,
            })
            .collect();
        if v.is_empty() { None } else { Some(v) }
    }
}

fn get_playable(fee: u8) -> PlayableState {
    match fee {
        0 => PlayableState::CopyrightUnavailable,
        1 => PlayableState::VipRequired,
        4 => PlayableState::PaidRequired,
        8 => PlayableState::TrialOnly,
        _ => PlayableState::Unknown,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Song {
    ar: Vec<Artist>,
    fee: u8,
    /*h: H,
    sq: H,
    hr: H,
    l: H,
    m: H,*/
    name: String,
    id: i64,
    dt: Option<u64>,
}

/*#[derive(Deserialize)]
pub struct H {
    br: i64,
    fid: i64,
    size: i64,
    vd: i64,
    sr: i64,
}

 #[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Privilege {
    id: i64,
    fee: i64,
    free_trial_privilege: FreeTrialPrivilege,
}


#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeTrialPrivilege {
    res_consumable: bool,
    user_consumable: bool,
    listen_type: i64,
    cannot_listen_reason: i64,
    play_reason: Option<serde_json::Value>,
    free_limit_tag_type: Option<serde_json::Value>,
}*/

//reuseable
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Album {
    artists: Vec<Artist>,
    pic_url: String,
    name: String,
    id: i64,
    size: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Artist {
    /*#[serde(rename = "img1v1Id")]
    img1_v1_id: f64,
    pic_url: String,
    id: i64,*/
    name: String,
}
