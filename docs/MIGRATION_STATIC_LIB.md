# MineRadio-api 静态链接库迁移文档

> **目标**：从 HTTP sidecar 进程转型为静态链接库（crate lib），保留 router 的全部处理流程，
> 不变动内部业务实现。仅改变调用入口——从 HTTP 路由映射为结构体点分函数调用。

## 设计原则

1. **路径 → 点分命名**：`/providers/{pid}/search` → `API.providers.{pid}.search()`
2. **内部封装**：只暴露 `API` 入口结构体、`types` 里的传入/传出结构体；内部服务、adapter 全不暴露
3. **不动业务**：每个函数的 body 直接内联原 handler 的实现逻辑，不重写业务代码
4. **统一错误**：所有函数返回 `Result<T, ApiError>`，`ApiError` 映射原 HTTP 状态码和错误码

---

## 一、公共类型

### 1.1 统一错误类型

```rust
/// 替代原 HTTP 响应信封中的 error 字段
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,       // 原 error.code，如 "NOT_FOUND", "PROVIDER_NOT_FOUND"
    pub message: String,    // 原 error.message
}

/// 所有 API 函数的统一返回
pub type ApiResult<T> = Result<T, ApiError>;
```

### 1.2 需要暴露的类型（全部来自 `src/types.rs`）

**传入结构体**：
- `Track` — 歌曲标识，几乎所有 provider 方法都需要
- `SongUrlOptions` — 获取歌曲 URL 时的可选参数
- `SearchType` — 搜索类型枚举（`Track`/`Album`/`Artist`/`Playlist`）
- `ProviderId` — 音源标识枚举（`Qq`/`Netease`/`Soda`/`Kugou`/`Spotify`）

**传出结构体**：
- `SongUrlResult`
- `TrackQualityAvailability` / `TrackQualityOption`
- `LyricPayload` / `LyricLine` / `LyricWord`
- `PlaylistSummary` / `PlaylistDetail`
- `AlbumSummary` / `AlbumDetail`
- `ProviderLoginStatus`
- `ProviderLoginQrKey` / `ProviderLoginQrImage` / `ProviderLoginQrCheck`
- `SongLikeAck` / `SongLikeCheckAck`
- `PlaylistAddSongAck`

### 1.3 代理/流式返回

音频和图片代理原返回二进制流（`Response<Body>`），迁移后有两种选择：

- **方案 A**（推荐）：返回 `Vec<u8>`，调用方自行处理
- **方案 B**：返回 `reqwest::Response` 或自定义 `Stream`，保留流式能力

下文按方案 A 编写，标记为 `ProxyBytes`：

```rust
pub struct ProxyBytes {
    pub data: Vec<u8>,
    pub content_type: String,
    pub headers: HashMap<String, String>,  // 额外响应头
}
```

---

## 二、API 入口结构体

```rust
/// 静态库的主入口。
/// 调用方构造一个 AppContext 后，所有 API 都从该实例调用。
pub struct AppContext { /* 内部字段不公开 */ }

impl AppContext {
    /// 使用默认配置初始化
    pub async fn init(config: LibraryConfig) -> Result<Self, InitError>;

    /// 顶层跨源 API
    pub fn search(&self) -> CrossSourceSearchBuilder<'_>;
    pub fn song_url(&self) -> CrossSourceSongUrlBuilder<'_>;
    pub async fn health(&self) -> HealthResponse;
    pub async fn diagnostics(&self) -> DiagnosticsPayload;
    pub async fn capabilities(&self) -> CapabilityMatrix;
    pub fn weather_radio(&self) -> WeatherRadioBuilder<'_>;
    pub async fn discover_home(&self) -> ApiResult<serde_json::Value>;
    pub fn shared_playlist(&self) -> SharedPlaylistBuilder<'_>;

    /// 音源入口 → API.providers.{pid}.{method}()
    pub fn providers(&self) -> ProviderNamespace<'_>;

    /// 播客入口 → API.podcast.{method}()
    pub fn podcast(&self) -> PodcastNamespace<'_>;

    /// 代理入口 → API.proxy.xxx()
    pub fn proxy(&self) -> ProxyNamespace<'_>;
}
```

---

## 三、路由 → 函数映射表

