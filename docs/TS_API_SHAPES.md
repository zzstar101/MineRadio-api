# TypeScript Sidecar API Shapes

This document records the TypeScript sidecar HTTP contract from `D:\project\Rust\Mineradio-Tauri\sidecars\api\src`.

Goal:
- Route paths and HTTP methods must match.
- Final HTTP response body shapes must match.
- Public helper/service entrypoints that the route layer calls are tracked for comparison.
- Internal TypeScript implementation details do not need to be copied into Rust unless they affect the public contract.

## Sources

- Route entry: `D:\project\Rust\Mineradio-Tauri\sidecars\api\src\server.ts`
- Route tests: `D:\project\Rust\Mineradio-Tauri\sidecars\api\src\server.test.ts`
- Envelope: `D:\project\Rust\Mineradio-Tauri\sidecars\api\src\http\envelope.ts`
- Shared schemas: `D:\project\Rust\Mineradio-Tauri\packages\shared\src\*.ts`
- Provider adapter contract: `D:\project\Rust\Mineradio-Tauri\sidecars\api\src\providers\provider-adapter.ts`

## Global Rules

### JSON success envelope

Most JSON endpoints return:

```ts
{ ok: true, data: ... }
```

Defined in:
- `D:\project\Rust\Mineradio-Tauri\sidecars\api\src\http\envelope.ts`
- `D:\project\Rust\Mineradio-Tauri\packages\shared\src\envelope.ts`

### JSON error envelope

Errors return:

```ts
{
  ok: false,
  error: {
    code: string,
    message: string,
    provider?: string,
    retryable: boolean,
    action?: string,
    playbackKeyReady?: boolean,
    restriction?: object,
    reason?: string,
    qqCode?: number,
    rawMessage?: string,
    tried?: string[]
  }
}
```

Defined in:
- `D:\project\Rust\Mineradio-Tauri\packages\shared\src\envelope.ts`

### CORS

JSON and preflight helpers use the same sidecar CORS headers from:
- `D:\project\Rust\Mineradio-Tauri\sidecars\api\src\http\envelope.ts`

### Notable envelope exceptions

- `/health` returns a top-level object and is **not** wrapped in `{ ok: true, data }`.
- `/diagnostics` returns a top-level object and is **not** wrapped in `{ ok: true, data }`.
- `/weather/radio` is wrapped as `{ ok: true, data: WeatherRadioResponse }`, and the inner `data` object itself also has `ok: true`.
- `/podcast/dj-beatmap` is wrapped as `{ ok: true, data: PodcastBeatmapResponse }`, and the inner `data` object itself also has `ok: true`.
- Proxy routes may return raw upstream bytes on success and JSON envelopes on failure.

## Shared Data Shapes

### `Track`

Defined in:
- `D:\project\Rust\Mineradio-Tauri\packages\shared\src\track.ts`

Shape:

```ts
{
  provider: "netease" | "qq" | "soda",
  id: string,
  sourceId: string,
  mediaMid?: string,
  title: string,
  artists: string[],
  album?: string,
  coverUrl?: string,
  durationMs?: number,
  qualityHints: string[],
  playableState: "unknown" | "playable" | "login_required" | "vip_required" | "paid_required" | "copyright_unavailable" | "trial_only" | "unavailable"
}
```

### `PlaylistSummary` / `PlaylistDetail`

Defined in:
- `D:\project\Rust\Mineradio-Tauri\packages\shared\src\playlist.ts`

`PlaylistSummary`:

```ts
{
  provider: "netease" | "qq" | "soda",
  id: string,
  name: string,
  coverUrl?: string,
  trackCount?: number,
  trackIds: string[],
  subscribed?: boolean
}
```

`PlaylistDetail` adds:

```ts
{
  tracks: Track[]
}
```

### `LyricPayload`

Defined in:
- `D:\project\Rust\Mineradio-Tauri\packages\shared\src\lyric.ts`

Shape:

