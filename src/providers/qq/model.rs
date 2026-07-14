use serde::Deserialize;

use crate::types::{AlbumDetail, AlbumSummary, Track};

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
                playable_state: "unknown".to_owned(),
                duration_ms: Some(l.interval as u64 * 1000),
                artwork_url: None,
            })
            .collect()
    }
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
                    album: l.album.name,
                    cover_url: cover_url.clone(),
                    quality_hints: vec!["standard".to_owned()],
                    playable_state: {
                        if l.pay.pay_play == 1 {
                            "付费可播放"
                        } else {
                            "可播放"
                        }
                    }
                    .to_string(),
                    duration_ms: Some(l.interval as u64 * 1000),
                    artwork_url: None,
                }
            })
            .collect();
        let info = self.info.data;
        let (album, singer) = (info.basic_info, info.singer);
        AlbumDetail {
            provider: "qq".to_owned(),
            id: album.album_mid.clone(),
            name: album.album_name,
            singer: singer.singer_list.into_iter().map(|s| s.name).collect(),
            cover_url: format!(
                "https://y.gtimg.cn/music/photo_new/T002R300x300M000{}.jpg",
                album.album_mid
            ),
            track_count: Some(song_list.total_num as u32),
            track_ids,
            subscribed: None,
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

    album: Album,

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

#[derive(Debug, Deserialize)]
pub(super) struct QqAlbumFavoritesResp {
    #[serde(rename = "req_0")]
    favorites: QqAlbumFavoritesResponse,
}

impl QqAlbumFavoritesResp {
    pub(super) fn standardize(self) -> Vec<AlbumSummary> {
        self.favorites
            .data
            .albums
            .into_iter()
            .map(|s| AlbumSummary {
                provider: "qq".to_owned(),
                id: s.mid.clone(),
                name: s.name,
                cover_url: format!(
                    "https://y.gtimg.cn/music/photo_new/T002R300x300M000{}.jpg",
                    s.mid
                ),
                track_count: s.songnum,
                track_ids: vec![],
                subscribed: Some(true),
            })
            .collect()
    }
}

#[derive(Debug, Deserialize)]
struct QqAlbumFavoritesResponse {
    data: QqAlbumFavoritesData,
}

#[derive(Debug, Deserialize)]
struct QqAlbumFavoritesData {
    //number: i64,
    //hasmore: i64,
    #[serde(rename = "v_list")]
    albums: Vec<Album>,
    //total: i64,
}

//Reusable Struct
#[derive(Debug, Deserialize)]
struct Album {
    //id: i64,
    mid: String,
    name: String,
    songnum: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct Singer {
    //mid: String,
    name: String,
}
