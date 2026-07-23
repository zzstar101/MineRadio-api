use serde::Deserialize;

use crate::types::{
    AlbumDetail, AlbumSummary, PlayableState, PlaylistDetail, PlaylistSummary, ProviderId, Track,
    TrackQualityAvailability, TrackQualityOption,
};

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
    singer: Vec<Identified>,
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
                provider: ProviderId::Qq,
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
pub(super) struct QqTrackDetailResp {
    req_0: QqTrackDetailReq,
}

impl QqTrackDetailResp {
    pub fn standardize(self) -> Option<TrackQualityAvailability> {
        let t = self.req_0.data.track_info;
        let qualities = t.file.standardize(Some(t.mid.clone()));
        if qualities.is_empty() {
            None
        } else {
            Some(TrackQualityAvailability {
                provider: ProviderId::Qq,
                track_id: t.mid,
                default_quality: qualities.first().map(|item| item.request_quality.clone()),
                qualities,
            })
        }
    }
}
#[derive(Debug, Deserialize)]
struct QqTrackDetailReq {
    data: QqTrackDetailData,
}

#[derive(Debug, Deserialize)]
struct QqTrackDetailData {
    track_info: QqTrackDetailInfo,
}

#[derive(Debug, Deserialize)]
struct QqTrackDetailInfo {
    mid: String,
    file: File,
}

#[derive(Debug, Deserialize)]
pub(super) struct QqLyricResp {
    req_0: QqLyricReq,
}

impl QqLyricResp {
    pub fn standardize(self) -> (Option<String>, Option<String>) {
        let a = self.req_0.data;
        (a.lyric, a.trans)
    }
}

