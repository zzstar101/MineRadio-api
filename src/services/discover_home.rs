use std::{
    collections::HashMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use crate::{
    providers::ProviderAdapter,
    providers::netease::client::NeteaseClient,
    services::podcast::{PodcastPageParams, PodcastService},
    types::ProviderId,
};

pub type NeteaseResponse = Value;
pub type DiscoverRequestParams = HashMap<String, Value>;

const PROVIDER_ORDER: [ProviderId; 3] = [ProviderId::Netease, ProviderId::Qq, ProviderId::Soda];

#[async_trait]
pub trait DiscoverRequester: Send + Sync {
    async fn personalized(&self, params: DiscoverRequestParams) -> anyhow::Result<NeteaseResponse>;
    async fn dj_hot(&self, params: DiscoverRequestParams) -> anyhow::Result<NeteaseResponse>;
    async fn recommend_resource(
        &self,
        params: DiscoverRequestParams,
    ) -> anyhow::Result<NeteaseResponse>;
    async fn recommend_songs(
        &self,
        params: DiscoverRequestParams,
    ) -> anyhow::Result<NeteaseResponse>;
}

#[derive(Clone)]
pub struct DiscoverHomeServiceOptions {
    pub provider_adapters: HashMap<ProviderId, Arc<dyn ProviderAdapter>>,
    pub podcast: PodcastService,
    pub discover_requester: Option<Arc<dyn DiscoverRequester>>,
}

#[async_trait]
impl DiscoverRequester for NeteaseClient {
    async fn personalized(&self, params: DiscoverRequestParams) -> anyhow::Result<NeteaseResponse> {
        Ok(self
            .personalized(read_u32(&params, "limit").unwrap_or(8))
            .await?)
    }

    async fn dj_hot(&self, params: DiscoverRequestParams) -> anyhow::Result<NeteaseResponse> {
        Ok(self
            .dj_hot(
                read_u32(&params, "limit").unwrap_or(6),
                read_u32(&params, "offset").unwrap_or(0),
            )
            .await?)
    }

    async fn recommend_resource(
        &self,
        _params: DiscoverRequestParams,
    ) -> anyhow::Result<NeteaseResponse> {
        Ok(self.recommend_resource().await?)
    }

    async fn recommend_songs(
        &self,
        _params: DiscoverRequestParams,
    ) -> anyhow::Result<NeteaseResponse> {
        Ok(self.recommend_songs().await?)
    }
}

pub async fn build_discover_home(options: DiscoverHomeServiceOptions) -> anyhow::Result<Value> {
    let updated_at = now_millis();
    let mut statuses = Vec::new();
    for provider in PROVIDER_ORDER {
        statuses.push(safe_login_status(provider, options.provider_adapters.get(&provider)).await);
    }
    let logged = statuses.iter().find(|status| status.logged_in).cloned();

    let Some(logged) = logged else {
        return Ok(json!({
            "loggedIn": false,
            "user": Value::Null,
            "dailySongs": [],
            "playlists": [],
            "podcasts": [],
            "mode": "starter",
            "updatedAt": updated_at
        }));
    };

    let logged_providers = statuses
        .iter()
        .filter(|status| status.logged_in)
        .map(|status| status.provider.clone())
        .collect::<Vec<_>>();

    let netease_discover = if logged_providers
        .iter()
        .any(|provider| *provider == ProviderId::Netease)
    {
        load_netease_discover(options.discover_requester.as_ref()).await
    } else {
        DiscoverBundle::default()
    };

    let adapter_playlists = if !netease_discover.playlists.is_empty() {
        Vec::new()
    } else {
        load_adapter_playlists(&options.provider_adapters, &logged_providers).await
    };
    let playlists = if !netease_discover.playlists.is_empty() {
        netease_discover.playlists.clone()
    } else {
        adapter_playlists
    }
    .into_iter()
    .filter(|playlist| has_non_empty_key(playlist, "id") && has_non_empty_key(playlist, "name"))
    .take(10)
    .collect::<Vec<_>>();

    let daily_songs = if !netease_discover.daily_songs.is_empty() {
        netease_discover
            .daily_songs
            .iter()
            .take(12)
            .cloned()
            .collect()
    } else if let Some(tracks) = first_playlist_tracks(&options.provider_adapters, &playlists).await
    {
        tracks
    } else {
        first_search_tracks(&options.provider_adapters, &logged_providers).await
    };

    let podcasts = if !netease_discover.podcasts.is_empty() {
        netease_discover.podcasts.iter().take(6).cloned().collect()
    } else {
        load_podcast_fallback(&options.podcast).await
    };

    Ok(json!({
        "loggedIn": true,
        "user": {
            "provider": logged.provider,
            "userId": logged.user_id,
            "nickname": logged.nickname,
            "avatarUrl": logged.avatar_url
        },
        "dailySongs": daily_songs,
        "playlists": playlists,
        "podcasts": podcasts,
        "mode": "member",
        "updatedAt": updated_at
    }))
}

#[derive(Clone, Default)]
struct LoggedStatus {
    provider: ProviderId,
    logged_in: bool,
    user_id: String,
    nickname: String,
    avatar_url: String,
}

#[derive(Clone, Default)]
struct DiscoverBundle {
    daily_songs: Vec<Value>,
    playlists: Vec<Value>,
    podcasts: Vec<Value>,
}

async fn safe_login_status(
    provider: ProviderId,
    adapter: Option<&Arc<dyn ProviderAdapter>>,
) -> LoggedStatus {
    let Some(adapter) = adapter else {
        return LoggedStatus {
            provider,
            ..Default::default()
        };
    };
    match adapter.login_status().await {
        Ok(status) => LoggedStatus {
            provider,
            logged_in: status.logged_in,
            user_id: status.user_id.unwrap_or_default(),
            nickname: status.nickname.unwrap_or_default(),
            avatar_url: status.avatar_url.unwrap_or_default(),
        },
        Err(_) => LoggedStatus {
            provider,
            ..Default::default()
        },
    }
}

async fn load_netease_discover(requester: Option<&Arc<dyn DiscoverRequester>>) -> DiscoverBundle {
    let Some(requester) = requester else {
        return DiscoverBundle::default();
    };

    let personalized = requester
        .personalized(hashmap([("limit", Value::from(8))]))
        .await
        .ok();
    let dj_hot = requester
        .dj_hot(hashmap([
            ("limit", Value::from(6)),
            ("offset", Value::from(0)),
        ]))
        .await
        .ok();
    let recommend_resource = requester.recommend_resource(HashMap::new()).await.ok();
    let recommend_songs = requester.recommend_songs(HashMap::new()).await.ok();

    let public_playlists = personalized
        .as_ref()
        .map(body_of)
        .map(|body| array_of(body.get("result")))
        .unwrap_or_default()
        .iter()
        .map(map_discover_playlist)
        .filter(|playlist| has_non_empty_key(playlist, "id") && has_non_empty_key(playlist, "name"))
        .take(8)
        .collect::<Vec<_>>();

    let podcasts = dj_hot
        .as_ref()
        .map(body_of)
        .map(|body| {
            first_array_from(
                &Value::Object(body),
                &["djRadios", "djradios", "radios", "data"],
            )
        })
        .unwrap_or_default()
        .iter()
        .map(map_podcast_radio)
        .filter(|podcast| {
            has_non_empty_key(podcast, "id")
                && has_non_empty_key(podcast, "name")
                && !is_low_signal_podcast_item(podcast)
        })
        .take(6)
        .collect::<Vec<_>>();

    let private_playlists = recommend_resource
        .as_ref()
        .map(body_of)
        .map(|body| array_of(body.get("recommend").or_else(|| body.get("data"))))
        .unwrap_or_default()
        .iter()
        .map(map_discover_playlist)
        .filter(|playlist| has_non_empty_key(playlist, "id") && has_non_empty_key(playlist, "name"))
        .take(6)
        .collect::<Vec<_>>();

    let daily_songs = recommend_songs
        .as_ref()
        .map(body_of)
        .map(|body| {
            let data = body
                .get("data")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            array_of(
                data.get("dailySongs")
                    .or_else(|| data.get("recommend"))
                    .or_else(|| body.get("recommend")),
            )
        })
        .unwrap_or_default()
        .iter()
        .map(map_netease_song)
        .filter(|track| has_non_empty_key(track, "id") && has_non_empty_key(track, "title"))
        .take(12)
        .collect::<Vec<_>>();

    let mut playlists = private_playlists;
    playlists.extend(public_playlists);
    playlists.truncate(10);

    DiscoverBundle {
        daily_songs,
        playlists,
        podcasts,
    }
}

async fn load_adapter_playlists(
    adapters: &HashMap<ProviderId, Arc<dyn ProviderAdapter>>,
    providers: &[ProviderId],
) -> Vec<Value> {
    let mut out = Vec::new();
    for provider in providers {
        if let Some(adapter) = adapters.get(provider) {
            if let Ok(playlists) = adapter.playlist_list().await {
                for playlist in playlists {
                    out.push(json!({
                        "provider": provider,
                        "id": playlist.id,
                        "name": playlist.name,
                        "trackCount": playlist.track_count
                    }));
                }
            }
        }
    }
    out.truncate(10);
    out
}

async fn first_playlist_tracks(
    adapters: &HashMap<ProviderId, Arc<dyn ProviderAdapter>>,
    playlists: &[Value],
) -> Option<Vec<Value>> {
    for playlist in playlists {
        let provider_str = string_value(record(playlist).get("provider"));
        let id = string_value(record(playlist).get("id"));
        if provider_str.is_empty() || id.is_empty() {
            continue;
        }
        let Ok(provider) = provider_str.parse::<ProviderId>() else {
            continue;
        };
        let Some(adapter) = adapters.get(&provider) else {
            continue;
        };
        if let Ok(detail) = adapter.playlist_detail(&id).await {
            let tracks = detail
                .tracks
                .into_iter()
                .map(|track| serde_json::to_value(track).unwrap_or(Value::Null))
                .filter(|track| has_non_empty_key(track, "id") && has_non_empty_key(track, "title"))
                .take(12)
                .collect::<Vec<_>>();
            if !tracks.is_empty() {
                return Some(tracks);
            }
        }
    }
    None
}

async fn first_search_tracks(
    adapters: &HashMap<ProviderId, Arc<dyn ProviderAdapter>>,
    providers: &[ProviderId],
) -> Vec<Value> {
    for provider in providers {
        let Some(adapter) = adapters.get(provider) else {
            continue;
        };
        if let Ok(tracks) = adapter.search("每日推荐", 12).await {
            let tracks = tracks
                .into_iter()
                .map(|track| serde_json::to_value(track).unwrap_or(Value::Null))
                .filter(|track| has_non_empty_key(track, "id") && has_non_empty_key(track, "title"))
                .take(12)
                .collect::<Vec<_>>();
            if !tracks.is_empty() {
                return tracks;
            }
        }
    }
    Vec::new()
}

async fn load_podcast_fallback(podcast: &PodcastService) -> Vec<Value> {
    podcast
        .hot(PodcastPageParams {
            limit: 6,
            offset: 0,
        })
        .await
        .ok()
        .and_then(|value| record(&value).get("podcasts").cloned())
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|item| has_non_empty_key(item, "id") && has_non_empty_key(item, "name"))
        .take(6)
        .collect()
}

