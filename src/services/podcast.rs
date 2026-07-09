use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{providers::netease::client::NeteaseClient, services::auth_session};

pub type NeteaseResponse = Value;

const PODCAST_SEARCH_TYPE: i64 = 1009;

#[async_trait]
pub trait PodcastRequester: Send + Sync {
    async fn cloudsearch(&self, params: HashMap<String, Value>) -> anyhow::Result<NeteaseResponse>;
    async fn dj_hot(&self, params: HashMap<String, Value>) -> anyhow::Result<NeteaseResponse>;
    async fn dj_detail(&self, params: HashMap<String, Value>) -> anyhow::Result<NeteaseResponse>;
    async fn dj_program(&self, params: HashMap<String, Value>) -> anyhow::Result<NeteaseResponse>;
    async fn dj_sublist(&self, params: HashMap<String, Value>) -> anyhow::Result<NeteaseResponse>;
    async fn user_audio(&self, params: HashMap<String, Value>) -> anyhow::Result<NeteaseResponse>;
    async fn dj_paygift(&self, params: HashMap<String, Value>) -> anyhow::Result<NeteaseResponse>;
    async fn record_recent_voice(
        &self,
        params: HashMap<String, Value>,
    ) -> anyhow::Result<NeteaseResponse>;
    async fn login_status(&self) -> anyhow::Result<PodcastLoginInfo>;
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodcastLoginInfo {
    pub logged_in: bool,
    pub user_id: Option<Value>,
}

#[derive(Clone, Default)]
pub struct PodcastServiceDeps {
    pub requester: Option<Arc<dyn PodcastRequester>>,
}

#[derive(Clone, Default)]
pub struct PodcastService {
    deps: PodcastServiceDeps,
}

impl PodcastService {
    pub async fn search(&self, params: PodcastSearchParams) -> anyhow::Result<Value> {
        let keywords = params.keywords.trim().to_owned();
        if keywords.is_empty() {
            return Ok(json!({ "podcasts": [], "total": 0 }));
        }

        let response = self
            .requester()?
            .cloudsearch(hashmap([
                ("keywords", Value::String(keywords)),
                ("type", Value::from(PODCAST_SEARCH_TYPE)),
                ("limit", Value::from(clamp_u32(params.limit, 6, 30, 18))),
            ]))
            .await?;
        let result = body_of(&response).get("result").cloned().unwrap_or(Value::Null);
        let raw = first_array_from(&result, &["djRadios", "djradios", "radios"]);

        let result_record = record(&result);
        Ok(json!({
            "podcasts": raw
                .iter()
                .map(map_podcast_radio)
                .filter(|item| has_non_empty_key(item, "id"))
                .collect::<Vec<_>>(),
            "total": number_i64(
                result_record
                    .get("djRadiosCount")
                    .or_else(|| result_record.get("djradiosCount"))
            )
            .unwrap_or(raw.len() as i64)
        }))
    }

    pub async fn hot(&self, params: PodcastPageParams) -> anyhow::Result<Value> {
        let response = self
            .requester()?
            .dj_hot(with_auth_params(hashmap([
                ("limit", Value::from(clamp_u32(params.limit, 6, 30, 18))),
                ("offset", Value::from(clamp_u32(params.offset, 0, u32::MAX, 0))),
            ])))
            .await?;
        let body = body_of(&response);
        let raw = first_array_from(
            &Value::Object(body.clone()),
            &["djRadios", "djradios", "radios", "data"],
        );

        Ok(json!({
            "podcasts": raw
                .iter()
                .map(map_podcast_radio)
                .filter(|item| has_non_empty_key(item, "id"))
                .collect::<Vec<_>>(),
            "more": body.get("hasMore").and_then(Value::as_bool).unwrap_or(false)
        }))
    }

