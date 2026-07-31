use serde_json::Value;

use crate::types::{PlayableState, ProviderId, Track};

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

/// 旧版 Value 映射路径，暂时保留用于新模型回退/对照测试。
#[allow(dead_code)]
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

#[allow(dead_code)]
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
}
