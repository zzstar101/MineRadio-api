use serde::Deserialize;

use crate::types::{AlbumDetail, AlbumSummary, PlayableState, Track, ProviderId};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseLyricResp {
    //lrc歌词
    lrc: NeteaseLyric,
    //逐字歌词
    yrc: NeteaseLyric,
    //lrc翻译歌词
    tlyric: NeteaseLyric,
}

impl NeteaseLyricResp {
    pub fn standardize(self) -> (Option<String>, Option<String>) {
        (self.yrc.lyric.or(self.lrc.lyric), self.tlyric.lyric)
    }
}
#[derive(Deserialize)]
pub struct NeteaseLyric {
    version: i64,

    lyric: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NeteaseAlbumListResp {
    data: Vec<Album>,
    count: i64,
    has_more: bool,
    paid_count: i64,
}

impl NeteaseAlbumListResp {
    pub(super) fn standardize(self) -> Vec<AlbumSummary> {
        self.data
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
            .collect()
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
            tracks,
        }
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
    al: Al,
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

#[derive(Deserialize)]
pub struct Al {
    name: String,
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
