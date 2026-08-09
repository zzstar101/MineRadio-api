use crate::types::LyricLine;
use crate::types::LyricWord;
use memchr::memchr;

///逐字歌词解析器
pub trait MemchrParsers {
    fn label(&self) -> String {
        "official".to_string()
    }

    fn syllable_delimiters(&self) -> (u8, u8) {
        (b'<', b'>')
    }
    //默认逐字行开始也是全局时长 但是汽水行开始是偏移行开始时长
    #[allow(unused_variables)]
    fn get_offset_time(&self, t1: u64, t2: u64) -> Result<u64, String> {
        Ok(t1)
    }
    fn parse(&self, lyrics: String) -> Result<Vec<LyricLine>, String> {
        self.parse_without_st(lyrics)
    }
    fn parse_syllables(
        &self,
        l: u64,
        c0: usize,
        content: &str,
    ) -> Result<(Vec<LyricWord>, String, usize), String> {
        let (left_delimiter, right_delimiter) = self.syllable_delimiters();
        let cbytes = content.as_bytes();
        let clen = cbytes.len();
        let mut cpos = 0;
        let mut result: Vec<LyricWord> = Vec::new();
        let mut line = String::new();
        let mut char_count = 0usize;
        let mut c0 = c0;
        while cpos < clen {
            // 找 '<'
            let Some(la) = memchr(left_delimiter, &cbytes[cpos..]) else {
                break;
            };

            let after_la = cpos + la + 1;
            if after_la >= clen || !cbytes[after_la].is_ascii_digit() {
                cpos += la + 1;
                continue;
            }
            cpos += la + 1;

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
            let time_ms = self.get_offset_time(time_ms, l)?;
            cpos += c1 + 1;

            // d1，兼容 <s,d> 和 <s,d,x>
            let next_comma = memchr(b',', &cbytes[cpos..]).map(|x| cpos + x);
            let next_angle = memchr(right_delimiter, &cbytes[cpos..]).map(|x| cpos + x);
            let d1_end = match (next_comma, next_angle) {
                (Some(nc), Some(na)) => nc.min(na),
                (Some(nc), None) => nc,
                (None, Some(na)) => na,
                (None, None) => break,
            };
            let duration_ms = content[cpos..d1_end].parse::<u64>().ok();

            // 跳到 '>' 后面
            let Some(ra) = memchr(right_delimiter, &cbytes[cpos..]) else {
                break;
            };
            cpos += ra + 1;

            // 文字在 '>' 到下一个 '<' 之间
            let text_end = memchr(left_delimiter, &cbytes[cpos..])
                .map(|x| cpos + x)
                .unwrap_or(clen);
            let text_raw = content[cpos..text_end].to_string();
            cpos = text_end;
            let dc = text_raw.encode_utf16().count();
            let word_c0 = c0;
            c0 += dc;
            char_count += dc;
            line.push_str(&text_raw);
            result.push(LyricWord {
                time_ms,
                duration_ms,
                text: Some(text_raw),
                c0: word_c0,
                c1: c0,
            });
        }

        Ok((result, line, char_count))
    }

    fn parse_without_st(&self, lyrics: String) -> Result<Vec<LyricLine>, String> {
        let mut lineinfo: Vec<LyricLine> = Vec::new();
        let src = lyrics.as_bytes();
        let len = src.len();
        let mut pos = 0;
        let mut c0 = 0;
        while pos < len {
            // 1. 找 '['
            let Some(lb) = memchr(b'[', &src[pos..]) else {
                break;
            };
            pos += lb + 1;

            // 2. tag1 必须是纯数字，否则跳过整个 [...]
            let Some(cm) = memchr(b',', &src[pos..]) else {
                break;
            };
            let tag1_str = &lyrics[pos..pos + cm];
            if !tag1_str.bytes().all(|b| b.is_ascii_digit()) {
                if let Some(rb) = memchr(b']', &src[pos..]) {
                    pos += rb + 1;
                } else {
                    break;
                }
                continue;
            }
            let time_ms = tag1_str
                .parse::<u64>()
                .map_err(|_| format!("parse time tag error: {}", tag1_str))?;
            pos += cm + 1;

            // 3. tag2 → d
            let Some(rb) = memchr(b']', &src[pos..]) else {
                break;
            };
            let duration_ms = lyrics[pos..pos + rb].parse::<u64>().ok();
            pos += rb + 1;

            // 4. content 到下一个 '[' 或末尾
            let content_end = memchr(b'[', &src[pos..]).map(|x| pos + x).unwrap_or(len);
            let content = lyrics[pos..content_end].trim();
            pos = content_end;
            let (words, text, char_count) = self.parse_syllables(time_ms, c0, content)?;
            c0 += char_count;
            lineinfo.push(LyricLine {
                time_ms,
                duration_ms,
                text,
                char_count: Some(char_count),
                translation: None,
                words: Some(words),
                source: Some(self.label()),
            });
        }

        Ok(lineinfo)
    }
}

