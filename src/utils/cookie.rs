#![allow(dead_code)]

use std::collections::HashMap;
use std::str::FromStr;

pub struct Cookie {
    pub map: HashMap<String, String>,
}

impl Cookie {
    pub fn new(cookie: &str) -> Self {
        Self {
            map: Self::parse(cookie),
        }
    }

    /// 插入或覆盖一个键值对。
    ///
    /// 用于登录迁移时收集各环节下发的 Cookie: 后写的覆盖先写的,
    /// 让服务器返回的真实值盖掉本地生成的占位随机值。
    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.map.insert(name.into(), value.into());
    }

    /// 按 ';' 分段收集键值对。
    ///
    /// - 值原样保存, 不做任何 percent 解码, 保证序列化回去时与原始输入一致
    /// - 引号感知: 双引号内的 ';' 不作为分隔符, 避免 `k="v1;v2"` 被错切
    /// - 无 '=' 的段(如 HttpOnly 属性)与空键名段丢弃; 空值段(`k=`)保留
    fn parse(cookie: &str) -> HashMap<String, String> {
        let mut map = HashMap::new();

        for segment in split_segments(cookie) {
            let segment = segment.trim();
            let Some((name, value)) = segment.split_once('=') else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            map.insert(name.to_owned(), value.trim().to_owned());
        }

        map
    }

    pub fn find<T>(&self, key: &str) -> Option<T>
    where
        T: FromStr,
    {
        self.map.get(key).and_then(|value| value.parse::<T>().ok())
    }

    pub fn find_or_default<T>(&self, key: &str) -> T
    where
        T: FromStr + Default,
    {
        self.find(key).unwrap_or_default()
    }

    pub fn find_or<T>(&self, key: &str, default: T) -> T
    where
        T: FromStr,
    {
        self.find(key).unwrap_or(default)
    }

    pub fn find_or_else<T, F>(&self, key: &str, default: F) -> T
    where
        T: FromStr,
        F: FnOnce() -> T,
    {
        self.find(key).unwrap_or_else(default)
    }

    pub fn first<T>(&self, keys: &[&str]) -> Option<T>
    where
        T: FromStr,
    {
        keys.iter().find_map(|key| self.find(key))
    }

    pub fn first_or_default<T>(&self, keys: &[&str]) -> T
    where
        T: FromStr + Default,
    {
        self.first(keys).unwrap_or_default()
    }

    pub fn first_or<T>(&self, keys: &[&str], default: T) -> T
    where
        T: FromStr,
    {
        self.first(keys).unwrap_or(default)
    }

    pub fn first_or_else<T, F>(&self, keys: &[&str], default: F) -> T
    where
        T: FromStr,
        F: FnOnce() -> T,
    {
        self.first(keys).unwrap_or_else(default)
    }
}

/// 按 ';' 分段, 但跳过双引号内的 ';'; 段内容原样返回
fn split_segments(cookie: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut in_quotes = false;
    let mut start = 0;

    for (idx, ch) in cookie.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ';' if !in_quotes => {
                segments.push(&cookie[start..idx]);
                start = idx + 1;
            }
            _ => {}
        }
    }

    segments.push(&cookie[start..]);
    segments
}

/// 序列化回 Cookie 头格式: `key=val; key2=val2`。
///
/// HashMap 遍历顺序不定, 这里按键名排序保证同一份数据输出稳定。
/// 值不做任何转码, 与解析时的原样保存对应。
impl From<Cookie> for String {
    fn from(cookie: Cookie) -> String {
        let mut keys: Vec<_> = cookie.map.keys().collect();
        keys.sort();

        keys.into_iter()
            .map(|name| format!("{name}={}", cookie.map[name]))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 已编码的 ';'(%3B) 不应被切开, 且往返保持一致
    #[test]
    fn percent_encoded_value_roundtrips() {
        let raw = "token=a%3Bb%3Dc; n=1";
        let cookie = Cookie::new(raw);
        assert_eq!(cookie.map.get("token").unwrap(), "a%3Bb%3Dc");
        let out: String = Cookie::new(raw).into();
        assert_eq!(out, "n=1; token=a%3Bb%3Dc");
    }

    /// 双引号内的 ';' 不作为分隔符
    #[test]
    fn quoted_semicolon_survives() {
        let cookie = Cookie::new(r#"k="v1;v2"; j=2"#);
        assert_eq!(cookie.map.get("k").unwrap(), r#""v1;v2""#);
        assert_eq!(cookie.find_or::<i32>("j", 0), 2);
    }

    /// base64 的 '=' 填充留在值里, 只有第一个 '=' 是分隔符
    #[test]
    fn base64_padding_kept() {
        let cookie = Cookie::new("t=abc==; x=1");
        assert_eq!(cookie.map.get("t").unwrap(), "abc==");
    }

    /// insert 后写覆盖先写(登录迁移场景: 服务器真实值盖掉本地随机值)
    #[test]
    fn insert_overrides() {
        let mut cookie = Cookie::new("MUSIC_U=rand; NMTID=rand");
        cookie.insert("NMTID", "server_value");
        let out: String = cookie.into();
        assert_eq!(out, "MUSIC_U=rand; NMTID=server_value");
    }

    /// 无 '=' 的属性段丢弃, 空值段保留
    #[test]
    fn attribute_segments_dropped() {
        let cookie = Cookie::new("a=1; HttpOnly; Path=/; b=");
        assert_eq!(cookie.map.get("a").unwrap(), "1");
        assert!(!cookie.map.contains_key("HttpOnly"));
        assert_eq!(cookie.map.get("b").unwrap(), "");
    }
}
