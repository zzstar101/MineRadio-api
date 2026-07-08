# MineRadio Sidecar API — TypeScript → Rust 迁移方案

> 源项目: `\Mineradio-Tauri\sidecars\api\`
> 目标项目: `\MineRadio-api\`
> 日期: 2026-07-08

---

## 目录

1. [架构总览](#1-架构总览)
2. [模块映射表](#2-模块映射表)
3. [依赖映射（npm → Cargo）](#3-依赖映射npm--cargo)
4. [关键数据结构](#4-关键数据结构)
5. [技术难点与对策](#5-技术难点与对策)
6. [实施阶段](#6-实施阶段)
7. [文件结构规划](#7-文件结构规划)
8. [测试策略](#8-测试策略)

---

## 1. 架构总览

当前 TypeScript 实现是一个基于 **Bun** 运行时的 HTTP 服务，作为 MineRadio 桌面应用的 **sidecar 进程**。它通过适配器模式对接三个音乐平台（网易云、QQ、汽水），并提供音频代理、图片代理、天气电台、播客、歌单导入等服务。

### 1.1 核心分层

```
┌─────────────────────────────────────────────────────┐
│  server.ts           HTTP Router (路由分发)           │
├─────────────────────────────────────────────────────┤
│  http/envelope.ts    API 响应封装 (ok/fail/json)      │
├──────────┬──────────┬──────────┬────────────────────┤
│ services │ services │ services │ services            │
│ audio-   │ auth-    │ cross-   │ weather-radio       │
│ proxy    │ session  │ source   │ podcast             │
│ image-   │ qr-login │ resolver │ discover-home       │
│ proxy    │ (×3)     │ fallback │ shared-playlist     │
│ soda-    │          │ diag-    │ sidecar-log         │
│ audio    │          │ nostics  │                     │
├──────────┴──────────┴──────────┴────────────────────┤
│  providers/                                         │
│  ├── provider-adapter.ts   Adapter 接口 + 错误类型    │
│  ├── registry.ts           全局 Provider 注册表       │
│  ├── netease/              网易云音乐适配器            │
│  │   ├── hana-client.ts    API 客户端 (hana-music-api)│
│  │   ├── map.ts            数据映射 + LRC/YRC 解析    │
│  │   └── netease-adapter.ts 适配器实现                │
│  ├── qq/                  QQ 音乐适配器               │
│  │   ├── qq-client.ts      API 客户端 (qq-music-api)  │
│  │   ├── map.ts            数据映射 + LRC/QRC 解析    │
│  │   └── qq-adapter.ts     适配器实现                 │
│  └── soda/                汽水音乐适配器              │
│      ├── soda-client.ts    API 客户端 (直接 fetch)    │
│      ├── map.ts            数据映射 + 歌词解析         │
│      └── soda-adapter.ts   适配器实现                 │
├─────────────────────────────────────────────────────┤
│  env.ts              环境变量 / 配置                  │
└─────────────────────────────────────────────────────┘
```

### 1.2 路由表（~35 个端点）

| 路由 | 方法 | 功能 |
|------|------|------|
| `/health` | GET | 健康检查 + 版本信息 |
| `/providers/capabilities` | GET | 所有 Provider 的能力矩阵 |
| `/diagnostics` | GET | 诊断信息 |
| `/audio-proxy` | GET | 通用音频代理 |
| `/providers/soda/audio-proxy` | GET | 汽水音乐 DRM 解密代理 |
| `/image-proxy` | GET | 图片代理（UA 伪装 + Referer） |
| `/weather/radio` | GET | 天气电台生成 |
| `/discover/home` | GET | 发现页聚合 |
| `/podcast/search` | GET | 播客搜索 |
| `/podcast/hot` | GET | 热门播客 |
| `/podcast/detail` | GET | 播客详情 |
| `/podcast/programs` | GET | 播客节目列表 |
| `/podcast/my` | GET | 我的播客收藏 |
| `/podcast/my/items` | GET | 收藏列表项目 |
| `/podcast/dj-beatmap` | GET | DJ 节拍分析 |
| `/shared-playlist/import` | POST | 分享歌单导入 |
| `/search` | GET | 跨平台搜索 |
| `/song-url` | POST | 跨平台歌曲 URL 解析 |
| `/providers/:pid/login-qr-key` | GET | 扫码登录 - 创建 Key |
| `/providers/:pid/login-qr-create` | GET | 扫码登录 - 生成二维码 |
| `/providers/:pid/login-qr-check` | GET | 扫码登录 - 轮询状态 |
| `/providers/:pid/session-cookie` | POST/DELETE | Cookie 管理 |
| `/providers/:pid/login-status` | GET | 登录状态查询 |
| `/providers/:pid/logout` | POST | 登出 |
| `/providers/:pid/search` | GET | 单平台搜索 |
| `/providers/:pid/song-url` | POST | 单平台歌曲 URL |
| `/providers/:pid/qualities` | POST | 音质列表 |
| `/providers/:pid/lyric` | POST | 歌词 |
| `/providers/:pid/playlists` | GET | 用户歌单 |
| `/providers/:pid/like` | POST | 喜欢/取消喜欢 |
| `/providers/:pid/like-check` | GET | 喜欢状态查询 |
| `/providers/:pid/playlists/add-song` | POST | 添加到歌单 |
| `/providers/:pid/playlists/:id` | GET | 歌单详情 |

---

## 2. 模块映射表

### 2.1 核心框架

| TypeScript 文件 | Rust 目标模块 | 说明 |
|-----------------|--------------|------|
| `server.ts` | `src/server.rs` + `src/router.rs` | HTTP 服务入口 + 路由分发。使用 `axum` 或 `actix-web` |
| `env.ts` | `src/config.rs` | 环境变量读取，使用 `dotenvy` + 结构体 |
| `http/envelope.ts` | `src/http/response.rs` | `ok()`/`fail()`/`json()` 封装，统一 `ApiResponse<T>` |

### 2.2 Provider 系统

| TypeScript 文件 | Rust 目标模块 | 说明 |
|-----------------|--------------|------|
| `providers/provider-adapter.ts` | `src/providers/mod.rs` + `src/providers/error.rs` | `ProviderAdapter` trait + 错误类型 |
| `providers/registry.ts` | `src/providers/registry.rs` | Provider 注册表 + `buildCapabilityMatrix()` |
| `providers/netease/hana-client.ts` | `src/providers/netease/client.rs` | 网易云 API 客户端 |
| `providers/netease/map.ts` | `src/providers/netease/map.rs` | 数据映射 + LRC/YRC 解析 |
| `providers/netease/netease-adapter.ts` | `src/providers/netease/adapter.rs` | 适配器实现 |
| `providers/qq/qq-client.ts` | `src/providers/qq/client.rs` | QQ 音乐 API 客户端 |
| `providers/qq/map.ts` | `src/providers/qq/map.rs` | 数据映射 + LRC/QRC 解析 |
| `providers/qq/qq-adapter.ts` | `src/providers/qq/adapter.rs` | 适配器实现 |
| `providers/soda/soda-client.ts` | `src/providers/soda/client.rs` | 汽水音乐 API 客户端 |
| `providers/soda/map.ts` | `src/providers/soda/map.rs` | 数据映射 + 歌词解析 |
| `providers/soda/soda-adapter.ts` | `src/providers/soda/adapter.rs` | 适配器实现 |

### 2.3 服务层

| TypeScript 文件 | Rust 目标模块 | 说明 |
|-----------------|--------------|------|
| `services/audio-proxy.ts` | `src/services/audio_proxy.rs` | 通用音频代理 |
| `services/image-proxy.ts` | `src/services/image_proxy.rs` | 图片代理（含 UA/Referer 伪装） |
| `services/soda-audio-proxy.ts` | `src/services/soda_audio_proxy.rs` | ⚠️ 汽水 DRM 解密代理 |
| `services/auth-session.ts` | `src/services/auth_session.rs` | Cookie 三层存储 |
| `services/netease-qr-login.ts` | `src/services/netease_qr_login.rs` | 网易云扫码登录 |
| `services/qq-qr-login.ts` | `src/services/qq_qr_login.rs` | QQ 扫码登录（含 hash33/gtk） |
| `services/soda-qr-login.ts` | `src/services/soda_qr_login.rs` | 汽水扫码登录 |
| `services/cross-source-resolver.ts` | `src/services/cross_source_resolver.rs` | 跨平台搜索 + 评分算法 |
| `services/diagnostics.ts` | `src/services/diagnostics.rs` | 诊断信息聚合 |
| `services/discover-home.ts` | `src/services/discover_home.rs` | 发现页数据聚合 |
| `services/fallback.ts` | `src/services/fallback.rs` | 错误规范化 |
| `services/podcast.ts` | `src/services/podcast.rs` | 播客服务 |
| `services/shared-playlist-import.ts` | `src/services/shared_playlist_import.rs` | ⚠️ 分享歌单导入（含 MD5） |
| `services/sidecar-log.ts` | `src/services/sidecar_log.rs` | 结构化日志 |
| `services/weather-radio.ts` | `src/services/weather_radio.rs` | 天气电台生成 |

---

## 3. 依赖映射（npm → Cargo）

### 3.1 框架与 HTTP

| npm 包 | Cargo crate | 说明 |
|--------|------------|------|
| `bun` (运行时) | — | Rust 直接编译为二进制，无需运行时 |
| `Bun.serve` (HTTP) | `axum 0.8` + `tokio 1` | 异步 HTTP 框架。备选: `actix-web` |
| `Bun.file()` / `fetch()` | `reqwest 0.12` | HTTP 客户端 |
| `Response` / `Request` | `axum::response::Response` | 请求/响应类型 |

### 3.2 音乐 API 客户端

| npm 包 | 替换方案 | 说明 |
|--------|---------|------|
| `hana-music-api` | **自实现** `reqwest` 调用 | 网易云音乐 API。该 npm 包本质是封装了 HTTP 请求 + 加密参数生成。需要自行实现 weapi/linuxapi 加密 |
| `qq-music-api` | **自实现** `reqwest` 调用 | QQ 音乐 API。同理需要自行实现请求签名 |

> ⚠️ **这是最大的迁移工作量**：`hana-music-api` 和 `qq-music-api` 内部包含了网易云和 QQ 音乐的**请求参数加密逻辑**（AES、RSA、MD5 等）。需要逆向我方实际调用的 API 端点列表，然后逐个用 Rust 实现对应的加密和请求逻辑。

**我方实际使用的 hana-music-api 端点：**

| 端点函数 | 用途 | 加密方式 |
|---------|------|---------|
| `cloudsearch` | 搜索 | weapi |
| `songDetail` | 歌曲详情 | weapi |
| `songUrl` / `songUrlV1` | 歌曲 URL | weapi / eapi |
| `lyric` / `lyricNew` | 歌词 | weapi / eapi |
| `playlistDetail` | 歌单详情 | weapi |
| `userPlaylist` | 用户歌单 | weapi |
| `loginStatus` | 登录状态 | weapi |
| `logout` | 登出 | weapi |
| `like` | 喜欢歌曲 | weapi |
| `songLikeCheck` | 喜欢状态查询 | weapi |
| `likelist` | 喜欢列表 | weapi |
| `playlistTracks` / `playlistTrackAdd` | 歌单操作 | weapi |
| `vipInfo` / `vipInfoV2` | VIP 信息 | weapi |
| `loginQrKey` / `loginQrCreate` / `loginQrCheck` | 扫码登录 | weapi |
| `personalized` | 推荐歌单 | weapi |
| `djHot` / `djDetail` / `djProgram` | 播客/DJ | weapi |
| `djSublist` / `userAudio` / `djPaygift` / `recordRecentVoice` | 播客收藏 | weapi |
| `recommendResource` / `recommendSongs` | 每日推荐 | weapi |

**我方实际使用的 qq-music-api 端点：**

| 端点函数 | 用途 |
|---------|------|
| `search` | 搜索 |
| `songDetail` (`song`) | 歌曲详情 |
| `songUrl` (`song/url`) | 歌曲 URL |
| `lyric` | 歌词 |
| `userSonglists` / `userCollectSonglists` | 用户歌单 |
| `playlistDetail` (`songlist`) | 歌单详情 |
| `addSongToPlaylist` (`songlist/add`) | 添加到歌单 |
| `loginStatus` (`user/detail`) | 登录状态 |
| `logout` (`user`) | 登出 |

除此之外，QQ 适配器还有**直接 fetch 调用**：
- `u.y.qq.com/cgi-bin/musicu.fcg` — 批量请求（VIP 信息 + 官方歌单详情 + 歌曲 URL 解析）
- `c.y.qq.com/splcloud/fcgi-bin/smartbox_new.fcg` — 搜索 fallback
- `c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg` — 歌词 fallback

### 3.3 数据处理

| npm 包 / Web API | Cargo crate | 说明 |
|------------------|------------|------|
| `@mineradio/shared` (Zod schemas) | `serde` + `schemars` / `validator` | 类型定义 + 序列化 + 校验 |
| `zod` | `validator` + `serde` | 运行时校验 |
| `Web Crypto API` (`crypto.subtle`) | `aes` + `ctr` crates 或 `ring` | ⚠️ AES-CTR 解密（Soda 音频） |
| `crypto.subtle.digest` | `md5` crate 或 `md-5` | MD5 哈希（Kugou 签名） |
| 自定义 LRC 正则解析 | `regex` | 歌词格式解析 |
| MP4 box 解析（手写） | `mp4` crate 或手写 `nom` 解析 | Soda 音频 MP4 容器解析 |

### 3.4 基础设施

| npm 包 / Node API | Cargo crate | 说明 |
|-------------------|------------|------|
| `node:fs` | `tokio::fs` | 异步文件 I/O |
| `node:path` | `std::path` | 路径操作 |
| `bun:test` | `#[cfg(test)]` + `tokio::test` | 测试框架 |
| 环境变量 `Bun.env` | `std::env` / `dotenvy` | 环境变量 |
| `console.log` | `tracing` + `tracing-subscriber` | 结构化日志 |

