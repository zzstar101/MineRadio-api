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

pub fn map_soda_song_to_track(raw: &Value) -> Track {
    let id = raw.get("id").map(value_to_string).unwrap_or_default();
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
        .filter(|item| !item.is_empty() && !item.eq_ignore_ascii_case("lossless"))
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
            });
        }
    }

    lines.sort_by_key(|line| line.time_ms);
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
        let body = caps.get(3).map(|value| value.as_str()).unwrap_or_default();
        let mut words = String::new();
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
            let Some(close) = body[after_open..].find('>').map(|index| after_open + index) else {
                break;
            };
            let text_start = close + 1;
            let next_open = body[text_start..]
                .find('<')
                .map(|index| text_start + index)
                .unwrap_or(body.len());
            let segment = &body[text_start..next_open];
            words.push_str(segment);
            pos = next_open;
        }

        let plain = if words.trim().is_empty() {
            word_marker_re.replace_all(body, "").trim().to_owned()
        } else {
            words
        };

        if plain.trim().is_empty() {
            continue;
        }
        lines.push(LyricLine {
            time_ms,
            text: plain,
        });
    }

    lines.sort_by_key(|line| line.time_ms);
    lines
}

pub fn map_soda_lyric_to_payload(_track_id: &str, lyric: &str, trans: &str) -> LyricPayload {
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
        .map(|line| {
            let text = translations
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
        .collect::<Vec<_>>();

    LyricPayload {
        lines,
        raw: if lyric.trim().is_empty() {
            None
        } else {
            Some(lyric.to_owned())
        },
    }
}

pub fn map_soda_playlist_to_summary(raw: &Value, id_hint: Option<&str>) -> PlaylistSummary {
    PlaylistSummary {
        id: raw
            .get("id")
            .map(value_to_string)
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
        track_count: raw
            .get("count_tracks")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok()),
    }
}

pub fn map_soda_playlist_detail_to_detail(raw: Option<&Value>, id_hint: Option<&str>) -> PlaylistDetail {
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
        })
        .map(map_soda_song_to_track)
        .collect::<Vec<_>>();

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
        .unwrap_or("tplv-b829550vbb")
        .trim();
    format!("{cdn}{uri}~{prefix}-crop-center:256:256.webp")
}

fn dedupe(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}
