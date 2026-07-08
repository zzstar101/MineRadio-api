# Pending Service Migration

This file tracks service modules that are not fully migrated yet, or are intentionally paused
because strict migration needs missing external capabilities.

## Blocked By `hana-music-api`

### `discover-home.ts` -> `src/services/discover_home.rs`

Status: placeholder.

Rust file exists, but `build_discover_home` is not implemented.

Needs equivalent capabilities for these `hana-music-api` functions:

- `personalized(params)` for public recommended playlists.
- `djHot(params)` for hot podcast/radio recommendations.
- `recommendResource(params)` for logged-in recommended playlists.
- `recommendSongs(params)` for logged-in daily songs.

Also depends on:

- provider adapter `login_status`, `playlist_list`, `playlist_detail`, and `search`
- podcast service `hot`
- Netease auth cookie lookup

### `podcast.ts` -> `src/services/podcast.rs`

Status: placeholder.

Rust file exists, but main methods still return not implemented and mappers return placeholder
values.

Needs equivalent capabilities for these `hana-music-api` functions:

- `cloudsearch(params)` with `type: 1009` for podcast search.
- `djHot(params)` for hot podcasts.
- `djDetail(params)` for podcast detail.
- `djProgram(params)` for podcast programs.
- `djSublist(params)` for collected podcasts.
- `userAudio(params)` for user-created podcasts.
- `djPaygift(params)` for paid podcast collections.
- `recordRecentVoice(params)` for liked/recent voice items.

Also depends on local analyzer capability currently loaded by TS from:

- `../../../../dj-analyzer.js`
- `analyzePodcastDjStream(url, { durationSec, userAgent })`
- `analyzePodcastDjIntro(url, { durationSec, introSec, userAgent })`

## Large Remaining Service

### `shared-playlist-import.ts` -> `src/services/shared_playlist_import.rs`

Status: partially migrated.

Rust now supports:

- shared link detection for QQ, Netease, Apple Music, Qishui, and Kugou
- adapter-backed playlist import for QQ / Netease / Soda
- `/shared-playlist/import` route wiring

Still missing:

- Apple Music HTML/JSON-LD parsing and iTunes lookup enrichment
- Qishui rendered/JSON track parsing
- Kugou share/API parsing, signing, and normalization
- import-only track generation

Suggested migration split:

- First: `detect_shared_playlist` and adapter-backed QQ/Netease/Soda import. Done.
- Second: Apple Music import.
- Third: Qishui and Kugou import helpers.

## Needs Final Wiring

These services have meaningful Rust implementations but still need route wiring and broader API
integration checks:

- `audio_proxy.rs`
- `image_proxy.rs`
- `soda_audio_proxy.rs`
- QR login services
- `cross_source_resolver.rs`
- `weather_radio.rs`

## Completed Service Chunks

Committed chunks so far:

- `feat: port service interfaces`
- `feat: port qr login services`
- `feat: port soda audio proxy`
- `feat: port cross source resolver`
- `feat: port weather radio service`
