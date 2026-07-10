use regex::Regex;
use serde_json::Value;

use crate::types::{LyricLine, LyricPayload, LyricWord, PlaylistDetail, PlaylistSummary, Track};

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

pub fn map_soda_song_to_track(raw: &Value) -> Track {
    let id = raw
        .get("id")
        .map(value_to_string)
        .unwrap_or_default()
        .trim()
        .to_owned();
    let artists = raw
        .get("artists")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|artist| artist.get("name").and_then(Value::as_str))
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    let album = raw.get("album").and_then(Value::as_object);
    let preview = raw.get("preview");
    let bit_rates = raw
        .get("bit_rates")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("quality").and_then(Value::as_str))
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();

    Track {
        id: id.clone(),
        provider: "soda".to_owned(),
        source_id: id,
        media_mid: None,
        title: raw
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned(),
        artists,
        album: album
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned(),
        cover_url: normalize_provider_image_url(&soda_sized_cover_url(
            album.and_then(|value| value.get("url_cover")),
        )),
        quality_hints: if bit_rates.is_empty() {
            vec!["standard".to_owned()]
        } else {
            dedupe(bit_rates)
        },
        playable_state: if preview.is_some() && !preview.unwrap_or(&Value::Null).is_null() {
            "trial_only".to_owned()
        } else {
            "unknown".to_owned()
        },
        duration_ms: raw.get("duration").and_then(Value::as_u64),
        artwork_url: None,
    }
}

pub fn parse_lrc(text: &str) -> Vec<LyricLine> {
    let Ok(marker_re) = Regex::new(r"\[(\d{1,3}):(\d{1,2})(?:[.:](\d{1,3}))?\]") else {
        return Vec::new();
    };
    let mut lines = Vec::new();

    for raw_line in text.lines() {
        let mut markers = Vec::new();
        for marker in marker_re.captures_iter(raw_line) {
            let min = marker
                .get(1)
                .and_then(|value| value.as_str().parse::<u64>().ok())
                .unwrap_or_default();
            let sec = marker
                .get(2)
                .and_then(|value| value.as_str().parse::<u64>().ok())
                .unwrap_or_default();
            let frac = marker
                .get(3)
                .map(|value| {
                    let mut padded = value.as_str().to_owned();
                    padded.push_str("000");
                    padded.chars().take(3).collect::<String>()
                })
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or_default();
            let end = marker.get(0).map(|value| value.end()).unwrap_or_default();
            markers.push((min * 60_000 + sec * 1_000 + frac, end));
        }
        if markers.is_empty() {
            continue;
        }
        let text = raw_line
            .get(markers.last().map(|(_, end)| *end).unwrap_or_default()..)
            .unwrap_or_default()
            .trim()
            .to_owned();
        for (time_ms, _) in markers {
            lines.push(LyricLine {
                time_ms,
                text: text.clone(),
                ..Default::default()
            });
        }
    }
    lines
}

