use serde_json::Value;

use crate::types::{PlayableState, ProviderId, Track};

#[derive(Clone, Debug, Default)]
pub struct KugouTrackMeta {
    pub album_id: u64,
    pub album_audio_id: u64,
    pub hq_hash: String,
    pub sq_hash: String,
    pub res_hash: String,
    pub title: String,
    pub duration_ms: u64,
}

#[cfg(test)]
pub fn map_kugou_song_to_track(raw: &Value) -> Track {
    map_kugou_song(raw).0
}

pub fn map_kugou_song(raw: &Value) -> (Track, KugouTrackMeta) {
    let hash = first_string(raw, &["FileHash", "hash", "Hash"]);
    let title = first_string(raw, &["SongName", "songname", "name", "filename"]);
    let album_audio_id = first_string(
        raw,
        &[
            "MixSongID",
            "mixsongid",
            "AlbumAudioID",
            "album_audio_id",
            "EMixSongID",
        ],
    );
    let duration_ms = raw
        .get("Duration")
        .or_else(|| raw.get("duration"))
        .or_else(|| raw.get("timelen"))
        .and_then(Value::as_u64)
        .map(|value| if value > 10_000 { value } else { value * 1_000 })
        .unwrap_or_default();
    let track = Track {
        id: hash.clone(),
        provider: ProviderId::Kugou,
        source_id: hash,
        media_mid: non_empty(album_audio_id.clone()),
        title: title.clone(),
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
        duration_ms: (duration_ms > 0).then_some(duration_ms),
        artwork_url: None,
    };
    let meta = KugouTrackMeta {
        album_id: number(raw, &["AlbumID", "album_id"]),
        album_audio_id: album_audio_id.parse().unwrap_or_default(),
        hq_hash: first_string(raw, &["HQFileHash", "hq_hash", "hqHash"]),
        sq_hash: first_string(raw, &["SQFileHash", "sq_hash", "sqHash"]),
        res_hash: first_string(raw, &["ResFileHash", "res_hash", "resHash"]),
        title,
        duration_ms,
    };
    (track, meta)
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

fn number(raw: &Value, fields: &[&str]) -> u64 {
    fields
        .iter()
        .find_map(|field| raw.get(*field))
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        })
        .unwrap_or_default()
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

    use super::{map_kugou_song, map_kugou_song_to_track};

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

    #[test]
    fn preserves_playback_metadata_for_adapter_cache() {
        let (_, metadata) = map_kugou_song(&json!({
            "FileHash": "STD", "AlbumID": 7, "AlbumAudioID": 42,
            "HQFileHash": "HQ", "SQFileHash": "SQ", "ResFileHash": "RES"
        }));

        assert_eq!(metadata.album_id, 7);
        assert_eq!(metadata.album_audio_id, 42);
        assert_eq!(metadata.hq_hash, "HQ");
        assert_eq!(metadata.sq_hash, "SQ");
        assert_eq!(metadata.res_hash, "RES");
    }
}
