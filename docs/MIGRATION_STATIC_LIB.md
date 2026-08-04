# MineRadio Static Library Migration

## Goal

Convert MineRadio from an HTTP-sidecar entry point into a Rust library by
adding a public wrapper. This is a repackaging project, not a rewrite of the
provider, parser, or service implementations.

The migration has two phases:

1. Build a complete public `Api` wrapper around the existing code.
2. Remove the HTTP server only after every required capability has a wrapper.

Do not merge or reorganize the existing provider, parser, service, or utility
files during phase 1. Make a local implementation change only when it is
required to remove an HTTP-only dependency or to expose the minimal arguments
used by a public method.

## Visibility Boundary

Only the library entry, configuration, API facade, public errors, and selected
domain types are visible to library consumers.

```text
src/
  lib.rs                    public crate entry and re-exports
  config.rs                 public LibraryConfig
  api.rs or api/            public Api facade

  types.rs                  private module; selected types are re-exported
  providers/                pub(crate)
  parsers/                  pub(crate)
  services/                 pub(crate)
  utils/                    pub(crate)
  router.rs                 retained during phase 1
  server.rs                 retained during phase 1
  http/                     retained during phase 1
```

`providers`, `parsers`, `services`, and `utils` must not be public. Consumers
must not call adapters, clients, parsers, or service implementations directly.

The intended `lib.rs` shape is:

```rust
pub mod api;
pub mod config;

mod types;
pub(crate) mod parsers;
pub(crate) mod providers;
pub(crate) mod services;
pub(crate) mod utils;

pub use api::{Api, ApiError, ApiErrorCode, ApiResult};
pub use config::LibraryConfig;
pub use types::{ProviderId, Track};
```

The final list of re-exported domain types is maintained by `api` and `lib`.
`types` does not need to become a public module.

## Existing Internal Layout

The current provider layering is retained:

```text
providers/
  qq/       adapter.rs, client.rs, model.rs
  netease/  adapter.rs, client.rs, model.rs, crypto.rs, map.rs
  soda/     adapter.rs, client.rs
  kugou/    adapter.rs, client.rs, model.rs, map.rs
  spotify/  adapter.rs, client.rs, map.rs
  registry.rs
  error.rs
```

`adapter.rs` remains the orchestration layer. `client.rs` remains responsible
for upstream transport. `model.rs` and `map.rs` remain provider-specific
response mapping. Public `Api` methods call the registered adapters; they do
not duplicate provider code or flatten these files.

The lyric parser layout also remains intact:

```text
parsers/
  lrc.rs
  netease.rs
  qqmusic.rs
  kugou.rs
  soda_music.rs
```

Provider adapters continue to select and invoke their own lyric parser. Parser
implementations are not public API methods.

QR login belongs to the corresponding provider implementation. During later
migration, the current QR login services move under the appropriate provider
module without changing their protocol logic. QQ web, QQ Music MQTT, and
WeChat remain distinct QR login kinds even though they ultimately authenticate
the QQ provider.

During phase 1, `QrLoginApi` relays the existing QR login services without
moving their implementation. Every `ProviderApi` contains a QR-login array.
Use `get` to select a protocol safely. QQ reserves index `0` for web QR login,
`1` for QQ Music MQTT, and `2` for WeChat. Netease, Soda, and Kugou expose
their sole protocol at index `0`; Spotify has no entries.

```rust
if let Some(qq_web) = api.qq.qr_login.get(0) {
    qq_web.create_key().await?;
}
if let Some(qq_music) = api.qq.qr_login.get(1) {
    qq_music.create_image(key).await?;
}
if let Some(qq_wechat) = api.qq.qr_login.get(2) {
    qq_wechat.check(key).await?;
}
```

## Public Object

`Api` is the single public object. It owns library lifetime, shared state, and
the business facade.

```rust
pub struct Api {
    pub qq: ProviderApi,       // holds the QQ ProviderAdapter
    pub netease: ProviderApi,  // holds the Netease ProviderAdapter
    pub soda: ProviderApi,
    pub kugou: ProviderApi,
    pub spotify: ProviderApi,
}

impl Api {
    pub async fn init(config: LibraryConfig) -> ApiResult<Self>;
    pub async fn shutdown(&self) -> ApiResult<()>;
}
```

`Api` replaces the lifetime and assembly responsibilities currently held by
`AppState` and `AppServices`. Its private state owns or constructs:

- `LibraryConfig`;
- shared HTTP clients;
- the registered provider adapters and provider registry;
- instance-scoped session storage;
- existing non-provider service instances;
- diagnostics/logging dependencies;
- registered background tasks and their shutdown signal.

`Api` does not contain replacement implementations for provider clients,
models, adapters, parsers, or services. Those remain in their current private
modules.

## Configuration

`LibraryConfig` owns the two filesystem locations needed by the library:

```rust
LibraryConfig {
    log_path: Some(app_data_dir.join("logs")),
    cookie_file: Some(app_data_dir.join("provider-sessions.json")),
    ..LibraryConfig::default()
}
```

`cookie_file` is the persisted provider-cookie JSON file. When it is `None`,
cookies are retained only in memory and no environment fallback is consulted.

`log_path` is optional. When it is `None`, MineRadio writes no file logs. A
path that is an existing directory, or a non-existing path with no extension,
is treated as a directory; MineRadio creates it and writes
`mineradio-YYYYMMDD-HHMMSS.jsonl`. A path with a file name/extension is used
directly. Each line in the log file is one redacted JSON object.

The current QR/session implementation is still internally global. Therefore a
process must initialize all `Api` instances with the same `cookie_file` and
`log_path`; a conflicting later initialization returns an error until the
session implementation is made instance-scoped.

