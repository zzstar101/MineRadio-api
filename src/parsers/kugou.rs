use crate::parsers::MemchrParsers;
use crate::types::LyricLine;
use crate::utils::cryptors::decrypt_krc;
///酷狗歌词解析器
pub struct KugouParser;
impl KugouParser {
    fn decrypt(&self, lyrics: &str) -> Result<String, String> {
        decrypt_krc(lyrics)
    }
    pub fn decrypt_and_parse(&self, lyrics: String) -> Result<Vec<LyricLine>, String> {
        let lyrics = self.decrypt(&lyrics)?;
        self.parse(lyrics)
    }
}
impl MemchrParsers for KugouParser {
    fn get_offset_time(&self, t1: u64, t2: u64) -> Result<u64, String> {
        t1.checked_add(t2)
            .ok_or_else(|| format!("add overflow {} {}", t1, t2))
    }
}
