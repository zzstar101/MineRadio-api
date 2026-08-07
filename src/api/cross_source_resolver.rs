use std::{
    cmp::Ordering,
    collections::HashMap,
    sync::{Arc, LazyLock},
};

use futures::future::join_all;
use regex::Regex;

use crate::{
    providers::{
        ProviderAdapter,
        error::{ProviderError, ProviderErrorCode},
    },
    types::{PlayableState, ProviderId, RecommendationPage, SongUrlOptions, SongUrlResult, Track},
};

pub type ProviderMap = HashMap<ProviderId, Arc<dyn ProviderAdapter>>;

pub const PROVIDER_IDS: [ProviderId; 5] = [
    ProviderId::Netease,
    ProviderId::Qq,
    ProviderId::Soda,
    ProviderId::Kugou,
    ProviderId::Spotify,
];

static BRACKETED_TEXT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[(（\[【].*?[)）\]】]").expect("valid bracket regex"));
static SEARCH_SEPARATOR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"[\s·、，。!！?？“”‘’|\-_/]+"#).expect("valid separator regex"));
static DERIVATIVE_QUERY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(live|现场|翻唱|cover|伴奏|instrumental|remix|dj|片段|demo|女声|男声|karaoke)")
        .expect("valid derivative query regex")
});
static JAY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"周杰伦|周杰倫|jay\s*chou").expect("valid jay regex"));
static JAY_CASE_INSENSITIVE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)周杰伦|周杰倫|jay\s*chou").expect("valid jay regex"));
static LIVE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(live|现场)").expect("valid live regex"));
static QQ_SEARCH_INTENT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(^|\s)qq($|\s)|qq音乐|qq音樂|周杰伦|周杰倫|jay\s*chou|jay")
        .expect("valid qq search intent regex")
});
static DERIVATIVE_RESULT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(翻唱|cover|伴奏|instrumental|remix|片段|demo|女声|男声|karaoke|完整版\s*cover|抖音版|dj版|合唱版|改编版|赵露思版|超燃|硬曲|剪辑|二创|tribute|made\s*famous\s*by)",
    )
    .expect("valid derivative result regex")
});

pub struct CrossSourceResolverDeps {
    pub providers: ProviderMap,
}

pub struct CrossSourceResolver {
    deps: CrossSourceResolverDeps,
}

impl CrossSourceResolver {
    pub async fn resolve_recommendation_page(&self) -> anyhow::Result<Vec<RecommendationPage>> {
        let requests = PROVIDER_IDS.into_iter().filter_map(|provider_id| {
            let provider = self.provider(&provider_id)?;
            Some(async move { provider.recommendation_page().await })
        });

        Ok(join_all(requests)
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect())
    }

    pub async fn resolve_search(
        &self,
        keyword: &str,
        provider: Option<ProviderId>,
        limit: u32,
    ) -> anyhow::Result<Vec<Track>> {
        let Some(provider_id) = provider else {
            return self.resolve_merged_search(keyword, limit).await;
        };
        let provider = self
            .provider(&provider_id)
            .ok_or_else(|| no_result_error(provider_id.clone(), "provider unavailable"))?;
        let tracks = provider.search_track(keyword, 0, limit).await?;

        if tracks.is_empty() {
            return Err(no_result_error(provider_id, "no matching tracks found"));
        }
        Ok(tracks)
    }