    pub async fn detail(&self, params: PodcastDetailParams) -> anyhow::Result<Value> {
        let rid = params.rid.trim().to_owned();
        if rid.is_empty() {
            anyhow::bail!("Missing podcast id");
        }

        let response = self
            .requester()?
            .dj_detail(with_auth_params(hashmap([("rid", Value::String(rid))])))
            .await?;
        let body = body_of(&response);
        let podcast = body
            .get("data")
            .or_else(|| body.get("djRadio"))
            .or_else(|| body.get("radio"))
            .cloned()
            .unwrap_or_else(|| Value::Object(body));

        Ok(json!({
            "podcast": map_podcast_radio(&podcast)
        }))
    }

    pub async fn programs(&self, params: PodcastProgramsParams) -> anyhow::Result<Value> {
        let rid = params.rid.trim().to_owned();
        if rid.is_empty() {
            anyhow::bail!("Missing podcast id");
        }

        let response = self
            .requester()?
            .dj_program(with_auth_params(hashmap([
                ("rid", Value::String(rid.clone())),
                ("limit", Value::from(clamp_u32(params.limit, 10, 60, 30))),
                ("offset", Value::from(clamp_u32(params.offset, 0, u32::MAX, 0))),
                ("asc", Value::Bool(false)),
            ])))
            .await?;
        let body = body_of(&response);
        let raw = first_array_from(&Value::Object(body.clone()), &["programs"]);
        let data = body.get("data").cloned().unwrap_or(Value::Null);
        let data_raw = if raw.is_empty() {
            first_array_from(&data, &["list", "programs"])
        } else {
            raw
        };

        let radio = data_raw.first().map(record).and_then(|item| item.get("radio").cloned());
        let radio = radio
            .as_ref()
            .map(map_podcast_radio)
            .unwrap_or_else(|| {
                json!({
                    "id": rid,
                    "rid": rid,
                    "name": ""
                })
            });

        Ok(json!({
            "radio": radio.clone(),
            "programs": data_raw
                .iter()
                .map(|item| map_podcast_program(item, Some(&radio)))
                .filter(|item| has_non_empty_key(item, "id") && has_non_empty_key(item, "title"))
                .collect::<Vec<_>>(),
            "more": body.get("more").and_then(Value::as_bool).unwrap_or(false),
            "total": number_i64(body.get("count")).unwrap_or(data_raw.len() as i64)
        }))
    }

    pub async fn my(&self) -> anyhow::Result<Value> {
        let info = self.login_status().await?;
        let keys = ["collect", "created", "liked"];
        if !info.logged_in || info.user_id.is_none() {
            return Ok(json!({
                "loggedIn": false,
                "collections": keys
                    .iter()
                    .map(|key| podcast_collection_meta(key, &[]))
                    .collect::<Vec<_>>()
            }));
        }

        let mut collections = Vec::new();
        for key in keys {
            let collection = match self.fetch_my_podcast_items(key, &info, 12, 0).await {
                Ok(data) => podcast_collection_meta(key, &data.items),
                Err(_) => podcast_collection_meta(key, &[]),
            };
            collections.push(collection);
        }

        Ok(json!({
            "loggedIn": true,
            "collections": collections
        }))
    }

    pub async fn my_items(&self, params: PodcastMyItemsParams) -> anyhow::Result<Value> {
        let info = self.login_status().await?;
        let key = if params.key.trim().is_empty() {
            "collect".to_owned()
        } else {
            params.key.trim().to_owned()
        };
        if !info.logged_in || info.user_id.is_none() {
            let mut out = podcast_collection_meta(&key, &[]);
            if let Value::Object(ref mut map) = out {
                map.insert("loggedIn".to_owned(), Value::Bool(false));
                map.insert("items".to_owned(), Value::Array(Vec::new()));
            }
            return Ok(out);
        }

        let data = self
            .fetch_my_podcast_items(
                &key,
                &info,
                clamp_u32(params.limit, 8, 60, 36),
                clamp_u32(params.offset, 0, u32::MAX, 0),
            )
            .await?;

        let mut out = podcast_collection_meta(&key, &data.items);
        if let Value::Object(ref mut map) = out {
            map.insert("loggedIn".to_owned(), Value::Bool(true));
            map.insert("itemType".to_owned(), Value::String(data.item_type.to_owned()));
            map.insert("items".to_owned(), Value::Array(data.items));
        }
        Ok(out)
    }

