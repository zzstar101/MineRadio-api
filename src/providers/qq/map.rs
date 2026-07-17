use serde_json::Value;

use crate::{
    parsers::{lrc, qq},
    types::{LyricLine, LyricPayload, PlayableState, PlaylistDetail, PlaylistSummary, Track},
};

pub fn normalize_provider_image_url(url: &str) -> String {
    let value = url.trim();
    if value.is_empty() {
        return String::new();
    }
    if let Some(stripped) = value.strip_prefix("//") {
        return format!("https://{stripped}");
    }
    value.replacen("http://", "https://", 1)
}

pub fn map_qq_song_to_track(raw: &Value) -> Track {
    let id = first_string(&[
        raw.get("songmid"),
        raw.get("mid"),
        raw.get("songid"),
        raw.get("id"),
    ]);
    let media_mid = first_string(&[
        raw.get("file").and_then(|value| value.get("media_mid")),
        raw.get("file").and_then(|value| value.get("strMediaMid")),
        raw.get("media_mid"),
        raw.get("strMediaMid"),
        raw.get("mediaMid"),
    ]);
    let artists = raw
        .get("singer")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|singer| singer.get("name").and_then(Value::as_str))
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    let artists = if artists.is_empty() {
        split_artist_text(
            raw.get("singername")
                .or_else(|| raw.get("singerName"))
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
    } else {
        artists
    };
    let album_mid = first_string(&[
        raw.get("albummid"),
        raw.get("album").and_then(|value| value.get("mid")),
        raw.get("album").and_then(|value| value.get("pmid")),
    ])
    .replace(|c: char| !c.is_ascii_alphanumeric(), "");
    let cover_url = raw
        .get("pic")
        .and_then(Value::as_str)
        .map(normalize_provider_image_url)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            if album_mid.is_empty() {
                String::new()
            } else {
                format!("https://y.gtimg.cn/music/photo_new/T002R300x300M000{album_mid}.jpg")
            }
        });

    Track {
        id: id.clone(),
        provider: "qq".to_owned(),
        source_id: id,
        media_mid: (!media_mid.is_empty()).then_some(media_mid),
        title: first_string(&[raw.get("songname"), raw.get("name"), raw.get("title")]),
        artists,
        album: first_string(&[
            raw.get("albumname"),
            raw.get("album").and_then(|value| value.get("name")),
            raw.get("album").and_then(|value| value.get("title")),
        ]),
        cover_url,
        quality_hints: vec!["standard".to_owned()],
        playable_state: PlayableState::Unknown,
        duration_ms: raw
            .get("interval")
            .and_then(Value::as_u64)
            .map(|value| value * 1_000),
        artwork_url: None,
    }
}

pub fn parse_lrc(text: &str) -> Vec<LyricLine> {
    lrc::parse_lrc(text)
}

pub fn parse_qrc(text: &str) -> Vec<LyricLine> {
    qq::parse_qrc_text(text)
}

pub fn map_qq_lyric_to_payload(
    track_id: &str,
    lyric: &str,
    trans: &str,
    qrc: &str,
    source: Option<&str>,
) -> LyricPayload {
    let mut line_source = source
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let base_lines = {
        let lrc_lines = parse_lrc(lyric);
        if lrc_lines.is_empty() && !qrc.trim().is_empty() {
            line_source = Some("qrc".to_owned());
            parse_qrc(qrc)
        } else {
            lrc_lines
        }
    };
    let translations = parse_lrc(trans)
        .into_iter()
        .map(|line| (line.time_ms, line.text))
        .collect::<std::collections::HashMap<_, _>>();
    let lines = base_lines
        .into_iter()
        .map(|mut line| {
            if let Some(source) = line_source.as_deref() {
                line.source = Some(source.to_owned());
            }
            line.translation = translations.get(&line.time_ms).cloned();
            line
        })
        .collect::<Vec<_>>();

    LyricPayload {
        provider: "qq".to_owned(),
        track_id: track_id.to_owned(),
        lines,
        has_translation: !trans.trim().is_empty() && !translations.is_empty(),
        is_word_by_word: false,
    }
}

pub fn map_qq_playlist_to_summary(raw: &Value, id_hint: Option<&str>) -> PlaylistSummary {
    PlaylistSummary {
        provider: "qq".to_owned(),
        id: {
            let id = first_string(&[
                raw.get("disstid"),
                raw.get("dissid"),
                raw.get("dirid"),
                raw.get("tid"),
                raw.get("id"),
            ]);
            if id.is_empty() {
                id_hint.unwrap_or_default().to_owned()
            } else {
                id
            }
        },
        name: first_string(&[
            raw.get("dissname"),
            raw.get("diss_name"),
            raw.get("name"),
            raw.get("title"),
        ]),
        cover_url: normalize_provider_image_url(&first_string(&[
            raw.get("logo"),
            raw.get("picurl"),
        ])),
        track_count: first_u32(&[
            raw.get("total_song_num"),
            raw.get("song_cnt"),
            raw.get("songnum"),
            raw.get("song_count"),
        ]),
        track_ids: raw
            .get("songlist")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        item.get("songmid")
                            .or_else(|| item.get("mid"))
                            .and_then(value_to_string)
                    })
                    .collect()
            })
            .unwrap_or_default(),
        collected: Some(false),
    }
}