### 3.5 其他

| TypeScript 功能 | Rust 实现 | 说明 |
|----------------|----------|------|
| `Promise.allSettled` | `tokio::join!` / `futures::future::join_all` | 并发请求 |
| `Map<ProviderId, string>` | `HashMap<ProviderId, String>` | Cookie 存储 |
| `AbortController` | `tokio::time::timeout` | 请求超时 |
| `JSON.parse` / `JSON.stringify` | `serde_json::from_str` / `serde_json::to_string` | JSON 处理 |
| URL 解析 | `url` crate | URL 参数提取 |
| Base64 | `base64` crate | Base64 编解码 |
| 依赖注入（函数参数） | Trait 泛型 / `Arc<dyn Trait>` | 可测试性 |

---

## 4. 关键数据结构

### 4.1 核心 Trait: `ProviderAdapter`

```rust
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn id(&self) -> ProviderId;

    async fn search(&self, keyword: &str, limit: u32) -> Result<Vec<Track>>;
    async fn song_url(&self, track: &Track, opts: Option<SongUrlOptions>) -> Result<SongUrlResult>;
    async fn track_qualities(&self, track: &Track) -> Result<TrackQualityAvailability>;
    async fn lyric(&self, track: &Track) -> Result<LyricPayload>;
    async fn playlist_list(&self) -> Result<Vec<PlaylistSummary>>;
    async fn playlist_detail(&self, id: &str) -> Result<PlaylistDetail>;
    async fn login_status(&self) -> Result<ProviderLoginStatus>;
    async fn logout(&self) -> Result<()>;

    // 可选方法（默认返回 NOT_IMPLEMENTED）
    async fn like_song(&self, id: &str, liked: bool) -> Result<SongLikeAck> {
        Err(ProviderError::not_implemented("like"))
    }
    async fn check_song_likes(&self, ids: &[String]) -> Result<SongLikeCheckAck> {
        Err(ProviderError::not_implemented("check_likes"))
    }
    async fn add_song_to_playlist(&self, playlist_id: &str, track_id: &str) -> Result<PlaylistAddSongAck> {
        Err(ProviderError::not_implemented("add_to_playlist"))
    }
}
```