    pub async fn dj_beatmap(&self, params: PodcastBeatmapParams) -> anyhow::Result<Value> {
        if !params.url.starts_with("http://") && !params.url.starts_with("https://") {
            anyhow::bail!("Invalid audio url");
        }
        // TODO: wire Rust-side podcast analyzer after the external analyzer contract is finalized.
        anyhow::bail!("podcast analyzer unavailable")
    }

    pub fn deps(&self) -> &PodcastServiceDeps {
        &self.deps
    }

    fn requester(&self) -> anyhow::Result<&Arc<dyn PodcastRequester>> {
        self.deps
            .requester
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("podcast requester missing"))
    }

    async fn login_status(&self) -> anyhow::Result<PodcastLoginInfo> {
        if auth_session::get_provider_cookie("netease").await.is_none() {
            return Ok(PodcastLoginInfo {
                logged_in: false,
                user_id: None,
            });
        }
        self.requester()?.login_status().await
    }

    async fn fetch_my_podcast_items(
        &self,
        key: &str,
        info: &PodcastLoginInfo,
        limit: u32,
        offset: u32,
    ) -> anyhow::Result<MyPodcastItems> {
        let requester = self.requester()?;
        if key == "collect" {
            let response = requester
                .dj_sublist(with_auth_params(hashmap([
                    ("limit", Value::from(limit)),
                    ("offset", Value::from(offset)),
                ])))
                .await?;
            let raw = first_array_from(
                &Value::Object(body_of(&response)),
                &["djRadios", "djradios", "radios", "data"],
            );
            return Ok(MyPodcastItems {
                item_type: "radio",
                items: raw
                    .iter()
                    .map(map_podcast_radio)
                    .filter(|item| has_non_empty_key(item, "id"))
                    .collect(),
            });
        }
        if key == "created" {
            let response = requester
                .user_audio(with_auth_params(hashmap([(
                    "uid",
                    info.user_id.clone().unwrap_or(Value::Null),
                )])))
                .await?;
            let raw =
                first_array_from(&Value::Object(body_of(&response)), &["data", "djRadios", "djradios", "radios"]);
            return Ok(MyPodcastItems {
                item_type: "radio",
                items: raw
                    .iter()
                    .map(map_podcast_radio)
                    .filter(|item| has_non_empty_key(item, "id"))
                    .collect(),
            });
        }
        if key == "paid" {
            let response = requester
                .dj_paygift(with_auth_params(hashmap([
                    ("limit", Value::from(limit)),
                    ("offset", Value::from(offset)),
                ])))
                .await?;
            let raw =
                first_array_from(&Value::Object(body_of(&response)), &["data", "djRadios", "djradios", "radios"]);
            return Ok(MyPodcastItems {
                item_type: "radio",
                items: raw
                    .iter()
                    .map(map_podcast_radio)
                    .filter(|item| has_non_empty_key(item, "id"))
                    .collect(),
            });
        }
        if key == "liked" {
            let response = requester
                .record_recent_voice(with_auth_params(hashmap([("limit", Value::from(limit))])))
                .await?;
            let raw = first_array_from(&Value::Object(body_of(&response)), &["data", "list", "resources"]);
            return Ok(MyPodcastItems {
                item_type: "voice",
                items: raw
                    .iter()
                    .map(map_podcast_voice)
                    .filter(|item| has_non_empty_key(item, "id") && has_non_empty_key(item, "title"))
                    .collect(),
            });
        }

        Ok(MyPodcastItems {
            item_type: "radio",
            items: Vec::new(),
        })
    }
}

pub fn create_podcast_service(deps: PodcastServiceDeps) -> PodcastService {
    PodcastService { deps }
}

pub fn create_podcast_service_with_client(client: Arc<NeteaseClient>) -> PodcastService {
    create_podcast_service(PodcastServiceDeps {
        requester: Some(Arc::new(NeteasePodcastRequester { client })),
    })
}

#[derive(Clone, Debug)]
pub struct PodcastSearchParams {
    pub keywords: String,
    pub limit: u32,
}