### 3.1 顶层路由（非 provider 路由）

| 原 HTTP 路由 | 方法 | 新函数签名 |
|---|---|---|
| `GET /health` | GET | `API.health() -> HealthResponse` |
| `GET /providers/capabilities` | GET | `API.capabilities() -> CapabilityMatrix` |
| `GET /diagnostics` | GET | `API.diagnostics() -> DiagnosticsPayload` |
| `GET /search?keyword=&provider=&type=&offset=&limit=` | GET | `API.search().keyword(k).provider(p?).kind(t).offset(o).limit(l).send() -> Vec<Track>` |
| `POST /song-url` | POST | `API.song_url().track(t).quality(q?).send() -> SongUrlResult` |
| `POST /shared-playlist/import` | POST | `API.shared_playlist().import(body).send() -> SharedPlaylistResult` |
| `GET /weather/radio?city=&lat=&lon=` | GET | `API.weather_radio().params(p).send() -> WeatherRadioResult` |
| `GET /discover/home` | GET | `API.discover_home() -> serde_json::Value` |
| `GET /audio-proxy?url=` | GET | `API.proxy().audio(url).send() -> ProxyBytes` |
| `GET /image-proxy?url=` | GET | `API.proxy().image(url).send() -> ProxyBytes` |
| `GET /providers/soda/audio-proxy?url=&playAuth=` | GET | `API.proxy().soda_audio(url, play_auth).send() -> ProxyBytes` |
| `GET /providers/spotify/audio-proxy?id=&quality=` | GET | `API.proxy().spotify_audio(id, quality).send() -> ProxyBytes` |
| `GET /providers/qq/audio-proxy?url=` | GET | `API.proxy().qq_audio(url).send() -> ProxyBytes` |

### 3.2 `/providers/{pid}/...` 路由

统一映射为 `API.providers.{pid}.{method}()`，其中 `{pid}` 为 `ProviderId` 枚举值的小写形式（`qq`/`netease`/`soda`/`kugou`/`spotify`）。

| 原 HTTP 路由 | 方法 | 新函数签名 |
|---|---|---|
| `GET /providers/{pid}/search?keyword=&type=&offset=&limit=` | GET | `API.providers.{pid}.search().keyword(k).kind(t).offset(o).limit(l).send() -> Vec<Track>` |
| `POST /providers/{pid}/song-url` body=`Track` | POST | `API.providers.{pid}.song_url().track(t).quality(q?).send() -> SongUrlResult` |
| `POST /providers/{pid}/qualities` body=`Track` | POST | `API.providers.{pid}.qualities(track).send() -> TrackQualityAvailability` |
| `POST /providers/{pid}/lyric` body=`Track` | POST | `API.providers.{pid}.lyric(track).send() -> LyricPayload` |
| `GET /providers/{pid}/playlists` | GET | `API.providers.{pid}.playlists().send() -> Vec<PlaylistSummary>` |
| `GET /providers/{pid}/playlists/{id}?offset=&limit=` | GET | `API.providers.{pid}.playlists().detail(id, offset, limit).send() -> PlaylistDetail` |
| `GET /providers/{pid}/albums` | GET | `API.providers.{pid}.albums().send() -> Vec<AlbumSummary>` |
| `GET /providers/{pid}/albums/{id}?offset=&limit=` | GET | `API.providers.{pid}.albums().detail(id, offset, limit).send() -> AlbumDetail` |
| `GET /providers/{pid}/login-status` | GET | `API.providers.{pid}.login_status().send() -> ProviderLoginStatus` |
| `POST /providers/{pid}/logout` | POST | `API.providers.{pid}.logout().send() -> ()` |
| `POST /providers/{pid}/like` body=`{id, liked}` | POST | `API.providers.{pid}.like(id, liked).send() -> SongLikeAck` |
| `GET /providers/{pid}/like-check?ids=` | GET | `API.providers.{pid}.like_check(ids).send() -> SongLikeCheckAck` |
| `POST /providers/{pid}/playlists/add-song` body=`{playlist_id, track_id}` | POST | `API.providers.{pid}.playlists().add_song(playlist_id, track_id).send() -> PlaylistAddSongAck` |
| `POST /providers/{pid}/playlists/del-song` body=`{playlist_id, track_id}` | POST | `API.providers.{pid}.playlists().del_song(playlist_id, track_id).send() -> PlaylistAddSongAck` |

