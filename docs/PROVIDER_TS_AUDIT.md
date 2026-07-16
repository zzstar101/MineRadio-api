# Remaining Provider TS Compatibility Gaps

Source of truth:
- TS root: `D:\project\Rust\Mineradio-Tauri\sidecars\api\src\providers`
- Rust root: `D:\project\Rust\MineRadio-api\src\providers`

Rule for this audit:
- Only record items that were directly checked against TS.
- Do not infer missing fields from route schemas when the TS provider code does not populate them.
- Fixes must follow TS provider `client`, `adapter`, and `map` behavior first, then adapt to Rust style.

## 1. Netease
- `[ ]` Lyric line structure is not aligned.
  - TS: `map.ts:128`, `map.ts:182`
  - Rust: `map.rs:142`, `map.rs:197`
  - TS `parseYrcText` emits `durationMs`, `charCount`, `source`, and `words[]`.
  - Rust `parse_yrc_text` flattens YRC to plain `time_ms + text` and drops all word timing metadata.
- `[ ]` Lyric payload shape is not aligned.
  - TS: `map.ts:182`
  - Rust: `map.rs:197`
  - TS returns `provider`, `trackId`, `lines[].translation`, `hasTranslation`, `isWordByWord`.
  - Rust merges translation text into `text`, returns `raw`, and omits provider/track flags.
- `[ ]` Playlist summary/detail mapping is missing TS fields.
  - TS: `map.ts:213`, `map.ts:241`
  - Rust: `map.rs:258`, `map.rs:281`
  - TS returns `provider`, `coverUrl`, `trackCount`, `trackIds`, `subscribed`, `tracks`.
  - Rust only keeps `id`, `name`, `track_count`, and `tracks`.
- `[ ]` Song URL result is much smaller than TS.
  - TS: `netease-adapter.ts:523`
  - Rust: `adapter.rs:112`
  - TS may return `trial`, `playable`, `level`, `br`, `requestedQuality`, VIP metadata, `restriction`, `reason`, `message`.
  - Rust only returns `url`, `quality`, `expires_at`.
- `[ ]` Trial handling is behaviorally different.
  - TS: `netease-adapter.ts:555` onward
  - Rust: `adapter.rs:134` onward
  - TS can return a trial playback result with restriction metadata.
  - Rust skips non-`playable` states and eventually raises an error.
- `[ ]` Quality availability shape is not aligned.
  - TS: `netease-adapter.ts:619`
  - Rust: `adapter.rs:170`
  - TS returns structured `TrackQualityAvailability` with `provider`, `trackId`, `defaultQuality`, and rich quality options.
  - Rust returns only `Vec<String>`.
- `[ ]` Mutation ack shapes are not aligned.
  - TS: `netease-adapter.ts:691`, `netease-adapter.ts:704`, `netease-adapter.ts:755`
  - Rust: `adapter.rs:283`, `adapter.rs:292`, `adapter.rs:333`
  - TS returns:
    - `SongLikeAck { provider, id, liked, code }`
    - `SongLikeCheckAck { provider, ids, liked }`
    - `PlaylistAddSongAck { provider, playlistId, trackId, success, code }`
  - Rust returns reduced variants only.

## 2. QQ
- `[ ]` Default search path is different.
  - TS: `qq-adapter.ts:835` onward
  - Rust: `adapter.rs:50`
  - TS default deps always use `smartboxSearch` first.
  - Rust tries `search` first and only falls back to smartbox when the first result is empty or errors.
- `[ ]` Song URL result is much smaller than TS.
  - TS: `qq-adapter.ts:848`
  - Rust: `adapter.rs:65`
  - TS returns `provider`, `proxied`, `trial`, `playable`, `level`, `quality`, `filename`, `requestedQuality`.
  - Rust returns only `url`, `quality`, `expires_at`.
- `[ ]` Song URL error metadata is not aligned.
  - TS: `qq-adapter.ts:761`
  - Rust: `adapter.rs:469`
  - TS attaches `playbackKeyReady`, `reason`, `qqCode`, `rawMessage`, `tried`, and a structured `restriction`.
  - Rust maps to a plain provider error without those TS fields.
- `[ ]` Quality availability shape is not aligned.
  - TS: `qq-adapter.ts:941`
  - Rust: `adapter.rs:128`
  - TS returns structured quality options with `size`, `format`, `short`, `detail`, and top-level `provider/trackId/defaultQuality`.
  - Rust returns only quality id strings.
