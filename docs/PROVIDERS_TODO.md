# Provider 能力 TODO 与举证

本文档记录五个 provider 的通用能力实现状态及代码举证。

- “人工测试并校验”列由人工填写，不根据单元测试自动勾选。

## 目录重构计划

目标目录：

```text
src/
├── api.rs
├── config.rs
├── types.rs
├── error.rs
├── provider.rs
├── qr_login_api.rs
├── cross_source.rs
├── podcast.rs
├── weather_radio.rs
├── auth_session.rs
├── sidecar_log.rs
├── qr_login/
│   ├── mod.rs
│   ├── common.rs
│   ├── qq.rs
│   ├── qq_music.rs
│   ├── wechat.rs
│   ├── netease.rs
│   ├── kugou.rs
│   ├── soda.rs
│   └── mqtt.rs
└── providers/
    ├── lyric/
    │   ├── mod.rs
    │   └── lrc.rs
    ├── qq/lyric.rs
    ├── netease/lyric.rs
    ├── kugou/lyric.rs
    └── soda/lyric.rs
```

- [x] 合并 `api/cross_source_resolver.rs` 与 `api/cross_source.rs`，使跨源逻辑保持单文件。
- [x] 让跨源链路直接使用 `ProviderError`，移除该链路中的 `anyhow`。
- [x] 移除 `api/`、`services/` 与 `parsers/` 目录，按上述目标移动模块并更新引用。
- [x] 将二维码登录以 `QrLoginKind` 注册表公开，不再挂在 `ProviderApi` 的无语义 `Vec` 上。

## 响应体建模 TODO

- [ ] 脆弱响应体建模：目前部分 provider 响应体直接按“坏了就整个请求失败”的方式建模，不做过多的 `Option` 兜底。为保持免试错完整性，后续可以把高频易变的字段逐步补成 `Option`/默认值，避免单个字段缺失导致整条响应失败。
- [ ] 收藏状态字段语义：`collected = null` 表示“该来源无法通过现有在线接口确认收藏状态”，不是接口漏填。
  - QQ：歌单/歌曲/电台的收藏状态没有公开的在线查询接口，统一填 `null`（`collected: None`）。
  - 网易推荐卡（模块 1 / “每日30首”）：推荐卡整体不提供收藏能力，且被标准化为“每日30首”的推荐类型本就不可收藏，故不提供收藏接口；后续如需支持推荐卡收藏再补充。

## 网易云（netease）

| 通用能力 | 代码实现 | 人工测试并校验 | 代码举证 |
| --- | :---: | :---: | --- |
| 注册到 ProviderRegistry | [x] | [x] | [api.rs](../src/api.rs) 的 `ApiInner::new` 组装 provider Map，[cross_source.rs](../src/cross_source.rs) 的 `PROVIDER_IDS` 维护清单 |
| 二维码登录 | [x] | [x] | [netease.rs](../src/qr_login/netease.rs) 的 `create_key`、`create_image`、`check` |
| 搜索 | [x] | [x] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `search` |
| 播放地址 | [x] | [x] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `song_url` |
| 音质列表 | [x] | [x] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `track_qualities` |
| 歌词 | [x] | [x] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `lyric` |
| 歌单列表 | [x] | [x] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `playlist_list` |
| 歌单详情 | [x] | [x] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `playlist_detail` |
| 登录状态 | [x] | [x] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `login_status` |
| 登出 | [x] | [ ] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `logout` |
| 收藏 / 取消收藏 | [x] | [ ] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `like_song` |
| 收藏状态查询 | [x] | [ ] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `check_song_likes` |
| 添加歌曲到歌单 | [x] | [ ] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `update_song_in_playlist`（`adding = true`） |
| 从歌单移除歌曲 | [ ] | [ ] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `update_song_in_playlist` 在 `adding = false` 时返回 `NOT_IMPLEMENTED` |
| 专辑列表 | [x] | [x] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `album_list` |
| 专辑详情 | [x] | [x] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `album_detail`（offset/limit 暂未透传） |

## QQ 音乐（qq）

