# MineRadio API

MineRadio-api 是一个 Rust 库（crate），为桌面音乐应用提供多平台音乐 provider、二维码登录、音频解密、播客节拍分析和分享歌单导入能力。宿主应用通过静态链接直接调用其公开 API。

---

> [!IMPORTANT]
> 本项目仅供学习、研究和技术交流使用，旨在提供 API 的调用封装与相关技术实现示例。
>
> 本项目不提供、存储、托管或分发任何音乐内容，也不授予任何平台内容、账号、数据或版权的使用权。使用者应自行遵守所在地区的法律法规以及相关平台的服务条款，并尊重音乐版权，支持正版内容。
>
> 严禁将本项目用于绕过付费、会员、地区或其他访问限制，或用于批量下载、未经授权的获取、复制、传播、再分发及其他侵犯版权或平台权益的行为。
>
> 未经项目作者明确授权，禁止将本项目及其衍生作品用于任何形式的商业用途，包括但不限于付费分发、商业软件集成、商业服务、销售或其他以盈利为目的的使用。
>
> 使用者应自行承担使用本项目所产生的一切风险与责任。项目作者不对因使用、修改、分发或无法使用本项目而造成的任何直接或间接损失承担责任。

---

## 快速开始

要求：Rust stable（项目使用 Rust 2024 edition）。

在 `Cargo.toml` 中添加依赖：

```toml
[dependencies]
mineradio_api = { git = "https://github.com/zzstar101/MineRadio-api.git" }
```

初始化库并调用：

```rust
use mineradio_api::{Api, ApiError, ApiErrorCode, LibraryConfig, ProviderId, Track};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = LibraryConfig {
        app_version: "1.0.0".into(),
        log_path: Some("mineradio.log".into()),
        cookie_file: Some("cookies.json".into()),
        ..Default::default()
    };

    let api = Api::init(config).await?;

    // 跨源搜索
    let tracks = api.search_tracks("关键词", None, 20).await?;
    let albums = api.search_albums("关键词", None, 20).await?;
    let playlists = api.search_playlists("关键词", None, 20).await?;

    // 使用指定 provider
    let status = api.qq.login_status().await?;

    // 优雅关闭
    api.shutdown().await?;

    Ok(())
}
```

## 核心架构

```
Api::init(LibraryConfig)
├── api.qq          ProviderApi    ← QQ 音乐
├── api.netease     ProviderApi    ← 网易云音乐
├── api.soda        ProviderApi    ← 汽水音乐
├── api.kugou       ProviderApi    ← 酷狗音乐
├── api.spotify     ProviderApi    ← Spotify
├── api.search_tracks()            ← 跨源搜索
├── api.search_albums()            ← 跨源专辑搜索
├── api.search_playlists()         ← 跨源歌单搜索
├── api.song_url()                 ← 跨源解析播放地址
├── api.recommendation_pages()     ← 发现页聚合
└── api.qr_login(kind)             ← 按协议获取二维码登录入口
```

`ProviderApi` 提供该平台的音乐能力（搜索、播放、歌词、歌单、收藏与登录状态管理）。二维码登录通过 `Api` 的全局协议注册表获取。

## 主要能力

| 能力 | 说明 |
| --- | --- |
| 音乐搜索 | 跨源 / 单源搜索歌曲、专辑、歌单 |
| 播放地址解析 | 解析各平台歌曲播放 URL 与音质可用性 |
| 歌词 | 获取 LRC 歌词，或逐字时间轴歌词 |
| 歌单管理 | 查看歌单，添加 / 移除歌曲 |
| 专辑 | 查看已收藏专辑与专辑详情 |
| 二维码登录 | QQ（QQ/微信/QQ音乐 三种协议）、网易云、汽水、酷狗 |
| Cookie 会话管理 | 本地持久化与内存模式，支持登出 |
| 发现页 / 推荐 | 单源与跨源聚合发现页 |
| 音频解密 | QQ 音乐、汽水音乐加密音频解密 |
| DJ 节拍图 | 基于音频数据分析生成节拍图 |
| 天气电台 | 根据天气信息生成推荐歌单 |

## API 概览

### 库级入口

- `Api::init(config: LibraryConfig)` — 初始化所有 provider、日志与会话持久化
- `api.shutdown()` — 优雅关闭，写入退出日志
- `api.search_tracks(keyword, provider, limit)` — 跨源搜索
- `api.search_albums(keyword, provider, limit)` — 跨源搜索专辑
- `api.search_playlists(keyword, provider, limit)` — 跨源搜索歌单
- `api.song_url(track, options)` — 跨源解析播放地址
- `api.recommendation_pages()` — 跨源聚合发现页

