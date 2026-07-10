# Pending Service Migration

This file tracks the remaining migration work based on the current Rust codebase state.

## Current Status

Most major sidecar services are now migrated and wired into the Rust router.

Recently completed chunks:

- `feat: wire Netease QR login and podcast routes`
- `feat: complete discover home service flow`
- `feat: complete shared playlist import external providers`
- `feat: migrate podcast dj beatmap analyzer to Rust`

## Completed Service Migrations

These service flows now have meaningful Rust implementations and route wiring:

- `discover-home.ts` -> `src/services/discover_home.rs`
- `podcast.ts` -> `src/services/podcast.rs`
- `shared-playlist-import.ts` -> `src/services/shared_playlist_import.rs`
- `audio-proxy.ts` -> `src/services/audio_proxy.rs`
- `image-proxy.ts` -> `src/services/image_proxy.rs`
- `soda-audio-proxy.ts` -> `src/services/soda_audio_proxy.rs`
- `cross-source-resolver.ts` -> `src/services/cross_source_resolver.rs`
- `weather-radio.ts` -> `src/services/weather_radio.rs`
- Netease QR login wiring
- QQ Music provider core flows
- Soda provider core flows
- Netease provider core flows

## Podcast

### `podcast.ts` -> `src/services/podcast.rs`

Status: migrated and wired.

Rust now supports:

- `search`
- `hot`
- `detail`
- `programs`
- `my`
- `myItems`
- `djBeatmap`
- Rust-side beatmap analyzer in `src/utils/podcast_analyzer.rs`

Remaining work:

- run end-to-end validation against real podcast audio URLs
- compare Rust beatmap output with the historical JS analyzer output on representative samples
- confirm long-audio fallback behavior under real network conditions

## Discover Home

### `discover-home.ts` -> `src/services/discover_home.rs`

Status: migrated and wired.

Rust now provides:

- logged-in / logged-out discover home composition
- Netease recommendation requester integration
- provider-backed playlist fallback behavior
- podcast hot fallback behavior

Remaining work:

- broader API integration verification with real provider/session states

## Shared Playlist Import

### `shared-playlist-import.ts` -> `src/services/shared_playlist_import.rs`

Status: core migration completed.

Rust now supports:

- shared link detection for QQ, Netease, Apple Music, Qishui, and Kugou
- adapter-backed playlist import for QQ / Netease / Soda
- Apple Music public metadata import
- Kugou shared playlist parsing and normalization
- import-only track generation
- `/shared-playlist/import` route wiring

Paused / intentionally not continuing:

- Qishui import completion is intentionally paused because Soda already has a full provider path

Remaining work:

- verify Apple Music and Kugou flows against real public share links
- confirm front-end compatibility of import-only track payloads

## Integration Verification Still Worth Doing

These services are wired, but still deserve broader real-world verification:

- `audio_proxy.rs`
- `image_proxy.rs`
- `soda_audio_proxy.rs`
- `cross_source_resolver.rs`
- `weather_radio.rs`

## Known Remaining TODOs

- `src/utils/cryptors/netease.rs`
  - add an EAPI decompressed-size limit after confirming the real payload size range

## Suggested Next Steps

1. Run real end-to-end checks for `podcast/dj-beatmap`.
2. Run real share-link checks for Apple Music and Kugou import.
3. Backfill targeted regression tests where current coverage is still mostly unit-level.
