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
        provider: "qq".to_owned(),
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
        playable_state: "unknown".to_owned(),
        duration_ms: raw
            .get("interval")
            .and_then(Value::as_u64)
            .map(|value| value * 1_000),
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

pub fn parse_qrc(text: &str) -> Vec<LyricLine> {
    let Ok(line_re) = Regex::new(r"\[(\d+),(\d+)\]([^\r\n]*)") else {
        return Vec::new();
    };
    let Ok(word_re) = Regex::new(r"\(\d+,\d+(?:,\d+)?\)") else {
        return Vec::new();
    };
    let mut lines = Vec::new();

    for caps in line_re.captures_iter(text) {
        let time_ms = caps
            .get(1)
            .and_then(|value| value.as_str().parse::<u64>().ok())
            .unwrap_or_default();
        let raw = caps.get(3).map(|value| value.as_str()).unwrap_or_default();
        let plain = word_re.replace_all(raw, "").trim().to_owned();
        if plain.is_empty() {
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

pub fn map_qq_lyric_to_payload(
    _track_id: &str,
    lyric: &str,
    trans: &str,
    qrc: &str,
) -> LyricPayload {
    let base_lines = {
        let lrc_lines = parse_lrc(lyric);
        if lrc_lines.is_empty() && !qrc.trim().is_empty() {
            parse_qrc(qrc)
        } else {
            lrc_lines
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

pub fn map_qq_playlist_to_summary(raw: &Value, id_hint: Option<&str>) -> PlaylistSummary {
    PlaylistSummary {
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
        track_count: first_u32(&[
            raw.get("total_song_num"),
            raw.get("song_cnt"),
            raw.get("songnum"),
            raw.get("song_count"),
        ]),
    }
}

pub fn map_qq_playlist_to_detail(raw: Option<&Value>, id_hint: Option<&str>) -> PlaylistDetail {
    let summary = map_qq_playlist_to_summary(raw.unwrap_or(&Value::Null), id_hint);
    let tracks = raw
        .and_then(|value| value.get("songlist"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(map_qq_song_to_track)
        .collect::<Vec<_>>();

    PlaylistDetail {
        id: summary.id,
        name: summary.name,
        tracks,
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
        value.as_u64().and_then(|value| u32::try_from(value).ok()).or_else(|| {
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
