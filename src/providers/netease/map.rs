use serde_json::Value;

use crate::{
    parsers::{lrc, netease},
    types::{LyricLine, LyricPayload, PlayableState, PlaylistDetail, PlaylistSummary, Track, ProviderId},
};

pub fn normalize_provider_image_url(url: &str) -> String {
    let value = url.trim();
    if value.is_empty() {
        return String::new();
    }
    if let Some(stripped) = value.strip_prefix("//") {
        return format!("https://{stripped}");
    }
    if value.len() >= 7 && value[..7].eq_ignore_ascii_case("http://") {
        return format!("https://{}", &value[7..]);
    }
    value.to_owned()
}

pub fn map_playable(
    fee: Option<i64>,
    code: Option<i64>,
    free_trial_info: Option<&Value>,
    has_cookie: bool,
    url: Option<&str>,
) -> PlayableState {
    if code == Some(200) && url.filter(|value| !value.is_empty()).is_some() {
        return PlayableState::Playable;
    }
    if code == Some(401) {
        return PlayableState::LoginRequired;
    }
    match fee.unwrap_or_default() {
        1 => {
            if has_cookie && url.filter(|value| !value.is_empty()).is_some() {
                PlayableState::Playable
            } else {
                PlayableState::VipRequired
            }
        }
        4 => PlayableState::PaidRequired,
        8 if free_trial_info.is_some() => PlayableState::TrialOnly,
        _ if url.filter(|value| !value.is_empty()).is_some() => PlayableState::Playable,
        _ => PlayableState::Unknown,
    }
}

pub fn map_hana_song_to_track(raw: &Value) -> Track {
    let id = raw.get("id").map(value_to_string).unwrap_or_default();
    let artists = raw
        .get("ar")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|artist| artist.get("name").and_then(Value::as_str))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let album = raw.get("al").and_then(Value::as_object);
    let fee = raw.get("fee").and_then(Value::as_i64);

    Track {
        id: id.clone(),
        provider: ProviderId::Netease,
        source_id: id,
        media_mid: None,
        title: raw
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        artists,
        album: album
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        cover_url: normalize_provider_image_url(
            album
                .and_then(|value| value.get("picUrl"))
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ),
        quality_hints: vec!["standard".to_owned()],
        playable_state: match fee.unwrap_or_default() {
            1 => PlayableState::VipRequired,
            4 => PlayableState::PaidRequired,
            8 => PlayableState::TrialOnly,
            _ => PlayableState::Unknown,
        },
        duration_ms: raw.get("dt").and_then(Value::as_u64),
        artwork_url: None,
    }
}

pub fn parse_lrc(text: &str) -> Vec<LyricLine> {
    lrc::parse_lrc(text)
}

pub fn parse_yrc_text(text: &str) -> Vec<LyricLine> {
    netease::parse_yrc_text(text)
}

pub fn map_hana_lyric_to_payload(
    track_id: &str,
    lrc: &str,
    tlyric: &str,
    klyric: Option<&str>,
    yrc: Option<&str>,
) -> LyricPayload {
    let base_lines = yrc
        .map(parse_yrc_text)
        .filter(|lines| !lines.is_empty())
        .unwrap_or_else(|| parse_lrc(lrc));
    let translation_lines = parse_lrc(tlyric);
    let translation_map = translation_lines
        .into_iter()
        .map(|line| (line.time_ms, line.text))
        .collect::<std::collections::HashMap<_, _>>();

    let lines: Vec<LyricLine> = base_lines
        .into_iter()
        .map(|mut line| {
            line.translation = translation_map.get(&line.time_ms).cloned();
            line
        })
        .collect();
    let is_word_by_word = lines.iter().any(|line| {
        line.words
            .as_ref()
            .map(|words| !words.is_empty())
            .unwrap_or(false)
    }) || !klyric.unwrap_or_default().trim().is_empty();

    LyricPayload {
        provider: ProviderId::Netease,
        track_id: track_id.to_owned(),
        lines,
        has_translation: !tlyric.trim().is_empty() && !translation_map.is_empty(),
        is_word_by_word,
    }
}

pub fn map_hana_playlist_to_summary(raw: &Value, id_hint: Option<&str>) -> PlaylistSummary {
    let id = raw
        .get("id")
        .map(value_to_string)
        .filter(|value| !value.is_empty())
        .or_else(|| id_hint.map(str::to_owned))
        .unwrap_or_default();
    let track_count = raw
        .get("trackCount")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok());
    let track_ids = raw
        .get("trackIds")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.get("id").map(value_to_string).or_else(|| match item {
                        Value::String(_) | Value::Number(_) => Some(value_to_string(item)),
                        _ => None,
                    })
                })
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    PlaylistSummary {
        provider: ProviderId::Netease,
        id,
        name: raw
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        cover_url: normalize_provider_image_url(
            raw.get("coverImgUrl")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ),
        track_count,
        track_ids,
        collected: Some(raw.get("collected").and_then(Value::as_bool) == Some(true)),
    }
}

pub fn map_hana_playlist_to_detail(raw: &Value, id_hint: Option<&str>) -> PlaylistDetail {
    let summary = map_hana_playlist_to_summary(raw, id_hint);
    let tracks = raw
        .get("tracks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(map_hana_song_to_track)
        .collect();

    PlaylistDetail {
        provider: summary.provider,
        id: summary.id,
        name: summary.name,
        cover_url: summary.cover_url,
        track_count: summary.track_count,
        track_ids: summary.track_ids,
        collected: summary.collected,
        tracks,
    }
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_lrc_lines() {
        let lines = parse_lrc("[00:01.20]hello\n[00:02.30]world");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].time_ms, 1_200);
        assert_eq!(lines[0].text, "hello");
    }

    #[test]
    fn prefers_yrc_when_available() {
        let payload = map_hana_lyric_to_payload(
            "1",
            "[00:01.00]fallback",
            "",
            None,
            Some("[1000,300](1000,100,0)hel(1100,100,0)lo"),
        );
        assert_eq!(payload.lines[0].text, "hello");
        assert_eq!(payload.lines[0].time_ms, 1_000);
    }

    #[test]
    fn maps_song_to_track() {
        let track = map_hana_song_to_track(&json!({
            "id": 42,
            "name": "Test",
            "ar": [{"name": "A"}],
            "al": {"name": "Album", "picUrl": "http://a/b.jpg"},
            "dt": 1234
        }));
        assert_eq!(track.id, "42");
        assert_eq!(track.cover_url, "https://a/b.jpg");
        assert_eq!(track.artists, vec!["A"]);
    }

    #[test]
    fn playlist_summary_defaults_collected_to_false() {
        let summary = map_hana_playlist_to_summary(&json!({ "id": 1 }), None);

        assert_eq!(summary.collected, Some(false));
    }
}