#[derive(Clone, Debug)]
pub struct PodcastPageParams {
    pub limit: u32,
    pub offset: u32,
}

#[derive(Clone, Debug)]
pub struct PodcastDetailParams {
    pub rid: String,
}

#[derive(Clone, Debug)]
pub struct PodcastProgramsParams {
    pub rid: String,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Clone, Debug)]
pub struct PodcastMyItemsParams {
    pub key: String,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodcastBeatmapParams {
    pub url: String,
    pub duration_sec: u32,
    pub intro_sec: Option<u32>,
}

struct MyPodcastItems {
    item_type: &'static str,
    items: Vec<Value>,
}

struct NeteasePodcastRequester {
    client: Arc<NeteaseClient>,
}

#[async_trait]
impl PodcastRequester for NeteasePodcastRequester {
    async fn cloudsearch(&self, params: HashMap<String, Value>) -> anyhow::Result<NeteaseResponse> {
        Ok(self
            .client
            .cloudsearch(
                &string_value(params.get("keywords")),
                number_u32(params.get("limit")).unwrap_or(18),
            )
            .await?)
    }

    async fn dj_hot(&self, params: HashMap<String, Value>) -> anyhow::Result<NeteaseResponse> {
        Ok(self
            .client
            .dj_hot(
                number_u32(params.get("limit")).unwrap_or(18),
                number_u32(params.get("offset")).unwrap_or(0),
            )
            .await?)
    }

    async fn dj_detail(&self, params: HashMap<String, Value>) -> anyhow::Result<NeteaseResponse> {
        Ok(self
            .client
            .dj_detail(&string_value(params.get("rid")))
            .await?)
    }

    async fn dj_program(&self, params: HashMap<String, Value>) -> anyhow::Result<NeteaseResponse> {
        Ok(self
            .client
            .dj_program(
                &string_value(params.get("rid")),
                number_u32(params.get("limit")).unwrap_or(30),
                number_u32(params.get("offset")).unwrap_or(0),
                params.get("asc").and_then(Value::as_bool).unwrap_or(false),
            )
            .await?)
    }

    async fn dj_sublist(&self, params: HashMap<String, Value>) -> anyhow::Result<NeteaseResponse> {
        Ok(self
            .client
            .dj_sublist(
                number_u32(params.get("limit")).unwrap_or(30),
                number_u32(params.get("offset")).unwrap_or(0),
            )
            .await?)
    }

    async fn user_audio(&self, params: HashMap<String, Value>) -> anyhow::Result<NeteaseResponse> {
        Ok(self
            .client
            .user_audio(&string_value(params.get("uid")))
            .await?)
    }

    async fn dj_paygift(
        &self,
        params: HashMap<String, Value>,
    ) -> anyhow::Result<NeteaseResponse> {
        Ok(self
            .client
            .dj_paygift(
                number_u32(params.get("limit")).unwrap_or(30),
                number_u32(params.get("offset")).unwrap_or(0),
            )
            .await?)
    }

    async fn record_recent_voice(
        &self,
        params: HashMap<String, Value>,
    ) -> anyhow::Result<NeteaseResponse> {
        Ok(self
            .client
            .record_recent_voice(number_u32(params.get("limit")).unwrap_or(100))
            .await?)
    }