```ts
{
  provider: "netease" | "qq" | "soda",
  trackId: string,
  lines: Array<{
    timeMs: number,
    text: string,
    translation?: string,
    durationMs?: number,
    charCount?: number,
    source?: string,
    words?: Array<{
      text?: string,
      timeMs: number,
      durationMs?: number,
      c0: number,
      c1: number
    }>
  }>,
  hasTranslation: boolean,
  isWordByWord: boolean
}
```

### `SongUrlResult`

Defined in:
- `D:\project\Rust\Mineradio-Tauri\packages\shared\src\song-url.ts`

Shape:

```ts
{
  url: string | null,
  proxied: boolean,
  provider?: string,
  trial?: boolean,
  playable?: boolean,
  level?: string,
  quality?: string,
  br?: number,
  requestedQuality?: string | null,
  loggedIn?: boolean,
  vipType?: number,
  vipLevel?: "none" | "vip" | "svip",
  isVip?: boolean,
  isSvip?: boolean,
  vipLabel?: string,
  vipIcon?: "netease-vip" | "netease-svip" | "qq-green-vip" | "qq-super-vip",
  vipIconUrl?: string,
  vipTier?: number,
  vipLevelName?: string,
  playbackKeyReady?: boolean,
  restriction?: object,
  reason?: string,
  message?: string,
  tried?: string[],
  filename?: string,
  qqCode?: number,
  rawMessage?: string
}
```

### `TrackQualityAvailability`

Defined in:
- `D:\project\Rust\Mineradio-Tauri\packages\shared\src\song-url.ts`

Shape:

```ts
{
  provider: string,
  trackId: string,
  defaultQuality?: string,
  qualities: Array<{
    provider: string,
    id: string,
    label: string,
    short?: string,
    detail?: string,
    requestQuality: string,
    level?: string,
    type?: string,
    br?: number,
    size?: number,
    format?: string,
    source: "resolved" | "declared"
  }>
}
```

### Session / login related shapes

Defined in:
- `D:\project\Rust\Mineradio-Tauri\packages\shared\src\session.ts`

`ProviderSessionCookieAck`:

```ts
{
  provider: "netease" | "qq" | "soda",
  stored: boolean
}
```

`ProviderLoginStatus`:

```ts
{
  provider: "netease" | "qq" | "soda",
  loggedIn: boolean,
  nickname?: string,
  avatarUrl?: string,
  userId?: string,
  vipType?: number,
  vipLevel?: "none" | "vip" | "svip",
  isVip?: boolean,
  isSvip?: boolean,
  vipLabel?: string,
  vipIcon?: "netease-vip" | "netease-svip" | "qq-green-vip" | "qq-super-vip",
  vipIconUrl?: string,
  vipTier?: number,
  vipLevelName?: string
}
```

`ProviderLogoutAck`:

```ts
{
  provider: "netease" | "qq" | "soda",
  loggedOut: boolean
}
```

`ProviderLoginQrKey`:

```ts
{
  provider: "netease" | "qq" | "soda",
  key: string
}
```

`ProviderLoginQrImage`:

```ts
{
  provider: "netease" | "qq" | "soda",
  key: string,
  img: string,
  url?: string
}
```

`ProviderLoginQrCheck`:

```ts
{
  provider: "netease" | "qq" | "soda",
  key: string,
  code: number,
  message?: string,
  loggedIn: boolean,
  scanned?: boolean,
  expired?: boolean,
  stored?: boolean
}
```

### Mutation acks

Defined in:
- `D:\project\Rust\Mineradio-Tauri\packages\shared\src\mutation.ts`

`SongLikeAck`:

```ts
{
  provider: "netease" | "qq" | "soda",
  id: string,
  liked: boolean,
  code?: number
}
```

`SongLikeCheckAck`:

```ts
{
  provider: "netease" | "qq" | "soda",
  ids: string[],
  liked: Record<string, boolean>
}
```

`PlaylistAddSongAck`:

```ts
{
  provider: "netease" | "qq" | "soda",
  playlistId: string,
  trackId: string,
  success: boolean,
  code?: number
}
```

### Discover

