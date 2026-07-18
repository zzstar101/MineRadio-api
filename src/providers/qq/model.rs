use serde::Deserialize;

use crate::types::{AlbumDetail, AlbumSummary, PlayableState, Track};

#[derive(Debug, Deserialize)]
pub(super) struct QqSearchResp {
    data: QqSearchData,
}

#[derive(Debug, Deserialize)]
struct QqSearchData {
    song: QqSearchSongData,
}

#[derive(Debug, Deserialize)]
struct QqSearchSongData {
    //curnum: i32,
    //curpage: i32,
    list: Vec<QqSearchSong>,
    //totalnum: i32
}

#[derive(Debug, Deserialize)]
struct QqSearchSong {
    albummid: String,
    albumname: String,
    interval: i32,
    singer: Vec<Singer>,
    songmid: String,
    songname: String,
}

impl QqSearchResp {
    pub(super) fn standardize(self) -> Vec<Track> {
        self.data
            .song
            .list
            .into_iter()
            .map(|l| Track {
                id: l.songmid.clone(),
                provider: "qq".to_owned(),
                source_id: l.songmid.clone(),
                media_mid: Some(l.songmid),
                title: l.songname,
                artists: l.singer.into_iter().map(|s| s.name).collect(),
                album: l.albumname,
                cover_url: format!(
                    "https://y.gtimg.cn/music/photo_new/T002R300x300M000{}.jpg",
                    l.albummid
                ),
                quality_hints: vec!["standard".to_owned()],
                playable_state: PlayableState::Unknown,
                duration_ms: Some(l.interval as u64 * 1000),
                artwork_url: None,
            })
            .collect()
    }
}

#[derive(Debug, Deserialize)]
pub struct QqLyricResp {
    req_0: QqLyricReq,
}

impl QqLyricResp {
    pub fn standardize(self) -> (Option<String>, Option<String>) {
        let a = self.req_0.data;
        (a.lyric, a.trans)
    }
}

#[derive(Debug, Deserialize)]
pub struct QqLyricReq {
    data: QqLyricData,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QqLyricData {
    //crypt: i64,
    lyric: Option<String>,
    trans: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct QqAlbumListResp {
    #[serde(rename = "req_0")]
    list: QqAlbumListResponse,
}

impl QqAlbumListResp {
    pub(super) fn standardize(self) -> Vec<AlbumSummary> {
        self.list
            .data
            .albums
            .into_iter()
            .map(|s| AlbumSummary {
                provider: "qq".to_owned(),
                id: s.mid.clone(),
                name: s.name,
                artists: s.singer.into_iter().map(|a| a.name).collect(),
                cover_url: format!(
                    "https://y.gtimg.cn/music/photo_new/T002R300x300M000{}.jpg",
                    s.mid
                ),
                track_count: s.songnum,
                track_ids: vec![],
                collected: Some(true),
            })
            .collect()
    }
}

#[derive(Debug, Deserialize)]
struct QqAlbumListResponse {
    data: QqAlbumListData,
}

#[derive(Debug, Deserialize)]
struct QqAlbumListData {
    //number: i64,
    //hasmore: i64,
    #[serde(rename = "v_list")]
    albums: Vec<Album>,
    //total: i64,
}

#[derive(Debug, Deserialize)]
pub(super) struct QqAlbumDetailResp {
    #[serde(rename = "req_0")]
    song_list: QqAlbumDetailSongListResponse,

    #[serde(rename = "req_1")]
    info: QqAlbumDetailInfoResponse,
}

impl QqAlbumDetailResp {
    pub(super) fn standardize(self) -> AlbumDetail {
        let song_list = self.song_list.data;
        let cover_url = format!(
            "https://y.gtimg.cn/music/photo_new/T002R300x300M000{}.jpg",
            song_list.album_mid
        );
        let mut track_ids = Vec::new();

        let info = self.info.data;
        let (album, artists) = (info.basic_info, info.singer);

        let tracks: Vec<Track> = song_list
            .song_list
            .into_iter()
            .map(|s| {
                let l = s.song_info;
                track_ids.push(l.mid.clone());
                Track {
                    id: l.mid.clone(),
                    provider: "qq".to_owned(),
                    source_id: l.mid.clone(),
                    media_mid: Some(l.mid),
                    title: l.title,
                    artists: l.singer.into_iter().map(|s| s.name).collect(),
                    album: album.album_name.clone(),
                    cover_url: cover_url.clone(),
                    quality_hints: vec!["standard".to_owned()],
                    playable_state: if l.pay.pay_play == 1 {
                        PlayableState::PaidRequired
                    } else {
                        PlayableState::Playable
                    },
                    duration_ms: Some(l.interval as u64 * 1000),
                    artwork_url: None,
                }
            })
            .collect();

        AlbumDetail {
            provider: "qq".to_owned(),
            id: album.album_mid.clone(),
            name: album.album_name,
            artists: artists.singer_list.into_iter().map(|s| s.name).collect(),
            cover_url: format!(
                "https://y.gtimg.cn/music/photo_new/T002R300x300M000{}.jpg",
                album.album_mid
            ),
            track_count: Some(song_list.total_num as u32),
            track_ids,
            collected: None,
            tracks,
        }
    }
}

#[derive(Debug, Deserialize)]
struct QqAlbumDetailSongListResponse {
    data: QqAlbumDetailSongListData,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QqAlbumDetailSongListData {
    album_mid: String,

    total_num: i64,

    song_list: Vec<QqAlbumDetailSongListEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QqAlbumDetailSongListEntry {
    song_info: QqAlbumDetailTrack,
}

#[derive(Debug, Deserialize)]
struct QqAlbumDetailTrack {
    //id: i64,
    mid: String,

    title: String,

    singer: Vec<Singer>,

    interval: i64,

    pay: QqAlbumDetailTrackPay,
}

#[derive(Debug, Deserialize)]
struct QqAlbumDetailTrackPay {
    pay_play: i64,
}

#[derive(Debug, Deserialize)]
struct QqAlbumDetailInfoResponse {
    data: QqAlbumDetailInfoData,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QqAlbumDetailInfoData {
    basic_info: QqAlbumDetailInfo,
    singer: QqAlbumDetailArtists,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QqAlbumDetailInfo {
    album_mid: String,

    album_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QqAlbumDetailArtists {
    singer_list: Vec<Singer>,
}

//Reusable Struct
#[derive(Debug, Deserialize)]
struct Album {
    //id: i64,
    mid: String,
    name: String,
    songnum: Option<u32>,
    #[serde(alias = "v_singer")]
    singer: Vec<Singer>,
}

#[derive(Debug, Deserialize)]
struct Singer {
    //id: String,
    //mid: String,
    name: String,
}