fn map_netease_song(raw: &Value) -> Value {
    let song = record(raw);
    let album = song
        .get("al")
        .or_else(|| song.get("album"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let artists = array_of(song.get("ar").or_else(|| song.get("artists")))
        .iter()
        .filter_map(|artist| value_string(record(artist).get("name")))
        .collect::<Vec<_>>();
    let fee = number_i64(song.get("fee")).unwrap_or(0);
    json!({
        "provider": "netease",
        "id": string_id(song.get("id")),
        "sourceId": string_id(song.get("id")),
        "title": string_value(song.get("name")),
        "artists": artists,
        "album": string_value(album.get("name")),
        "coverUrl": string_value(album.get("picUrl").or_else(|| album.get("coverUrl"))),
        "durationMs": number_u64(song.get("dt").or_else(|| song.get("duration"))),
        "qualityHints": ["standard"],
        "playableState": match fee {
            1 => "vip_required",
            4 => "paid_required",
            8 => "trial_only",
            _ => "unknown",
        }
    })
}

fn map_discover_playlist(raw: &Value) -> Value {
    let playlist = record(raw);
    let ui_element = playlist
        .get("uiElement")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let image = ui_element
        .get("image")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    json!({
        "provider": "netease",
        "id": string_id(
            playlist
                .get("id")
                .or_else(|| playlist.get("resourceId"))
                .or_else(|| playlist.get("creativeId"))
        ),
        "name": string_value(playlist.get("name").or_else(|| playlist.get("title"))),
        "coverUrl": string_value(
            playlist
                .get("picUrl")
                .or_else(|| playlist.get("coverImgUrl"))
                .or_else(|| playlist.get("coverUrl"))
                .or_else(|| image.get("imageUrl"))
        ),
        "trackCount": number_u64(
            playlist
                .get("trackCount")
                .or_else(|| playlist.get("songCount"))
                .or_else(|| playlist.get("programCount"))
        ),
        "trackIds": [],
        "collected": playlist.get("collected").and_then(Value::as_bool).unwrap_or(false)
    })
}

fn map_podcast_radio(raw: &Value) -> Value {
    let radio = record(raw);
    let dj = radio
        .get("dj")
        .or_else(|| radio.get("djSimple"))
        .or_else(|| radio.get("djUser"))
        .or_else(|| radio.get("creator"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let id = string_id(
        radio
            .get("id")
            .or_else(|| radio.get("rid"))
            .or_else(|| radio.get("radioId")),
    );
    json!({
        "id": id.clone(),
        "rid": id,
        "name": string_value(radio.get("name").or_else(|| radio.get("radioName"))),
        "coverUrl": string_value(
            radio
                .get("picUrl")
                .or_else(|| radio.get("picURL"))
                .or_else(|| radio.get("coverUrl"))
                .or_else(|| radio.get("coverImgUrl"))
                .or_else(|| radio.get("avatarUrl"))
        ),
        "description": string_value(
            radio
                .get("desc")
                .or_else(|| radio.get("description"))
                .or_else(|| radio.get("rcmdText"))
        ),
        "djName": string_value(dj.get("nickname").or_else(|| radio.get("djName")).or_else(|| radio.get("nickname"))),
        "category": string_value(radio.get("category").or_else(|| radio.get("categoryName"))),
        "programCount": number_i64(
            radio
                .get("programCount")
                .or_else(|| radio.get("programNum"))
                .or_else(|| radio.get("programCnt"))
        )
        .unwrap_or(0),
        "subCount": number_i64(
            radio
                .get("subCount")
                .or_else(|| radio.get("subedCount"))
                .or_else(|| radio.get("subscriberCount"))
        )
        .unwrap_or(0)
    })
}

fn is_low_signal_podcast_item(item: &Value) -> bool {
    let record = record(item);
    let text = format!(
        "{} {} {} {}",
        string_value(record.get("name")),
        string_value(record.get("djName")),
        string_value(record.get("category")),
        string_value(record.get("description"))
    )
    .to_lowercase();
    [
        "购买播客",
        "付费精品",
        "qzone",
        "空间背景音乐",
        "背景音乐",
        "四只烤翅",
        "试纸烤翅",
    ]
    .iter()
    .any(|pattern| text.contains(pattern))
}

fn body_of(response: &Value) -> Map<String, Value> {
    response
        .get("body")
        .and_then(Value::as_object)
        .cloned()
        .or_else(|| response.as_object().cloned())
        .unwrap_or_default()
}

fn first_array_from(value: &Value, keys: &[&str]) -> Vec<Value> {
    let source = record(value);
    for key in keys {
        if let Some(items) = source.get(*key).and_then(Value::as_array) {
            return items.clone();
        }
    }
    Vec::new()
}

fn array_of(value: Option<&Value>) -> Vec<Value> {
    value.and_then(Value::as_array).cloned().unwrap_or_default()
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
    let value = string_value(value);
    if value.is_empty() { None } else { Some(value) }
}

fn number_i64(value: Option<&Value>) -> Option<i64> {
    value
        .and_then(Value::as_i64)
        .or_else(|| value.and_then(Value::as_u64).map(|number| number as i64))
}

fn number_u64(value: Option<&Value>) -> Option<u64> {
    value.and_then(Value::as_u64).or_else(|| {
        value
            .and_then(Value::as_i64)
            .and_then(|number| u64::try_from(number).ok())
    })
}

fn has_non_empty_key(value: &Value, key: &str) -> bool {
    !string_value(record(value).get(key)).is_empty()
}

fn read_u32(params: &HashMap<String, Value>, key: &str) -> Option<u32> {
    params
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .or_else(|| {
            params
                .get(key)
                .and_then(Value::as_i64)
                .and_then(|value| u32::try_from(value).ok())
        })
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn hashmap<const N: usize>(pairs: [(&str, Value); N]) -> HashMap<String, Value> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        providers::{ProviderAdapter, ProviderResult, error::ProviderError},
        services::podcast::{
            PodcastLoginInfo, PodcastRequester, PodcastServiceDeps, create_podcast_service,
        },
        types::{
            LyricPayload, PlaylistAddSongAck, PlaylistDetail, PlaylistSummary, ProviderLoginStatus,
            SongLikeAck, SongLikeCheckAck, SongUrlOptions, SongUrlResult, Track,
            TrackQualityAvailability,
        },
    };

    #[derive(Clone, Default)]
    struct MockProviderAdapter {
        id: ProviderId,
        login_status: ProviderLoginStatus,
        playlists: Vec<PlaylistSummary>,
        playlist_detail: Option<PlaylistDetail>,
        search_tracks: Vec<Track>,
    }

    #[async_trait]
    impl ProviderAdapter for MockProviderAdapter {
        fn id(&self) -> ProviderId {
            self.id
        }

        async fn search(&self, _keyword: &str, _limit: u32) -> ProviderResult<Vec<Track>> {
            Ok(self.search_tracks.clone())
        }

        async fn song_url(
            &self,
            _track: &Track,
            _opts: Option<SongUrlOptions>,
        ) -> ProviderResult<SongUrlResult> {
            unimplemented!()
        }

        async fn track_qualities(
            &self,
            _track: &Track,
        ) -> ProviderResult<TrackQualityAvailability> {
            unimplemented!()
        }

        async fn lyric(&self, _track: &Track) -> ProviderResult<LyricPayload> {
            unimplemented!()
        }

        async fn playlist_list(&self) -> ProviderResult<Vec<PlaylistSummary>> {
            Ok(self.playlists.clone())
        }

        async fn playlist_detail(&self, _id: &str) -> ProviderResult<PlaylistDetail> {
            Ok(self.playlist_detail.clone().unwrap_or_default())
        }

        async fn login_status(&self) -> ProviderResult<ProviderLoginStatus> {
            Ok(self.login_status.clone())
        }

        async fn logout(&self) -> ProviderResult<()> {
            unimplemented!()
        }

        async fn like_song(&self, _id: &str, _liked: bool) -> ProviderResult<SongLikeAck> {
            Ok(SongLikeAck::default())
        }

        async fn check_song_likes(&self, _ids: &[String]) -> ProviderResult<SongLikeCheckAck> {
            Ok(SongLikeCheckAck::default())
        }

        async fn add_song_to_playlist(
            &self,
            _playlist_id: &str,
            _track_id: &str,
        ) -> ProviderResult<PlaylistAddSongAck> {
            Ok(PlaylistAddSongAck::default())
        }
    }

    #[derive(Clone, Default)]
    struct MockDiscoverRequester {
        personalized_body: Value,
        dj_hot_body: Value,
        recommend_resource_body: Value,
        recommend_songs_body: Value,
    }

    #[async_trait]
    impl DiscoverRequester for MockDiscoverRequester {
        async fn personalized(
            &self,
            _params: DiscoverRequestParams,
        ) -> anyhow::Result<NeteaseResponse> {
            Ok(json!({ "body": self.personalized_body }))
        }

        async fn dj_hot(&self, _params: DiscoverRequestParams) -> anyhow::Result<NeteaseResponse> {
            Ok(json!({ "body": self.dj_hot_body }))
        }

        async fn recommend_resource(
            &self,
            _params: DiscoverRequestParams,
        ) -> anyhow::Result<NeteaseResponse> {
            Ok(json!({ "body": self.recommend_resource_body }))
        }

        async fn recommend_songs(
            &self,
            _params: DiscoverRequestParams,
        ) -> anyhow::Result<NeteaseResponse> {
            Ok(json!({ "body": self.recommend_songs_body }))
        }
    }

    #[derive(Clone, Default)]
    struct MockPodcastRequester {
        dj_hot_body: Value,
    }

    #[async_trait]
    impl PodcastRequester for MockPodcastRequester {
        async fn cloudsearch(&self, _params: HashMap<String, Value>) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }

        async fn dj_hot(&self, _params: HashMap<String, Value>) -> anyhow::Result<Value> {
            Ok(self.dj_hot_body.clone())
        }

        async fn dj_detail(&self, _params: HashMap<String, Value>) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }

        async fn dj_program(&self, _params: HashMap<String, Value>) -> anyhow::Result<Value> {
            Ok(Value::Null)
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
            Ok(PodcastLoginInfo::default())
        }
    }

    fn make_track(provider: ProviderId, id: &str, title: &str) -> Track {
        Track {
            id: id.to_owned(),
            provider,
            source_id: id.to_owned(),
            title: title.to_owned(),
            artists: vec!["tester".to_owned()],
            ..Default::default()
        }
    }

    fn podcast_service(radios: Vec<Value>) -> PodcastService {
        create_podcast_service(PodcastServiceDeps {
            requester: Some(Arc::new(MockPodcastRequester {
                dj_hot_body: json!({
                    "djRadios": radios
                }),
            })),
            beatmap_analyzer: None,
        })
    }

    fn adapter_map(
        adapters: Vec<Arc<dyn ProviderAdapter>>,
    ) -> HashMap<ProviderId, Arc<dyn ProviderAdapter>> {
        adapters
            .into_iter()
            .map(|adapter| (adapter.id(), adapter))
            .collect()
    }

    #[tokio::test]
    async fn returns_starter_envelope_when_no_provider_logged_in() {
        let response = build_discover_home(DiscoverHomeServiceOptions {
            provider_adapters: adapter_map(vec![
                Arc::new(MockProviderAdapter {
                    id: ProviderId::Netease,
                    ..Default::default()
                }),
                Arc::new(MockProviderAdapter {
                    id: ProviderId::Qq,
                    ..Default::default()
                }),
            ]),
            podcast: podcast_service(Vec::new()),
            discover_requester: None,
        })
        .await
        .unwrap();

        assert_eq!(response["loggedIn"], false);
        assert_eq!(response["mode"], "starter");
        assert!(response["dailySongs"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn prefers_netease_recommendation_sources_when_available() {
        let response = build_discover_home(DiscoverHomeServiceOptions {
            provider_adapters: adapter_map(vec![Arc::new(MockProviderAdapter {
                id: ProviderId::Netease,
                login_status: ProviderLoginStatus {
                    provider: ProviderId::Netease,
                    logged_in: true,
                    nickname: Some("tester".to_owned()),
                    user_id: Some("42".to_owned()),
                    avatar_url: None,
                    ..Default::default()
                },
                ..Default::default()
            })]),
            podcast: podcast_service(Vec::new()),
            discover_requester: Some(Arc::new(MockDiscoverRequester {
                personalized_body: json!({
                    "result": [{
                        "id": 7001,
                        "name": "public playlist",
                        "picUrl": "https://img.example/public.jpg",
                        "trackCount": 24
                    }]
                }),
                dj_hot_body: json!({
                    "djRadios": [{
                        "id": 8001,
                        "name": "hot radio",
                        "picUrl": "https://img.example/radio.jpg",
                        "dj": { "nickname": "host" }
                    }]
                }),
                recommend_resource_body: json!({
                    "recommend": [{
                        "id": 7002,
                        "name": "private playlist",
                        "coverImgUrl": "https://img.example/private.jpg",
                        "trackCount": 8
                    }]
                }),
                recommend_songs_body: json!({
                    "data": {
                        "dailySongs": [{
                            "id": 9001,
                            "name": "daily song",
                            "ar": [{ "name": "Alice" }],
                            "al": { "name": "album", "picUrl": "https://img.example/song.jpg" },
                            "dt": 210000,
                            "fee": 0
                        }]
                    }
                }),
            })),
        })
        .await
        .unwrap();

        assert_eq!(response["user"]["userId"], "42");
        assert_eq!(response["dailySongs"][0]["title"], "daily song");
        assert_eq!(response["playlists"][0]["name"], "private playlist");
        assert_eq!(response["playlists"][1]["name"], "public playlist");
        assert_eq!(response["podcasts"][0]["name"], "hot radio");
    }

    #[tokio::test]
    async fn falls_back_to_playlist_detail_then_search_when_daily_songs_missing() {
        let response = build_discover_home(DiscoverHomeServiceOptions {
            provider_adapters: adapter_map(vec![Arc::new(MockProviderAdapter {
                id: ProviderId::Soda,
                login_status: ProviderLoginStatus {
                    provider: ProviderId::Soda,
                    logged_in: true,
                    nickname: Some("soda user".to_owned()),
                    user_id: Some("soda-42".to_owned()),
                    avatar_url: None,
                    ..Default::default()
                },
                playlists: vec![PlaylistSummary {
                    provider: ProviderId::Soda,
                    id: "playlist-1".to_owned(),
                    name: "empty playlist".to_owned(),
                    cover_url: String::new(),
                    track_count: Some(0),
                    track_ids: Vec::new(),
                    collected: Some(false),
                }],
                search_tracks: vec![make_track(ProviderId::Soda, "track-1", "search fallback")],
                ..Default::default()
            })]),
            podcast: podcast_service(Vec::new()),
            discover_requester: None,
        })
        .await
        .unwrap();

        assert_eq!(response["mode"], "member");
        assert_eq!(response["user"]["provider"], "soda");
        assert_eq!(response["dailySongs"][0]["title"], "search fallback");
    }

    #[test]
    fn read_u32_supports_signed_and_unsigned_values() {
        let params = HashMap::from([
            ("limit".to_owned(), Value::from(8_u64)),
            ("offset".to_owned(), Value::from(3_i64)),
        ]);
        assert_eq!(read_u32(&params, "limit"), Some(8));
        assert_eq!(read_u32(&params, "offset"), Some(3));
    }

    fn _ignore_provider_error(_: ProviderError) {}
}