### 3.3 二维码登录路由

`/providers/{pid}/login-qr-*` 的 `{pid}` 比普通 provider 路由更宽——它还接受 `qqmusic` 和 `wx`（见 `QrLoginKind`）。

| 原 HTTP 路由 | 方法 | 新函数签名 |
|---|---|---|
| `GET /providers/{pid}/login-qr-key` | GET | `API.providers.{pid}.login_qr().key().send() -> ProviderLoginQrKey` |
| `GET /providers/{pid}/login-qr-create?key=` | GET | `API.providers.{pid}.login_qr().create(key).send() -> ProviderLoginQrImage` |
| `GET /providers/{pid}/login-qr-check?key=` | GET | `API.providers.{pid}.login_qr().check(key).send() -> ProviderLoginQrCheck` |

QR 登录支持的 provider 名：`qq` / `qqmusic` / `wx` / `netease` / `soda` / `kugou`

### 3.4 Session Cookie 路由

| 原 HTTP 路由 | 方法 | 新函数签名 |
|---|---|---|
| `POST /providers/{pid}/session-cookie` body=`{cookie}` | POST | `API.providers.{pid}.session_cookie().set(cookie).send() -> SessionCookieResult` |
| `DELETE /providers/{pid}/session-cookie` | DELETE | `API.providers.{pid}.session_cookie().clear().send() -> SessionCookieResult` |
| `POST /providers/{pid}/session-cookie/clear` | POST | （同上，合并为一个 `.clear()` 方法） |

Session cookie 支持的 provider：`qq` / `netease` / `soda` / `kugou` / `spotify`

### 3.5 播客路由

| 原 HTTP 路由 | 方法 | 新函数签名 |
|---|---|---|
| `GET /podcast/search?keywords=&limit=` | GET | `API.podcast().search(keywords, limit).send() -> PodcastSearchResult` |
| `GET /podcast/hot?offset=&limit=` | GET | `API.podcast().hot(offset, limit).send() -> PodcastHotResult` |
| `GET /podcast/detail?id=` | GET | `API.podcast().detail(id).send() -> PodcastDetailResult` |
| `GET /podcast/programs?id=&offset=&limit=` | GET | `API.podcast().programs(id, offset, limit).send() -> PodcastProgramsResult` |
| `GET /podcast/my` | GET | `API.podcast().my().send() -> PodcastMyResult` |
| `GET /podcast/my/items?key=&offset=&limit=` | GET | `API.podcast().my_items(key, offset, limit).send() -> PodcastMyItemsResult` |
| `GET /podcast/dj-beatmap?url=&duration=&intro=` | GET | `API.podcast().dj_beatmap(url, duration, intro).send() -> DjBeatmapResult` |

---

## 四、Builder 模式详解

对于参数较多的调用，使用 Builder 模式，让必填参数在构造时传入，可选参数链式设置：

### 4.1 跨源搜索

```rust
// 原: GET /search?keyword=xxx&provider=netease&type=track&offset=0&limit=20
// 新:
let tracks = ctx.search()
    .keyword("xxx")           // 必填
    .provider(ProviderId::Netease)  // 可选，不填则跨源聚合
    .kind(SearchType::Track)  // 可选，默认 Track
    .offset(0)                // 可选，默认 0
    .limit(20)                // 可选，默认 20
    .send()                   // async -> ApiResult<Vec<Track>>
    .await?;
```

### 4.2 Provider 搜索

```rust
// 原: GET /providers/netease/search?keyword=xxx&type=album&offset=0&limit=10
// 新:
let albums = ctx.providers().netease().search()
    .keyword("xxx")
    .kind(SearchType::Album)
    .offset(0)
    .limit(10)
    .send()
    .await?;
```

### 4.3 歌曲 URL

```rust
// 跨源:
let url = ctx.song_url()
    .track(my_track)
    .quality("lossless")
    .send()
    .await?;

// 指定 provider:
let url = ctx.providers().qq().song_url()
    .track(my_track)
    .quality("128k")
    .send()
    .await?;
```

### 4.4 播放列表操作