Defined in:
- `D:\project\Rust\Mineradio-Tauri\packages\shared\src\discover.ts`

Shape:

```ts
{
  loggedIn: boolean,
  user: {
    provider: "netease" | "qq" | "soda",
    userId?: string,
    nickname?: string,
    avatarUrl?: string
  } | null,
  dailySongs: Track[],
  playlists: PlaylistSummary[],
  podcasts: PodcastRadio[],
  mode: "starter" | "member",
  updatedAt: number
}
```

### Podcast

Defined in:
- `D:\project\Rust\Mineradio-Tauri\packages\shared\src\podcast.ts`

`PodcastRadio`:

```ts
{
  id: string,
  rid: string,
  name: string,
  coverUrl?: string,
  description?: string,
  djName?: string,
  category?: string,
  programCount?: number,
  subCount?: number
}
```

`PodcastProgram` extends `Track` with:

```ts
{
  type: "podcast",
  programId?: string,
  radioId?: string,
  radioName?: string,
  djName?: string,
  description?: string,
  createTime?: number,
  serialNum?: number
}
```

Response shapes:

```ts
PodcastSearchResponse = { podcasts: PodcastRadio[], total: number }
PodcastHotResponse = { podcasts: PodcastRadio[], more: boolean }
PodcastDetailResponse = { podcast: PodcastRadio }
PodcastProgramsResponse = { radio: Partial<PodcastRadio> & { id?: string, rid?: string }, programs: PodcastProgram[], more: boolean, total: number }
PodcastMyResponse = { loggedIn: boolean, collections: PodcastCollection[] }
PodcastMyItemsResponse = { loggedIn: boolean, key: string, title: string, sub?: string, itemType: "radio" | "voice", count: number, coverUrl?: string, items: Array<PodcastRadio | PodcastProgram> }
PodcastBeatmapResponse = { ok: true, map: Record<string, unknown> }
```

### Weather radio

Defined in:
- `D:\project\Rust\Mineradio-Tauri\packages\shared\src\weather.ts`

Shape:

```ts
{
  ok: true,
  weather: {
    provider: string,
    location: {
      name: string,
      country?: string,
      admin1?: string,
      latitude: number | null,
      longitude: number | null,
      timezone?: string,
      fallback?: boolean
    },
    label: string,
    weatherCode: number | null,
    temperature: number | null,
    apparentTemperature: number | null,
    humidity: number | null,
    precipitation: number | null,
    cloudCover: number | null,
    windSpeed: number | null,
    windGusts: number | null,
    isDay: number | null,
    time?: string,
    updatedAt: number,
    error?: string,
    mood: {
      key: string,
      title: string,
      tagline: string,
      energy: number,
      warmth: number,
      focus: number,
      melancholy: number,
      keywords: string[]
    }
  },
  radio: {
    title: string,
    subtitle: string,
    seedQueries: string[],
    songs: Track[],
    updatedAt: number
  }
}
```

### Shared playlist import

Defined in:
- `D:\project\Rust\Mineradio-Tauri\packages\shared\src\shared-playlist.ts`

Request shape:

```ts
{
  text?: string,
  url?: string
}
```

Response shape:

```ts
{
  provider: "netease" | "qq" | "kugou" | "qishui" | "apple-music",
  playlist: {
    provider: "netease" | "qq" | "kugou" | "qishui" | "apple-music",
    id: string,
    name: string,
    coverUrl?: string,
    trackCount?: number,
    trackIds: string[],
    subscribed?: boolean,
    sourceUrl?: string
  },
  tracks: Track[],
  trackCount: number,
  loadedCount: number,
  partial: boolean,
  partialReason: string
}
```

## Route Inventory

### Top-level routes

