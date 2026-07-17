use crate::parsers2::{MemchrParsers, lrc::*};
use crate::types::LyricLine;
use crate::types::LyricWord;
use crate::utils::cryptors::decrypt_qrc;
use memchr::memchr;
///QQ音乐LRC歌词解析器
pub struct QQMusicLrcParser;
impl LrcParser for QQMusicLrcParser {}
///QQ音乐逐字歌词解析器
pub struct QQMusicParser;
impl QQMusicParser {
    fn decrypt(&self, lyrics: &str) -> Result<String, String> {
        decrypt_qrc(lyrics)
    }
    pub fn decrypt_and_parse(&self, lyrics: String) -> Result<Vec<LyricLine>, String> {
        let lyrics = self.decrypt(&lyrics)?;
        self.parse(lyrics)
    }
}
impl MemchrParsers for QQMusicParser {
    fn parse_syllables(
        &self,
        _l: u64,
        c0: usize,
        content: &str,
    ) -> Result<(Vec<LyricWord>, String, usize), String> {
        let cbytes = content.as_bytes();
        let clen = cbytes.len();
        let mut cpos = 0;
        let mut result: Vec<LyricWord> = Vec::new();
        let mut line = String::new();
        let mut char_count = 0usize;
        while cpos < clen {
            let Some(lp) = memchr(b'(', &cbytes[cpos..]) else {
                break;
            };

            let after_lp = cpos + lp + 1;
            if after_lp >= clen || !cbytes[after_lp].is_ascii_digit() {
                cpos += lp + 1;
                continue;
            }

            let text_raw = content[cpos..cpos + lp].to_string();
            cpos += lp + 1;

            // s1
            let Some(c1) = memchr(b',', &cbytes[cpos..]) else {
                break;
            };
            let time_ms = content[cpos..cpos + c1].parse::<u64>().map_err(|e| {
                format!(
                    "s1 parse error: {:?} raw={:?}",
                    e,
                    &content[cpos..cpos + c1]
                )
            })?;
            cpos += c1 + 1;

            //  (s,d,x)
            let next_comma = memchr(b',', &cbytes[cpos..]).map(|x| cpos + x);
            let next_paren = memchr(b')', &cbytes[cpos..]).map(|x| cpos + x);
            let d1_end = match (next_comma, next_paren) {
                (Some(nc), Some(np)) => nc.min(np),
                (Some(nc), None) => nc,
                (None, Some(np)) => np,
                (None, None) => break,
            };
            let duration_ms = content[cpos..d1_end].parse::<u64>().ok();

            let Some(rp) = memchr(b')', &cbytes[cpos..]) else {
                break;
            };
            cpos += rp + 1;
            let dc = text_raw.encode_utf16().count();
            char_count += dc;
            line.push_str(&text_raw);
            result.push(LyricWord {
                time_ms,
                duration_ms,
                text: Some(text_raw),
                c0,
                c1: c0 + dc,
            });
        }

        Ok((result, line, char_count))
    }
}
