# Provider API

本文档描述 MineRadio API 的 provider 统一模型与路由。所有示例均使用 JSON；文件编码为 UTF-8。

当前已注册的 provider 为：`netease`、`qq`、`soda`。以下 `{pid}` 可替换为其中之一，例如 `/providers/soda/search`。

> 酷狗目前只有内部 `KugouClient` 核心请求封装，尚未注册 adapter，因此没有 `/providers/kugou/*` 路由。

## 通用约定

- 地址示例使用 `http://127.0.0.1:PORT`，请替换为实际 sidecar 地址。
- 成功响应统一为 `{"ok":true,"data":...}`。
- 失败响应统一为 `{"ok":false,"error":{"code":"...","message":"..."}}`。
- 所有路由均支持 `OPTIONS` 跨域预检。
- Provider 路由中的未知 provider 返回 `404 PROVIDER_NOT_FOUND`；已知但未挂载的 provider 返回 `501 PROVIDER_UNAVAILABLE`。
- 需要登录态的操作可先使用二维码登录，或通过 `session-cookie` 写入 Cookie。

## ProviderAdapter 模型

每个已注册 provider 实现同一个 `ProviderAdapter`。必需方法如下：

| 方法 | 用途 | 对应路由 |
| --- | --- | --- |
| `search` | 搜索歌曲 | `GET /providers/{pid}/search` |
| `song_url` | 获取播放地址 | `POST /providers/{pid}/song-url` |
| `track_qualities` | 查询可用音质 | `POST /providers/{pid}/qualities` |
| `lyric` | 获取歌词 | `POST /providers/{pid}/lyric` |
| `playlist_list` | 获取歌单列表 | `GET /providers/{pid}/playlists` |
| `playlist_detail` | 获取歌单详情 | `GET /providers/{pid}/playlists/{id}` |
| `login_status` | 查询登录状态 | `GET /providers/{pid}/login-status` |
| `logout` | 登出 | `POST /providers/{pid}/logout` |

可选方法为 `like_song`、`check_song_likes`、`add_song_to_playlist`。未实现时返回 `501 NOT_IMPLEMENTED`。

## 数据模型

### Track

需要 Track 的接口都接受以下 JSON。`id`、`provider`、`sourceId`、`title`、`artists` 是调用时必须提供的核心字段。

```json
{
  "id": "0039MnYb0qxYhV",
  "provider": "qq",
  "sourceId": "0039MnYb0qxYhV",
  "mediaMid": "0039MnYb0qxYhV",
  "title": "示例歌曲",
  "artists": ["示例歌手"],
  "album": "示例专辑",
  "coverUrl": "https://example.com/cover.jpg",
  "qualityHints": ["standard", "lossless"],
  "playableState": "unknown",
  "durationMs": 210000,
  "artworkUrl": "https://example.com/artwork.jpg"
}
```

常见响应模型：

| 模型 | 关键字段 |
| --- | --- |
| `SongUrlResult` | `url`、`quality`、`proxied`、`provider`、`playable`、`trial`、`br`、VIP 信息 |
| `TrackQualityAvailability` | `provider`、`trackId`、`defaultQuality`、`qualities[]`；每个音质含 `label`、`requestQuality`、`level`、`br`、`format` |
| `LyricPayload` | `provider`、`trackId`、`lines[]`、`hasTranslation`、`isWordByWord` |
| `PlaylistSummary` | `provider`、`id`、`name`、`coverUrl`、`trackCount`、`trackIds` |
| `PlaylistDetail` | `PlaylistSummary` 字段加 `tracks[]` |
| `ProviderLoginStatus` | `provider`、`loggedIn`，以及可选的昵称、用户 ID、头像和 VIP 信息 |

## 路由

### 能力与会话

| 方法 | 路由 | 参数 / 请求体 | 说明 |
| --- | --- | --- | --- |
| GET | `/providers/capabilities` | 无 | 返回当前声明的 provider 能力矩阵。 |
| POST | `/providers/{pid}/session-cookie` | `{"cookie":"name=value; ..."}` | 写入运行时 Cookie，并持久化到会话存储。 |
| DELETE | `/providers/{pid}/session-cookie` | 无 | 清除运行时和持久化 Cookie。 |
| GET | `/providers/{pid}/login-status` | 无 | 查询当前登录态。 |
| POST | `/providers/{pid}/logout` | 无 | 调用上游登出并清除本地 Cookie。 |

写入 Soda Cookie 示例：

```bash
curl -X POST http://127.0.0.1:PORT/providers/soda/session-cookie \
  -H "content-type: application/json" \
  -d '{"cookie":"sessionid=example; token=example"}'
```

### 二维码登录

QQ、网易云和 Soda 均提供以下三步接口。

