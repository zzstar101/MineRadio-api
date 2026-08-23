#![allow(dead_code)]

use std::collections::HashMap;
use std::str::FromStr;

pub struct Cookie {
    pub map: HashMap<String, String>,
}

impl Cookie {
    pub fn new(cookie: &str) -> Self {
        let map = cookie
            .split(';')
            .filter_map(|segment| {
                let (name, value) = segment.trim().split_once('=')?;

                Some((name.trim().to_owned(), value.trim().to_owned()))
            })
            .collect();

        Self { map }
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