```rust
// 获取歌单列表
let playlists = ctx.providers().netease().playlists().send().await?;

// 获取歌单详情
let detail = ctx.providers().netease().playlists()
    .detail("playlist_id_123", offset=0, limit=100)
    .send().await?;

// 添加歌曲到歌单
let ack = ctx.providers().netease().playlists()
    .add_song("playlist_id", "track_id")
    .send().await?;

// 从歌单删除歌曲
let ack = ctx.providers().netease().playlists()
    .del_song("playlist_id", "track_id")
    .send().await?;
```

### 4.5 二维码登录流程

```rust
// Step 1: 获取 key
let qr_key = ctx.providers().netease().login_qr().key().send().await?;
// → ProviderLoginQrKey { provider: Netease, key: "xxx" }

// Step 2: 生成二维码图片
let qr_img = ctx.providers().netease().login_qr()
    .create(&qr_key.key)
    .send().await?;
// → ProviderLoginQrImage { provider: Netease, key: "xxx", img: "base64...", url: Some("...") }

// Step 3: 轮询检查扫码状态
let check = ctx.providers().netease().login_qr()
    .check(&qr_key.key)
    .send().await?;
// → ProviderLoginQrCheck { logged_in: true, ... }
```

### 4.6 播客

```rust
let results = ctx.podcast().search("关键词", limit=18).send().await?;
let hot = ctx.podcast().hot(offset=0, limit=18).send().await?;
let detail = ctx.podcast().detail("rid_or_id").send().await?;
let programs = ctx.podcast().programs("rid", offset=0, limit=30).send().await?;
let my = ctx.podcast().my().send().await?;
let items = ctx.podcast().my_items("collect", offset=0, limit=36).send().await?;
let beatmap = ctx.podcast().dj_beatmap("url", duration_sec=300, intro_sec=None).send().await?;
```

---

## 五、ProviderNamespace 结构

```rust
pub struct ProviderNamespace<'a> { ctx: &'a AppContext }

impl<'a> ProviderNamespace<'a> {
    pub fn qq(&self) -> ProviderHandle<'a>;
    pub fn netease(&self) -> ProviderHandle<'a>;
    pub fn soda(&self) -> ProviderHandle<'a>;
    pub fn kugou(&self) -> ProviderHandle<'a>;
    pub fn spotify(&self) -> ProviderHandle<'a>;

    // 兼容 QrLoginKind 扩展：qqmusic / wx
    pub fn qqmusic(&self) -> QrLoginHandle<'a>;
    pub fn wx(&self) -> QrLoginHandle<'a>;
}
```

`ProviderHandle<'a>` 上暴露的方法：

```rust
impl<'a> ProviderHandle<'a> {
    pub fn search(&self) -> ProviderSearchBuilder<'a>;
    pub fn song_url(&self) -> ProviderSongUrlBuilder<'a>;
    pub fn qualities(&self, track: &Track) -> ProviderQualitiesBuilder<'a>;
    pub fn lyric(&self, track: &Track) -> ProviderLyricBuilder<'a>;
    pub fn playlists(&self) -> PlaylistNamespace<'a>;
    pub fn albums(&self) -> AlbumNamespace<'a>;
    pub fn login_status(&self) -> ProviderLoginStatusBuilder<'a>;
    pub fn logout(&self) -> ProviderLogoutBuilder<'a>;
    pub fn like(&self, id: &str, liked: bool) -> ProviderLikeBuilder<'a>;
    pub fn like_check(&self, ids: &[String]) -> ProviderLikeCheckBuilder<'a>;
    pub fn login_qr(&self) -> QrLoginNamespace<'a>;
    pub fn session_cookie(&self) -> SessionCookieNamespace<'a>;
}
```

### 5.1 每个 Builder 的签名

以下是迁移后每个 builder 函数的签名汇总，参数来自原 router handler 的解构逻辑：

