use serde_json::Value;

use crate::types::{PlayableState, PlaylistSummary, ProviderId, Track};

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

pub fn map_qq_playlist_to_summary(raw: &Value, id_hint: Option<&str>) -> PlaylistSummary {
    PlaylistSummary {
        provider: ProviderId::Qq,
        id: {
            let id = first_string(&[
                raw.get("disstid"),
                raw.get("dissid"),
                raw.get("dirid"),
                raw.get("tid"),
                raw.get("id"),
            ]);
            if id.is_empty() {
                id_hint.unwrap_or_default().to_owned()
            } else {
                id
            }
        },
        name: first_string(&[
            raw.get("dissname"),
            raw.get("diss_name"),
            raw.get("name"),
            raw.get("title"),
        ]),
        cover_url: normalize_provider_image_url(&first_string(&[
            raw.get("logo"),
            raw.get("picurl"),
        ])),
        track_count: first_u32(&[
            raw.get("total_song_num"),
            raw.get("song_cnt"),
            raw.get("songnum"),
            raw.get("song_count"),
        ]),
        track_ids: raw
            .get("songlist")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        item.get("songmid")
                            .or_else(|| item.get("mid"))
                            .and_then(value_to_string)
                    })
                    .collect()
            })
            .unwrap_or_default(),
        collected: Some(false),
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

fn first_u32(values: &[Option<&Value>]) -> Option<u32> {
    values.iter().copied().flatten().find_map(|value| {
        value
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .or_else(|| {
                value
                    .as_i64()
                    .and_then(|value| u64::try_from(value).ok())
                    .and_then(|value| u32::try_from(value).ok())
            })
    })
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
