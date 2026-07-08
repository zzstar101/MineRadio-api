use regex::Regex;
use serde_json::Value;

use crate::types::{LyricLine, LyricPayload, PlaylistDetail, PlaylistSummary, Track};

pub fn normalize_provider_image_url(url: &str) -> String {
    let value = url.trim();
    if value.is_empty() {
        return String::new();
    }
    if let Some(stripped) = value.strip_prefix("//") {
        return format!("https:{stripped}");
    }
    value.replacen("http://", "https://", 1)
}

pub fn map_playable(
    fee: Option<i64>,
    code: Option<i64>,
    free_trial_info: Option<&Value>,
    has_cookie: bool,
    url: Option<&str>,
) -> String {
    if code == Some(200) && url.filter(|value| !value.is_empty()).is_some() {
        return "playable".to_owned();
    }
    if code == Some(401) {
        return "login_required".to_owned();
    }
    match fee.unwrap_or_default() {
        1 => {
            if has_cookie && url.filter(|value| !value.is_empty()).is_some() {
                "playable".to_owned()
            } else {
                "vip_required".to_owned()
            }
        }
        4 => "paid_required".to_owned(),
        8 if free_trial_info.is_some() => "trial_only".to_owned(),
        _ if url.filter(|value| !value.is_empty()).is_some() => "playable".to_owned(),
        _ => "unknown".to_owned(),
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
        provider: "netease".to_owned(),
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
            1 => "vip_required",
            4 => "paid_required",
            8 => "trial_only",
            _ => "unknown",
        }
        .to_owned(),
        duration_ms: raw.get("dt").and_then(Value::as_u64),
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
            });
        }
    }

    lines.sort_by_key(|line| line.time_ms);
    lines
}

pub fn parse_yrc_text(text: &str) -> Vec<LyricLine> {
    let Ok(line_re) = Regex::new(r"^\[(\d+),(\d+)\](.*)$") else {
        return Vec::new();
    };
    let Ok(word_re) = Regex::new(r"\((\d+),(\d+),\d+\)([^()]*)") else {
        return Vec::new();
    };
    let spacer_re = Regex::new(r"\s+").ok();
    let marker_re = Regex::new(r"\(\d+,\d+,\d+\)").ok();

    let mut lines = Vec::new();
    for raw_line in text.lines() {
        let Some(caps) = line_re.captures(raw_line) else {
            continue;
        };
        let time_ms = caps
            .get(1)
            .and_then(|value| value.as_str().parse::<u64>().ok())
            .unwrap_or_default();
        let body = caps.get(3).map(|value| value.as_str()).unwrap_or_default();
        let mut full_text = String::new();

        for word in word_re.captures_iter(body) {
            let fragment = word.get(3).map(|value| value.as_str()).unwrap_or_default();
            let normalized = spacer_re
                .as_ref()
                .map(|re| re.replace_all(fragment, " ").to_string())
                .unwrap_or_else(|| fragment.to_owned());
            if !normalized.is_empty() {
                full_text.push_str(&normalized);
            }
        }

        if full_text.trim().is_empty() {
            full_text = marker_re
                .as_ref()
                .map(|re| re.replace_all(body, "").to_string())
                .unwrap_or_else(|| body.to_owned());
        }

        let text = spacer_re
            .as_ref()
            .map(|re| re.replace_all(full_text.trim(), " ").to_string())
            .unwrap_or_else(|| full_text.trim().to_owned());
        if text.is_empty() {
            continue;
        }

        lines.push(LyricLine { time_ms, text });
    }

    lines.sort_by_key(|line| line.time_ms);
    lines
}

pub fn map_hana_lyric_to_payload(
    _track_id: &str,
    lrc: &str,
    tlyric: &str,
    _klyric: Option<&str>,
    yrc: Option<&str>,
) -> LyricPayload {
    let base_lines = yrc
        .map(parse_yrc_text)
        .filter(|lines| !lines.is_empty())
        .unwrap_or_else(|| parse_lrc(lrc));
    let translation_lines = parse_lrc(tlyric);

    if translation_lines.is_empty() {
        return LyricPayload {
            lines: base_lines,
            raw: if lrc.trim().is_empty() {
                None
            } else {
                Some(lrc.to_owned())
            },
        };
    }

    let translation_map = translation_lines
        .into_iter()
        .map(|line| (line.time_ms, line.text))
        .collect::<std::collections::HashMap<_, _>>();

    let lines = base_lines
        .into_iter()
        .map(|line| {
            let text = translation_map
                .get(&line.time_ms)
                .map(|translation| {
                    if translation.trim().is_empty() {
                        line.text.clone()
                    } else if line.text.trim().is_empty() {
                        translation.clone()
                    } else {
                        format!("{}\n{}", line.text, translation)
                    }
                })
                .unwrap_or_else(|| line.text.clone());
            LyricLine {
                time_ms: line.time_ms,
                text,
            }
        })
        .collect();

    LyricPayload {
        lines,
        raw: if lrc.trim().is_empty() {
            None
        } else {
            Some(lrc.to_owned())
        },
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

    PlaylistSummary {
        id,
        name: raw
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        track_count,
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
        id: summary.id,
        name: summary.name,
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
}