pub fn parse_soda_lyric_text(text: &str) -> Vec<LyricLine> {
    let Ok(line_re) = Regex::new(r"^\[(\d+),(\d+)\](.*)$") else {
        return Vec::new();
    };
    let Ok(word_marker_re) = Regex::new(r"<\d+,\d+(?:,\d+)?>") else {
        return Vec::new();
    };
    let mut lines = Vec::new();

    for raw_line in text.lines() {
        let Some(caps) = line_re.captures(raw_line) else {
            continue;
        };
        let time_ms = caps
            .get(1)
            .and_then(|value| value.as_str().parse::<u64>().ok())
            .unwrap_or_default();
        let line_duration_ms = caps
            .get(2)
            .and_then(|value| value.as_str().parse::<u64>().ok())
            .unwrap_or_default();
        let body = caps.get(3).map(|value| value.as_str()).unwrap_or_default();
        let mut full_text = String::new();
        let mut words = Vec::new();
        let mut pos = 0usize;

        while pos < body.len() {
            let Some(open) = body[pos..].find('<').map(|index| pos + index) else {
                break;
            };
            let after_open = open + 1;
            if body
                .as_bytes()
                .get(after_open)
                .map(|byte| byte.is_ascii_digit())
                != Some(true)
            {
                pos = after_open;
                continue;
            }
            let Some(comma1_rel) = body[after_open..].find(',') else {
                break;
            };
            let comma1 = after_open + comma1_rel;
            let raw_start = body[after_open..comma1].parse::<u64>().unwrap_or_default();
            let Some(close) = body[after_open..].find('>').map(|index| after_open + index) else {
                break;
            };
            let duration_end = body[comma1 + 1..close]
                .find(',')
                .map(|index| comma1 + 1 + index)
                .unwrap_or(close);
            let raw_duration = body[comma1 + 1..duration_end]
                .parse::<u64>()
                .unwrap_or_default();
            let text_start = close + 1;
            let next_open = body[text_start..]
                .find('<')
                .map(|index| text_start + index)
                .unwrap_or(body.len());
            let segment = &body[text_start..next_open];
            let c0 = utf16_len(&full_text);
            full_text.push_str(segment);
            if !segment.is_empty() {
                words.push(LyricWord {
                    text: Some(segment.to_owned()),
                    time_ms: time_ms + raw_start,
                    duration_ms: Some(raw_duration),
                    c0,
                    c1: utf16_len(&full_text),
                });
            }
            pos = next_open;
        }

        let plain = if full_text.trim().is_empty() {
            word_marker_re.replace_all(body, "").trim().to_owned()
        } else {
            full_text
        };

        if plain.trim().is_empty() {
            continue;
        }
        lines.push(LyricLine {
            time_ms,
            duration_ms: Some(line_duration_ms),
            text: plain.clone(),
            source: Some(if words.is_empty() {
                "soda-line".to_owned()
            } else {
                "soda-word".to_owned()
            }),
            words: (!words.is_empty()).then_some(words),
            char_count: Some(utf16_len(&plain).max(1)),
            ..Default::default()
        });
    }

    finalize_lyric_line_durations(lines)
}

pub fn map_soda_lyric_to_payload(track_id: &str, lyric: &str, trans: &str) -> LyricPayload {
    let base_lines = {
        let soda_lines = parse_soda_lyric_text(lyric);
        if soda_lines.is_empty() {
            parse_lrc(lyric)
        } else {
            soda_lines
        }
    };
    let translations = parse_lrc(trans)
        .into_iter()
        .map(|line| (line.time_ms, line.text))
        .collect::<std::collections::HashMap<_, _>>();
    let lines = base_lines
        .into_iter()
        .map(|mut line| {
            line.translation = translations
                .get(&line.time_ms)
                .cloned()
                .filter(|value| !value.is_empty());
            line
        })
        .collect::<Vec<_>>();
    let is_word_by_word = lines.iter().any(|line| {
        line.words
            .as_ref()
            .map(|words| !words.is_empty())
            .unwrap_or(false)
    });

    LyricPayload {
        provider: "soda".to_owned(),
        track_id: track_id.to_owned(),
        lines,
        has_translation: !translations.is_empty(),
        is_word_by_word,
    }
}