    pub async fn resolve_song_url(
        &self,
        track: Track,
        opts: Option<SongUrlOptions>,
    ) -> anyhow::Result<SongUrlResult> {
        let opts = opts.unwrap_or_default();
        let import_only = is_import_only_track(&track);
        let attempts = self.ordered_providers(if import_only {
            None
        } else {
            Some(track.provider)
        });
        let mut first_error: Option<anyhow::Error> = None;

        for provider_id in attempts {
            let Some(adapter) = self.provider(&provider_id) else {
                continue;
            };

            if !import_only && provider_id == track.provider {
                match adapter.song_url(&track, Some(opts.clone())).await {
                    Ok(result) => return Ok(result),
                    Err(err) => {
                        if first_error.is_none() {
                            first_error = Some(err.into());
                        }
                    }
                }
                continue;
            }

            let keyword = build_switch_keyword(&track);
            match adapter.search_track(&keyword, 0, 5).await {
                Ok(candidates) => {
                    for candidate in candidates {
                        match adapter.song_url(&candidate, Some(opts.clone())).await {
                            Ok(result) => return Ok(result),
                            Err(err) => {
                                if first_error.is_none() {
                                    first_error = Some(err.into());
                                }
                            }
                        }
                    }
                }
                Err(err) => {
                    if first_error.is_none() {
                        first_error = Some(err.into());
                    }
                }
            }
        }

        if let Some(err) = first_error {
            return Err(err);
        }
        Err(no_url_error(track.provider, "no playable song URL found"))
    }

    async fn resolve_merged_search(&self, keyword: &str, limit: u32) -> anyhow::Result<Vec<Track>> {
        let provider_count = self.deps.providers.len() as u32;
        if provider_count == 0 {
            return Err(no_result_error(
                ProviderId::Netease,
                "no providers registered",
            ));
        }
        let provider_limit = limit.div_ceil(provider_count).max(1);
        let mut ranked = Vec::new();

        let searches =
            self.deps
                .providers
                .iter()
                .enumerate()
                .map(|(provider_index, (_, adapter))| {
                    let keyword = keyword.to_owned();
                    async move {
                        let result = adapter.search_track(&keyword, 0, provider_limit).await;
                        (provider_index, result)
                    }
                });

        let search_results = join_all(searches).await;

        for (provider_index, result) in search_results {
            match result {
                Ok(tracks) => {
                    ranked.extend(tracks.into_iter().enumerate().map(|(source_index, track)| {
                        RankedTrack {
                            score: score_search_track(&track, keyword, source_index),
                            track,
                            provider_index,
                            source_index,
                        }
                    }));
                }
                Err(_) => continue,
            }
        }

        let mut seen = std::collections::HashSet::new();
        ranked.retain(|entry| {
            let track = &entry.track;
            let fallback = format!("{}|{}", track.title, track.artists.join("/"));
            let id = if !track.id.is_empty() {
                track.id.as_str()
            } else if !track.source_id.is_empty() {
                track.source_id.as_str()
            } else {
                fallback.as_str()
            };
            seen.insert(format!("{}:{id}", track.provider))
        });
        ranked.sort_by(|a, b| compare_ranked_tracks(a, b));

        let merged = ranked
            .into_iter()
            .take(limit as usize)
            .map(|entry| entry.track)
            .collect::<Vec<_>>();
        if !merged.is_empty() {
            return Ok(merged);
        }
        Err(no_result_error(
            self.deps
                .providers
                .keys()
                .next()
                .cloned()
                .unwrap_or_else(|| ProviderId::Netease),
            "no matching tracks found",
        ))
    }

    fn ordered_providers(&self, preferred: Option<ProviderId>) -> Vec<ProviderId> {
        let Some(preferred) = preferred else {
            return PROVIDER_IDS.to_vec();
        };
        std::iter::once(preferred)
            .chain(
                PROVIDER_IDS
                    .into_iter()
                    .filter(|provider_id| *provider_id != preferred),
            )
            .collect()
    }

    fn provider(&self, provider_id: &ProviderId) -> Option<Arc<dyn ProviderAdapter>> {
        self.deps.providers.get(provider_id).cloned()
    }
}

pub fn create_cross_source_resolver(deps: CrossSourceResolverDeps) -> CrossSourceResolver {
    CrossSourceResolver { deps }
}

struct RankedTrack {
    track: Track,
    provider_index: usize,
    source_index: usize,
    score: f64,
}

fn compare_ranked_tracks(a: &RankedTrack, b: &RankedTrack) -> Ordering {
    b.score
        .partial_cmp(&a.score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| a.provider_index.cmp(&b.provider_index))
        .then_with(|| a.source_index.cmp(&b.source_index))
}

