use serde_json::Value;

use crate::types::{AlbumSummary, PlayableState, PlaylistSummary, ProviderId, Track};

pub const LIKED_PLAYLIST_ID: &str = "spotify-liked";

pub fn track(value: &Value, album_override: Option<&Value>) -> Option<Track> {
    if value.get("is_local").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let id = string(value, "id");
    let title = string(value, "name");
    if id.is_empty() || title.is_empty() {
        return None;
    }
    let album = album_override.unwrap_or_else(|| value.get("album").unwrap_or(&Value::Null));
    Some(Track {
        id: id.clone(),
        source_id: id,
        provider: ProviderId::Spotify,
        title,
        artists: names(value.get("artists")),
        album: string(album, "name"),
        cover_url: image(album.get("images")),
        duration_ms: value.get("duration_ms").and_then(Value::as_u64),
        artwork_url: Some(image(album.get("images"))).filter(|value| !value.is_empty()),
        quality_hints: Vec::new(),
        playable_state: PlayableState::Unavailable,
        media_mid: None,
    })
}

pub fn playlist(value: &Value) -> Option<PlaylistSummary> {
    let id = string(value, "id");
    if id.is_empty() {
        return None;
    }
    Some(PlaylistSummary {
        provider: ProviderId::Spotify,
        id,
        name: string(value, "name"),
        cover_url: image(value.get("images")),
        track_count: value
            .pointer("/tracks/total")
            .or_else(|| value.pointer("/items/total"))
            .and_then(Value::as_u64)
            .and_then(|value| value.try_into().ok()),
        track_ids: Vec::new(),
        collected: None,
    })
}

pub fn liked_playlist(total: u32, cover_url: String) -> PlaylistSummary {
    PlaylistSummary {
        provider: ProviderId::Spotify,
        id: LIKED_PLAYLIST_ID.to_owned(),
        name: "Spotify Liked Songs".to_owned(),
        cover_url,
        track_count: Some(total),
        track_ids: Vec::new(),
        collected: Some(true),
    }
}

pub fn album(value: &Value) -> Option<AlbumSummary> {
    let id = string(value, "id");
    if id.is_empty() {
        return None;
    }
    Some(AlbumSummary {
        provider: ProviderId::Spotify,
        id,
        name: string(value, "name"),
        artists: names(value.get("artists")),
        cover_url: image(value.get("images")),
        track_count: value
            .get("total_tracks")
            .or_else(|| value.pointer("/tracks/total"))
            .and_then(Value::as_u64)
            .and_then(|value| value.try_into().ok()),
        track_ids: Vec::new(),
        collected: None,
    })
}

pub fn names(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("name").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}
pub fn image(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_array)
        .and_then(|images| {
            images.iter().max_by_key(|image| {
                image
                    .get("width")
                    .and_then(Value::as_u64)
                    .unwrap_or_default()
            })
        })
        .and_then(|image| image.get("url"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
pub fn string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::track;
    use serde_json::json;
    #[test]
    fn maps_track_as_metadata_only() {
        let value = json!({"id":"track-1","name":"Song","artists":[{"name":"Artist"}],"album":{"name":"Album","images":[{"url":"cover","width":640}]},"duration_ms":123});
        let track = track(&value, None).unwrap();
        assert_eq!(track.provider.as_str(), "spotify");
        assert_eq!(track.artists, ["Artist"]);
        assert_eq!(track.cover_url, "cover");
        assert_eq!(track.playable_state.as_str(), "unavailable");
    }
}
