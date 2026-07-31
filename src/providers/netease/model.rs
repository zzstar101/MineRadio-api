use serde::Deserialize;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::{
    AlbumDetail, AlbumSummary, PlayableState, PlaylistDetail, PlaylistSummary, ProviderId,
    ProviderLoginStatus, Track, VipLevel,
};

use super::map::normalize_provider_image_url;

#[derive(Deserialize)]
pub(super) struct NeteaseSearchTrackResp {
    result: NeteaseSearchTrackResult,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NeteaseSearchTrackResult {
    #[serde(default)]
    songs: Vec<NeteaseSearchTrack>,
    #[serde(rename = "songCount", alias = "total", default)]
    song_count: u64,
    #[serde(default)]
    more: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NeteaseSearchTrack {
    id: i64,
    name: String,
    #[serde(rename = "ar", default)]
    artists: Vec<NameOnly>,
    #[serde(rename = "al")]
    album: Option<NeteaseSearchTrackAlbum>,
    #[serde(default)]
    fee: u8,
    #[serde(default)]
    dt: Option<u64>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NeteaseSearchTrackAlbum {
    #[serde(default)]
    name: String,
    #[serde(default)]
    pic_url: String,
}

impl NeteaseSearchTrackResp {
    pub(super) fn standardize(self) -> Vec<Track> {
        let _ = (self.result.song_count, self.result.more);
        self.result
            .songs
            .into_iter()
            .map(|song| {
                let id = song.id.to_string();
                let album = song.album.unwrap_or_default();
                Track {
                    id: id.clone(),
                    provider: ProviderId::Netease,
                    source_id: id,
                    media_mid: None,
                    title: song.name,
                    artists: song.artists.into_iter().map(|artist| artist.name).collect(),
                    album: album.name,
                    cover_url: normalize_provider_image_url(&album.pic_url),
                    quality_hints: vec!["standard".to_owned()],
                    playable_state: get_playable(song.fee),
                    duration_ms: song.dt,
                    artwork_url: None,
                }
            })
            .collect()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NeteaseLyricResp {
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
                lyric: (!inner.lyric.is_empty()).then_some(inner.lyric),
            },
            tlyric: inner.tlyric.unwrap_or(NeteaseLyric { lyric: None }),
            yrc: inner.yrc.unwrap_or(NeteaseLyric { lyric: None }),
        }
    }
}

#[derive(Deserialize)]
pub(super) struct NeteaseLyric {
    pub(super) lyric: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NeteaseAlbumListResp {
    data: Vec<Album>,
    //has_more: bool,
}

impl NeteaseAlbumListResp {
    pub(super) fn standardize(self) -> Option<Vec<AlbumSummary>> {
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
        (!v.is_empty()).then_some(v)
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NeteaseSearchAlbumResp {
    result: NeteaseSearchAlbumData,
}

#[derive(Deserialize)]
struct NeteaseSearchAlbumData {
    albums: Vec<NeteaseSearchAlbum>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NeteaseSearchAlbum {
    id: u64,
    name: String,
    pic_url: String,
    artist: NameOnly,
}

impl NeteaseSearchAlbumResp {
    pub(super) fn standardize(self) -> Option<Vec<AlbumSummary>> {
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
        (!v.is_empty()).then_some(v)
    }
}

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
        (!v.is_empty()).then_some(v)
    }
}

#[derive(Deserialize)]
pub(crate) struct NeteaseLoginStatusResp {
    profile: Option<NeteaseLoginStatusProfile>,
}

impl NeteaseLoginStatusResp {
    pub(crate) fn standardize(self) -> ProviderLoginStatus {
        let Some(profile) = self.profile else {
            return ProviderLoginStatus {
                provider: ProviderId::Netease,
                logged_in: false,
                ..Default::default()
            };
        };

        ProviderLoginStatus {
            provider: ProviderId::Netease,
            logged_in: true,
            nickname: profile.nickname,
            user_id: Some(profile.user_id.to_string()),
            avatar_url: profile.avatar_url,
            ..Default::default()
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NeteaseLoginStatusProfile {
    user_id: i64,
    nickname: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct NeteasePlaylistListResp {
    playlist: Vec<NeteasePlaylist>,
}

impl NeteasePlaylistListResp {
    pub(super) fn standardize(self) -> Option<Vec<PlaylistSummary>> {
        let v: Vec<PlaylistSummary> = self
            .playlist
            .into_iter()
            .map(|l| PlaylistSummary {
                provider: ProviderId::Netease,
                id: l.id.to_string(),
                name: l.name,
                cover_url: l.cover_img_url,
                track_count: l.track_count,
                track_ids: vec![],
                collected: Some(true),
            })
            .collect();
        (!v.is_empty()).then_some(v)
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NeteasePlaylist {
    track_count: Option<u32>,
    cover_img_url: String,
    name: String,
    id: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NeteasePlaylistDetailResp {
    playlist: Playlist,
}

impl NeteasePlaylistDetailResp {
    pub(super) fn standardize(self) -> PlaylistDetail {
        let l = self.playlist;
        let d = l.detail;
        let mut track_ids = Vec::new();
        let tracks = l
            .tracks
            .into_iter()
            .map(|t| {
                let s = t.song;
                track_ids.push(s.id.to_string());
                Track {
                    id: s.id.to_string(),
                    provider: ProviderId::Netease,
                    source_id: s.id.to_string(),
                    media_mid: None,
                    title: s.name,
                    artists: s.ar.into_iter().map(|a| a.name).collect(),
                    album: t.al.name,
                    cover_url: t.al.pic_url,
                    quality_hints: vec!["standard".to_owned()],
                    duration_ms: s.dt,
                    playable_state: get_playable(s.fee),
                    artwork_url: None,
                }
            })
            .collect();
        PlaylistDetail {
            provider: ProviderId::Netease,
            id: d.id.to_string(),
            name: d.name,
            cover_url: d.cover_img_url,
            track_count: d.track_count,
            track_ids,
            collected: Some(l.subscribed),
            tracks,
            has_more: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Playlist {
    #[serde(flatten)]
    detail: NeteasePlaylist,
    subscribed: bool,
    tracks: Vec<NeteaseTrack>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NeteaseTrack {
    #[serde(flatten)]
    song: Song,
    al: Al,
}

#[derive(Deserialize)]
struct Al {
    name: String,
    #[serde(rename = "picUrl")]
    pic_url: String,
}

#[derive(Deserialize)]
pub(super) struct NeteaseVipInfoResp {
    data: NeteaseVipInfoData,
}

const NETEASE_VIP_LEVEL_NAMES: [&str; 11] = [
    "", "壹", "贰", "叁", "肆", "伍", "陆", "柒", "捌", "玖", "拾",
];

impl NeteaseVipInfoResp {
    pub(super) fn standardize(self, l: ProviderLoginStatus) -> ProviderLoginStatus {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or_default();
        let redplus_active = self.data.redplus.is_active(now_ms);
        let music_associator_active = self.data.associator.is_active(now_ms);
        let vip_level = if redplus_active {
            VipLevel::Svip
        } else if music_associator_active {
            VipLevel::Vip
        } else {
            VipLevel::None
        };
        let vip_icon_url = if redplus_active {
            self.data
                .redplus
                .dynamic_icon_url
                .or(self.data.redplus.icon_url)
        } else if music_associator_active {
            self.data
                .associator
                .dynamic_icon_url
                .or(self.data.associator.icon_url)
        } else {
            None
        };
        let vip_tier = match vip_level {
            VipLevel::Svip => Some(self.data.redplus.vip_level),
            VipLevel::Vip => Some(self.data.associator.vip_level),
            VipLevel::None => None,
        }
        .filter(|level| *level > 0);
        let vip_type = match vip_level {
            VipLevel::Svip => Some(11),
            VipLevel::Vip => Some(1),
            VipLevel::None => Some(0),
        };
        let vip_level_name = vip_tier.and_then(vip_level_name_of);
        let vip_label = match vip_level {
            VipLevel::Svip => Some("黑胶SVIP".to_owned()),
            VipLevel::Vip => Some("黑胶VIP".to_owned()),
            VipLevel::None => None,
        };

        ProviderLoginStatus {
            vip_type,
            vip_level: Some(vip_level),
            is_vip: Some(vip_level != VipLevel::None),
            is_svip: Some(vip_level == VipLevel::Svip),
            vip_label,
            vip_icon: match vip_level {
                VipLevel::Svip => Some("netease-svip".to_owned()),
                VipLevel::Vip => Some("netease-vip".to_owned()),
                _ => None,
            },
            vip_icon_url,
            vip_tier,
            vip_level_name,
            ..l
        }
    }
}

fn vip_level_name_of(tier: i64) -> Option<String> {
    NETEASE_VIP_LEVEL_NAMES
        .get(tier as usize)
        .map(|value| (*value).to_owned())
        .or_else(|| Some(tier.to_string()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NeteaseVipInfoData {
    //svip
    redplus: Associator,
    //vip
    associator: Associator,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Associator {
    expire_time: i64,
    icon_url: Option<String>,
    dynamic_icon_url: Option<String>,
    vip_level: i64,
}

impl Associator {
    fn is_active(&self, now_ms: i64) -> bool {
        self.expire_time > now_ms
    }
}

//reuseable
#[derive(Deserialize)]
struct NameOnly {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Song {
    ar: Vec<NameOnly>,
    fee: u8,
    name: String,
    id: i64,
    dt: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Album {
    artists: Vec<NameOnly>,
    pic_url: String,
    name: String,
    id: i64,
    size: Option<u32>,
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn standardizes_cloudsearch_track_response() {
        let response: NeteaseSearchTrackResp = serde_json::from_value(json!({
            "result": {
                "songs": [{
                    "id": 42,
                    "name": "Test",
                    "ar": [{"name": "Artist"}],
                    "al": {"name": "Album", "picUrl": "http://a/b.jpg"},
                    "fee": 0,
                    "dt": 1234
                }],
                "songCount": 1,
                "more": false
            }
        }))
        .expect("cloudsearch response should deserialize");

        let tracks = response.standardize();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].id, "42");
        assert_eq!(tracks[0].title, "Test");
        assert_eq!(tracks[0].artists, vec!["Artist"]);
        assert_eq!(tracks[0].album, "Album");
        assert_eq!(tracks[0].cover_url, "https://a/b.jpg");
        assert_eq!(tracks[0].duration_ms, Some(1234));
    }
}