The host application owns the Tokio runtime. `Api::init`, all `Api` calls, and
`Api::shutdown` must run inside that runtime. `shutdown` cancels and awaits
library-owned background work. The library must not create a Tokio runtime per
API call.

## Public API Shape

Use direct async methods. Do not use HTTP request structs, route-shaped JSON
bodies, builders, or a trailing `.send()` method in the public library API.

```rust
let api = Api::init(config).await?;

let tracks = api.search_tracks("keyword", None, 20).await?;
let url = api.song_url(track, Some(options)).await?;
let lyric = api.netease.lyric(&track).await?;
let detail = api.qq.playlist_detail(playlist_id, 0, 100).await?;

api.shutdown().await?;
```

The static provider namespace is the primary public entry:

```text
api.qq
api.netease
api.soda
api.kugou
api.spotify
```

`ProviderId` remains a public domain identifier for returned data, persistence,
cross-source selection, and dynamic lookup. It is not required for ordinary
static calls such as `api.qq`.

Provider facade methods preserve the existing `ProviderAdapter` signatures and
delegate directly to the adapter they hold. A provider song URL, quality, or
lyric call therefore continues to receive `&Track` and `SongUrlOptions` where
the adapter requires them. The wrapper removes HTTP body and query structs; it
does not split or reimplement adapter methods.

Search is split by result type:

```rust
pub async fn search_tracks(...) -> ApiResult<Vec<Track>>;
pub async fn search_albums(...) -> ApiResult<Vec<AlbumSummary>>;
pub async fn search_playlists(...) -> ApiResult<Vec<PlaylistSummary>>;
```

The current cross-source implementation supports tracks only. Its public
methods copy the old resolver behavior without retaining HTTP query/body
parsing:

```rust
api.search_tracks("keyword", Some(ProviderId::Qq), 20).await?;
api.song_url(track, Some(options)).await?;
```

Do not use one runtime `SearchType` parameter for these public calls, because
one Rust method cannot truthfully promise all three static result types.

Zero-argument and simple operations are direct methods, for example:

```rust
api.qq.login_status().await?;
api.qq.logout().await?;
if let Some(qr_login) = api.qq.qr_login.get(0) {
    qr_login.create_key().await?;
}
```

## Public Errors

Every public asynchronous operation returns `ApiResult<T>`.

```rust
pub struct ApiError {
    pub code: ApiErrorCode,
    pub message: String,
}
```

`ApiErrorCode` is a flat, stable, consumer-facing code set:

```text
BAD_REQUEST
NOT_FOUND
LOGIN_REQUIRED
UNAVAILABLE
COPYRIGHT_UNAVAILABLE
PAID_REQUIRED
TRIAL_ONLY
VIP_REQUIRED
INVALID_RESPONSE
NOT_IMPLEMENTED
INTERNAL
```

It must not contain route names, provider registry details, storage mechanism
details, or resource-specific variants such as `NO_URL` or `NO_PLAYLIST`.

The old `providers::error::ProviderError` and `ProviderErrorCode` remain
private migration inputs. Mapping them to `ApiError` is introduced only when a
public wrapper is connected to that code. The old error types are removed only
after every caller has migrated.

Each public `Api` operation is executed behind a panic boundary. An unwind
panic is logged with internal context and returned to callers only as
`ApiErrorCode::Internal`. Spawned background tasks must handle and log their
own `JoinError`; an outer request boundary cannot catch those panics.

## Sessions, Diagnostics, and Tasks

The current global session singleton and environment/file fallback behavior are
HTTP-sidecar assumptions. They are migrated to instance-scoped state owned by
`Api`. Services that require session data receive that state through their
existing dependency or constructor paths.

Diagnostics and logging are dependencies of `Api`, not implicit router effects.
The final logging sink is supplied by the host or configured through
`LibraryConfig`; raw upstream details and panic payloads do not become public
`ApiError.message` values.

## Proxy and Decryption Scope

HTTP audio and image proxy endpoints are not part of the static library API.
They depend on HTTP requests, response streaming, Range semantics, and large
binary transfer that should remain outside Tauri IPC.

The audio proxy implementations are retained until the HTTP server is removed.
Their pure decryption routines are preserved and later exposed through a small
non-HTTP decryption module. No `Vec<u8>` proxy replacement is introduced.

## Migration Order

1. Finalize `LibraryConfig`, `Api`, public errors, and public domain
   re-exports.
2. Keep the existing HTTP server running while adding wrappers one capability
   at a time. Reuse existing adapters, parsers, and services.
3. Refactor only the dependencies that block wrapping: global session state,
   router-only diagnostics, HTTP request/response types, or route-shaped input
   parsing.
4. Add the provider-specific QR login wrappers under the provider namespace.
5. Once every required capability is available through `Api`, make the router
   call the same wrappers or remove it with the sidecar.
6. Remove `router`, `server`, `http`, and proxy endpoints only after the static
   library replacement is complete.
7. Perform optional provider/service file consolidation only after migration;
   it is not part of this plan.

## Out of Scope During Phase 1

- Rewriting provider protocol code.
- Splitting or replacing `ProviderAdapter`; it remains the facade-to-provider
  boundary until the static-library migration is complete.
- Moving parser logic into `api`.
- Collapsing `adapter`, `client`, `model`, or `map` files.
- Exposing provider adapters or service implementations publicly.
- Replacing proxy endpoints with IPC blobs.
- Creating a synchronous facade or per-call Tokio runtime.

## Deferred Follow-Up

After every public API capability has migrated and the HTTP sidecar has been
removed, evaluate whether `ProviderAdapter` should be split by capability (for
example search, library, playback, and authentication). That work must be a
separate refactor with its own compatibility and test plan; it is not required
for the initial static-library wrapper.