pub fn map_soda_playlist_to_summary(raw: &Value, id_hint: Option<&str>) -> PlaylistSummary {
    PlaylistSummary {
        provider: "soda".to_owned(),
        id: raw
            .get("id")
            .map(value_to_string)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .or_else(|| id_hint.map(str::to_owned))
            .unwrap_or_default(),
        name: raw
            .get("title")
            .or_else(|| raw.get("public_title"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned(),
        cover_url: normalize_provider_image_url(&soda_sized_cover_url(raw.get("url_cover"))),
        track_count: raw
            .get("count_tracks")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok()),
        track_ids: Vec::new(),
        subscribed: Some(raw.get("is_private").and_then(Value::as_bool) == Some(false)),
    }
}

pub fn map_soda_playlist_detail_to_detail(
    raw: Option<&Value>,
    id_hint: Option<&str>,
) -> PlaylistDetail {
    let playlist = raw.and_then(|value| value.get("playlist"));
    let summary = map_soda_playlist_to_summary(playlist.unwrap_or(&Value::Null), id_hint);
    let tracks = raw
        .and_then(|value| value.get("media_resources"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item.get("entity")
                .and_then(|entity| entity.get("track_wrapper"))
                .and_then(|wrapper| wrapper.get("track"))
                .filter(|track| track.is_object())
        })
        .map(map_soda_song_to_track)
        .collect::<Vec<_>>();

    PlaylistDetail {
        provider: summary.provider,
        id: summary.id,
        name: summary.name,
        cover_url: summary.cover_url,
        track_count: summary.track_count,
        track_ids: summary.track_ids,
        subscribed: summary.subscribed,
        tracks,
    }
}

fn finalize_lyric_line_durations(mut lines: Vec<LyricLine>) -> Vec<LyricLine> {
    lines.sort_by_key(|line| line.time_ms);
    for index in 0..lines.len() {
        let next_time = lines.get(index + 1).map(|line| line.time_ms);
        if let Some(current) = lines.get_mut(index) {
            let inferred = next_time
                .filter(|time| *time > current.time_ms)
                .map(|time| time - current.time_ms)
                .unwrap_or(4_800);
            let duration = current
                .duration_ms
                .filter(|value| *value > 0)
                .unwrap_or(inferred);
            let duration = duration.clamp(450, 12_000);
            current.duration_ms = Some(duration);
            current.char_count = Some(
                current
                    .char_count
                    .unwrap_or_else(|| utf16_len(&current.text).max(1)),
            );
        }
    }
    lines
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => String::new(),
    }
}

fn soda_sized_cover_url(cover: Option<&Value>) -> String {
    let Some(cover) = cover else {
        return String::new();
    };
    let uri = cover
        .get("uri")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if uri.is_empty() {
        return String::new();
    }
    let cdn = cover
        .get("urls")
        .and_then(Value::as_array)
        .and_then(|urls| urls.first())
        .and_then(Value::as_str)
        .unwrap_or("https://p3-luna.douyinpic.com/img/");
    let prefix = cover
        .get("template_prefix")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let prefix = if prefix.is_empty() {
        "tplv-b829550vbb"
    } else {
        prefix
    };
    format!("{cdn}{uri}~{prefix}-crop-center:256:256.webp")
}

fn dedupe(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn utf16_len(value: &str) -> usize {
    value.encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn soda_song_track_keeps_lossless_quality_hint() {
        let track = map_soda_song_to_track(&json!({
            "id": "123",
            "name": "Demo Song",
            "artists": [{ "name": "Alice" }],
            "album": { "name": "Demo Album" },
            "bit_rates": [
                { "quality": "highest" },
                { "quality": "lossless" },
                { "quality": "higher" }
            ]
        }));

        assert_eq!(track.quality_hints, vec!["highest", "lossless", "higher"]);
    }

    #[test]
    fn soda_playlist_detail_skips_null_tracks_and_defaults_subscribed_false() {
        let detail = map_soda_playlist_detail_to_detail(
            Some(&json!({
                "playlist": {
                    "id": "pl-1",
                    "title": "Playlist"
                },
                "media_resources": [
                    { "entity": { "track_wrapper": { "track": null } } },
                    { "entity": { "track_wrapper": { "track": { "id": "t-1", "name": "Track 1" } } } }
                ]
            })),
            Some("pl-1"),
        );

        assert_eq!(detail.subscribed, Some(false));
        assert_eq!(detail.tracks.len(), 1);
        assert_eq!(detail.tracks[0].id, "t-1");
    }

    #[test]
    fn soda_lyric_zero_duration_uses_inferred_duration() {
        let lines = parse_soda_lyric_text("[1000,0]hello\n[3000,0]world");

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].duration_ms, Some(2000));
        assert_eq!(lines[1].duration_ms, Some(4800));
    }
}
