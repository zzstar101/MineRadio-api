pub mod lrc;
pub mod netease;
pub mod qqmusic;
pub mod soda_music;

use crate::types::LyricLine;

pub(crate) fn finalize_lyric_line_durations(mut lines: Vec<LyricLine>) -> Vec<LyricLine> {
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
            current.duration_ms = Some(duration.clamp(450, 12_000));
            current.char_count = Some(
                current
                    .char_count
                    .unwrap_or_else(|| utf16_len(&current.text).max(1)),
            );
        }
    }
    lines
}

pub(crate) fn utf16_len(value: &str) -> usize {
    value.encode_utf16().count()
}