### 4.2 错误类型

```rust
pub enum ProviderErrorCode {
    LoginRequired,
    Unavailable,
    CopyrightUnavailable,
    PaidRequired,
    TrialOnly,
    VipRequired,
    NotImplemented,
    Internal,
}

pub struct ProviderError {
    pub code: ProviderErrorCode,
    pub provider: ProviderId,
    pub message: String,
    pub retryable: bool,
    pub action: Option<String>,
    pub restriction: Option<PlaybackRestriction>,
    pub qq_code: Option<i32>,
    pub raw_message: Option<String>,
    pub tried: Option<Vec<String>>,
    pub playback_key_ready: Option<bool>,
}
```

### 4.3 统一响应信封

```rust
#[derive(Serialize)]
#[serde(tag = "ok")]
pub enum ApiResponse<T: Serialize> {
    #[serde(rename = "ok")]
    Success { data: T },
    #[serde(rename = "error")]
    Error { error: ApiError },
}
```

### 4.4 Cookie 三层存储

```rust
pub struct AuthSession {
    /// 运行时内存
    runtime: RwLock<HashMap<ProviderId, String>>,
    /// 持久化文件路径
    persisted: Option<PathBuf>,
}

impl AuthSession {
    pub fn get_cookie(&self, provider: ProviderId) -> Option<String> {
        // 优先级: runtime > persisted file > env var
    }
}
```