    async fn login_status(&self) -> anyhow::Result<PodcastLoginInfo> {
        let body = self.client.login_status().await?;
        let profile = body
            .get("profile")
            .or_else(|| body.get("data").and_then(|data| data.get("profile")))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(PodcastLoginInfo {
            logged_in: !profile.is_null(),
            user_id: record(&profile).get("userId").cloned(),
        })
    }
}

pub fn map_podcast_radio(raw: &Value) -> Value {
    let source = record(raw);
    let dj = source
        .get("dj")
        .or_else(|| source.get("djSimple"))
        .or_else(|| source.get("djUser"))
        .or_else(|| source.get("creator"))
        .map(record)
        .unwrap_or_default();
    let id = string_id(
        source
            .get("id")
            .or_else(|| source.get("rid"))
            .or_else(|| source.get("radioId")),
    );
    json!({
        "id": id.clone(),
        "rid": id,
        "name": string_value(source.get("name").or_else(|| source.get("radioName"))),
        "coverUrl": string_value(
            source
                .get("picUrl")
                .or_else(|| source.get("picURL"))
                .or_else(|| source.get("coverUrl"))
                .or_else(|| source.get("coverImgUrl"))
                .or_else(|| source.get("avatarUrl"))
        ),
        "description": string_value(
            source
                .get("desc")
                .or_else(|| source.get("description"))
                .or_else(|| source.get("rcmdText"))
        ),
        "djName": string_value(
            dj.get("nickname")
                .or_else(|| source.get("djName"))
                .or_else(|| source.get("nickname"))
        ),
        "category": string_value(source.get("category").or_else(|| source.get("categoryName"))),
        "programCount": number_i64(
            source
                .get("programCount")
                .or_else(|| source.get("programNum"))
                .or_else(|| source.get("programCnt"))
        )
        .unwrap_or(0),
        "subCount": number_i64(
            source
                .get("subCount")
                .or_else(|| source.get("subedCount"))
                .or_else(|| source.get("subscriberCount"))
        )
        .unwrap_or(0)
    })
}

pub fn map_podcast_program(raw: &Value, fallback_radio: Option<&Value>) -> Value {
    let source = record(raw);
    let main_song = source
        .get("mainSong")
        .or_else(|| source.get("song"))
        .or_else(|| source.get("mainTrack"))
        .map(record)
        .unwrap_or_default();
    let radio_value = source
        .get("radio")
        .cloned()
        .or_else(|| fallback_radio.cloned())
        .unwrap_or(Value::Null);
    let mapped_radio = safe_map_podcast_radio(&radio_value, fallback_radio);
    let artists = raw_artists(main_song.get("ar").or_else(|| main_song.get("artists")));
    let album = main_song
        .get("al")
        .or_else(|| main_song.get("album"))
        .map(record)
        .unwrap_or_default();
    let radio_record = record(&radio_value);
    let dj = source
        .get("dj")
        .or_else(|| radio_record.get("dj"))
        .map(record)
        .unwrap_or_default();
    let playable_id = string_id(
        main_song
            .get("id")
            .or_else(|| source.get("mainSongId"))
            .or_else(|| source.get("songId")),
    );
    json!({
        "type": "podcast",
        "provider": "netease",
        "id": playable_id.clone(),
        "sourceId": playable_id,
        "title": string_value(source.get("name").or_else(|| main_song.get("name"))),
        "artists": if artists.is_empty() {
            vec![first_non_empty(&[
                value_string(record(&mapped_radio).get("name")),
                value_string(dj.get("nickname")),
                value_string(record(&mapped_radio).get("djName")),
                Some("Podcast".to_owned()),
            ])]
        } else {
            artists
        },
        "album": first_non_empty(&[
            value_string(record(&mapped_radio).get("name")),
            value_string(album.get("name")),
            Some("Podcast".to_owned())
        ]),
        "coverUrl": first_non_empty(&[
            value_string(source.get("coverUrl")),
            value_string(source.get("cover")),
            value_string(source.get("blurCoverUrl")),
            value_string(record(&mapped_radio).get("coverUrl")),
            value_string(album.get("picUrl"))
        ]),
        "durationMs": number_u64(
            source
                .get("duration")
                .or_else(|| main_song.get("dt"))
                .or_else(|| main_song.get("duration"))
        ),
        "qualityHints": ["standard"],
        "playableState": "unknown",
        "programId": string_id(source.get("id").or_else(|| source.get("programId"))),
        "radioId": value_string(record(&mapped_radio).get("id")).unwrap_or_default(),
        "radioName": value_string(record(&mapped_radio).get("name")).unwrap_or_default(),
        "djName": first_non_empty(&[
            value_string(record(&mapped_radio).get("djName")),
            value_string(dj.get("nickname"))
        ]),
        "description": string_value(source.get("description").or_else(|| source.get("desc"))),
        "createTime": number_i64(source.get("createTime")).unwrap_or(0),
        "serialNum": number_i64(source.get("serialNum").or_else(|| source.get("serial"))).unwrap_or(0)
    })
}

pub fn podcast_collection_meta(key: &str, items: &[Value]) -> Value {
    let (meta_key, title, sub, item_type) = match key {
        "collect" => ("collect", "收藏播客", "你收藏的播客", "radio"),
        "created" => ("created", "创建播客", "你创建的播客", "radio"),
        "liked" => ("liked", "喜欢的声音", "收藏或最近喜欢的声音", "voice"),
        other => (other, other, "", "radio"),
    };
    let first = items.first().map(record).unwrap_or_default();
    json!({
        "key": meta_key,
        "title": title,
        "sub": sub,
        "itemType": item_type,
        "count": items.len(),
        "coverUrl": first_non_empty(&[
            value_string(first.get("coverUrl")),
            value_string(first.get("cover")),
            value_string(first.get("picUrl"))
        ])
    })
}

fn map_podcast_voice(raw: &Value) -> Value {
    let raw_record = record(raw);
    let source = raw_record
        .get("resource")
        .or_else(|| raw_record.get("voice"))
        .or_else(|| raw_record.get("data"))
        .or_else(|| raw_record.get("program"))
        .cloned()
        .unwrap_or_else(|| raw.clone());
    let source_record = record(&source);
    let main_song = source_record
        .get("mainSong")
        .or_else(|| source_record.get("song"))
        .or_else(|| source_record.get("track"))
        .cloned()
        .unwrap_or(Value::Null);
    let main_song_record = record(&main_song);
    let radio = source_record
        .get("radio")
        .or_else(|| source_record.get("djRadio"))
        .or_else(|| source_record.get("voiceList"))
        .or_else(|| source_record.get("podcast"))
        .cloned()
        .unwrap_or(Value::Null);

    let mut merged = source_record.clone();
    merged.insert(
        "id".to_owned(),
        source_record
            .get("programId")
            .or_else(|| source_record.get("voiceId"))
            .or_else(|| source_record.get("id"))
            .cloned()
            .unwrap_or(Value::Null),
    );
    merged.insert(
        "name".to_owned(),
        source_record
            .get("name")
            .or_else(|| source_record.get("songName"))
            .or_else(|| source_record.get("title"))
            .or_else(|| main_song_record.get("name"))
            .cloned()
            .unwrap_or(Value::Null),
    );

    let mut merged_main_song = record(&main_song).clone();
    merged_main_song.insert(
        "id".to_owned(),
        source_record
            .get("trackId")
            .or_else(|| source_record.get("songId"))
            .or_else(|| source_record.get("mainSongId"))
            .or_else(|| main_song_record.get("id"))
            .or_else(|| source_record.get("id"))
            .cloned()
            .unwrap_or(Value::Null),
    );
    merged.insert("mainSong".to_owned(), Value::Object(merged_main_song));

    let mut merged_radio = record(&radio).clone();
    merged_radio.insert(
        "name".to_owned(),
        merged_radio
            .get("name")
            .or_else(|| merged_radio.get("radioName"))
            .or_else(|| merged_radio.get("voiceListName"))
            .or_else(|| source_record.get("podcastName"))
            .or_else(|| source_record.get("djName"))
            .cloned()
            .unwrap_or(Value::Null),
    );
    merged.insert("radio".to_owned(), Value::Object(merged_radio));

    map_podcast_program(&Value::Object(merged), None)
}

fn safe_map_podcast_radio(raw: &Value, fallback: Option<&Value>) -> Value {
    let mapped = map_podcast_radio(raw);
    if has_non_empty_key(&mapped, "id") {
        return mapped;
    }
    let fallback_record = fallback.map(record).unwrap_or_default();
    let raw_record = record(raw);
    let id = first_non_empty(&[
        value_string(fallback_record.get("id")),
        value_string(fallback_record.get("rid")),
        value_string(raw_record.get("id")),
        value_string(raw_record.get("rid")),
    ]);
    json!({
        "id": if id.is_empty() { "unknown" } else { &id },
        "rid": if id.is_empty() { "unknown" } else { &id },
        "name": first_non_empty(&[value_string(fallback_record.get("name"))]),
        "coverUrl": first_non_empty(&[value_string(fallback_record.get("coverUrl"))]),
        "description": first_non_empty(&[value_string(fallback_record.get("description"))]),
        "djName": first_non_empty(&[value_string(fallback_record.get("djName"))]),
        "category": first_non_empty(&[value_string(fallback_record.get("category"))]),
        "programCount": number_i64(fallback_record.get("programCount")).unwrap_or(0),
        "subCount": number_i64(fallback_record.get("subCount")).unwrap_or(0)
    })
}

fn first_array_from(value: &Value, keys: &[&str]) -> Vec<Value> {
    let source = record(value);
    for key in keys {
        if let Some(items) = source.get(*key).and_then(Value::as_array) {
            return items.clone();
        }
        if let Some(nested) = source.get(*key) {
            let nested_record = record(nested);
            for nested_key in ["list", "data", "resources"] {
                if let Some(items) = nested_record.get(nested_key).and_then(Value::as_array) {
                    return items.clone();
                }
            }
        }
    }
    Vec::new()
}

fn body_of(response: &Value) -> Map<String, Value> {
    response
        .get("body")
        .and_then(Value::as_object)
        .cloned()
        .or_else(|| response.as_object().cloned())
        .unwrap_or_default()
}

fn with_auth_params(params: HashMap<String, Value>) -> HashMap<String, Value> {
    params
}

fn hashmap<const N: usize>(pairs: [(&str, Value); N]) -> HashMap<String, Value> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn raw_artists(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items.iter()
                .filter_map(|item| value_string(record(item).get("name")))
                .collect()
        })
        .unwrap_or_default()
}

