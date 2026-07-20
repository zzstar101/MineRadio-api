use serde_json::Value;

use crate::types::{PlayableState, Track, ProviderId};

pub fn map_kugou_song_to_track(raw: &Value) -> Track {
    let hash = first_string(raw, &["FileHash", "hash", "Hash"]);
    Track {
        id: hash.clone(),
        provider: ProviderId::Kugou,
        source_id: hash,
        media_mid: non_empty(first_string(raw, &["AlbumAudioID", "album_audio_id"])),
        title: first_string(raw, &["SongName", "songname", "filename"]),
        artists: split_artists(&first_string(
            raw,
            &["SingerName", "singername", "author_name"],
        )),
        album: first_string(raw, &["AlbumName", "album_name"]),
        cover_url: first_string(raw, &["Image", "image", "img"])
            .replace("{size}", "400")
            .replace("{width}", "400")
            .replace("{height}", "400"),
        quality_hints: vec![
            "standard".to_owned(),
            "higher".to_owned(),
            "lossless".to_owned(),
        ],
        playable_state: PlayableState::Unknown,
        duration_ms: raw
            .get("Duration")
            .or_else(|| raw.get("duration"))
            .and_then(Value::as_u64)
            .map(|seconds| seconds * 1_000),
        artwork_url: None,
    }
}

fn first_string(raw: &Value, fields: &[&str]) -> String {
    fields
        .iter()
        .find_map(|field| raw.get(*field))
        .map(value_to_string)
        .unwrap_or_default()
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.trim().to_owned(),
        Value::Number(value) => value.to_string(),
        _ => String::new(),
    }
}

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn split_artists(value: &str) -> Vec<String> {
    value
        .split(['、', '/', '&'])
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::map_kugou_song_to_track;

    #[test]
    fn maps_kugou_search_song() {
        let track = map_kugou_song_to_track(&json!({
            "FileHash": "ABC",
            "SongName": "Song",
            "SingerName": "A / B",
            "AlbumAudioID": 42,
            "Duration": 120
        }));

        assert_eq!(track.source_id, "ABC");
        assert_eq!(track.artists, ["A", "B"]);
        assert_eq!(track.media_mid.as_deref(), Some("42"));
        assert_eq!(track.duration_ms, Some(120_000));
    }
}