### ProviderApi（每个 provider 独立提供）

- `provider.id()` — 返回 `ProviderId`（`Qq` / `Netease` / `Soda` / `Kugou` / `Spotify`）
- `provider.search_track(keyword, offset, limit)` — 搜索歌曲
- `provider.search_album(keyword, offset, limit)` — 搜索专辑
- `provider.search_playlist(keyword, offset, limit)` — 搜索歌单
- `provider.song_url(track, options)` — 解析播放地址
- `provider.track_qualities(track)` — 查询音质可用性
- `provider.lyric(track)` — 获取歌词
- `provider.playlist_list()` — 已收藏歌单列表
- `provider.playlist_detail(id, offset, limit)` — 歌单详情
- `provider.album_list()` — 已收藏专辑列表
- `provider.album_detail(id, offset, limit)` — 专辑详情
- `provider.login_status()` — 登录状态
- `provider.logout()` — 登出
- `provider.like_song(id, liked)` — 收藏 / 取消收藏
- `provider.check_song_likes(ids)` — 批量查询收藏状态
- `provider.update_song_in_playlist(playlist_id, track_id, adding)` — 添加 / 移除歌单歌曲
- `provider.recommendation_page()` — 发现页

### QrLoginApi

- `api.qr_login(kind)` — 按 `QrLoginKind` 获取协议；可用值为 `Qq`、`QqMusic`、`Wechat`、`Netease`、`Kugou`、`Soda`
- `api.qr_login_kinds()` — 枚举当前已注册的二维码登录协议
- `qr.create_key()` — 获取二维码 key
- `qr.create_image(key)` — 生成二维码图片
- `qr.check(key)` — 轮询扫码状态

### 导出工具函数

库根路径直接导出以下函数，无需通过 `Api` 实例：

```rust
use mineradio_api::{
    decrypt_qq_audio,        // 解密 QQ 音乐音频
    decrypt_soda_audio,      // 解密汽水音乐音频
    AudioDecryptResult,      // 解密结果类型
    analyze_podcast_dj_beatmap, // 分析播客 DJ 节拍图
    PodcastDjBeatMap,
    PodcastDjBeat,
    PodcastDjPulseBeat,
    PodcastAudioFormat,
    PodcastDjAnalyzerParams,
    log_runtime,             // 写入结构化运行时日志
    spawn_runtime_log,       // 启动后台日志提交任务
};
```

## 配置

`LibraryConfig` 结构体由宿主在初始化时传入：

```rust
pub struct LibraryConfig {
    pub app_version: String,        // 应用版本号，写入日志
    pub api_version: String,        // API 版本号
    pub schema_version: String,     // schema 版本号
    pub log_path: Option<PathBuf>,  // JSONL 日志路径；None 不写文件
    pub cookie_file: Option<PathBuf>, // Cookie 持久化文件；None 仅内存
}
```

所有字段均可使用 `Default`：

```rust
let config = LibraryConfig {
    cookie_file: Some("cookies.json".into()),
    ..Default::default()
};
```

## 错误处理

所有公开 API 返回 `ApiResult<T>`（即 `Result<T, ApiError>`）。`ApiError` 包含稳定错误码和人类可读信息：

```rust
pub struct ApiError {
    pub code: ApiErrorCode,  // BAD_REQUEST, NOT_FOUND, LOGIN_REQUIRED, ...
    pub message: String,
}
```

## 项目结构

```text
src/
├── lib.rs              crate 根，公开导出 API、类型与工具函数
├── api.rs              Api 生命周期与 facade
├── config.rs           LibraryConfig 配置结构
├── types.rs            共享数据类型（Track、Playlist、Lyric 等）
├── qr_login/           按 QrLoginKind 平铺的二维码登录协议
├── providers/          音源 adapter/client/model 与内部歌词解析
├── cross_source.rs     跨源检索、排序与切源策略
├── auth_session.rs     Cookie 会话存储
├── sidecar_log.rs      运行时日志
├── utils/               加密、音频分析与通用工具
└── vendor/              内嵌的 librespot 组件（audio/core/metadata/protocol）
docs/                    文档
```

## 开发与验证

```powershell
cargo fmt --check
cargo test
cargo check
```

只运行指定测试：

```powershell
cargo test <测试名称>
```

## 鸣谢

- QQ 音乐 MQTT 扫码登录和 sign 计算实现参考于 [AstronW/netease-qq-music-api](https://github.com/AstronW/netease-qq-music-api)，相关移植代码遵循其 MIT 许可证。

## 许可

本项目采用 [LICENSE](LICENSE) 中声明的许可协议。