pub fn map_qq_playlist_to_detail_official(
    raw: Option<&Value>,
    id_hint: Option<&str>,
) -> PlaylistDetail {
    let dirinfo = raw.and_then(|d| d.get("dirinfo")).unwrap_or(&Value::Null);
    let songs = raw
        .and_then(|d| d.get("songlist"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut track_ids = Vec::with_capacity(songs.len());
    let mut tracks = Vec::with_capacity(songs.len());

    for song in songs {
        let id = song
            .get("id")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .to_string();
        let album = song.get("album");
        let album_name = album
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let album_mid = album
            .and_then(|value| value.get("mid"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let singers = song
            .get("singer")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut artists = Vec::with_capacity(singers.len());
        for singer in singers {
            if let Some(name) = singer.get("name").and_then(Value::as_str) {
                artists.push(name.to_owned());
            }
        }

        track_ids.push(id.clone());
        tracks.push(Track {
            id: id.clone(),
            source_id: id,
            provider: "qq".to_owned(),
            media_mid: song
                .get("songlist")
                .and_then(Value::as_str)
                .map(str::to_owned),
            title: song
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            artists,
            album: album_name,
            cover_url: format!(
                "http://y.gtimg.cn/music/photo_new/T002R500x500M000{album_mid}.jpg?n=1"
            ),
            quality_hints: vec!["standard".to_owned()],
            playable_state: PlayableState::Unknown,
            duration_ms: song
                .get("interval")
                .and_then(Value::as_u64)
                .map(|value| value * 1_000),
            artwork_url: None,
        });
    }

    PlaylistDetail {
        provider: "qq".to_owned(),
        id: dirinfo
            .get("id")
            .and_then(Value::as_u64)
            .map(|i| i.to_string())
            .unwrap_or(id_hint.unwrap_or("").to_string()),
        name: dirinfo
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        cover_url: dirinfo
            .get("picurl")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        collected: Some(false),
        track_count: Some(dirinfo.get("songnum").and_then(Value::as_u64).unwrap_or(0) as u32),
        track_ids: track_ids,
        tracks,
    }
}

pub fn map_qq_playlist_to_detail(raw: Option<&Value>, id_hint: Option<&str>) -> PlaylistDetail {
    let summary = map_qq_playlist_to_summary(raw.unwrap_or(&Value::Null), id_hint);
    let tracks = raw
        .and_then(|value| value.get("songlist"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(map_qq_song_to_track)
        .collect::<Vec<_>>();

    PlaylistDetail {
        provider: summary.provider,
        id: summary.id,
        name: summary.name,
        cover_url: summary.cover_url,
        track_count: summary.track_count,
        track_ids: summary.track_ids,
        collected: summary.collected,
        tracks,
    }
}

fn first_string(values: &[Option<&Value>]) -> String {
    values
        .iter()
        .copied()
        .flatten()
        .find_map(value_to_string)
        .unwrap_or_default()
}

fn first_u32(values: &[Option<&Value>]) -> Option<u32> {
    values.iter().copied().flatten().find_map(|value| {
        value
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .or_else(|| {
                value
                    .as_i64()
                    .and_then(|value| u64::try_from(value).ok())
                    .and_then(|value| u32::try_from(value).ok())
            })
    })
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) if !value.trim().is_empty() => Some(value.trim().to_owned()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn split_artist_text(text: &str) -> Vec<String> {
    text.split(['/', ',', '，', '、'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::map_qq_playlist_to_detail_official;

    #[test]
    fn official_playlist_maps_tracks_and_track_ids_in_songlist_order() {
        let raw = json!({
            "dirinfo": {
                "id": 123,
                "title": "官方歌单",
                "picurl": "https://example.com/playlist.jpg",
                "songnum": 2
            },
            "songlist": [
                {
                    "id": 1,
                    "songlist": "media-one",
                    "title": "歌曲一",
                    "singer": [{ "name": "歌手一" }],
                    "album": { "name": "专辑一", "mid": "album-one" },
                    "interval": 180
                },
                {
                    "id": 2,
                    "title": "歌曲二",
                    "singer": [],
                    "album": { "name": "专辑二", "mid": "album-two" }
                }
            ]
        });

        let detail = map_qq_playlist_to_detail_official(Some(&raw), Some("fallback"));

        assert_eq!(detail.id, "123");
        assert_eq!(detail.track_ids, ["1", "2"]);
        assert_eq!(detail.tracks.len(), 2);
        assert_eq!(detail.tracks[0].source_id, "1");
        assert_eq!(detail.tracks[0].media_mid.as_deref(), Some("media-one"));
        assert_eq!(detail.tracks[0].artists, ["歌手一"]);
        assert_eq!(detail.tracks[0].duration_ms, Some(180_000));
        assert_eq!(
            detail.tracks[1].cover_url,
            "http://y.gtimg.cn/music/photo_new/T002R500x500M000album-two.jpg?n=1"
        );
    }
}