#[derive(Debug, Deserialize)]
struct QqLyricReq {
    data: QqLyricData,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QqLyricData {
    //crypt: i64,
    lyric: Option<String>,
    trans: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct QqPlaylistList1Resp {
    req_0: QqPlaylistList1Req,
}
impl QqPlaylistList1Resp {
    pub fn standardize(self) -> Option<Vec<PlaylistSummary>> {
        let v: Vec<PlaylistSummary> = self
            .req_0
            .data
            .v_playlist
            .into_iter()
            .map(|l| PlaylistSummary {
                provider: ProviderId::Qq,
                id: l.tid.to_string(),
                name: l.dir_name,
                cover_url: l.pic_url,
                track_count: l.song_num,
                track_ids: vec![],
                collected: Some(true),
            })
            .collect();
        if v.is_empty() { None } else { Some(v) }
    }
}
#[derive(Deserialize)]
struct QqPlaylistList1Req {
    data: QqPlaylistList1Data,
}

#[derive(Deserialize)]
struct QqPlaylistList1Data {
    v_playlist: Vec<QqPlaylistList1Playlist>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QqPlaylistList1Playlist {
    //dir_id: i64, //观察到收藏需要知道'我喜欢'歌单的dir_id
    dir_name: String,

    tid: i64,

    song_num: Option<u32>,

    pic_url: String,
}

#[derive(Deserialize)]
pub struct QqPlaylistList2Resp {
    req_0: QqPlaylistList2Req,
}

impl QqPlaylistList2Resp {
    pub fn standardize(self) -> Option<Vec<PlaylistSummary>> {
        let v: Vec<PlaylistSummary> = self
            .req_0
            .data
            .v_list
            .into_iter()
            .map(|l| PlaylistSummary {
                provider: ProviderId::Qq,
                id: l.tid.to_string(),
                name: l.name,
                cover_url: l.logo,
                track_count: l.songnum,
                track_ids: vec![],
                collected: Some(true),
            })
            .collect();
        if v.is_empty() { None } else { Some(v) }
    }
}

#[derive(Deserialize)]
pub struct QqPlaylistList2Req {
    data: QqPlaylistList2Data,
}

#[derive(Deserialize)]
pub struct QqPlaylistList2Data {
    //number: i64,
    //hasmore: i64,
    v_list: Vec<QqPlaylistList2Playlist>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QqPlaylistList2Playlist {
    tid: i64,
    //dir_id: i64,
    name: String,
    songnum: Option<u32>,
    logo: String,
}

#[derive(Deserialize)]
pub(super) struct QqPlaylistDetailResp {
    req_0: QqPlaylistDetailRespReq,
}

impl QqPlaylistDetailResp {
    pub fn standardize(self) -> PlaylistDetail {
        let data = self.req_0.data;
        let info = data.dirinfo;
        let songlist = data.songlist;
        let mut track_ids = Vec::new();
        let tracks = songlist
            .into_iter()
            .map(|t| {
                track_ids.push(t.mid.clone());
                Track {
                    id: t.mid.clone(),
                    provider: ProviderId::Qq,
                    source_id: t.mid.clone(),
                    media_mid: Some(t.mid),
                    title: t.title,
                    artists: t.singer.into_iter().map(|s| s.name).collect(),
                    album: t.album.name,
                    cover_url: format!(
                        "https://y.gtimg.cn/music/photo_new/T002R300x300M000{}.jpg",
                        t.album.mid
                    ),
                    quality_hints: vec!["standard".to_owned()],
                    playable_state: PlayableState::Unknown,
                    duration_ms: Some(t.interval as u64 * 1000),
                    artwork_url: None,
                }
            })
            .collect();
        PlaylistDetail {
            provider: ProviderId::Qq,
            id: info.id.to_string(),
            name: info.title,
            cover_url: info.picurl,
            track_count: Some(info.songnum),
            track_ids,
            collected: None,
            has_more: None,
            tracks,
        }
    }
}

#[derive(Deserialize)]
struct QqPlaylistDetailRespReq {
    data: QqPlaylistDetailData,
}

#[derive(Deserialize)]
struct QqPlaylistDetailData {
    dirinfo: Dirinfo,

    songlist: Vec<Songlist>,
}

#[derive(Deserialize)]
struct Dirinfo {
    id: i64,

    title: String,

    picurl: String,

    songnum: u32,
}

#[derive(Deserialize)]
struct Songlist {
    mid: String,
    //name: String,
    title: String,
    //subtitle: String,
    interval: i64,

    singer: Vec<Identified>,

    album: Identified,
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
                provider: ProviderId::Qq,
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
                    provider: ProviderId::Qq,
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
            provider: ProviderId::Qq,
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
            has_more: None,
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

    singer: Vec<Identified>,

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
    singer_list: Vec<Identified>,
}

//Reusable Struct
#[derive(Debug, Deserialize)]
struct File {
    //HQ旧值
    size_320mp3: i64,

    size_ape: i64,
    //SQ无损
    size_flac: i64,

    size_128mp3: i64,
    //两个同时作为标准音质判断
    size_96ogg: i64,

    size_96aac: i64,
    //size_new[0]为臻品母带, [3]为HQ音效, [7]为NAC音效, 待其他迁移后检查音质具体标签
    //size_new: Vec<i64>,
}

impl File {
    fn standardize(self, id: Option<String>) -> Vec<TrackQualityOption> {
        let mut v: Vec<String> = Vec::new();
        if self.size_flac != 0 {
            v.push("flac".to_string());
        }
        if self.size_320mp3 != 0 {
            v.push("320".to_string());
        }
        if self.size_ape != 0 {
            v.push("ape".to_string());
        }
        if self.size_128mp3 != 0 {
            v.push("128".to_string());
        }
        if self.size_96aac != 0 || self.size_96ogg != 0 {
            v.push("aac".to_string());
        }
        v.into_iter()
            .map(|quality| TrackQualityOption {
                provider: ProviderId::Qq,
                label: qq_quality_label(&quality).to_owned(),
                id: id.clone().unwrap_or(quality.clone()),
                request_quality: quality.clone(),
                level: Some(quality.clone()),
                source: "declared".to_owned(),
                ..Default::default()
            })
            .collect()
    }
}

fn qq_quality_label(quality: &str) -> &'static str {
    match quality {
        "flac" => "FLAC",
        "ape" => "APE",
        "320" => "320k MP3",
        "128" => "128k MP3",
        "m4a" => "AAC",
        _ => "QQ",
    }
}

#[derive(Debug, Deserialize)]
struct Album {
    //id: i64,
    mid: String,
    name: String,
    songnum: Option<u32>,
    #[serde(alias = "v_singer")]
    singer: Vec<Identified>,
}

#[derive(Debug, Deserialize)]
pub struct Identified {
    pub mid: String,
    pub name: String,
}

// ── DoSearchForQQMusicDesktop 多类型搜索响应（参考 netease-qq-music-api）──

#[derive(Debug, Deserialize)]
pub(super) struct QqMultiSearchResp {
    result: QqMultiSearchResult,
}

#[derive(Debug, Deserialize)]
struct QqMultiSearchResult {
    data: QqMultiSearchData,
}

#[derive(Debug, Deserialize)]
struct QqMultiSearchData {
    body: QqMultiSearchBody,
    meta: QqMultiSearchMeta,
}

#[derive(Debug, Deserialize)]
struct QqMultiSearchBody {
    #[serde(default)]
    song: QqMultiSearchSongSection,
    #[serde(default)]
    album: QqMultiSearchAlbumSection,
    #[serde(default)]
    songlist: QqMultiSearchSonglistSection,
}

#[derive(Debug, Default, Deserialize)]
struct QqMultiSearchSongSection {
    #[serde(default)]
    list: Vec<QqMultiSearchSong>,
}

#[derive(Debug, Default, Deserialize)]
struct QqMultiSearchAlbumSection {
    #[serde(default)]
    list: Vec<QqMultiSearchAlbum>,
}

#[derive(Debug, Default, Deserialize)]
struct QqMultiSearchSonglistSection {
    #[serde(default)]
    list: Vec<QqMultiSearchSonglist>,
}

#[derive(Debug, Deserialize)]
struct QqMultiSearchMeta {
    #[serde(default)]
    nextpage: i32,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QqMultiSearchSong {
    mid: String,
    name: String,
    #[serde(default)]
    singer: Vec<QqMultiSearchSinger>,
    #[serde(default)]
    album: QqMultiSearchSongAlbum,
}

#[derive(Debug, Deserialize)]
struct QqMultiSearchSinger {
    mid: String,
    name: String,
}

#[derive(Debug, Default, Deserialize)]
struct QqMultiSearchSongAlbum {
    mid: String,
    name: String,
    #[serde(default)]
    pmid: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QqMultiSearchAlbum {
    #[serde(rename = "albumMID")]
    album_mid: String,
    album_name: String,
    #[serde(default)]
    album_pic: String,
    #[serde(rename = "singerMID", default)]
    singer_mid: String,
    #[serde(default)]
    singer_name: String,
}

#[derive(Debug, Deserialize)]
struct QqMultiSearchSonglist {
    dissid: String,
    dissname: String,
    #[serde(default)]
    imgurl: String,
}

impl QqMultiSearchResp {
    pub(super) fn standardize_albums(self) -> Vec<AlbumSummary> {
        self.result
            .data
            .body
            .album
            .list
            .into_iter()
            .map(|a| AlbumSummary {
                provider: ProviderId::Qq,
                id: a.album_mid,
                name: a.album_name,
                artists: if a.singer_name.is_empty() {
                    vec![]
                } else {
                    vec![a.singer_name]
                },
                cover_url: if a.album_pic.is_empty() {
                    format!("https://y.gtimg.cn/music/photo_new/T002R300x300M000{}.jpg", "")
                } else {
                    a.album_pic
                },
                track_count: None,
                track_ids: vec![],
                collected: None,
            })
            .collect()
    }

    pub(super) fn standardize_playlists(self) -> Vec<PlaylistSummary> {
        self.result
            .data
            .body
            .songlist
            .list
            .into_iter()
            .map(|s| PlaylistSummary {
                provider: ProviderId::Qq,
                id: s.dissid,
                name: s.dissname,
                cover_url: s.imgurl,
                track_count: None,
                track_ids: vec![],
                collected: None,
            })
            .collect()
    }

    pub(super) fn has_more(&self) -> bool {
        self.result.data.meta.nextpage > 0
    }
}