pub struct UniversalLrcParser;
impl LrcParser for UniversalLrcParser {}

pub trait LrcParser {
    fn parse_lrc_time(&self, tag: &str) -> Result<u64, String> {
        let tag = tag.trim();
        let (minutes_str, rest) = tag
            .split_once(':')
            .ok_or_else(|| format!("时间标签缺少 ':' : {:?}", tag))?;
        let (seconds_str, centis_str) = rest
            .split_once('.')
            .ok_or_else(|| format!("时间标签缺少 '.' : {:?}", tag))?;

        if minutes_str.is_empty() || seconds_str.is_empty() || centis_str.is_empty() {
            return Err(format!("时间标签不完整: {:?}", tag));
        }

        let minutes = minutes_str
            .parse::<u64>()
            .map_err(|_| format!("parse minutes_str error: {}", minutes_str))?;
        let seconds = seconds_str
            .parse::<u64>()
            .map_err(|_| format!("parse seconds_str error: {}", seconds_str))?;
        let centis = centis_str
            .parse::<u64>()
            .map_err(|_| format!("parse centis_str error: {}", centis_str))?;

        Ok(minutes * 60_000 + seconds * 1_000 + centis * 10)
    }

    fn parse(&self, lyrics: String) -> Result<Vec<LyricLine>, String> {
        self.parse_without_st(lyrics)
    }

    fn parse_without_st(&self, lyrics: String) -> Result<Vec<LyricLine>, String> {
        let mut lineinfo: Vec<LyricLine> = Vec::new();
        let len = lyrics.len();
        let cbytes = lyrics.as_bytes();
        let mut c = 0;

        while c < len {
            let Some(lb) = memchr(b'[', &cbytes[c..]) else {
                break;
            };
            c += lb + 1;

            if c >= len || !cbytes[c].is_ascii_digit() {
                if let Some(rb) = memchr(b']', &cbytes[c..]) {
                    c += rb + 1;
                } else {
                    break;
                }
                continue;
            }

            let Some(rb) = memchr(b']', &cbytes[c..]) else {
                break;
            };
            let tag = &lyrics[c..c + rb];
            let time_ms = self.parse_lrc_time(tag)?;
            c += rb + 1;

            let content_end = memchr(b'[', &cbytes[c..]).map(|x| c + x).unwrap_or(len);
            let text = lyrics[c..content_end]
                .trim_matches(|ch| ch == '\r' || ch == '\n')
                .to_string();
            c = content_end;

            lineinfo.push(LyricLine {
                time_ms,
                duration_ms: None,
                char_count: Some(text.clone().encode_utf16().count()),
                text,
                words: None,
                translation: None,
                source: None,
            });
        }

        Ok(lineinfo)
    }
}

#[cfg(test)]
mod tests {
    use super::MemchrParsers;

    struct TestParser;

    impl MemchrParsers for TestParser {}

    #[test]
    fn word_character_ranges_continue_across_lines() {
        let lines = TestParser
            .parse("[0,100]<0,50>你<50,50>好[100,100]<0,50>A<50,50>😀".to_string())
            .unwrap();

        let ranges: Vec<(usize, usize)> = lines
            .iter()
            .flat_map(|line| line.words.as_ref().unwrap())
            .map(|word| (word.c0, word.c1))
            .collect();

        assert_eq!(ranges, vec![(0, 1), (1, 2), (2, 3), (3, 5)]);
        assert_eq!(lines[0].char_count, Some(2));
        assert_eq!(lines[1].char_count, Some(3));
    }

    use super::LrcParser;

    struct Dummy;
    impl LrcParser for Dummy {}

    #[test]
    fn parse_lrc_time_rejects_bad_input() {
        let parser = Dummy;
        assert!(parser.parse_lrc_time("not-a-time").is_err());
        assert!(parser.parse_lrc_time("00:01").is_err());
        assert!(parser.parse_lrc_time("00:01.").is_err());
        assert!(parser.parse_lrc_time("00:xx.10").is_err());
    }

    #[test]
    fn parse_lrc_time_accepts_valid_input() {
        let parser = Dummy;
        assert_eq!(parser.parse_lrc_time("01:02.03").unwrap(), 62_030);
    }
}
