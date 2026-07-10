use regex::Regex;

use crate::{
    parsers::{finalize_lyric_line_durations, utf16_len},
    types::{LyricLine, LyricWord},
};

pub fn parse_yrc_text(text: &str) -> Vec<LyricLine> {
    let Ok(line_re) = Regex::new(r"^\[(\d+),(\d+)\](.*)$") else {
        return Vec::new();
    };

    let mut lines = Vec::new();
    for raw_line in text.lines() {
        let Some(caps) = line_re.captures(raw_line) else {
            continue;
        };
        let line_start_ms = caps
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
        let mut cursor = 0usize;
        while cursor < body.len() {
            let Some(open_rel) = body[cursor..].find('(') else {
                break;
            };
            let open = cursor + open_rel;
            let after_open = open + 1;
            if body
                .as_bytes()
                .get(after_open)
                .map(|byte| byte.is_ascii_digit())
                != Some(true)
            {
                cursor = after_open;
                continue;
            }
            let Some(close_rel) = body[after_open..].find(')') else {
                break;
            };
            let close = after_open + close_rel;
            let marker = &body[after_open..close];
            let mut parts = marker.split(',');
            let raw_start = parts
                .next()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or_default();
            let raw_duration = parts
                .next()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or_default();
            if parts.next().is_none() {
                cursor = close + 1;
                continue;
            }
            let text_start = close + 1;
            let next_open = body[text_start..]
                .find('(')
                .map(|index| text_start + index)
                .unwrap_or(body.len());
            let fragment = body[text_start..next_open].replace(char::is_whitespace, " ");
            if !fragment.is_empty() {
                let abs_start = if raw_start >= line_start_ms.saturating_sub(500) {
                    raw_start
                } else {
                    line_start_ms + raw_start
                };
                let c0 = utf16_len(&full_text);
                full_text.push_str(&fragment);
                words.push(LyricWord {
                    text: Some(fragment),
                    time_ms: abs_start,
                    duration_ms: Some(raw_duration.max(60)),
                    c0,
                    c1: utf16_len(&full_text),
                });
            }
            cursor = next_open;
        }

        if full_text.is_empty() {
            let Ok(marker_re) = Regex::new(r"\(\d+,\d+,\d+\)") else {
                continue;
            };
            full_text = marker_re.replace_all(body, "").to_string();
        }

        let leading = full_text
            .chars()
            .take_while(|ch| ch.is_whitespace())
            .count();
        let text = full_text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            continue;
        }

        if !words.is_empty() {
            words = words
                .into_iter()
                .map(|word| {
                    let c0 = word.c0.saturating_sub(leading).min(utf16_len(&text));
                    let c1 = word.c1.saturating_sub(leading).min(utf16_len(&text));
                    LyricWord {
                        c0,
                        c1: c1.max(c0),
                        ..word
                    }
                })
                .filter(|word| word.c1 > word.c0)
                .collect();
        }

        lines.push(LyricLine {
            time_ms: line_start_ms,
            duration_ms: Some(line_duration_ms),
            text: text.clone(),
            source: Some(if words.is_empty() {
                "yrc-line".to_owned()
            } else {
                "yrc-word".to_owned()
            }),
            words: (!words.is_empty()).then_some(words),
            char_count: Some(utf16_len(&text).max(1)),
            ..Default::default()
        });
    }

    finalize_lyric_line_durations(lines)
}