```rust
// === ProviderHandle ===

// search
API.providers.{pid}.search()
    .keyword(k: &str)       // 必填，空字符串直接返回 error("keyword required")
    .kind(t: SearchType)    // 可选，默认 SearchType::Track
    .offset(o: u32)         // 可选，默认 0
    .limit(l: u32)          // 可选，默认 20，最小 1
    .send() -> ApiResult<Vec<Track>>
// 注意: SearchType::Track | Artist → search_track()
//       SearchType::Album → search_album()
//       SearchType::Playlist → search_playlist()

// song_url
API.providers.{pid}.song_url()
    .track(t: &Track)       // 必填
    .quality(q: Option<String>)  // 可选
    .send() -> ApiResult<SongUrlResult>

// qualities
API.providers.{pid}.qualities(track: &Track)
    .send() -> ApiResult<TrackQualityAvailability>

// lyric
API.providers.{pid}.lyric(track: &Track)
    .send() -> ApiResult<LyricPayload>

// === PlaylistNamespace ===
API.providers.{pid}.playlists()
    .send() -> ApiResult<Vec<PlaylistSummary>>

API.providers.{pid}.playlists()
    .detail(id: &str, offset: u32, limit: u32)  // offset 默认 0，limit 默认 100
    .send() -> ApiResult<PlaylistDetail>

API.providers.{pid}.playlists()
    .add_song(playlist_id: &str, track_id: &str)
    .send() -> ApiResult<PlaylistAddSongAck>

API.providers.{pid}.playlists()
    .del_song(playlist_id: &str, track_id: &str)
    .send() -> ApiResult<PlaylistAddSongAck>

// === AlbumNamespace ===
API.providers.{pid}.albums()
    .send() -> ApiResult<Vec<AlbumSummary>>

API.providers.{pid}.albums()
    .detail(id: &str, offset: u32, limit: u32)  // offset 默认 0，limit 默认 100
    .send() -> ApiResult<AlbumDetail>

// login_status
API.providers.{pid}.login_status()
    .send() -> ApiResult<ProviderLoginStatus>

// logout
API.providers.{pid}.logout()
    .send() -> ApiResult<()>

// like
API.providers.{pid}.like(id: &str, liked: bool)
    .send() -> ApiResult<SongLikeAck>

// like_check
API.providers.{pid}.like_check(ids: &[String])  // 必填，空则 error("ids required")
    .send() -> ApiResult<SongLikeCheckAck>

// === QrLoginNamespace ===
API.providers.{pid}.login_qr()
    .key()
    .send() -> ApiResult<ProviderLoginQrKey>

API.providers.{pid}.login_qr()
    .create(key: &str)
    .send() -> ApiResult<ProviderLoginQrImage>

API.providers.{pid}.login_qr()
    .check(key: &str)
    .send() -> ApiResult<ProviderLoginQrCheck>

// === SessionCookieNamespace ===
API.providers.{pid}.session_cookie()
    .set(cookie: &str)
    .send() -> ApiResult<SessionCookieResult>

API.providers.{pid}.session_cookie()
    .clear()
    .send() -> ApiResult<SessionCookieResult>
```

---

## 六、内部实现映射（不动业务逻辑）

每个 Builder 的 `.send()` 方法内联原始 handler 的实现，**一字不改地搬运**。下面列出关键映射关系：

### 6.1 顶层函数映射

| 函数 | 原 handler | 关键逻辑搬运 |
|---|---|---|
| `API.health()` | `async fn health()` | 构造 `HealthResponse`，字段来自 `state.config` |
| `API.capabilities()` | `async fn provider_capabilities()` | 调 `state.providers.build_capability_matrix()` |
| `API.diagnostics()` | `async fn diagnostics()` | 调 `services::diagnostics::snapshot(&state)` |
| `API.search().send()` | `async fn search()` | `search_keyword` 提取 → `build_cross_source_resolver` → `resolve_search` |
| `API.song_url().send()` | `async fn song_url()` | `parse_song_url_body` → `build_cross_source_resolver` → `resolve_song_url` |
| `API.shared_playlist().import().send()` | `async fn shared_playlist_import()` | `import_shared_playlist(body, deps)` |
| `API.weather_radio().send()` | `async fn weather_radio()` | `state.services.weather_radio.build(params)` |
| `API.discover_home()` | `async fn discover_home()` | `build_discover_home(DiscoverHomeServiceOptions{...})` |
| `API.proxy().audio().send()` | `async fn audio_proxy()` | `state.services.audio_proxy.resolve(AudioProxyRequest{...})` |
| `API.proxy().image().send()` | `async fn image_proxy()` | `state.services.image_proxy.resolve(ImageProxyRequest{...})` |
| `API.proxy().soda_audio().send()` | `async fn soda_audio_proxy()` | `state.services.soda_audio_proxy.resolve(SodaAudioProxyRequest{...})` |
| `API.proxy().spotify_audio().send()` | `async fn spotify_audio_proxy()` | `state.services.spotify_audio_proxy.resolve(...)` |
| `API.proxy().qq_audio().send()` | `async fn qq_audio_proxy_handler()` | `get_provider_cookie` + `state.services.qq_audio_proxy.resolve(...)` |

