use regex::Regex;

use crate::{
    parsers::{finalize_lyric_line_durations, utf16_len},
    types::{LyricLine, LyricWord},
};

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
            let Some(open_rel) = body[cursor..].find('<') else {
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
            let Some(comma1_rel) = body[after_open..].find(',') else {
                break;
            };
            let comma1 = after_open + comma1_rel;
            let raw_start = body[after_open..comma1].parse::<u64>().unwrap_or_default();
            let Some(close_rel) = body[after_open..].find('>') else {
                break;
            };
            let close = after_open + close_rel;
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
                    time_ms: line_start_ms + raw_start,
                    duration_ms: Some(raw_duration),
                    c0,
                    c1: utf16_len(&full_text),
                });
            }
            cursor = next_open;
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
            time_ms: line_start_ms,
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