fn is_import_only_track(track: &Track) -> bool {
    starts_with_import(&track.id) || starts_with_import(&track.source_id)
}

fn starts_with_import(value: &str) -> bool {
    value
        .get(..7)
        .map(|prefix| prefix.eq_ignore_ascii_case("import:"))
        .unwrap_or(false)
}

fn build_switch_keyword(track: &Track) -> String {
    std::iter::once(track.title.as_str())
        .chain(track.artists.iter().map(String::as_str))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_search_text(value: &str) -> String {
    let lower = value.to_lowercase();
    let without_brackets = BRACKETED_TEXT_RE.replace_all(&lower, "");
    SEARCH_SEPARATOR_RE
        .replace_all(&without_brackets, "")
        .to_string()
}

fn score_search_track(track: &Track, keyword: &str, source_index: usize) -> f64 {
    let q = normalize_search_text(keyword);
    let title = normalize_search_text(&track.title);
    let artists = normalize_search_text(&track.artists.join(""));
    let album = normalize_search_text(&track.album);
    let raw = format!(
        "{} {} {}",
        track.title,
        track.artists.join(" "),
        track.album
    )
    .to_lowercase();
    let asks_derivative = DERIVATIVE_QUERY_RE.is_match(keyword);
    let derivative = search_looks_like_derivative(&raw);
    let artist_mentioned = search_mentions_known_artist(keyword, &track.artists.join(" "));
    let original_artists = canonical_original_artists_for_search(keyword, track);
    let original_artist_match = song_artist_matches_any(track, &original_artists);
    let mut score = 0.0;

    if title == q {
        score += 90.0;
    } else if title.starts_with(&q) {
        score += 55.0;
    } else if title.contains(&q) {
        score += 32.0;
    }
    if !title.is_empty() && !q.is_empty() && q.contains(&title) {
        score += if title.chars().count() >= 2 {
            68.0
        } else {
            18.0
        };
    }
    if original_artist_match
        && !title.is_empty()
        && !q.is_empty()
        && (title == q || q.contains(&title) || title.contains(&q))
    {
        score += 122.0;
    } else if !asks_derivative
        && !original_artists.is_empty()
        && !title.is_empty()
        && !q.is_empty()
        && (title == q || q.contains(&title) || title.contains(&q))
    {
        score -= 58.0;
    }
    if artist_mentioned {
        score += 96.0;
    } else if !artists.is_empty() && !q.is_empty() && q.contains(&artists) {
        score += 64.0;
    } else if !artists.is_empty() && artists.contains(&q) {
        score += 22.0;
    }
    if artist_mentioned && !title.is_empty() && q.contains(&title) {
        score += 34.0;
    }
    if JAY_CASE_INSENSITIVE_RE.is_match(keyword) && !artist_mentioned {
        score -= 28.0;
    }
    if !album.is_empty() && (album.contains(&q) || q.contains(&album)) {
        score += 8.0;
    }
    if track.provider == ProviderId::Qq {
        score += if search_intent_prefers_qq(keyword) {
            48.0
        } else {
            4.0
        };
    }
    if track.playable_state != PlayableState::Playable
        && track.playable_state != PlayableState::Unknown
        && track.playable_state != PlayableState::TrialOnly
    {
        score -= 12.0;
    }
    if !asks_derivative {
        if derivative {
            score -= if artist_mentioned { 76.0 } else { 96.0 };
        }
        if LIVE_RE.is_match(&raw) {
            score -= if artist_mentioned { 28.0 } else { 42.0 };
        }
        if !original_artists.is_empty()
            && search_looks_like_same_title_cover(
                track,
                &q,
                &title,
                &album,
                &raw,
                original_artist_match,
                source_index,
            )
        {
            score -= 46.0;
        }
    }
    score - source_index as f64 * 0.75
}

fn search_intent_prefers_qq(keyword: &str) -> bool {
    QQ_SEARCH_INTENT_RE.is_match(&keyword.to_lowercase())
}

fn search_mentions_known_artist(keyword: &str, artist: &str) -> bool {
    let raw_q = keyword.to_lowercase();
    let raw_artist = artist.to_lowercase();
    if raw_artist.is_empty() {
        return false;
    }
    if JAY_RE.is_match(&raw_q) && JAY_RE.is_match(&raw_artist) {
        return true;
    }
    let q = normalize_search_text(keyword);
    let a = normalize_search_text(artist);
    !a.is_empty() && a.chars().count() >= 2 && q.contains(&a)
}

fn search_looks_like_derivative(text: &str) -> bool {
    DERIVATIVE_RESULT_RE.is_match(text)
}

fn canonical_original_artists_for_search(keyword: &str, track: &Track) -> Vec<String> {
    let q = normalize_search_text(keyword);
    let title = normalize_search_text(&track.title);
    let joined = format!("{q} {title}");
    let rules = [
        (vec!["日落大道"], vec!["梁博"]),
        (
            vec!["beautyandabeat", "beauty and a beat"],
            vec!["justin bieber", "nicki minaj"],
        ),
    ];
    let mut artists = Vec::new();
    for (titles, rule_artists) in rules {
        let matched = titles.iter().any(|candidate| {
            let normalized_title = normalize_search_text(candidate);
            let title_matches = !title.is_empty()
                && (title == normalized_title || title.contains(&normalized_title));
            !normalized_title.is_empty() && (joined.contains(&normalized_title) || title_matches)
        });
        if !matched {
            continue;
        }
        for artist in rule_artists {
            if !artists.iter().any(|existing| existing == artist) {
                artists.push(artist.to_owned());
            }
        }
    }
    artists
}

fn song_artist_matches_any(track: &Track, artists: &[String]) -> bool {
    let track_artist = normalize_search_text(&track.artists.join(""));
    if track_artist.is_empty() || artists.is_empty() {
        return false;
    }
    artists.iter().any(|artist| {
        let normalized = normalize_search_text(artist);
        !normalized.is_empty()
            && (track_artist.contains(&normalized) || normalized.contains(&track_artist))
    })
}

fn search_looks_like_same_title_cover(
    track: &Track,
    q: &str,
    title: &str,
    album: &str,
    raw: &str,
    original_artist_match: bool,
    source_index: usize,
) -> bool {
    if q.is_empty() || title.is_empty() || original_artist_match {
        return false;
    }
    let same_title = title == q || q.contains(title) || title.starts_with(q);
    if !same_title {
        return false;
    }
    let self_titled_single = !album.is_empty()
        && (album == title || album == q || album.contains(title) || title.contains(album));
    self_titled_single
        || search_looks_like_derivative(raw)
        || source_index > 0
        || track.playable_state == PlayableState::Unavailable
}

fn no_result_error(provider: ProviderId, message: &str) -> anyhow::Error {
    ProviderError {
        code: ProviderErrorCode::NoResult,
        provider,
        message: message.to_owned(),
        retryable: false,
        action: None,
        raw_message: None,
    }
    .into()
}

fn no_url_error(provider: ProviderId, message: &str) -> anyhow::Error {
    ProviderError {
        code: ProviderErrorCode::NoUrl,
        provider,
        message: message.to_owned(),
        retryable: true,
        action: None,
        raw_message: None,
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        providers,
        types::{
            LyricPayload, PlaylistDetail, PlaylistSummary, ProviderLoginStatus, RecommendationPage,
            SongUrlResult, TrackQualityAvailability,
        },
    };
    use async_trait::async_trait;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tokio::{sync::Barrier, time::timeout};

    type Calls = Arc<Mutex<Vec<String>>>;

    #[derive(Clone)]
    struct MockProvider {
        id: ProviderId,
        calls: Calls,
        search_result: Vec<Track>,
        search_error: Option<ProviderError>,
        search_barrier: Option<Arc<Barrier>>,
        song_url_result: Option<SongUrlResult>,
        song_url_error: Option<ProviderError>,
        recommendation_page: Option<RecommendationPage>,
    }

    impl MockProvider {
        fn new(id: ProviderId, calls: Calls) -> Self {
            Self {
                id,
                calls,
                search_result: Vec::new(),
                search_error: None,
                search_barrier: None,
                song_url_result: None,
                song_url_error: None,
                recommendation_page: None,
            }
        }

        fn with_search(mut self, tracks: Vec<Track>) -> Self {
            self.search_result = tracks;
            self
        }

        fn with_search_error(mut self, code: ProviderErrorCode, message: &str) -> Self {
            self.search_error = Some(provider_error(self.id, code, message, false));
            self
        }

        fn with_search_barrier(mut self, search_barrier: Arc<Barrier>) -> Self {
            self.search_barrier = Some(search_barrier);
            self
        }

        fn with_song_url(mut self, url: &str) -> Self {
            self.song_url_result = Some(SongUrlResult {
                url: url.to_owned(),
                proxied: false,
                provider: None,
                trial: None,
                vip_level: None,
                expires_at: None,
            });
            self
        }

        fn with_recommendation_page(mut self, page: RecommendationPage) -> Self {
            self.recommendation_page = Some(page);
            self
        }
    }

    #[async_trait]
    impl ProviderAdapter for MockProvider {
        fn id(&self) -> ProviderId {
            self.id.clone()
        }

        async fn search_track(
            &self,
            keyword: &str,
            _offset: u32,
            limit: u32,
        ) -> providers::ProviderResult<Vec<Track>> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{}:search:{keyword}:{limit}", self.id));
            if let Some(search_barrier) = &self.search_barrier {
                search_barrier.wait().await;
            }
            if let Some(err) = &self.search_error {
                return Err(err.clone());
            }
            Ok(self.search_result.clone())
        }

        async fn song_url(
            &self,
            track: &Track,
            _opts: Option<SongUrlOptions>,
        ) -> providers::ProviderResult<SongUrlResult> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{}:songUrl:{}", self.id, track.id));
            if let Some(err) = &self.song_url_error {
                return Err(err.clone());
            }
            self.song_url_result
                .clone()
                .ok_or_else(|| provider_error(self.id, ProviderErrorCode::NoUrl, "no url", true))
        }

        async fn track_qualities(
            &self,
            _track: &Track,
        ) -> providers::ProviderResult<TrackQualityAvailability> {
            Ok(TrackQualityAvailability::default())
        }

        async fn lyric(&self, _track: &Track) -> providers::ProviderResult<LyricPayload> {
            Err(provider_error(
                self.id,
                ProviderErrorCode::NoResult,
                "no lyric",
                false,
            ))
        }

        async fn playlist_list(&self) -> providers::ProviderResult<Vec<PlaylistSummary>> {
            Err(provider_error(
                self.id,
                ProviderErrorCode::NoPlaylist,
                "no playlists",
                false,
            ))
        }

        async fn playlist_detail(
            &self,
            _id: &str,
            _offset: u32,
            _limit: u32,
        ) -> providers::ProviderResult<PlaylistDetail> {
            Err(provider_error(
                self.id,
                ProviderErrorCode::NoPlaylist,
                "no playlist",
                false,
            ))
        }

        async fn login_status(&self) -> providers::ProviderResult<ProviderLoginStatus> {
            Ok(ProviderLoginStatus::default())
        }

        async fn logout(&self) -> providers::ProviderResult<()> {
            Ok(())
        }

        async fn recommendation_page(&self) -> providers::ProviderResult<RecommendationPage> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{}:recommendationPage", self.id));
            self.recommendation_page.clone().ok_or_else(|| {
                provider_error(
                    self.id,
                    ProviderErrorCode::NotImplemented,
                    "recommendation page unavailable",
                    false,
                )
            })
        }
    }

    fn provider_error(
        provider: ProviderId,
        code: ProviderErrorCode,
        message: &str,
        retryable: bool,
    ) -> ProviderError {
        ProviderError {
            code,
            provider,
            message: message.to_owned(),
            retryable,
            action: None,
            raw_message: None,
        }
    }

    fn track(provider: ProviderId, id: &str, title: &str, artists: &[&str]) -> Track {
        Track {
            provider,
            id: id.to_owned(),
            source_id: id.to_owned(),
            media_mid: None,
            title: title.to_owned(),
            artists: artists.iter().map(|artist| (*artist).to_owned()).collect(),
            album: String::new(),
            cover_url: String::new(),
            quality_hints: Vec::new(),
            playable_state: PlayableState::Playable,
            duration_ms: None,
            artwork_url: None,
        }
    }

    fn resolver(providers: Vec<MockProvider>) -> CrossSourceResolver {
        let providers = providers
            .into_iter()
            .map(|provider| {
                (
                    provider.id(),
                    Arc::new(provider) as Arc<dyn ProviderAdapter>,
                )
            })
            .collect();
        create_cross_source_resolver(CrossSourceResolverDeps { providers })
    }

    #[tokio::test]
    async fn resolve_search_with_explicit_provider_uses_only_that_provider() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let resolver =
            resolver(vec![
                MockProvider::new(ProviderId::Netease, Arc::clone(&calls))
                    .with_search(vec![track(ProviderId::Netease, "n-1", "夜航", &["星野"])]),
                MockProvider::new(ProviderId::Qq, Arc::clone(&calls)),
            ]);

        let result = resolver
            .resolve_search("夜航", Some(ProviderId::Netease), 5)
            .await
            .unwrap();

        assert_eq!(result[0].title, "夜航");
        assert_eq!(calls.lock().unwrap().as_slice(), &["netease:search:夜航:5"]);
    }

    #[tokio::test]
    async fn resolve_recommendation_page_collects_successful_provider_pages() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let resolver = resolver(vec![
            MockProvider::new(ProviderId::Netease, Arc::clone(&calls)),
            MockProvider::new(ProviderId::Qq, Arc::clone(&calls)).with_recommendation_page(
                RecommendationPage {
                    provider: ProviderId::Qq,
                    list: Vec::new(),
                },
            ),
        ]);

        let pages = resolver.resolve_recommendation_page().await.unwrap();

        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].provider, ProviderId::Qq);
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &["netease:recommendationPage", "qq:recommendationPage"]
        );
    }

    #[tokio::test]
    async fn resolve_search_without_provider_merges_results_with_stable_dedupe() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let resolver = resolver(vec![
            MockProvider::new(ProviderId::Netease, Arc::clone(&calls)).with_search(vec![
                track(ProviderId::Netease, "n-1", "夜航", &["星野"]),
                track(ProviderId::Netease, "same", "同名", &["Ada"]),
            ]),
            MockProvider::new(ProviderId::Qq, Arc::clone(&calls)).with_search(vec![
                track(ProviderId::Qq, "q-1", "夜航", &["星野"]),
                track(ProviderId::Qq, "same", "同名", &["Ada"]),
            ]),
        ]);

        let result = resolver.resolve_search("夜航", None, 3).await.unwrap();

        let mut ids = result
            .iter()
            .map(|track| format!("{}:{}", track.provider, track.id))
            .collect::<Vec<_>>();
        ids.sort();
        assert_eq!(ids, vec!["netease:n-1", "qq:q-1", "qq:same"]);
        assert!(
            calls
                .lock()
                .unwrap()
                .iter()
                .all(|call| call.ends_with(":2"))
        );
    }

    #[tokio::test]
    async fn resolve_search_without_provider_keeps_successful_results_when_a_provider_fails() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let resolver = resolver(vec![
            MockProvider::new(ProviderId::Netease, Arc::clone(&calls))
                .with_search_error(ProviderErrorCode::Unavailable, "offline"),
            MockProvider::new(ProviderId::Qq, Arc::clone(&calls)).with_search(vec![track(
                ProviderId::Qq,
                "q-1",
                "夜航",
                &["星野"],
            )]),
        ]);

        let result = resolver.resolve_search("夜航", None, 2).await.unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].provider, ProviderId::Qq);
    }

    #[tokio::test]
    async fn resolve_search_without_provider_starts_all_provider_searches_concurrently() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let search_barrier = Arc::new(Barrier::new(2));
        let resolver = resolver(vec![
            MockProvider::new(ProviderId::Netease, Arc::clone(&calls))
                .with_search(vec![track(ProviderId::Netease, "n-1", "夜航", &["星野"])])
                .with_search_barrier(Arc::clone(&search_barrier)),
            MockProvider::new(ProviderId::Qq, Arc::clone(&calls))
                .with_search(vec![track(ProviderId::Qq, "q-1", "夜航", &["星野"])])
                .with_search_barrier(search_barrier),
        ]);

        let result = timeout(
            Duration::from_secs(1),
            resolver.resolve_search("夜航", None, 2),
        )
        .await
        .expect("concurrent searches should reach the barrier")
        .unwrap();

        assert_eq!(result.len(), 2);
    }

    #[tokio::test]
    async fn resolve_search_with_explicit_provider_does_not_fall_back() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let resolver = resolver(vec![
            MockProvider::new(ProviderId::Netease, Arc::clone(&calls)).with_search(Vec::new()),
            MockProvider::new(ProviderId::Qq, Arc::clone(&calls)).with_search(vec![track(
                ProviderId::Qq,
                "q-1",
                "夜航",
                &["星野"],
            )]),
        ]);

        let error = resolver
            .resolve_search("夜航", Some(ProviderId::Netease), 3)
            .await
            .unwrap_err();

        assert!(error.downcast_ref::<ProviderError>().is_some_and(|error| {
            matches!(&error.code, ProviderErrorCode::NoResult)
                && error.provider == ProviderId::Netease
        }));
        assert_eq!(calls.lock().unwrap().as_slice(), &["netease:search:夜航:3"]);
    }

    #[tokio::test]
    async fn resolve_song_url_tries_direct_provider_first_and_returns_its_url() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let resolver = resolver(vec![
            MockProvider::new(ProviderId::Netease, Arc::clone(&calls))
                .with_song_url("https://n.example/song.m4a"),
            MockProvider::new(ProviderId::Qq, Arc::clone(&calls)),
        ]);

        let result = resolver
            .resolve_song_url(track(ProviderId::Netease, "n-1", "夜航", &["星野"]), None)
            .await
            .unwrap();

        assert_eq!(result.url, "https://n.example/song.m4a");
        assert_eq!(calls.lock().unwrap().as_slice(), &["netease:songUrl:n-1"]);
    }

    #[tokio::test]
    async fn resolve_song_url_searches_fallback_provider_by_title_and_artists() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let resolver = resolver(vec![
            MockProvider::new(ProviderId::Netease, Arc::clone(&calls))
                .with_search_error(ProviderErrorCode::Unavailable, "netease down"),
            MockProvider::new(ProviderId::Qq, Arc::clone(&calls))
                .with_search(vec![track(ProviderId::Qq, "q-9", "夜航", &["星野"])])
                .with_song_url("https://q.example/song.m4a"),
        ]);

        let result = resolver
            .resolve_song_url(track(ProviderId::Netease, "n-1", "夜航", &["星野"]), None)
            .await
            .unwrap();

        assert_eq!(result.url, "https://q.example/song.m4a");
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &[
                "netease:songUrl:n-1",
                "qq:search:夜航 星野:5",
                "qq:songUrl:q-9"
            ]
        );
    }

    #[tokio::test]
    async fn resolve_song_url_searches_import_only_tracks_instead_of_direct_id() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut import_track = track(
            ProviderId::Netease,
            "import:apple-music:1",
            "夜航",
            &["星野"],
        );
        import_track.source_id = "import:apple-music:1".to_owned();
        let resolver = resolver(vec![
            MockProvider::new(ProviderId::Netease, Arc::clone(&calls))
                .with_search(vec![track(
                    ProviderId::Netease,
                    "n-match",
                    "夜航",
                    &["星野"],
                )])
                .with_song_url("https://n.example/match.m4a"),
        ]);

        let result = resolver.resolve_song_url(import_track, None).await.unwrap();

        assert_eq!(result.url, "https://n.example/match.m4a");
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &["netease:search:夜航 星野:5", "netease:songUrl:n-match"]
        );
    }
}
