use serde::{Serialize, Deserialize};

use crate::types::{AlbumDetail, AlbumSummary, Track};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NeteaseAlbumListResp {
    data: Vec<Album>,
    count: i64,
    has_more: bool,
    paid_count: i64,
}

impl NeteaseAlbumListResp {
    pub(super) fn standardize(self) -> Vec<AlbumSummary> {
        self
        .data
        .into_iter()
        .map(|a| {
            AlbumSummary { provider: "netease".to_owned(),
            id: a.id.to_string(), name: a.name, cover_url: a.pic_url, track_count: a.size, track_ids: vec![], subscribed: Some(true) }
        })
        .collect()
    }
}


#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NeteaseAlbumDetailResp {
    songs: Vec<Song>,
    album: Album,
}

impl NeteaseAlbumDetailResp {
    pub(super) fn standardize(self) -> AlbumDetail {
        let a = self.album;
        let mut track_ids = Vec::new();
        let tracks :Vec<Track> = self
        .songs
        .into_iter()
        .map(|t| {
            track_ids.push(t.id.to_string());
            Track {
                id: t.id.to_string(),
                provider: "netease".to_owned(),
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
        AlbumDetail { provider: "netease".to_owned(), id: a.id.to_string(), name: a.name, singer: a.artists.into_iter().map(|a| a.name).collect(), cover_url: a.pic_url, track_count: a.size, track_ids, subscribed: Some(false), tracks }
    }
}

fn get_playable(fee: u8) -> String {
    match fee {
        0 => "免费或无版权",
        1 => "VIP 歌曲",
        4 => "购买专辑",
        8 => "非会员可免费播放低音质，会员可播放高音质及下载",
        _ => "unknown"
    }.to_string()
}




#[derive(Serialize, Deserialize)]
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

#[derive(Serialize, Deserialize)]
pub struct Al {
    name: String,
}

/*#[derive(Serialize, Deserialize)]
pub struct H {
    br: i64,
    fid: i64,
    size: i64,
    vd: i64,
    sr: i64,
}

 #[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Privilege {
    id: i64,
    fee: i64,
    free_trial_privilege: FreeTrialPrivilege,
}


#[derive(Serialize, Deserialize)]
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
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Album {
    artists: Vec<Artist>,
    pic_url: String,
    name: String,
    id: i64,
    size: Option<u32>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Artist {
    /*#[serde(rename = "img1v1Id")]
    img1_v1_id: f64,
    pic_url: String,
    id: i64,*/

    name: String,
}
