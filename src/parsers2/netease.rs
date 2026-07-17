use crate::parsers2::{MemchrParsers, lrc::*};
use memchr::{memchr, memchr2};

///网易LRC歌词解析器
pub struct NeteaseLrcParser {
    pub version: u8,
}
impl LrcParser for NeteaseLrcParser {
    fn parse_lrc_time(&self, tag: &str) -> Result<u64, String> {
        let tbytes = tag.as_bytes();

        // 找第一个 ':'
        let Some(col) = memchr(b':', tbytes) else {
            return Err(format!("时间标签缺少 ':' : {:?}", tag));
        };

        let minutes = tag[..col]
            .parse::<u64>()
            .map_err(|_| format!("parse minutes error: {}", &tag[..col]))?;

        // col 之后找 ':' 或 '.'，看哪个先出现来盲判格式
        let Some(sep) = memchr2(b':', b'.', &tbytes[col + 1..]) else {
            return Err(format!("时间标签缺少第二个分隔符: {:?}", tag));
        };
        let sep = col + 1 + sep; // 转绝对偏移

        let seconds = tag[col + 1..sep]
            .parse::<u64>()
            .map_err(|_| format!("parse seconds error: {}", &tag[col + 1..sep]))?;
        let centis = tag[sep + 1..]
            .parse::<u64>()
            .map_err(|_| format!("parse centis error: {}", &tag[sep + 1..]))?;

        // ':' → v3 毫秒直接用，'.' → v4 百分秒 *10
        match tbytes[sep] {
            b'.' => Ok(minutes * 60_000 + seconds * 1_000 + centis),
            _ => Ok(minutes * 60_000 + seconds * 1_000 + centis * 10),
        }
    }
}

///网易逐字歌词解析器
pub struct NeteaseParser;

impl MemchrParsers for NeteaseParser {
    fn syllable_delimiters(&self) -> (u8, u8) {
        (b'(', b')')
    }
}
