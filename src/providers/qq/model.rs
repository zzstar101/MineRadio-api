use std::collections::HashMap;

use regex::Regex;
use serde::{Deserialize, de::IgnoredAny};

use crate::types::{
    AlbumDetail, AlbumSummary, PlayableState, PlaylistDetail, PlaylistSummary, ProviderId,
    ProviderLoginStatus, RecommendationCard, RecommendationCardKind, RecommendationModule,
    RecommendationModuleKind, RecommendationPage, SongUrlResult, Track, TrackQualityAvailability,
    TrackQualityOption, VipLevel,
};

#[derive(Deserialize)]
pub(super) struct QqSearchResp {
    data: QqSearchData,
}

#[derive(Deserialize)]
struct QqSearchData {
    song: QqSearchSongData,
}

#[derive(Deserialize)]
struct QqSearchSongData {
    //curnum: i32,
    //curpage: i32,
    list: Vec<QqSearchSong>,
    //totalnum: i32
}

#[derive(Deserialize)]
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

#[derive(Deserialize)]
pub(super) struct QqTrackDetailResp {
    req_0: QqTrackDetailReq,
}

impl QqTrackDetailResp {
    pub fn standardize(self) -> Option<TrackQualityAvailability> {
        let t = self.req_0.data.track_info;
        let qualities = t.file.standardize(Some(t.mid.clone()));
        (!qualities.is_empty()).then_some(TrackQualityAvailability {
            provider: ProviderId::Qq,
            track_id: t.mid,
            default_quality: qualities.first().map(|item| item.request_quality.clone()),
            qualities,
        })
    }
}
#[derive(Deserialize)]
struct QqTrackDetailReq {
    data: QqTrackDetailData,
}

#[derive(Deserialize)]
struct QqTrackDetailData {
    track_info: QqTrackDetailInfo,
}

#[derive(Deserialize)]
struct QqTrackDetailInfo {
    mid: String,
    file: File,
}

#[derive(Deserialize)]
pub(super) struct QqLyricResp {
    req_0: QqLyricReq,
}

impl QqLyricResp {
    pub fn standardize(self) -> (Option<String>, Option<String>) {
        let a = self.req_0.data;
        (a.lyric, a.trans)
    }
}

#[derive(Deserialize)]
struct QqLyricReq {
    data: QqLyricData,
}

#[derive(Deserialize)]
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
    pub fn liked_dirid(&self) -> Option<u64> {
        self.req_0
            .data
            .v_playlist
            .first()
            .and_then(|playlist| u64::try_from(playlist.dir_id).ok())
    }

    pub fn tid_to_dirid(&self) -> HashMap<u64, u64> {
        self.req_0
            .data
            .v_playlist
            .iter()
            .filter_map(|playlist| {
                Some((
                    u64::try_from(playlist.tid).ok()?,
                    u64::try_from(playlist.dir_id).ok()?,
                ))
            })
            .collect()
    }

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
        (!v.is_empty()).then_some(v)
    }
}

#[derive(Deserialize)]
pub(super) struct QqPlaylistSongWriteResp {
    req_0: QqPlaylistSongWriteReq,
}

impl QqPlaylistSongWriteResp {
    pub fn succeeded(&self) -> bool {
        self.req_0.data.result.update_time.is_some()
    }
}

#[derive(Deserialize)]
struct QqPlaylistSongWriteReq {
    data: QqPlaylistSongWriteData,
}

#[derive(Deserialize)]
struct QqPlaylistSongWriteData {
    result: QqPlaylistSongWriteResult,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QqPlaylistSongWriteResult {
    update_time: Option<IgnoredAny>,
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
    dir_id: i64,

    dir_name: String,

    tid: i64,

    song_num: Option<u32>,

    pic_url: String,
}

#[derive(Deserialize)]
pub(super) struct QqPlaylistList2Resp {
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
        (!v.is_empty()).then_some(v)
    }
}

#[derive(Deserialize)]
struct QqPlaylistList2Req {
    data: QqPlaylistList2Data,
}

