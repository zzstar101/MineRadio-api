use regex::Regex;

use crate::types::LyricLine;

pub fn parse_lrc(text: &str) -> Vec<LyricLine> {
    let Ok(marker_re) = Regex::new(r"\[(\d{1,3}):(\d{1,2})(?:[.:](\d{1,3}))?\]") else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for raw_line in text.split(['\r', '\n']) {
        if raw_line.is_empty() {
            continue;
        }
        let mut marks = Vec::new();
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
            marks.push((min * 60_000 + sec * 1_000 + frac, end));
        }
        if marks.is_empty() {
            continue;
        }
        let text = raw_line
            .get(marks.last().map(|(_, end)| *end).unwrap_or_default()..)
            .unwrap_or_default()
            .trim()
            .to_owned();
        for (time_ms, _) in marks {
            out.push(LyricLine {
                time_ms,
                text: text.clone(),
                ..Default::default()
            });
        }
    }
    out
}