| 通用能力 | 代码实现 | 人工测试并校验 | 代码举证 |
| --- | :---: | :---: | --- |
| 注册到 ProviderRegistry | [x] | [x] | [api.rs](../src/api.rs) 的 `ApiInner::new` 组装 provider Map，[cross_source.rs](../src/cross_source.rs) 的 `PROVIDER_IDS` 维护清单 |
| 二维码登录 | [x] | [x] | [qq.rs](../src/qr_login/qq.rs)、[qq_music.rs](../src/qr_login/qq_music.rs)、[wechat.rs](../src/qr_login/wechat.rs) |
| 搜索 | [x] | [x] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `search` |
| 播放地址 | [x] | [x] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `song_url` |
| 音质列表 | [x] | [x] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `track_qualities` |
| 歌词 | [x] | [x] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `lyric` |
| 歌单列表 | [x] | [x] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `playlist_list` |
| 歌单详情 | [x] | [x] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `playlist_detail` |
| 登录状态 | [x] | [x] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `login_status` |
| 登出 | [x] | [x] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `logout` |
| 收藏 / 取消收藏 | [x] | [x] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `like_song` |
| 收藏状态查询 | [ ] | [ ] | [mod.rs](../src/providers/qq/mod.rs) 已明确搜索接口/详情接口无法得知是否收藏单曲/歌单 |
| 添加歌曲到歌单 | [x] | [x] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `update_song_in_playlist`（`adding = true`） |
| 从歌单移除歌曲 | [x] | [x] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `update_song_in_playlist`（`adding = false`） |
| 专辑列表 | [x] | [x] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `album_list` |
| 专辑详情 | [x] | [x] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `album_detail` |

## 汽水音乐（soda）

| 通用能力 | 代码实现 | 人工测试并校验 | 代码举证 |
| --- | :---: | :---: | --- |
| 注册到 ProviderRegistry | [x] | [x] | [api.rs](../src/api.rs) 的 `ApiInner::new` 组装 provider Map，[cross_source.rs](../src/cross_source.rs) 的 `PROVIDER_IDS` 维护清单 |
| 二维码登录 | [x] | [x] | [soda.rs](../src/qr_login/soda.rs) 的 `create_image`、`check` |
| 搜索 | [x] | [x] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `search` |
| 播放地址 | [x] | [x] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `song_url` |
| 音质列表 | [x] | [x] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `track_qualities` |
| 歌词 | [x] | [x] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `lyric` |
| 歌单列表 | [x] | [x] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `playlist_list` |
| 歌单详情 | [x] | [x] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `playlist_detail` |
| 登录状态 | [x] | [x] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `login_status` |
| 登出 | [x] | [x] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `logout` |
| 收藏 / 取消收藏 | [x] | [x] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `like_song` |
| 收藏状态查询 | [x] | [x] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `check_song_likes` |
| 添加歌曲到歌单 | [x] | [x] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `update_song_in_playlist`（`adding = true`） |
| 从歌单移除歌曲 | [x] | [x] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `update_song_in_playlist`（`adding = false`） |
| 专辑列表 | [x] | [x] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `album_list` |
| 专辑详情 | [x] | [x] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `album_detail` |

## 酷狗（kugou）

| 通用能力 | 代码实现 | 人工测试并校验 | 代码举证 |
| --- | :---: | :---: | --- |
| 注册到 ProviderRegistry | [x] | [x] | [api.rs](../src/api.rs) 的 `ApiInner::new` 组装 provider Map，[cross_source.rs](../src/cross_source.rs) 的 `PROVIDER_IDS` 维护清单 |
| 二维码登录 | [x] | [x] | [kugou.rs](../src/qr_login/kugou.rs) |
| 搜索 | [x] | [x] | [adapter.rs](../src/providers/kugou/adapter.rs) 的 `search`，调用 [client.rs](../src/providers/kugou/client.rs) 的 `search` |
| 播放地址 | [x] | [x] | [adapter.rs](../src/providers/kugou/adapter.rs) 的 `song_url`，调用 [client.rs](../src/providers/kugou/client.rs) 的 `song_url` |
| 音质列表 | [x] | [ ] | [adapter.rs](../src/providers/kugou/adapter.rs) 的 `track_qualities` |
| 歌词 | [x] | [x] | [adapter.rs](../src/providers/kugou/adapter.rs) 的 `lyric`，调用 `lyric_search` 与 `lyric_krc`/`lyric` |
| 歌单列表 | [x] | [x] | [adapter.rs](../src/providers/kugou/adapter.rs) 的 `playlist_list`，调用 H5 `get_all_list` |
| 歌单详情 | [x] | [x] | [adapter.rs](../src/providers/kugou/adapter.rs) 的 `playlist_detail`，调用 H5 `get_list_all_file` |
| 登录状态 | [x] | [x] | [adapter.rs](../src/providers/kugou/adapter.rs) 的 `login_status` |
| 登出 | [x] | [ ] | [adapter.rs](../src/providers/kugou/adapter.rs) 的 `logout` |
| 收藏 / 取消收藏 | [x] | [ ] | [adapter.rs](../src/providers/kugou/adapter.rs) 的 `like_song`，通过默认收藏歌单写入 |
| 收藏状态查询 | [x] | [ ] | [adapter.rs](../src/providers/kugou/adapter.rs) 的 `check_song_likes` |
| 添加歌曲到歌单 | [x] | [ ] | [adapter.rs](../src/providers/kugou/adapter.rs) 的 `update_song_in_playlist`（`adding = true`） |
| 从歌单移除歌曲 | [x] | [ ] | [adapter.rs](../src/providers/kugou/adapter.rs) 的 `update_song_in_playlist`（`adding = false`） |
| 专辑列表 | [x] | [x] | [adapter.rs](../src/providers/kugou/adapter.rs) 的 `album_list`，调用 `user_collection_list` 并通过 [model.rs](../src/providers/kugou/model.rs) 的 `KugouCollectionResp::standardize_albums` 标准化 |
| 专辑详情 | [x] | [x] | [adapter.rs](../src/providers/kugou/adapter.rs) 的 `album_detail`，调用 [client.rs](../src/providers/kugou/client.rs) 的 `album_detail` 与 `album_songs` |
| 核心请求封装与签名 | [x] | [ ] | [client.rs](../src/providers/kugou/client.rs) 的 `KugouClient::request`、`signature_*`、`sign_key` |

