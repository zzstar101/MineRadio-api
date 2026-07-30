use serde::{Deserialize, Serialize};

use crate::types::{AlbumSummary, PlaylistSummary, ProviderId};

use super::client::KugouCookie;

#[derive(Clone, Debug, Default)]
pub(super) struct KugouAuth {
    pub user_id: String,
    pub token: String,
    pub mid: String,
    pub dfid: String,
    pub nickname: String,
    pub avatar_url: String,
    pub logged_in: bool,
}

impl KugouAuth {
    pub fn from_cookie(cookie: &KugouCookie) -> Self {
        let kugou = cookie
            .get("KuGoo")
            .or_else(|| cookie.get("kugou"))
            .or_else(|| cookie.get("Kugou"))
            .map(|value| parse_compound_cookie(value))
            .unwrap_or_default();
        let user_id = digits(first_value(
            cookie,
            &kugou,
            &["userid", "UserId", "KugooID", "kugouID", "uid"],
        ));
        let token = first_value(cookie, &kugou, &["token", "Token", "t", "T"]);
        let mid = first_value(
            cookie,
            &kugou,
            &["kg_mid", "KG_MID", "KUGOU_API_MID", "mid"],
        );
        let dfid = first_value(cookie, &kugou, &["kg_dfid", "KG_DFID", "dfid", "DFID"]);
        let nickname = first_value(
            cookie,
            &kugou,
            &["NickName", "nickname", "UserName", "username"],
        );
        let avatar_url = first_value(cookie, &kugou, &["Pic", "pic", "avatar"]);
        let logged_in = (!user_id.is_empty() && user_id != "0")
            || cookie.contains_key("KuGoo")
            || cookie.contains_key("kugou")
            || cookie.contains_key("Kugou");
        Self {
            user_id,
            token,
            mid,
            dfid,
            nickname,
            avatar_url,
            logged_in,
        }
    }

    pub fn playback_ready(&self) -> bool {
        !self.user_id.is_empty() && self.user_id != "0" && !self.token.is_empty()
    }
}

fn first_value(cookie: &KugouCookie, compound: &KugouCookie, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| cookie.get(*key).or_else(|| compound.get(*key)))
        .map(|value| value.trim().to_owned())
        .unwrap_or_default()
}

fn parse_compound_cookie(value: &str) -> KugouCookie {
    let decoded = urlencoding::decode(value).unwrap_or_else(|_| value.into());
    decoded
        .split('&')
        .filter_map(|item| {
            let (key, value) = item.split_once('=')?;
            (!key.trim().is_empty()).then(|| (key.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

fn digits(value: String) -> String {
    value.chars().filter(char::is_ascii_digit).collect()
}

#[derive(Serialize)]
pub(super) struct KugouPlaylistListRequest<'a> {
    pub userid: u64,
    pub token: &'a str,
    pub total_ver: u32,
    pub r#type: u8,
    pub page: u32,
    pub pagesize: u32,
}

#[derive(Serialize)]
pub(super) struct KugouPlaylistTracksRequest<'a> {
    pub listid: u64,
    pub userid: u64,
    pub area_code: u8,
    pub show_relate_goods: u8,
    pub pagesize: u32,
    pub allplatform: u8,
    pub show_cover: u8,
    pub r#type: u8,
    pub token: &'a str,
    pub page: u32,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct KugouSongResource<'a> {
    pub number: u8,
    pub name: &'a str,
    pub hash: &'a str,
    pub size: u8,
    pub sort: u8,
    pub timelen: u64,
    pub bitrate: u8,
    pub album_id: u64,
    pub mixsongid: u64,
}

#[derive(Serialize)]
pub(super) struct KugouAddSongRequest<'a> {
    pub userid: u64,
    pub token: &'a str,
    pub listid: u64,
    pub list_ver: u8,
    pub r#type: u8,
    pub slow_upload: u8,
    pub scene: &'static str,
    pub data: Vec<KugouSongResource<'a>>,
}

#[derive(Serialize)]
pub(super) struct KugouDeleteSongRequest<'a> {
    pub listid: u64,
    pub userid: u64,
    pub token: &'a str,
    pub r#type: u8,
    pub list_ver: u8,
    pub data: Vec<KugouDeleteSongResource>,
}

#[derive(Serialize)]
pub(super) struct KugouDeleteSongResource {
    pub fileid: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KugouLyricResp {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub contenttype: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KugouLyricSearchResp {
    #[serde(default)]
    candidates: Vec<KugouLyricCandidate>,
    #[serde(default)]
    data: Option<KugouLyricSearchData>,
}

#[derive(Deserialize)]
struct KugouLyricSearchData {
    #[serde(default)]
    candidates: Vec<KugouLyricCandidate>,
}

impl KugouLyricSearchResp {
    pub(super) fn first_candidate(&self) -> Option<&KugouLyricCandidate> {
        self.candidates
            .iter()
            .chain(self.data.iter().flat_map(|d| d.candidates.iter()))
            .find(|c| !c.id.is_empty() && !c.access_key.is_empty())
    }
}

#[derive(Deserialize)]
pub(super) struct KugouLyricCandidate {
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "accesskey")]
    pub access_key: String,
}

#[derive(Deserialize)]
pub(super) struct KugouCollectionResp {
    data: KugouCollectionData,
}

impl KugouCollectionResp {
    pub(super) fn standardize_playlists(self) -> Option<Vec<PlaylistSummary>> {
        let v: Vec<PlaylistSummary> = self
            .data
            .info
            .into_iter()
            .filter_map(|item| {
                if item.list_ver == 2 {
                    return None;
                }
                Some(PlaylistSummary {
                    provider: ProviderId::Kugou,
                    id: item.global_collection_id,
                    name: item.name,
                    cover_url: item.pic,
                    track_count: item.m_count,
                    track_ids: Vec::new(),
                    collected: Some(true),
                })
            })
            .collect();
        (!v.is_empty()).then_some(v)
    }

    pub(super) fn standardize_albums(self) -> Option<Vec<AlbumSummary>> {
        let v: Vec<AlbumSummary> = self
            .data
            .info
            .into_iter()
            .filter_map(|item| {
                if item.list_ver != 2 {
                    return None;
                }
                Some(AlbumSummary {
                    provider: ProviderId::Kugou,
                    id: item.global_collection_id,
                    name: item.name,
                    artists: item.authors?.into_iter().map(|a| a.author_name).collect(),
                    cover_url: item.pic,
                    track_count: item.m_count,
                    track_ids: Vec::new(),
                    collected: Some(true),
                })
            })
            .collect();
        (!v.is_empty()).then_some(v)
    }
}

#[derive(Deserialize)]
struct KugouCollectionData {
    info: Vec<KugouCollectionInfo>,
}

#[derive(Deserialize)]
struct KugouCollectionInfo {
    list_ver: i64,
    m_count: Option<u32>,
    global_collection_id: String,
    pic: String,
    //listid: i64,
    name: String,
    authors: Option<Vec<Author>>,
}

#[derive(Deserialize)]
struct Author {
    author_name: String,
    //author_id: i64,
}
