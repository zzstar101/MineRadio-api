use regex::Regex;

use crate::types::LyricLine;

pub fn parse_qrc_text(text: &str) -> Vec<LyricLine> {
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
        let duration_ms = caps
            .get(2)
            .and_then(|value| value.as_str().parse::<u64>().ok());
        let plain = word_re
            .replace_all(
                caps.get(3).map(|value| value.as_str()).unwrap_or_default(),
                "",
            )
            .trim()
            .to_owned();
        if plain.is_empty() {
            continue;
        }
        lines.push(LyricLine {
            time_ms,
            duration_ms,
            text: plain,
            source: Some("qrc".to_owned()),
            ..Default::default()
        });
    }
    lines
}