## Spotify（spotify）

| 通用能力 | 代码实现 | 人工测试并校验 | 代码举证 |
| --- | :---: | :---: | --- |
| 注册到 ProviderRegistry | [x] | [ ] | [api.rs](../src/api.rs) 的 `ApiInner::new` 组装 provider Map，[cross_source.rs](../src/cross_source.rs) 的 `PROVIDER_IDS` 维护清单 |
| 二维码登录 | [ ] | [ ] | 未建立 Spotify QR 登录服务 |
| 搜索 | [x] | [ ] | [adapter.rs](../src/providers/spotify/adapter.rs) 的 `search_track` |
| 播放地址 | [x] | [ ] | [adapter.rs](../src/providers/spotify/adapter.rs) 的 `song_url`，通过 librespot 音频代理返回播放地址 |
| 音质列表 | [x] | [ ] | [adapter.rs](../src/providers/spotify/adapter.rs) 的 `track_qualities`，调用 `SpotifyClient::available_qualities` |
| 歌词 | [ ] | [ ] | [adapter.rs](../src/providers/spotify/adapter.rs) 的 `lyric`（官方 API 不提供歌词） |
| 歌单列表 | [x] | [ ] | [adapter.rs](../src/providers/spotify/adapter.rs) 的 `playlist_list` |
| 歌单详情 | [x] | [ ] | [adapter.rs](../src/providers/spotify/adapter.rs) 的 `playlist_detail` |
| 登录状态 | [x] | [ ] | [adapter.rs](../src/providers/spotify/adapter.rs) 的 `login_status` |
| 登出 | [x] | [ ] | [adapter.rs](../src/providers/spotify/adapter.rs) 的 `logout` |
| 收藏 / 取消收藏 | [x] | [ ] | [adapter.rs](../src/providers/spotify/adapter.rs) 的 `like_song` |
| 收藏状态查询 | [x] | [ ] | [adapter.rs](../src/providers/spotify/adapter.rs) 的 `check_song_likes` |
| 添加歌曲到歌单 | [x] | [ ] | [adapter.rs](../src/providers/spotify/adapter.rs) 的 `update_song_in_playlist`（`adding = true`） |
| 从歌单移除歌曲 | [x] | [ ] | [adapter.rs](../src/providers/spotify/adapter.rs) 的 `update_song_in_playlist`（`adding = false`） |
| 专辑列表 | [x] | [ ] | [adapter.rs](../src/providers/spotify/adapter.rs) 的 `album_list` |
| 专辑详情 | [x] | [ ] | [adapter.rs](../src/providers/spotify/adapter.rs) 的 `album_detail` |

## 能力对外暴露方式

HTTP sidecar 与 `router.rs` 已随静态库迁移移除；上述通用能力现在通过公开的 `Api`/`ProviderApi` 方法暴露（见 [lib.rs](../src/lib.rs) 的 re-export 与 [api.rs](../src/api.rs)）。旧 HTTP 路由参考见 [PROVIDERS_API.md](PROVIDERS_API.md)（已标记为历史）。
