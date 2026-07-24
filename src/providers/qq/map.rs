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
    value.replacen("http://", "https://", 1)
}

pub fn map_qq_song_to_track(raw: &Value) -> Track {
    let id = first_string(&[
        raw.get("songmid"),
        raw.get("mid"),
        raw.get("songid"),
        raw.get("id"),
    ]);
    let media_mid = first_string(&[
        raw.get("file").and_then(|value| value.get("media_mid")),
        raw.get("file").and_then(|value| value.get("strMediaMid")),
        raw.get("media_mid"),
        raw.get("strMediaMid"),
        raw.get("mediaMid"),
    ]);
    let artists = raw
        .get("singer")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|singer| singer.get("name").and_then(Value::as_str))
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    let artists = if artists.is_empty() {
        split_artist_text(
            raw.get("singername")
                .or_else(|| raw.get("singerName"))
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
    } else {
        artists
    };
    let album_mid = first_string(&[
        raw.get("albummid"),
        raw.get("album").and_then(|value| value.get("mid")),
        raw.get("album").and_then(|value| value.get("pmid")),
    ])
    .replace(|c: char| !c.is_ascii_alphanumeric(), "");
    let cover_url = raw
        .get("pic")
        .and_then(Value::as_str)
        .map(normalize_provider_image_url)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            if album_mid.is_empty() {
                String::new()
            } else {
                format!("https://y.gtimg.cn/music/photo_new/T002R300x300M000{album_mid}.jpg")
            }
        });

    Track {
        id: id.clone(),
        provider: ProviderId::Qq,
        source_id: id,
        media_mid: (!media_mid.is_empty()).then_some(media_mid),
        title: first_string(&[raw.get("songname"), raw.get("name"), raw.get("title")]),
        artists,
        album: first_string(&[
            raw.get("albumname"),
            raw.get("album").and_then(|value| value.get("name")),
            raw.get("album").and_then(|value| value.get("title")),
        ]),
        cover_url,
        quality_hints: vec!["standard".to_owned()],
        playable_state: PlayableState::Unknown,
        duration_ms: raw
            .get("interval")
            .and_then(Value::as_u64)
            .map(|value| value * 1_000),
        artwork_url: None,
    }
}

fn first_string(values: &[Option<&Value>]) -> String {
    values
        .iter()
        .copied()
        .flatten()
        .find_map(value_to_string)
        .unwrap_or_default()
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) if !value.trim().is_empty() => Some(value.trim().to_owned()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn split_artist_text(text: &str) -> Vec<String> {
    text.split(['/', ',', '，', '、'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}