| 方法 | 路由 | 参数 | 返回数据 |
| --- | --- | --- | --- |
| GET | `/providers/{pid}/login-qr-key` | 无 | `ProviderLoginQrKey`：`provider`、`key`。 |
| GET | `/providers/{pid}/login-qr-create` | `?key=...` | `ProviderLoginQrImage`：`provider`、`key`、`img`（data URL）以及可选 `url`。 |
| GET | `/providers/{pid}/login-qr-check` | `?key=...` | `ProviderLoginQrCheck`：`code`、`message`、`loggedIn`、`scanned`、`expired`、`stored`。 |

QQ 登录示例：

```bash
curl "http://127.0.0.1:PORT/providers/qq/login-qr-key"
curl "http://127.0.0.1:PORT/providers/qq/login-qr-create?key=<上一步的 key>"
curl "http://127.0.0.1:PORT/providers/qq/login-qr-check?key=<上一步的 key>"
```

### 搜索与播放

| 方法 | 路由 | 参数 / 请求体 | 说明 |
| --- | --- | --- | --- |
| GET | `/providers/{pid}/search` | `keyword` 或 `q` 必填；`limit` 可选，默认 20 | 返回 `Track[]`。 |
| POST | `/providers/{pid}/song-url` | `Track`，或 `{"track":Track,"quality":"lossless"}` | 返回 `SongUrlResult`。 |
| POST | `/providers/{pid}/qualities` | `Track` | 返回 `TrackQualityAvailability`。 |
| POST | `/providers/{pid}/lyric` | `Track` | 返回 `LyricPayload`。 |

Soda 搜索示例：

```bash
curl "http://127.0.0.1:PORT/providers/soda/search?keyword=%E5%91%A8%E6%9D%B0%E4%BC%A6&limit=10"
```

获取播放地址示例：

```bash
curl -X POST http://127.0.0.1:PORT/providers/qq/song-url \
  -H "content-type: application/json" \
  -d '{
    "track": {
      "id": "0039MnYb0qxYhV",
      "provider": "qq",
      "sourceId": "0039MnYb0qxYhV",
      "title": "示例歌曲",
      "artists": ["示例歌手"]
    },
    "quality": "lossless"
  }'
```

### 歌单与收藏

| 方法 | 路由 | 参数 / 请求体 | 说明 |
| --- | --- | --- | --- |
| GET | `/providers/{pid}/playlists` | 无 | 返回当前账号的 `PlaylistSummary[]`。 |
| GET | `/providers/{pid}/playlists/{id}` | 路径参数 `id` | 返回 `PlaylistDetail`。 |
| POST | `/providers/{pid}/like` | `{"id":"歌曲 ID","liked":true}` | 收藏或取消收藏。 |
| GET | `/providers/{pid}/like-check` | `?ids=id1,id2` | 返回各歌曲的收藏状态。 |
| POST | `/providers/{pid}/playlists/add-song` | `{"playlist_id":"歌单 ID","track_id":"歌曲 ID"}` | 向歌单添加歌曲。 |

收藏检查示例：

```bash
curl "http://127.0.0.1:PORT/providers/netease/like-check?ids=123,456"
```

### Soda 专用音频代理

| 方法 | 路由 | 参数 | 说明 |
| --- | --- | --- | --- |
| GET | `/providers/soda/audio-proxy` | `url`（或 `target`）必填；`playAuth` 可选 | 代理 Soda 音频流，可透传 Range 请求。 |

```bash
curl "http://127.0.0.1:PORT/providers/soda/audio-proxy?url=https%3A%2F%2Fexample.com%2Faudio.mp3&playAuth=<授权值>"
```

## 当前支持情况

能力矩阵由 `/providers/capabilities` 返回；以下为当前实现状态。`add-song` 有路由但不在能力矩阵中，因此单独列出。

| Provider | 已声明能力 | 已实现的额外写操作 |
| --- | --- | --- |
| `netease` | 搜索、播放地址、歌词、歌单列表/详情、登录状态、登出、收藏、音质 | 添加歌曲到歌单 |
| `qq` | 搜索、播放地址、歌词、歌单列表/详情、登录状态、登出、音质 | 添加歌曲到歌单；不支持收藏与收藏检查 |
| `soda` | 搜索、播放地址、歌词、歌单列表/详情、登录状态、登出、收藏、音质 | 收藏检查；不支持添加歌曲到歌单 |

provider 可能因未登录、VIP/版权限制或上游不可用而返回错误。常见状态码如下：

| HTTP 状态 | 错误码示例 | 含义 |
| --- | --- | --- |
| 400 | `BAD_REQUEST` | 参数缺失或请求体不符合模型。 |
| 401 | `LOGIN_REQUIRED` | 需要登录。 |
| 404 | `PROVIDER_NOT_FOUND`、`NO_RESULT`、`NO_URL`、`NO_PLAYLIST` | provider 或资源不存在。 |
| 501 | `NOT_IMPLEMENTED`、`PROVIDER_UNAVAILABLE` | 当前方法或 provider 尚未接入。 |
| 502 | `UNAVAILABLE`、`VIP_REQUIRED`、`PAID_REQUIRED` 等 | 上游服务、版权或付费限制。 |
| 500 | `INTERNAL` | 服务内部错误。 |