### 6.2 Provider 函数映射

| 函数 | 原 handler | 关键逻辑搬运 |
|---|---|---|
| `ProviderHandle::search().send()` | `async fn provider_search()` | `pid.parse::<ProviderId>()` → `registry.get(&id)` → 按 `search_type` 分发 |
| `ProviderHandle::song_url().send()` | `async fn provider_song_url()` | `parse_song_url_body` → `provider.song_url(&track, options)` |
| `ProviderHandle::qualities().send()` | `async fn provider_qualities()` | `parse_track_body` → `provider.track_qualities(&track)` |
| `ProviderHandle::lyric().send()` | `async fn provider_lyric()` | `parse_track_body` → `provider.lyric(&track)` |
| `PlaylistNamespace::send()` | `async fn provider_playlists()` | `provider.playlist_list()` |
| `PlaylistNamespace::detail().send()` | `async fn provider_playlist_detail()` | `provider.playlist_detail(&id, offset, limit)` |
| `AlbumNamespace::send()` | `async fn provider_albums()` | `provider.album_list()` |
| `AlbumNamespace::detail().send()` | `async fn provider_album_detail()` | `provider.album_detail(&id, offset, limit)` |
| `ProviderHandle::login_status().send()` | `async fn provider_login_status()` | `provider.login_status()` |
| `ProviderHandle::logout().send()` | `async fn provider_logout()` | 检查 runtime cookie → `provider.logout()` → `clear_runtime_provider_cookie` |
| `ProviderHandle::like().send()` | `async fn provider_like()` | `provider.like_song(&body.id, body.liked)` |
| `ProviderHandle::like_check().send()` | `async fn provider_like_check()` | `parse_like_check_ids` → `provider.check_song_likes(&ids)` |
| `PlaylistNamespace::add_song().send()` | `async fn provider_playlist_add_song()` | `provider.update_song_in_playlist(...)` |
| `PlaylistNamespace::del_song().send()` | `async fn provider_playlist_del_song()` | `provider.update_song_in_playlist(..., false)` |

### 6.3 错误映射（原 `provider_error_response`）

HTTP status → `ApiError.code` 映射保持原样：

| ProviderErrorCode | 原 HTTP Status | ApiError.code |
|---|---|---|
| `LoginRequired` | 401 | `"LOGIN_REQUIRED"` |
| `NotImplemented` | 501 | `"NOT_IMPLEMENTED"` |
| `InvalidResponse` / `NoResult` / `NoUrl` / `NoPlaylist` | 404 | `"INVALID_RESPONSE"` 等 |
| `Unavailable` / `CopyrightUnavailable` / `PaidRequired` / `TrialOnly` / `VipRequired` | 502 | `"UNAVAILABLE"` 等 |
| `Internal` | 500 | `"INTERNAL"` |

provider 未找到 → `"PROVIDER_NOT_FOUND"`；provider 未注册 → `"PROVIDER_UNAVAILABLE"`。

---

## 七、使用示例（迁移前 vs 迁移后）

### 迁移前（HTTP sidecar）

```rust
// 前端 TypeScript 代码
const resp = await fetch(`http://127.0.0.1:${port}/search?keyword=七里香&limit=20`);
const { ok, data, error } = await resp.json();

const resp2 = await fetch(`http://127.0.0.1:${port}/providers/netease/lyric`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(track),
});
```

### 迁移后（静态链接库）

```rust
// 宿主 Rust 代码
use mineradio_api::{AppContext, LibraryConfig, types::*};

let ctx = AppContext::init(LibraryConfig::default()).await?;