| Route | Method | TS location | Success body shape | Tests |
| --- | --- | --- | --- | --- |
| `/health` | `GET` | `server.ts:121` | `HealthResponseSchema` top-level object | `server.test.ts:30` |
| `/providers/capabilities` | `GET` | `server.ts:136` | `{ ok: true, data: CapabilityMatrix }` | registry behavior covered indirectly |
| `/diagnostics` | `GET` | `server.ts:143` | `DiagnosticsPayload` top-level object | `services/diagnostics.test.ts` |
| `/audio-proxy` | `GET` | `server.ts:149` | raw upstream bytes on success; JSON error envelope on failure | `services/audio-proxy.test.ts` |
| `/providers/soda/audio-proxy` | `GET` | `server.ts:156` | raw upstream/decrypted bytes on success; JSON error envelope on failure | `services/soda-audio-proxy.test.ts` |
| `/image-proxy` | `GET` | `server.ts:164` | raw upstream image bytes on success; JSON error envelope on failure | `services/image-proxy.test.ts` |
| `/weather/radio` | `GET` | `server.ts:171` | `{ ok: true, data: WeatherRadioResponse }` | `server.test.ts:202` |
| `/discover/home` | `GET` | `server.ts:178` | `{ ok: true, data: DiscoverHomeResponse }` | `server.test.ts:754`, `791`, `871`, `944`, `1067` |
| `/podcast/search` | `GET` | `server.ts:189` | `{ ok: true, data: PodcastSearchResponse }` | `server.test.ts:265` |
| `/podcast/hot` | `GET` | `server.ts:197` | `{ ok: true, data: PodcastHotResponse }` | service and schema tests |
| `/podcast/detail` | `GET` | `server.ts:205` | `{ ok: true, data: PodcastDetailResponse }` | service and schema tests |
| `/podcast/programs` | `GET` | `server.ts:212` | `{ ok: true, data: PodcastProgramsResponse }` | `server.test.ts:296` |
| `/podcast/my` | `GET` | `server.ts:221` | `{ ok: true, data: PodcastMyResponse }` | `server.test.ts:339` |
| `/podcast/my/items` | `GET` | `server.ts:227` | `{ ok: true, data: PodcastMyItemsResponse }` | `server.test.ts:339` |
| `/podcast/dj-beatmap` | `GET` | `server.ts:236` | `{ ok: true, data: PodcastBeatmapResponse }` | `server.test.ts:379` |
| `/shared-playlist/import` | `POST` | `server.ts:252` | `{ ok: true, data: SharedPlaylistImportResult }` | `server.test.ts:151`, `188` |
| `/api/shared-playlist/import` | `POST` | `server.ts:252` | same as `/shared-playlist/import` | covered by same branch |
| `/search` | `GET` | `server.ts:300` | `{ ok: true, data: Track[] }` | `server.test.ts:116` and search tests |
| `/song-url` | `POST` | `server.ts:341` | `{ ok: true, data: SongUrlResult }` | route and adapter tests |

### Provider routes

All provider routes are handled in:
- `server.ts:390` through `server.ts:682`

Provider route prefix:

```txt
/providers/{providerId}/...
providerId ∈ {"netease","qq","soda"}
```