- `[ ]` QRC parsing is not aligned.
  - TS: `map.ts:153`
  - Rust: `map.rs:141`
  - TS keeps `durationMs` and `source: "qrc"`.
  - Rust drops both.
- `[ ]` Lyric payload shape is not aligned.
  - TS: `map.ts:175`
  - Rust: `map.rs:170`
  - TS returns `provider`, `trackId`, separate `translation`, `hasTranslation`, `isWordByWord: false`, and optional `source`.
  - Rust merges translation into `text` and returns `raw`.
- `[ ]` Playlist summary/detail mapping is missing TS fields.
  - TS: `map.ts:209`, `map.ts:246`
  - Rust: `map.rs:220`, `map.rs:251`
  - TS returns `provider`, `coverUrl`, `trackCount`, `trackIds`, `subscribed`, `tracks`.
  - Rust only keeps `id`, `name`, `track_count`, and `tracks`.
- `[ ]` Playlist add ack shape is not aligned.
  - TS: `qq-adapter.ts:1032`
  - Rust: `adapter.rs:301`
  - TS returns `{ provider, playlistId, trackId, success, code }`.
  - Rust returns only `{ playlist_id, track_id }`.

## 3. Soda
- `[ ]` Lyric line structure is not aligned.
  - TS: `map.ts:156`, `map.ts:244`
  - Rust: `map.rs:126`, `map.rs:172`
  - TS `parseSodaLyricText` emits `durationMs`, `charCount`, `source`, and `words[]`.
  - Rust flattens the line to plain text and drops word timing metadata.
- `[ ]` Lyric payload shape is not aligned.
  - TS: `map.ts:244`
  - Rust: `map.rs:172`
  - TS returns `provider`, `trackId`, `lines[].translation`, `hasTranslation`, `isWordByWord`.
  - Rust merges translation into `text` and returns `raw`.
- `[ ]` Playlist summary/detail mapping is missing TS fields.
  - TS: `map.ts:266`, `map.ts:280`, `map.ts:300`
  - Rust: `map.rs:238`, `map.rs:260`
  - TS returns `provider`, `coverUrl`, `trackCount`, `trackIds`, `subscribed`, `tracks`.
  - Rust only keeps `id`, `name`, `track_count`, and `tracks`.
- `[ ]` Song URL result is much smaller than TS.
  - TS: `soda-adapter.ts:314`
  - Rust: `adapter.rs:84`
  - TS returns `provider`, `proxied`, `trial`, `playable`, `level`, `quality`, `filename`.
  - Rust returns only `url`, `quality`, `expires_at`.
- `[ ]` Quality availability shape is not aligned.
  - TS: `soda-adapter.ts:365`
  - Rust: `adapter.rs:146`
  - TS returns structured quality options and top-level `provider/trackId/defaultQuality`.
  - Rust returns only quality id strings.
- `[ ]` Mutation ack shapes are not aligned.
  - TS: `soda-adapter.ts:410`, `soda-adapter.ts:440`
  - Rust: `adapter.rs:255`, `adapter.rs:278`
  - TS returns:
    - `SongLikeAck { provider, id, liked, code }`
    - `SongLikeCheckAck { provider, ids, liked }`
  - Rust returns reduced variants only.

## 4. Shared blocker visible from provider audit

These are not guesses. They follow directly from the provider TS outputs above.

- `[ ]` `src/types.rs` is too small for TS provider outputs.
  - Current Rust `LyricPayload`, `LyricLine`, `PlaylistSummary`, `PlaylistDetail`, `SongUrlResult`, `TrackQualityAvailability`, `ProviderLoginStatus`, `SongLikeAck`, `SongLikeCheckAck`, `PlaylistAddSongAck` cannot represent what TS providers already return.
- `[ ]` Provider fixes should start by expanding shared Rust response types to the TS contract, then rewire provider `map` and `adapter` code to fill them.

## 5. Recommended repair order

1. `[ ]` Expand shared Rust types to the TS route contract.
2. `[ ]` Fix all three provider `map` lyric payloads and line parsers.
3. `[ ]` Fix all three provider playlist summary/detail mappers.
4. `[ ]` Fix provider song-url outputs: Netease, QQ, Soda.
5. `[ ]` Fix provider qualities outputs: Netease, QQ, Soda.
6. `[ ]` Fix mutation ack outputs where the TS providers already define them.
