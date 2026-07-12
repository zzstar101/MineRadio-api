# Provider 能力 TODO 与举证

本文档记录四个 provider 的通用能力实现状态及代码举证。

- `[x]`：已在 Rust 代码中找到对应实现。
- `[ ]`：尚未实现，或留给人工测试与校验后填写。
- “人工测试并校验”列由人工填写，不根据单元测试自动勾选。

## 网易云（netease）

| 通用能力 | 代码实现 | 人工测试并校验 | 代码举证 |
| --- | :---: | :---: | --- |
| 注册到 ProviderRegistry | [x] | [ ] | [registry.rs](../src/providers/registry.rs) 的 `PROVIDER_IDS` 与 `build_capability_matrix` |
| 二维码登录 | [x] | [ ] | [netease_qr_login.rs](../src/services/netease_qr_login.rs) 的 `create_key`、`create_image`、`check` |
| 搜索 | [x] | [ ] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `search` |
| 播放地址 | [x] | [ ] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `song_url` |
| 音质列表 | [x] | [ ] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `track_qualities` |
| 歌词 | [x] | [ ] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `lyric` |
| 歌单列表 | [x] | [ ] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `playlist_list` |
| 歌单详情 | [x] | [ ] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `playlist_detail` |
| 登录状态 | [x] | [ ] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `login_status` |
| 登出 | [x] | [ ] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `logout` |
| 收藏 / 取消收藏 | [x] | [ ] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `like_song` |
| 收藏状态查询 | [x] | [ ] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `check_song_likes` |
| 添加歌曲到歌单 | [x] | [ ] | [adapter.rs](../src/providers/netease/adapter.rs) 的 `add_song_to_playlist` |

## QQ 音乐（qq）

| 通用能力 | 代码实现 | 人工测试并校验 | 代码举证 |
| --- | :---: | :---: | --- |
| 注册到 ProviderRegistry | [x] | [ ] | [registry.rs](../src/providers/registry.rs) 的 `PROVIDER_IDS` 与 `build_capability_matrix` |
| 二维码登录 | [x] | [ ] | [qq_qr_login.rs](../src/services/qq_qr_login.rs) 的 `create_key`、`create_image`、`check` |
| 搜索 | [x] | [ ] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `search` |
| 播放地址 | [x] | [ ] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `song_url` |
| 音质列表 | [x] | [ ] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `track_qualities` |
| 歌词 | [x] | [ ] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `lyric` |
| 歌单列表 | [x] | [ ] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `playlist_list` |
| 歌单详情 | [x] | [ ] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `playlist_detail` |
| 登录状态 | [x] | [ ] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `login_status` |
| 登出 | [x] | [ ] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `logout` |
| 收藏 / 取消收藏 | [ ] | [ ] | 未覆写 `ProviderAdapter::like_song`，默认返回 `NOT_IMPLEMENTED` |
| 收藏状态查询 | [ ] | [ ] | 未覆写 `ProviderAdapter::check_song_likes`，默认返回 `NOT_IMPLEMENTED` |
| 添加歌曲到歌单 | [x] | [ ] | [adapter.rs](../src/providers/qq/adapter.rs) 的 `add_song_to_playlist` |

## Soda（soda）

| 通用能力 | 代码实现 | 人工测试并校验 | 代码举证 |
| --- | :---: | :---: | --- |
| 注册到 ProviderRegistry | [x] | [ ] | [registry.rs](../src/providers/registry.rs) 的 `PROVIDER_IDS` 与 `build_capability_matrix` |
| 二维码登录 | [x] | [ ] | [soda_qr_login.rs](../src/services/soda_qr_login.rs) 的 `create_image`、`check` |
| 搜索 | [x] | [ ] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `search` |
| 播放地址 | [x] | [ ] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `song_url` |
| 音质列表 | [x] | [ ] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `track_qualities` |
| 歌词 | [x] | [ ] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `lyric` |
| 歌单列表 | [x] | [ ] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `playlist_list` |
| 歌单详情 | [x] | [ ] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `playlist_detail` |
| 登录状态 | [x] | [ ] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `login_status` |
| 登出 | [x] | [ ] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `logout` |
| 收藏 / 取消收藏 | [x] | [ ] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `like_song` |
| 收藏状态查询 | [x] | [ ] | [adapter.rs](../src/providers/soda/adapter.rs) 的 `check_song_likes` |
| 添加歌曲到歌单 | [ ] | [ ] | 未覆写 `ProviderAdapter::add_song_to_playlist`，默认返回 `NOT_IMPLEMENTED` |

## 酷狗（kugou）

| 通用能力 | 代码实现 | 人工测试并校验 | 代码举证 |
| --- | :---: | :---: | --- |
| 注册到 ProviderRegistry | [ ] | [ ] | [registry.rs](../src/providers/registry.rs) 当前仅列出 `netease`、`qq`、`soda` |
| 二维码登录 | [ ] | [ ] | 未建立 Kugou QR 登录服务 |
| 搜索 | [ ] | [ ] | 未建立 `KugouAdapter` |
| 播放地址 | [ ] | [ ] | 未建立 `KugouAdapter` |
| 音质列表 | [ ] | [ ] | 未建立 `KugouAdapter` |
| 歌词 | [ ] | [ ] | 未建立 `KugouAdapter` |
| 歌单列表 | [ ] | [ ] | 未建立 `KugouAdapter` |
| 歌单详情 | [ ] | [ ] | 未建立 `KugouAdapter` |
| 登录状态 | [ ] | [ ] | 未建立 `KugouAdapter` |
| 登出 | [ ] | [ ] | 未建立 `KugouAdapter` |
| 收藏 / 取消收藏 | [ ] | [ ] | 未建立 `KugouAdapter` |
| 收藏状态查询 | [ ] | [ ] | 未建立 `KugouAdapter` |
| 添加歌曲到歌单 | [ ] | [ ] | 未建立 `KugouAdapter` |
| 核心请求封装与签名 | [x] | [ ] | [client.rs](../src/providers/kugou/client.rs) 的 `KugouClient::request`、`signature_*`、`sign_key` |

## 路由共用举证

已注册 provider 的通用 HTTP 路由定义在 [router.rs](../src/router.rs)：

- `/providers/{pid}/login-qr-key`
- `/providers/{pid}/login-qr-create`
- `/providers/{pid}/login-qr-check`
- `/providers/{pid}/session-cookie` 与 `/providers/{pid}/session-cookie/clear`
- `/providers/{pid}/search`
- `/providers/{pid}/song-url`
- `/providers/{pid}/qualities`
- `/providers/{pid}/lyric`
- `/providers/{pid}/playlists` 与 `/providers/{pid}/playlists/{id}`
- `/providers/{pid}/login-status`、`/providers/{pid}/logout`
- `/providers/{pid}/like`、`/providers/{pid}/like-check`
- `/providers/{pid}/playlists/add-song`