// 跨源搜索
let tracks = ctx.search()
    .keyword("七里香")
    .limit(20)
    .send()
    .await?;

// 指定 provider 获取歌词
let lyric = ctx.providers().netease()
    .lyric(&track)
    .send()
    .await?;

// 二维码登录
let qr_key = ctx.providers().qq().login_qr().key().send().await?;
let qr_img = ctx.providers().qq().login_qr().create(&qr_key.key).send().await?;
// ... 展示二维码给用户，轮询 check ...
let check = ctx.providers().qq().login_qr().check(&qr_key.key).send().await?;
```

---

## 八、实施步骤

### Phase 1: 改 Cargo.toml

```toml
[lib]
name = "mineradio_api"
crate-type = ["lib"]  # 或 ["cdylib", "lib"] 如需动态库

# 移除不再需要的依赖（如 axum、tower、tower-http 的 CORS 层）
# 保留核心依赖：reqwest、serde、serde_json、tokio、async-trait 等
```

### Phase 2: 新建 `src/lib.rs`

```rust
// 原来的 mod 声明移到这里（移除 router、server、http/response）
mod config;
mod librespot_audio;
mod librespot_core;
mod librespot_metadata;
mod librespot_protocol;
mod parsers;
mod providers;
mod services;
mod types;
mod utils;

// 新增 API 入口
pub mod api;  // AppContext + Builder 们

// 公开暴露的类型
pub use types::*;
```

### Phase 3: 新建 `src/api.rs` + `src/api/` 子模块

将 router.rs 中每个 handler 的逻辑抽成对应的 builder `.send()` 实现，按命名空间组织：

```
src/api/
  mod.rs          → AppContext, init
  search.rs       → CrossSourceSearchBuilder
  song_url.rs     → CrossSourceSongUrlBuilder
  proxy.rs        → ProxyNamespace (audio/image/soda/spotify/qq)
  podcast.rs      → PodcastNamespace
  providers/
    mod.rs        → ProviderNamespace, ProviderHandle
    search.rs     
    song_url.rs   
    playlist.rs   
    album.rs      
    login.rs      → login_status, logout
    like.rs       
    qr_login.rs   
    session.rs    
  error.rs        → ApiError, ProviderError → ApiError 映射
```

### Phase 4: 移除旧文件

- `src/main.rs` — 不再有 bin 入口
- `src/router.rs` — 逻辑已迁移到 api 子模块
- `src/server.rs` — `AppState`/`AppServices` 构建逻辑移入 `AppContext::init()`
- `src/http/` — JSON 信封逻辑移入 `api/error.rs`

### Phase 5: 测试

为每个 builder 编写集成测试，对比迁移前后的返回值。原 `router.rs` 的单元测试（`like_check_ids` / `is_known_provider`）保留迁移。

---

## 九、不变更的内部模块

以下模块**零改动**，直接沿用：

| 模块 | 说明 |
|---|---|
| `src/types.rs` | 所有领域类型不变 |
| `src/providers/` | ProviderAdapter trait + 5 个 provider 实现不变 |
| `src/services/` | 22 个业务服务模块不变（audio_proxy, podcast, qr_login 等） |
| `src/parsers/` | 歌词解析器不变 |
| `src/utils/cryptors/` | 加解密工具不变 |
| `src/vendor/librespot_*` | Spotify 协议实现不变 |
| `src/config.rs` | 配置读取逻辑不变（改名为 `LibraryConfig`） |

---

## 十、待定问题

1. **代理返回类型**：音频/图片代理返回 `Vec<u8>` 还是自定义流？取决于宿主是 Tauri 桌面端还是其他嵌入场景。
2. **Cookie/Session 持久化**：`auth_session` 的 JSON 文件持久化策略是否保留？
3. **日志**：`sidecar_log` 是保留 JSONL 文件输出，还是改为 `tracing` 回调给宿主？
4. **`/providers/{pid}/song-url` 的 body 兼容**：原 handler 接受裸 `Track` 或 `{track, quality}`，迁移后通过 builder `.track()` + `.quality()` 自然区分，不需要兼容裸 JSON 解析。
5. **Proxy 别名**：原 `ProxyQuery` 接受 `url` 或 `target` 别名，迁移后统一为 `url` 参数。