fn record(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn string_id(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        Some(other) if !other.is_null() => other.to_string(),
        _ => String::new(),
    }
}

fn string_value(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(boolean)) => boolean.to_string(),
        Some(other) if !other.is_null() => other.to_string(),
        _ => String::new(),
    }
}

fn value_string(value: Option<&Value>) -> Option<String> {
    let text = string_value(value);
    if text.is_empty() { None } else { Some(text) }
}

fn number_i64(value: Option<&Value>) -> Option<i64> {
    value
        .and_then(Value::as_i64)
        .or_else(|| value.and_then(Value::as_u64).map(|number| number as i64))
}

fn number_u32(value: Option<&Value>) -> Option<u32> {
    number_i64(value).and_then(|number| u32::try_from(number).ok())
}

fn number_u64(value: Option<&Value>) -> Option<u64> {
    value
        .and_then(Value::as_u64)
        .or_else(|| value.and_then(Value::as_i64).and_then(|number| u64::try_from(number).ok()))
}

fn clamp_u32(value: u32, min: u32, max: u32, fallback: u32) -> u32 {
    if value == 0 && fallback != 0 {
        return fallback;
    }
    value.clamp(min, max)
}

fn first_non_empty(values: &[Option<String>]) -> String {
    values.iter().flatten().find(|value| !value.is_empty()).cloned().unwrap_or_default()
}