---

## 5. 技术难点与对策

### 5.1 ⚠️ 高难度：网易云 API 加密 (weapi / eapi)

**现状**: `hana-music-api` npm 包内部实现了网易云音乐的请求参数加密，包括 AES-CBC 加密、RSA 公钥加密、MD5 摘要等。

**对策**:
1. 研究已有 Rust 实现，如 [`netease-cloud-music-api-rust`](https://github.com/rippleq/netease-cloud-music-api-rust) 或 [`ncm-api-rs`](https://github.com/liningmuse/ncm-api-rs)
2. 从 `hana-music-api` 源码中提取加密参数（公钥、IV、密钥生成规则）
3. 使用 `aes` + `cbc` + `rsa` + `md-5` crate 自行实现
4. 优先实现我方实际调用的端点（~20 个），而非全量 API

**涉及 crate**: `aes`, `cbc`, `rsa`, `md-5`, `base64`, `rand`

### 5.2 ⚠️ 高难度：QQ 音乐 API 签名

**现状**: `qq-music-api` npm 包实现了 QQ 音乐的请求签名（`g_tk`、`qm_keyst` 等参数的生成）。

**对策**:
1. 从我方实际使用的端点反推签名需求
2. QQ 音乐的签名相对网易云简单——主要是 CSRF token (`g_tk`) 的计算，基于 `p_skey` 的 hash33
3. `hash33` 和 `gtkFromPskey` 已在 `qq-qr-login.ts` 中有清晰实现，直接翻译为 Rust

### 5.3 ⚠️ 高难度：Soda 音频 DRM 解密

**现状**: `soda-audio-proxy.ts` 使用 Web Crypto API 对汽水音乐的 MP4 文件进行 AES-CTR 解密。

**技术要点**:
1. **MP4 box 解析**: 定位 `moov/trak/mdia/minf/stbl/stsz` 和 `senc` boxes
2. **IV 提取**: 从 `senc` box 中提取每个 sample 的 IV
3. **AES-CTR 解密**: 对每个 sample 独立解密
4. **Spade 密钥提取**: XOR 混淆算法提取 `playAuth` 中的密钥
5. **stsd box 修复**: 将 `enca` (加密) 替换为 `mp4a` (明文)

**对策**:
- 使用 `mp4` crate 或手写 `nom`/`binrw` 解析器来读取 MP4 结构
- 使用 `aes` + `ctr` crate 进行解密
- 使用 `bytes` crate 进行高效字节操作
- 需要仔细编写测试，用已知的测试向量验证解密正确性

**涉及 crate**: `aes`, `ctr`, `bytes`, `nom` (或 `binrw`)

### 5.4 中难度：MD5 实现（Kugou 签名）

**现状**: `shared-playlist-import.ts` 中有 ~130 行手写 MD5 用于 Kugou API 签名。

**对策**: 直接使用 `md-5` crate，无需手写。

### 5.5 中难度：QR 登录流程

**现状**: QQ QR 登录涉及 5 步重定向链（`ptqrshow → ptqrlogin → check_sig → authorize → musicu.fcg`）。

**对策**:
- 使用 `reqwest` 的 cookie store 自动管理 cookie
- 每个流程步骤作为独立函数实现
- 使用 `tokio::time::timeout` 替代 `AbortController`

### 5.6 中难度：歌词解析（LRC / YRC / QRC / Soda）

**现状**: 四个不同格式的歌词解析器，使用正则表达式。

**对策**:
- 使用 `regex` crate
- 每种格式一个独立解析函数
- 统一输出 `Vec<LyricLine>` 结构

### 5.7 低难度：跨平台搜索评分算法

**现状**: `cross-source-resolver.ts` 中有复杂的评分和去重逻辑。

**对策**: 直接翻译算法逻辑，Rust 模式匹配和迭代器天然适合这类数据处理。

### 5.8 低难度：DJ Beatmap 分析器

**现状**: `podcast.ts` 动态导入外部 JS 文件 `../../../../dj-analyzer.js`。

**对策**: 如果该 JS 文件是第三方模块，可以通过 `std::process::Command` 调用外部程序；或者用 Rust 重写该分析器；或者暂时跳过此功能。

---

## 6. 实施阶段

### Phase 0: 基础设施搭建 (预计 2-3 天)

- [ ] 配置 `Cargo.toml` 依赖
- [ ] 搭建 `axum` HTTP 服务骨架
- [ ] 实现 `config.rs` 环境变量读取
- [ ] 实现 `http/response.rs` 统一响应信封
- [ ] 实现 `services/sidecar_log.rs` 结构化日志
- [ ] 定义 `src/types.rs` 中的所有共享数据结构（从 `@mineradio/shared` 迁移）

**依赖**: 无
**产出**: 可启动的 HTTP 服务，`/health` 端点可用

### Phase 1: Provider 基础设施 (预计 3-4 天)

- [ ] 实现 `ProviderAdapter` trait
- [ ] 实现 `ProviderError` 错误类型
- [ ] 实现 `ProviderRegistry`
- [ ] 实现 `buildCapabilityMatrix()`
- [ ] 实现 `services/auth_session.rs` Cookie 三层存储
- [ ] 实现 `services/fallback.rs` 错误规范化

**依赖**: Phase 0
**产出**: Provider 框架就绪，`/providers/capabilities` 端点可用

### Phase 2: 网易云 Adapter (预计 5-7 天)

- [ ] 实现 weapi/eapi 加密模块
- [ ] 实现 `netease/client.rs` API 客户端
- [ ] 实现 `netease/map.rs` 数据映射 + LRC/YRC 解析
- [ ] 实现 `netease/adapter.rs` 适配器
- [ ] 实现 `services/netease_qr_login.rs` 扫码登录
- [ ] 单元测试

**依赖**: Phase 1
**关键风险**: weapi/eapi 加密实现

### Phase 3: QQ 音乐 Adapter (预计 4-5 天)

- [ ] 实现 `qq/client.rs` API 客户端（含签名算法）
- [ ] 实现 `qq/map.rs` 数据映射 + LRC/QRC 解析
- [ ] 实现 `qq/adapter.rs` 适配器
- [ ] 实现 `services/qq_qr_login.rs` 扫码登录（含 OAuth 重定向链）
- [ ] 单元测试

**依赖**: Phase 1
**关键风险**: OAuth 重定向链 + musicu.fcg 批量请求

### Phase 4: 汽水音乐 Adapter (预计 3-4 天)

- [ ] 实现 `soda/client.rs` API 客户端
- [ ] 实现 `soda/map.rs` 数据映射 + 歌词解析
- [ ] 实现 `soda/adapter.rs` 适配器
- [ ] 实现 `services/soda_qr_login.rs` 扫码登录
- [ ] 实现 `services/soda_audio_proxy.rs` DRM 解密代理 ⚠️
- [ ] 单元测试

**依赖**: Phase 1
**关键风险**: AES-CTR 解密 + MP4 box 解析

### Phase 5: 代理与工具服务 (预计 2-3 天)

- [ ] 实现 `services/audio_proxy.rs`
- [ ] 实现 `services/image_proxy.rs`
- [ ] 实现 `services/cross_source_resolver.rs`（搜索 + 评分 + fallback）
- [ ] 实现 `services/diagnostics.rs`

**依赖**: Phase 2-4 (需要 Provider 可用)

### Phase 6: 内容服务 (预计 3-4 天)

- [ ] 实现 `services/podcast.rs`
- [ ] 实现 `services/discover_home.rs`
- [ ] 实现 `services/weather_radio.rs`
- [ ] 实现 `services/shared_playlist_import.rs`（Apple Music / Qishui / Kugou 导入）

**依赖**: Phase 5

### Phase 7: 路由集成与端到端测试 (预计 2-3 天)

- [ ] 实现 `router.rs` 全部路由
- [ ] 集成所有 service 到 `server.rs`
- [ ] 端到端测试（使用 `reqwest` 作为 HTTP 客户端 + mock 外部 API）
- [ ] 性能基准测试（与 Bun 版本对比）

**依赖**: Phase 2-6

### 总工期估算: 约 24-33 天

---

## 7. 文件结构规划

```
D:\project\Rust\MineRadio-api\
├── Cargo.toml
├── Cargo.lock
├── docs/
│   └── MIGRATION.md          ← 本文档
├── src/
│   ├── main.rs               # 入口，Bun.serve → axum::serve
│   ├── config.rs             # env.ts → 环境变量
│   ├── types.rs              # @mineradio/shared → serde 类型定义
│   ├── router.rs             # server.ts 中的路由分发逻辑
│   ├── server.rs             # 应用状态组装 + 依赖注入
│   ├── http/
│   │   └── response.rs       # envelope.ts → ok/fail/json
│   ├── providers/
│   │   ├── mod.rs            # provider-adapter.ts → trait + 错误类型
│   │   ├── error.rs          # ProviderError / ProviderNotImplementedError
│   │   ├── registry.rs       # registry.ts → 注册表 + capability matrix
│   │   ├── netease/
│   │   │   ├── mod.rs
│   │   │   ├── crypto.rs     # weapi / eapi 加密 (新模块)
│   │   │   ├── client.rs     # hana-client.ts
│   │   │   ├── map.rs        # map.ts → 数据映射 + LRC/YRC 解析
│   │   │   └── adapter.rs    # netease-adapter.ts
│   │   ├── qq/
│   │   │   ├── mod.rs
│   │   │   ├── client.rs     # qq-client.ts
│   │   │   ├── map.rs        # map.ts → 数据映射 + LRC/QRC 解析
│   │   │   ├── adapter.rs    # qq-adapter.ts
│   │   │   └── sign.rs       # g_tk / hash33 签名 (新模块)
│   │   └── soda/
│   │       ├── mod.rs
│   │       ├── client.rs     # soda-client.ts
│   │       ├── map.rs        # map.ts → 数据映射 + 歌词解析
│   │       └── adapter.rs    # soda-adapter.ts
│   └── services/
│       ├── mod.rs
│       ├── audio_proxy.rs    # 通用音频代理
│       ├── image_proxy.rs    # 图片代理
│       ├── soda_audio_proxy.rs  # ⚠️ DRM 解密代理
│       ├── auth_session.rs   # Cookie 三层存储
│       ├── netease_qr_login.rs  # 网易云扫码登录
│       ├── qq_qr_login.rs    # QQ 扫码登录
│       ├── soda_qr_login.rs  # 汽水扫码登录
│       ├── cross_source_resolver.rs  # 跨平台搜索
│       ├── diagnostics.rs    # 诊断聚合
│       ├── discover_home.rs  # 发现页
│       ├── fallback.rs       # 错误规范化
│       ├── podcast.rs        # 播客服务
│       ├── shared_playlist_import.rs  # ⚠️ 分享歌单导入
│       ├── sidecar_log.rs    # 结构化日志
│       └── weather_radio.rs  # 天气电台
└── tests/
    └── integration/          # 集成测试
        └── api_test.rs
```

---

## 8. 测试策略

### 8.1 单元测试
- 每个 Rust 模块包含 `#[cfg(test)] mod tests`
- 使用依赖注入（trait 泛型参数）实现 mock，模拟外部 API 响应
- 参考现有 `.test.ts` 文件的测试用例

### 8.2 集成测试
- 使用 `axum::test` 进行 HTTP 层测试
- 使用 `wiremock` 或 `httpmock` 模拟外部 API
- 参考现有测试中的 fixture 数据

### 8.3 回归测试
- 将现有 TS 测试用例作为验收标准
- 确保 Rust 版本在各种错误场景下的行为与 TS 版本一致
- 所有现有 TS 测试覆盖的场景应在 Rust 测试中复现

### 8.4 现有测试统计

| 文件 | 测试数 | 覆盖内容 |
|------|--------|---------|
| `envelope.test.ts` | 4 | API 响应封装 |
| `registry.test.ts` | 2 | Provider 注册表 |
| `hana-client.test.ts` | 2 | 网易云客户端 |
| `map.test.ts` (netease) | 14 | 数据映射 + LRC/YRC |
| `netease-adapter.test.ts` | 23 | 网易云适配器 |
| `qq-client.test.ts` | 4 | QQ 客户端 |
| `qq-adapter.test.ts` | 26 | QQ 适配器 |
| `soda-adapter.test.ts` | 22 | 汽水适配器 |
| `audio-proxy.test.ts` | 5 | 音频代理 |
| `image-proxy.test.ts` | 5 | 图片代理 |
| `soda-audio-proxy.test.ts` | 7 | DRM 解密代理 |
| `auth-session.test.ts` | 6 | Cookie 存储 |
| `qq-qr-login.test.ts` | 4 | QQ 扫码登录 |
| `soda-qr-login.test.ts` | 7 | 汽水扫码登录 |
| `cross-source-resolver.test.ts` | 11 | 跨平台搜索 |
| `diagnostics.test.ts` | 6 | 诊断聚合 |
| `podcast.test.ts` | 7 | 播客服务 |
| `shared-playlist-import.test.ts` | 11 | 分享歌单导入 |
| `sidecar-log.test.ts` | 7 | 日志服务 |
| `weather-radio.test.ts` | 4 | 天气电台 |
| **合计** | **~177** | |

---

## 附录 A: 环境变量清单

| 变量名 | 用途 | 默认值 |
|--------|------|--------|
| `MINERADIO_SIDECAR_PORT` | 服务端口 | `0` (OS 自动分配) |
| `MINERADIO_APP_VERSION` | 应用版本 | `0.0.0-dev` |
| `MINERADIO_API_VERSION` | API 版本 | `0.1.0` |
| `MINERADIO_SCHEMA_VERSION` | Schema 版本 | `0.1.0` |
| `MINERADIO_NETEASE_COOKIE` | 网易云 Cookie (env fallback) | — |
| `MINERADIO_QQ_COOKIE` | QQ 音乐 Cookie (env fallback) | — |
| `MINERADIO_SODA_COOKIE` | 汽水音乐 Cookie (env fallback) | — |
| `MINERADIO_SESSION_FILE` | Cookie 持久化文件路径 | `$MINERADIO_APP_DATA_DIR/provider-sessions.json` |
| `MINERADIO_SIDECAR_LOG_FILE` | Sidecar 日志文件路径 | — |
| `MINERADIO_APP_DATA_DIR` | 应用数据目录 | — |

## 附录 B: 外部 API 域名汇总

| 域名 | 用途 | 涉及模块 |
|------|------|---------|
| `api.qishui.com` | 汽水音乐 REST API | soda-client, soda-qr-login |
| `u.y.qq.com` | QQ 音乐批量 API | qq-client, qq-adapter, qq-qr-login |
| `c.y.qq.com` | QQ 音乐搜索/歌词 | qq-adapter |
| `y.gtimg.cn` | QQ 音乐图片 CDN | qq/map |
| `ssl.ptlogin2.qq.com` | QQ 扫码登录 | qq-qr-login |
| `graph.qq.com` | QQ OAuth | qq-qr-login |
| `music.163.com` | 网易云音乐 | netease (via hana) |
| `music.apple.com` / `itunes.apple.com` | Apple Music 歌单导入 | shared-playlist-import |
| `qishui.douyin.com` / `music.douyin.com` | 汽水分享页 | shared-playlist-import |
| `t.kugou.com` / `mobiles.kugou.com` / `m.kugou.com` | 酷狗歌单导入 | shared-playlist-import |
| `api.open-meteo.com` | 天气数据 | weather-radio |
| `geocoding-api.open-meteo.com` | 地理编码 | weather-radio |