#[derive(Deserialize)]
struct QqPlaylistList2Data {
    //number: i64,
    //hasmore: i64,
    v_list: Vec<QqPlaylistList2Playlist>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QqPlaylistList2Playlist {
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
                t.standardize(None, None)
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

    songlist: Vec<QqTrack>,
}

#[derive(Deserialize)]
struct Dirinfo {
    id: i64,

    title: String,

    picurl: String,

    songnum: u32,
}

#[derive(Deserialize)]
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

#[derive(Deserialize)]
struct QqAlbumListResponse {
    data: QqAlbumListData,
}

#[derive(Deserialize)]
struct QqAlbumListData {
    //number: i64,
    //hasmore: i64,
    #[serde(rename = "v_list")]
    albums: Vec<Album>,
    //total: i64,
}

#[derive(Deserialize)]
pub(super) struct QqAlbumDetailResp {
    #[serde(rename = "req_0")]
    song_list: QqAlbumDetailSongListResponse,

    #[serde(rename = "req_1")]
    info: QqAlbumDetailInfoResponse,
}

impl QqAlbumDetailResp {
    pub(super) fn standardize(self) -> AlbumDetail {
        let song_list = self.song_list.data;
        let mut track_ids = Vec::new();

        let info = self.info.data;
        let (album, artists) = (info.basic_info, info.singer);
        let default_album_mid = Some(song_list.album_mid.clone());
        let default_album_name = Some(album.album_name.clone());

        let tracks: Vec<Track> = song_list
            .song_list
            .into_iter()
            .map(|s| {
                let l = s.song_info;
                track_ids.push(l.mid.clone());
                l.standardize(default_album_mid.clone(), default_album_name.clone())
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

#[derive(Deserialize)]
struct QqAlbumDetailSongListResponse {
    data: QqAlbumDetailSongListData,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QqAlbumDetailSongListData {
    album_mid: String,

    total_num: i64,

    song_list: Vec<QqAlbumDetailSongListEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QqAlbumDetailSongListEntry {
    song_info: QqTrack,
}

#[derive(Deserialize)]
struct QqAlbumDetailInfoResponse {
    data: QqAlbumDetailInfoData,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QqAlbumDetailInfoData {
    basic_info: QqAlbumDetailInfo,
    singer: QqAlbumDetailArtists,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QqAlbumDetailInfo {
    album_mid: String,

    album_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QqAlbumDetailArtists {
    singer_list: Vec<Identified>,
}

#[derive(Deserialize)]
pub(super) struct QqMultiSearchResp {
    result: QqMultiSearchResult,
}

#[derive(Deserialize)]
struct QqMultiSearchResult {
    data: QqMultiSearchData,
}

#[derive(Deserialize)]
struct QqMultiSearchData {
    body: QqMultiSearchBody,
}

#[derive(Deserialize)]
struct QqMultiSearchBody {
    song: Option<QqMultiSearchSongSection>,
    album: Option<QqMultiSearchAlbumSection>,
    songlist: Option<QqMultiSearchSonglistSection>,
}

#[derive(Deserialize)]
struct QqMultiSearchSongSection {
    list: Vec<QqTrack>,
}

#[derive(Deserialize)]
struct QqMultiSearchAlbumSection {
    #[serde(default)]
    list: Vec<QqMultiSearchAlbum>,
}

#[derive(Deserialize)]
struct QqMultiSearchSonglistSection {
    #[serde(default)]
    list: Vec<QqMultiSearchSonglist>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QqMultiSearchAlbum {
    #[serde(rename = "albumMID")]
    album_mid: String,
    album_name: String,
    album_pic: Option<String>,
    singer_name: Option<String>,
}

#[derive(Deserialize)]
struct QqMultiSearchSonglist {
    dissid: String,
    dissname: String,
    imgurl: Option<String>,
}

impl QqMultiSearchResp {
    pub(super) fn standardize_albums(self) -> Option<Vec<AlbumSummary>> {
        let list = self.result.data.body.album?.list;
        let v: Vec<AlbumSummary> = list
            .into_iter()
            .map(|a| AlbumSummary {
                provider: ProviderId::Qq,
                id: a.album_mid,
                name: a.album_name,
                artists: a.singer_name.map_or(vec![], |n| vec![n]),
                cover_url: a.album_pic.unwrap_or_else(|| {
                    format!(
                        "https://y.gtimg.cn/music/photo_new/T002R300x300M000{}.jpg",
                        ""
                    )
                }),
                track_count: None,
                track_ids: vec![],
                collected: None,
            })
            .collect();
        (!v.is_empty()).then_some(v)
    }

    pub(super) fn standardize_playlists(self) -> Option<Vec<PlaylistSummary>> {
        let list = self.result.data.body.songlist?.list;
        let v: Vec<PlaylistSummary> = list
            .into_iter()
            .map(|s| PlaylistSummary {
                provider: ProviderId::Qq,
                id: s.dissid,
                name: s.dissname,
                cover_url: s.imgurl.unwrap_or_default(),
                track_count: None,
                track_ids: vec![],
                collected: None,
            })
            .collect();
        (!v.is_empty()).then_some(v)
    }

    pub(super) fn standardize_songs(self) -> Option<Vec<Track>> {
        let list = self.result.data.body.song?.list;
        let v: Vec<Track> = list
            .into_iter()
            .map(|track| track.standardize(None, None))
            .collect();
        (!v.is_empty()).then_some(v)
    }
}

#[derive(Deserialize)]
pub(super) struct QqLoginStatusResp {
    data: QqLoginStatusData,
}

impl QqLoginStatusResp {
    pub(super) fn standardize(
        self,
        vip_icon_response: Option<QqVipIconResp>,
    ) -> ProviderLoginStatus {
        let creator = self.data.creator;
        let user_id = creator.encrypt_uin.trim().to_owned();
        let mut status = ProviderLoginStatus {
            provider: ProviderId::Qq,
            logged_in: !user_id.is_empty(),
            ..Default::default()
        };

        status.nickname = (!creator.nick.trim().is_empty()).then_some(creator.nick);
        status.avatar_url = (!creator.headpic.is_empty()).then_some(creator.headpic);
        status.user_id = (!user_id.is_empty()).then_some(user_id.clone());
        let mut expired_vip_icon = None;
        let mut active_vip_icon = None;
        if let Some(vip_icon_response) = vip_icon_response {
            for icon in &vip_icon_response.get_vip_icon.data.user_info_ui.iconlist {
                match vip_badge_icon(&icon.src_url) {
                    Some(icon @ VipBadgeIcon::Active { .. }) => {
                        active_vip_icon = Some(icon);
                        break;
                    }
                    Some(icon @ VipBadgeIcon::Expired { .. }) => expired_vip_icon = Some(icon),
                    None => {}
                }
            }
        }

        apply_vip_icon_status(&mut status, active_vip_icon.or(expired_vip_icon));
        status
    }

    pub(super) fn encrypted_uin(&self) -> String {
        self.data.creator.encrypt_uin.clone()
    }
}

#[derive(Deserialize)]
struct QqLoginStatusData {
    creator: QqLoginProfile,
}

#[derive(Deserialize)]
struct QqLoginProfile {
    nick: String,
    headpic: String,
    encrypt_uin: String,
}

#[derive(Deserialize)]
struct QqVipIconUserInfo {
    iconlist: Vec<QqVipIconItem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QqVipIconItem {
    src_url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct QqVipIconResp {
    get_vip_icon: QqVipIconPayload,
}

#[derive(Deserialize)]
struct QqVipIconPayload {
    data: QqVipIconData,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct QqVipIconData {
    #[serde(rename = "UserInfoUI")]
    user_info_ui: QqVipIconUserInfo,
}

enum VipBadgeIcon {
    Active {
        url: String,
        level: VipLevel,
        tier: i64,
    },
    Expired {
        url: String,
    },
}

fn apply_vip_icon_status(status: &mut ProviderLoginStatus, badge_icon: Option<VipBadgeIcon>) {
    match badge_icon {
        Some(VipBadgeIcon::Active { url, level, tier }) => {
            status.vip_type = Some(if level == VipLevel::Svip { 11 } else { 1 });
            status.vip_level = Some(level.clone());
            status.is_vip = Some(true);
            status.is_svip = Some(level == VipLevel::Svip);
            status.vip_label = Some(
                match level {
                    VipLevel::Svip => "QQ SVIP",
                    VipLevel::Vip => "QQ VIP",
                    VipLevel::None => unreachable!(),
                }
                .to_owned(),
            );
            status.vip_icon = Some(
                match level {
                    VipLevel::Svip => "qq-super-vip",
                    VipLevel::Vip => "qq-green-vip",
                    VipLevel::None => unreachable!(),
                }
                .to_owned(),
            );
            status.vip_icon_url = Some(url);
            status.vip_tier = Some(tier);
            status.vip_level_name = Some(tier.to_string());
        }
        Some(VipBadgeIcon::Expired { url }) => status.vip_icon_url = Some(url),
        None => {}
    }
}

fn vip_badge_icon(value: &str) -> Option<VipBadgeIcon> {
    const VIP_ICON_PREFIX: &str = "https://y.qq.com/mediastyle/lv-icon/v14/2x/";
    let url = value.trim();
    let filename = url.strip_prefix(VIP_ICON_PREFIX)?;
    let pattern = Regex::new(r"^(?P<expired>d-)?(?P<kind>svip|vip)(?P<tier>\d+)\.png$").ok()?;
    let captures = pattern.captures(filename)?;
    if captures.name("expired").is_some() {
        return Some(VipBadgeIcon::Expired {
            url: url.to_owned(),
        });
    }
    let level = match captures.name("kind")?.as_str() {
        "svip" => VipLevel::Svip,
        "vip" => VipLevel::Vip,
        _ => return None,
    };
    let tier = captures.name("tier")?.as_str().parse().ok()?;
    Some(VipBadgeIcon::Active {
        url: url.to_owned(),
        level,
        tier,
    })
}

#[derive(Deserialize)]
pub(super) struct QqSongUrlResp {
    req_0: QqSongUrlReq,
}

impl QqSongUrlResp {
    pub(super) fn standardize(self, cdn: &str, en: bool) -> Option<SongUrlResult> {
        let data = self.req_0.data;
        if !data.msg.contains("fnameHitCache_200") {
            return None;
        }
        let (url, ekey) = data
            .midurlinfo
            .into_iter()
            .find(|_a| true)
            .map(|i| (i.purl, i.ekey))?;
        if url.trim().is_empty() {
            return None;
        }
        let url = if !ekey.trim().is_empty() && en {
            format!(
                "audio-proxy?url={}/{}&key={}&provider=qq",
                urlencoding::encode(cdn.trim_end_matches('/')),
                urlencoding::encode(url.trim_start_matches('/')),
                urlencoding::encode(&ekey)
            )
        } else {
            format!(
                "audio-proxy?url={}/{}&provider=qq",
                urlencoding::encode(cdn.trim_end_matches('/')),
                urlencoding::encode(url.trim_start_matches('/')),
            )
        };

        Some(SongUrlResult {
            url: url,
            proxied: en,
            provider: Some(ProviderId::Qq),
            trial: Some(false),
            expires_at: None,
            ..Default::default()
        })
    }
}

#[derive(Deserialize)]
struct QqSongUrlReq {
    data: QqSongUrlData,
}

#[derive(Deserialize)]
struct QqSongUrlData {
    msg: String,
    midurlinfo: Vec<QqSongUrlInfo>,
}

#[derive(Deserialize)]
struct QqSongUrlInfo {
    purl: String,
    ekey: String,
}

#[derive(Deserialize)]
pub(super) struct QqCdnTestResp {
    code: i64,
    modulecdn: Modulecdn,
}

pub(super) struct QqCdnDispatch {
    pub(super) sips: Vec<String>,
    pub(super) test_file: String,
}

impl QqCdnTestResp {
    pub(super) fn standardize(self) -> Option<QqCdnDispatch> {
        let data = self.modulecdn.data;
        if self.code != 0 || data.retcode != 0 {
            return None;
        }
        let sips = data
            .sip
            .into_iter()
            .map(|sip| format!("{}/", sip.trim_end_matches('/')))
            .filter(|sip| !sip.trim().is_empty())
            .collect::<Vec<_>>();
        (!sips.is_empty()).then_some(QqCdnDispatch {
            sips,
            test_file: data.testfilewifi,
        })
    }
}

#[derive(Deserialize)]
struct Modulecdn {
    data: CdnData,
}

#[derive(Deserialize)]
struct CdnData {
    retcode: i64,
    sip: Vec<String>,
    testfilewifi: String,
}

#[derive(Deserialize)]
pub(super) struct QqRecommendationResp {
    req_0: QqRecommendationReq,
}

impl QqRecommendationResp {
    pub(super) fn track_ids(&self) -> Vec<u32> {
        self.req_0
            .data
            .v_shelf
            .iter()
            .filter(|shelf| shelf.id == 207)
            .flat_map(|shelf| shelf.v_niche.iter())
            .flat_map(|niche| niche.v_card.iter())
            .filter_map(|card| card.id.parse::<u32>().ok())
            .collect()
    }

    pub(super) fn standardize(
        self,
        mid_by_id: Option<&HashMap<String, String>>,
    ) -> Option<RecommendationPage> {
        let list: Vec<RecommendationModule> = self
            .req_0
            .data
            .v_shelf
            .into_iter()
            .filter_map(|shelf| shelf.standardize(mid_by_id))
            .collect();
        if list.is_empty() {
            return None;
        }
        Some(RecommendationPage {
            provider: ProviderId::Qq,
            list,
        })
    }
}

#[derive(Deserialize)]
pub(super) struct QqRecommendationReq {
    pub(super) data: QqRecommendationData,
}

#[derive(Deserialize)]
pub(super) struct QqRecommendationData {
    pub(super) v_shelf: Vec<VShelf>,
}

#[derive(Deserialize)]
pub(super) struct VShelf {
    pub(super) id: u16,
    pub(super) title_content: String,
    pub(super) title_template: String,
    pub(super) v_niche: Vec<VNiche>,
}

impl VShelf {
    fn standardize(
        self,
        mid_by_id: Option<&HashMap<String, String>>,
    ) -> Option<RecommendationModule> {
        let (mid_by_id, kind) = match self.id {
            207 => (mid_by_id?, RecommendationModuleKind::Track),
            _ => (
                &HashMap::<String, String>::new(),
                match self.id {
                    271 | 205 => RecommendationModuleKind::Playlist,
                    272 | 301 => RecommendationModuleKind::Mixed,
                    _ => return None,
                },
            ),
        };

        let list: Vec<RecommendationCard> = self
            .v_niche
            .into_iter()
            .flat_map(|niche| niche.v_card.into_iter())
            .filter_map(|card| card.standardize(mid_by_id))
            .collect();
        if list.is_empty() {
            return None;
        }
        Some(RecommendationModule {
            title: self.title_template.replace("{String}", &self.title_content),
            list,
            kind,
        })
    }
}

#[derive(Deserialize)]
pub(super) struct VNiche {
    pub(super) v_card: Vec<VCard>,
}

#[derive(Deserialize)]
pub(super) struct VCard {
    pub(super) cover: String,
    pub(super) id: String,
    pub(super) subtitle: String,
    pub(super) title: String,
    #[serde(alias = "type")]
    t: u16,
}

impl VCard {
    fn standardize(self, mid_by_id: &HashMap<String, String>) -> Option<RecommendationCard> {
        let (id, kind) = match self.t {
            200 => (
                mid_by_id.get(&self.id)?.clone(),
                RecommendationCardKind::Track,
            ),
            500 => (self.id, RecommendationCardKind::Playlist),
            700 => (self.id, RecommendationCardKind::Stream),
            900 => (
                if self.title.contains("杜比") {
                    return None;
                } else {
                    22000.to_string()
                },
                RecommendationCardKind::Stream,
            ),
            _ => return None,
        };

        Some(RecommendationCard {
            id,
            title: self.title,
            subtitle: self.subtitle,
            cover_url: self.cover,
            collected: None,
            kind,
        })
    }
}

#[derive(Deserialize)]
pub(super) struct QqTrackInfo {
    //实际上有全功能但是这里的用途是转换id->mid
    req_0: QqTrackReq,
}

impl QqTrackInfo {
    pub(super) fn standardize(self) -> Option<HashMap<String, String>> {
        let mids = self
            .req_0
            .data
            .tracks
            .into_iter()
            .map(|track| (track.id.to_string(), track.mid))
            .collect::<HashMap<_, _>>();
        (!mids.is_empty()).then_some(mids)
    }
}

#[derive(Deserialize)]
struct QqTrackReq {
    data: QqTrackData,
}

#[derive(Deserialize)]
struct QqTrackData {
    tracks: Vec<QqTrackIdMid>,
}

#[derive(Deserialize)]
struct QqTrackIdMid {
    id: i64,
    mid: String,
}

#[derive(Deserialize)]
pub(super) struct QqRadioDetailResp {
    req_0: QqRadioDetailReq,
}

impl QqRadioDetailResp {
    pub(super) fn standardize(self) -> Option<Track> {
        self.req_0
            .data
            .tracks
            .into_iter()
            .next()
            .map(|t| t.standardize(None, None))
    }
}
#[derive(Deserialize)]
struct QqRadioDetailReq {
    data: QqRadioDetailData,
}

#[derive(Deserialize)]
struct QqRadioDetailData {
    tracks: Vec<QqTrack>,
}

#[derive(Deserialize)]
pub struct QqRadarResp {
    req_0: QqRadarReq,
}

impl QqRadarResp {
    pub(super) fn standardize(self) -> Option<Track> {
        self.req_0
            .data
            .vec_songs
            .into_iter()
            .next()
            .map(|t| t.track.standardize(None, None))
    }
}

#[derive(Deserialize)]
pub struct QqRadarReq {
    data: QqRadarData,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct QqRadarData {
    vec_songs: Vec<QqRadarSong>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct QqRadarSong {
    track: QqTrack,
}

//Reusable Struct
#[derive(Deserialize)]
struct QqTrack {
    mid: String,
    //name: String,
    title: String,
    //subtitle: String,
    interval: Option<u16>, //这里是s为单位

    singer: Vec<Identified>,

    album: Option<Identified>,

    pay: QqAlbumDetailTrackPay,
}

#[derive(Deserialize)]
struct QqAlbumDetailTrackPay {
    pay_play: i64,
}

impl QqTrack {
    fn standardize(
        self,
        default_album_mid: Option<String>,
        default_album_name: Option<String>,
    ) -> Track {
        let album_mid = self
            .album
            .as_ref()
            .map(|album| album.mid.clone())
            .or(default_album_mid)
            .unwrap_or_default();
        let album_name = self
            .album
            .as_ref()
            .map(|album| album.name.clone())
            .or(default_album_name)
            .unwrap_or_default();

        Track {
            id: self.mid.clone(),
            provider: ProviderId::Qq,
            source_id: self.mid.clone(),
            media_mid: Some(self.mid),
            title: self.title,
            artists: self.singer.into_iter().map(|s| s.name).collect(),
            album: album_name,
            cover_url: format!(
                "https://y.gtimg.cn/music/photo_new/T002R300x300M000{}.jpg",
                album_mid
            ),
            quality_hints: vec!["128k".to_owned()],
            playable_state: if self.pay.pay_play == 1 {
                PlayableState::PaidRequired
            } else {
                PlayableState::Playable
            },
            duration_ms: self.interval.map(|s| s as u64 * 1000),
            artwork_url: None,
        }
    }
}

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    size_320mp3: i64,
    #[serde(default)]
    size_flac: i64,
    #[serde(default)]
    size_128mp3: i64,
    #[serde(default)]
    size_new: Vec<i64>,
}

impl File {
    fn standardize(self, id: Option<String>) -> Vec<TrackQualityOption> {
        let mut v: Vec<String> = Vec::new();
        if size_new_at(&self.size_new, 8) {
            v.push("atmos".to_string());
        }
        if size_new_at(&self.size_new, 3) {
            v.push("premium".to_string());
        }
        if size_new_at(&self.size_new, 0) {
            v.push("master".to_string());
        }
        if self.size_flac != 0 {
            v.push("flac".to_string());
        }
        if self.size_320mp3 != 0 {
            v.push("320k".to_string());
        }
        if size_new_at(&self.size_new, 7) {
            v.push("nac".to_string());
        }
        if self.size_128mp3 != 0 {
            v.push("128k".to_string());
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

fn size_new_at(size_new: &[i64], index: usize) -> bool {
    size_new.get(index).is_some_and(|size| *size > 0)
}

fn qq_quality_label(quality: &str) -> &'static str {
    match quality {
        "atmos" => "臻品全景音",
        "premium" => "臻品音质",
        "master" => "臻品母带",
        "flac" => "SQ无损",
        "320k" => "HQ高品质",
        "nac" => "NAC品质",
        "128k" => "标准品质",
        _ => "QQ",
    }
}

#[derive(Deserialize)]
struct Album {
    //id: i64,
    mid: String,
    name: String,
    songnum: Option<u32>,
    #[serde(alias = "v_singer")]
    singer: Vec<Identified>,
}

#[derive(Deserialize)]
struct Identified {
    mid: String,
    name: String,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::{
        Identified, QqAlbumDetailTrackPay, QqLoginProfile, QqLoginStatusData, QqLoginStatusResp,
        QqPlaylistList1Resp, QqPlaylistSongWriteResp, QqRecommendationResp, QqTrack, QqTrackInfo,
    };
    use crate::types::RecommendationCardKind;

    #[test]
    fn track_uses_default_album_when_album_is_missing() {
        let track = QqTrack {
            mid: "track-mid".to_owned(),
            title: "Track".to_owned(),
            interval: None,
            singer: vec![],
            album: None,
            pay: QqAlbumDetailTrackPay { pay_play: 0 },
        }
        .standardize(
            Some("default-mid".to_owned()),
            Some("Default Album".to_owned()),
        );

        assert_eq!(track.album, "Default Album");
        assert_eq!(
            track.cover_url,
            "https://y.gtimg.cn/music/photo_new/T002R300x300M000default-mid.jpg"
        );

        let track = QqTrack {
            mid: "track-mid".to_owned(),
            title: "Track".to_owned(),
            interval: None,
            singer: vec![],
            album: Some(Identified {
                mid: "track-album-mid".to_owned(),
                name: "Track Album".to_owned(),
            }),
            pay: QqAlbumDetailTrackPay { pay_play: 0 },
        }
        .standardize(
            Some("default-mid".to_owned()),
            Some("Default Album".to_owned()),
        );

        assert_eq!(track.album, "Track Album");
        assert!(track.cover_url.ends_with("track-album-mid.jpg"));
    }

    #[test]
    fn login_status_survives_missing_vip_response() {
        let response = QqLoginStatusResp {
            data: QqLoginStatusData {
                creator: QqLoginProfile {
                    nick: "user".to_owned(),
                    headpic: "https://example.invalid/avatar.jpg".to_owned(),
                    encrypt_uin: "encrypted-uin".to_owned(),
                },
            },
        };

        let status = response.standardize(None);

        assert!(status.logged_in);
        assert_eq!(status.nickname.as_deref(), Some("user"));
        assert_eq!(status.is_vip, None);
    }

    #[test]
    fn created_playlists_preserve_tid_to_dirid_mapping() {
        let response: QqPlaylistList1Resp = serde_json::from_value(json!({
            "req_0": {
                "data": {
                    "v_playlist": [{
                        "dirId": 101,
                        "dirName": "Created playlist",
                        "tid": 202,
                        "songNum": 3,
                        "picUrl": "https://example.invalid/cover.jpg"
                    }]
                }
            }
        }))
        .expect("deserialize created QQ playlist");

        assert_eq!(response.tid_to_dirid().get(&202), Some(&101));
        assert_eq!(response.liked_dirid(), Some(101));
    }

    #[test]
    fn playlist_song_write_success_requires_update_time() {
        let response: QqPlaylistSongWriteResp = serde_json::from_value(json!({
            "req_0": {
                "code": 0,
                "data": {
                    "msg": "",
                    "result": {
                        "dirDesc": "",
                        "dirId": 0,
                        "dirName": "",
                        "dirPicUrl": "",
                        "songlist": [],
                        "tid": 0,
                        "updateTime": 0
                    },
                    "retCode": 0
                }
            }
        }))
        .expect("deserialize QQ playlist write response");

        assert!(response.succeeded());
    }

    #[test]
    fn recommendation_standardizes_known_shelves_and_replaces_track_ids_with_mids() {
        let response: QqRecommendationResp = serde_json::from_value(json!({
            "req_0": {
                "data": {
                    "v_shelf": [
                        { "id": 205, "title_content": "", "title_template": "歌单", "v_niche": [{ "v_card": [{ "cover": "playlist-cover", "id": "playlist-id", "subtitle": "", "title": "歌单", "type": 500 }] }] },
                        { "id": 207, "title_content": "", "title_template": "单曲", "v_niche": [{ "v_card": [{ "cover": "track-cover", "id": "123", "subtitle": "歌手", "title": "单曲", "type": 200 }, { "cover": "missing-cover", "id": "456", "subtitle": "", "title": "缺失 MID", "type": 200 }] }] },
                        { "id": 271, "title_content": "", "title_template": "双列歌单", "v_niche": [{ "v_card": [{ "cover": "double-playlist-cover", "id": "double-playlist-id", "subtitle": "", "title": "双列歌单", "type": 500 }] }] },
                        { "id": 272, "title_content": "", "title_template": "电台", "v_niche": [{ "v_card": [{ "cover": "radio-cover", "id": "radio-id", "subtitle": "", "title": "电台", "type": 400 }] }] },
                        { "id": 301, "title_content": "", "title_template": "个性电台", "v_niche": [{ "v_card": [{ "cover": "personal-radio-cover", "id": "personal-radio-id", "subtitle": "", "title": "个性电台", "type": 700 }] }] },
                        { "id": 206, "title_content": "", "title_template": "Banner", "v_niche": [{ "v_card": [{ "cover": "banner-cover", "id": "banner-id", "subtitle": "", "title": "Banner", "type": 900 }] }] }
                    ]
                }
            }
        }))
        .expect("deserialize recommendation response");

        assert_eq!(response.track_ids(), vec![123, 456]);

        let page = response.standardize(Some(&HashMap::from([(
            "123".to_owned(),
            "0039MnYb0qxYhV".to_owned(),
        )])));
        assert_eq!(page.is_some(), true);
        let page = page.unwrap();
        assert_eq!(page.list.len(), 4);
        assert_eq!(page.list[1].list.len(), 1);
        assert_eq!(page.list[1].list[0].id, "0039MnYb0qxYhV");
        assert!(matches!(
            page.list[1].list[0].kind,
            RecommendationCardKind::Track
        ));
        // 电台 shelf (id=272, type=400) 卡片全部被滤掉，整个模块被移除
        assert!(matches!(
            page.list[3].list[0].kind,
            RecommendationCardKind::Stream
        ));
    }

    #[test]
    fn track_info_standardizes_mids_by_numeric_id() {
        let response: QqTrackInfo = serde_json::from_value(json!({
            "req_0": {
                "data": {
                    "tracks": [
                        { "id": 456, "mid": "004mW2K50JTkls" },
                        { "id": 123, "mid": "0039MnYb0qxYhV" }
                    ]
                }
            }
        }))
        .expect("deserialize track info response");

        let mid_by_id = response.standardize().expect("non-empty track info");

        assert_eq!(mid_by_id.get("123"), Some(&"0039MnYb0qxYhV".to_owned()));
        assert_eq!(mid_by_id.get("456"), Some(&"004mW2K50JTkls".to_owned()));
    }
}