| Route suffix | Method | TS location | Success body shape | Tests |
| --- | --- | --- | --- | --- |
| `login-qr-key` | `GET` | `server.ts:390` | `{ ok: true, data: ProviderLoginQrKey }` | `server.test.ts:417+` |
| `login-qr-create` | `GET` | `server.ts:403` | `{ ok: true, data: ProviderLoginQrImage }` | `server.test.ts:417+` |
| `login-qr-check` | `GET` | `server.ts:436` | `{ ok: true, data: ProviderLoginQrCheck }` | `server.test.ts:417+` |
| `session-cookie` | `POST` | `server.ts:453` | `{ ok: true, data: ProviderSessionCookieAck }` | `server.test.ts:535` |
| `session-cookie` | `DELETE` | `server.ts:477` | `{ ok: true, data: ProviderSessionCookieAck }` | `server.test.ts:535` |
| `session-cookie/clear` | `POST` | `server.ts:479` | `{ ok: true, data: ProviderSessionCookieAck }` | same branch as delete |
| `login-status` | `GET` | `server.ts:486` | `{ ok: true, data: ProviderLoginStatus }` | `server.test.ts:398`, `407` |
| `logout` | `POST` | `server.ts:491` | `{ ok: true, data: ProviderLogoutAck }` | `server.test.ts:557`, `605`, `638`, `1478` |
| `search` | `GET` | `server.ts:528` | `{ ok: true, data: Track[] }` | `server.test.ts:103`, route tests |
| `song-url` | `POST` | `server.ts:548` | `{ ok: true, data: SongUrlResult }` | route tests and adapter tests |
| `qualities` | `POST` | `server.ts:569` | `{ ok: true, data: TrackQualityAvailability }` | shared schema + adapter tests |
| `lyric` | `POST` | `server.ts:589` | `{ ok: true, data: LyricPayload }` | adapter tests |
| `playlists` | `GET` | `server.ts:609` | `{ ok: true, data: PlaylistSummary[] }` | adapter tests |
| `like` | `POST` | `server.ts:614` | `{ ok: true, data: SongLikeAck }` | mutation schema + route tests |
| `like-check` | `GET` | `server.ts:637` | `{ ok: true, data: SongLikeCheckAck }` | mutation schema + route tests |
| `playlists/add-song` | `POST` | `server.ts:659` | `{ ok: true, data: PlaylistAddSongAck }` | mutation schema + route tests |
| `playlists/{id}` | `GET` | `server.ts:682` | `{ ok: true, data: PlaylistDetail }` | adapter tests |

## Input / Validation Notes

These matter because Rust should match the same route-level behavior:

- `providerId` is validated against `ProviderIdSchema` from `packages/shared/src/provider.ts`.
- `/search` and `/providers/{pid}/search` require a non-blank `keyword`.
- `/song-url` and `/providers/{pid}/song-url` accept either:
  - `{ track, quality? }`
  - or a raw `Track` body for fallback parsing.
- `/providers/{pid}/qualities` and `/providers/{pid}/lyric` require a raw `Track` body.
- `/providers/{pid}/like` requires `{ id, liked }`.
- `/providers/{pid}/like-check` requires `ids` or `id` query input parsed into a non-empty string array.
- `/providers/{pid}/playlists/add-song` requires `{ playlistId, trackId }`.
- `/providers/{pid}/login-qr-create` and `/providers/{pid}/login-qr-check` require non-empty `key`.
- `/podcast/dj-beatmap` rejects non-HTTP(S) `url` with `400 BAD_REQUEST`.

## High-Risk Mismatch Checklist

These are the first routes worth checking in Rust because their TypeScript shapes are easy to get subtly wrong:

- `/providers/{pid}/login-status`
  - must include `data.provider`
  - may include many optional VIP fields
- `/providers/{pid}/session-cookie`
  - ack must include `provider`
  - response must never echo cookie content
- `/providers/{pid}/logout`
  - success body is `{ provider, loggedOut: true }`
- `/providers/{pid}/qualities`
  - shape is not just a string list; it includes `provider`, `trackId`, `defaultQuality`, and structured `qualities`
- `/providers/{pid}/lyric`
  - shape includes `provider`, `trackId`, `hasTranslation`, `isWordByWord`, and rich `lines`
- `/discover/home`
  - must use the shared schema shape exactly
- `/weather/radio`
  - is double-wrapped: outer success envelope + inner object containing `ok: true`
- `/podcast/dj-beatmap`
  - is double-wrapped: outer success envelope + inner object containing `ok: true`
- `/shared-playlist/import`
  - result shape must include `playlist`, `tracks`, `trackCount`, `loadedCount`, `partial`, `partialReason`

## Suggested Rust Verification Order

1. [ ] Global envelope and error shape
2. [x] `/providers/{pid}/login-status`
3. [ ] `/providers/{pid}/session-cookie`
4. [ ] `/providers/{pid}/logout`
5. [ ] `/providers/{pid}/qualities`
6. [ ] `/providers/{pid}/lyric`
7. [ ] `/discover/home`
8. [ ] `/weather/radio`
9. [ ] `/podcast/*`
10. [ ] `/shared-playlist/import`
11. [ ] Remaining search / playlist / mutation routes