fn has_non_empty_key(value: &Value, key: &str) -> bool {
    !string_value(record(value).get(key)).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Default)]
    struct MockRequester {
        cloudsearch: Option<Value>,
        dj_program: Option<Value>,
        login_status: Option<PodcastLoginInfo>,
    }

    #[async_trait]
    impl PodcastRequester for MockRequester {
        async fn cloudsearch(&self, _params: HashMap<String, Value>) -> anyhow::Result<Value> {
            Ok(self.cloudsearch.clone().unwrap_or(Value::Null))
        }
        async fn dj_hot(&self, _params: HashMap<String, Value>) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn dj_detail(&self, _params: HashMap<String, Value>) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn dj_program(&self, _params: HashMap<String, Value>) -> anyhow::Result<Value> {
            Ok(self.dj_program.clone().unwrap_or(Value::Null))
        }
        async fn dj_sublist(&self, _params: HashMap<String, Value>) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn user_audio(&self, _params: HashMap<String, Value>) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn dj_paygift(&self, _params: HashMap<String, Value>) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn record_recent_voice(
            &self,
            _params: HashMap<String, Value>,
        ) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn login_status(&self) -> anyhow::Result<PodcastLoginInfo> {
            Ok(self.login_status.clone().unwrap_or_default())
        }
    }

    #[test]
    fn map_podcast_radio_preserves_metadata_fallbacks() {
        let radio = map_podcast_radio(&json!({
            "rid": 42,
            "radioName": "夜听",
            "picUrl": "cover",
            "dj": { "nickname": "DJ" },
            "categoryName": "情感",
            "programNum": 7,
            "subedCount": 9
        }));

        assert_eq!(radio["id"], "42");
        assert_eq!(radio["name"], "夜听");
        assert_eq!(radio["coverUrl"], "cover");
        assert_eq!(radio["djName"], "DJ");
        assert_eq!(radio["programCount"], 7);
    }

    #[test]
    fn map_podcast_program_maps_main_song_to_playable_track() {
        let program = map_podcast_program(&json!({
            "id": "p1",
            "name": "第 1 期",
            "radio": { "id": "r1", "name": "电台", "picUrl": "r-cover" },
            "mainSong": {
                "id": 100,
                "name": "音频",
                "ar": [{ "name": "主播" }],
                "al": { "name": "专辑", "picUrl": "song-cover" },
                "dt": 120000
            }
        }), None);

        assert_eq!(program["type"], "podcast");
        assert_eq!(program["id"], "100");
        assert_eq!(program["programId"], "p1");
        assert_eq!(program["title"], "第 1 期");
        assert_eq!(program["radioName"], "电台");
    }

    #[tokio::test]
    async fn podcast_service_search_maps_radios() {
        let service = create_podcast_service(PodcastServiceDeps {
            requester: Some(Arc::new(MockRequester {
                cloudsearch: Some(json!({
                    "body": {
                        "result": {
                            "djRadios": [{ "id": 1, "name": "播客" }],
                            "djRadiosCount": 1
                        }
                    }
                })),
                ..Default::default()
            })),
        });

        let result = service
            .search(PodcastSearchParams {
                keywords: "故事".to_owned(),
                limit: 18,
            })
            .await
            .unwrap();

        assert_eq!(result["podcasts"][0]["name"], "播客");
        assert_eq!(result["total"], 1);
    }

    #[tokio::test]
    async fn podcast_service_programs_maps_radio_and_programs() {
        let service = create_podcast_service(PodcastServiceDeps {
            requester: Some(Arc::new(MockRequester {
                dj_program: Some(json!({
                    "body": {
                        "programs": [{
                            "id": "p1",
                            "name": "节目",
                            "radio": { "id": "r1", "name": "电台" },
                            "mainSong": { "id": "s1", "name": "音频", "ar": [], "al": {}, "dt": 1000 }
                        }],
                        "more": true,
                        "count": 1
                    }
                })),
                ..Default::default()
            })),
        });

        let result = service
            .programs(PodcastProgramsParams {
                rid: "r1".to_owned(),
                limit: 30,
                offset: 0,
            })
            .await
            .unwrap();

        assert_eq!(result["radio"]["id"], "r1");
        assert_eq!(result["programs"][0]["id"], "s1");
        assert_eq!(result["more"], true);
    }

    #[tokio::test]
    async fn podcast_service_returns_logged_out_baseline_collections() {
        let service = create_podcast_service(PodcastServiceDeps {
            requester: Some(Arc::new(MockRequester {
                login_status: Some(PodcastLoginInfo {
                    logged_in: false,
                    user_id: None,
                }),
                ..Default::default()
            })),
        });

        let result = service.my().await.unwrap();

        assert_eq!(result["loggedIn"], false);
        assert_eq!(result["collections"][0]["key"], "collect");
        assert_eq!(result["collections"][1]["key"], "created");
        assert_eq!(result["collections"][2]["key"], "liked");
    }
}
