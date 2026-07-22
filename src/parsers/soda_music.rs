use crate::parsers::MemchrParsers;
///汽水音乐逐字歌词解析器
pub struct SodaParser;
impl MemchrParsers for SodaParser {
    fn get_offset_time(&self, t1: u64, t2: u64) -> Result<u64, String> {
        t1.checked_add(t2)
            .ok_or_else(|| format!("add overflow {} {}", t1, t2))
    }
}
