use serde::{Deserialize};

use crate::types::Track;

#[derive(Debug, Deserialize)]
pub(super) struct QQSearchResp {
    data: Data,
}

impl QQSearchResp {
    pub fn standardize(self) -> Vec<Track> {
        self
        .data
        .song
        .list
        .into_iter()
        .map(|l| {  
            Track {
                id: l.songmid.clone(),
                provider: "qq".to_owned(),
                source_id: l.songmid.clone(),
                media_mid: Some(l.songmid),
                title: l.songname,
                artists: l.singer.into_iter().map(|s| s.name).collect(),
                album: l.albumname,
                cover_url: format!("https://y.gtimg.cn/music/photo_new/T002R300x300M000{}.jpg", l.albummid),
                quality_hints: vec!["standard".to_owned()],
                playable_state: "unknown".to_owned(),
                duration_ms: Some(l.interval as u64 * 1000),
                artwork_url: None,      
            }
        })
        .collect()
    }
}


#[derive(Debug, Deserialize)]
struct Singer {
    name: String,
}

#[derive(Debug, Deserialize)]
struct List {
    albummid: String,
    albumname: String,
    interval: i32,
    singer: Vec<Singer>,
    songmid: String,
    songname: String,
}

#[derive(Debug, Deserialize)]
struct Song {
    //curnum: i32,
    //curpage: i32,
    list: Vec<List>,
    //totalnum: i32
}

#[derive(Debug, Deserialize)]
struct Data {
    song: Song,
}

