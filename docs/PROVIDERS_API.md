# MineRadio API 路由参考

本文档记录当前 Rust sidecar 已注册的全部 HTTP 路由。所有 JSON 示例均为 UTF-8，地址中的 `PORT` 请替换为实际监听端口。

## 通用约定

- 除 `/health`、代理路由外，成功 JSON 响应使用 `{"ok":true,"data":...}`。
- 失败 JSON 响应使用 `{"ok":false,"error":{"code":"...","message":"..."}}`。
- 所有路由支持 `OPTIONS` CORS 预检。
- `{pid}` 是 provider 路径参数，例如 `qq`、`netease` 或 `soda`。
- `Track` 请求体的最小示例：

```json
{
  "id": "0039MnYb0qxYhV",
  "provider": "qq",
  "sourceId": "0039MnYb0qxYhV",
  "title": "示例歌曲",
  "artists": ["示例歌手"]
}
```

可选的 Track 字段为 `mediaMid`、`album`、`coverUrl`、`qualityHints`、`playableState`、`durationMs` 和 `artworkUrl`。

## 基础与诊断

### GET `/health`

用途：检查服务版本与 provider 状态。无需参数。

```json
{
  "ok": true,
  "appVersion": "0.1.0",
  "apiVersion": "v1",
  "schemaVersion": "1",
  "providers": ["netease", "qq", "soda"],
  "providerStatus": { "version": "0.1.0", "providers": [] }
}
```

### GET `/providers/capabilities`

用途：获取 provider 能力矩阵。无需参数。

```json
{
  "ok": true,
  "data": {
    "version": "0.1.0",
    "providers": [
      { "providerId": "qq", "available": true, "capabilities": ["search", "songUrl"], "message": "online" }
    ]
  }
}
```

### GET `/diagnostics`

用途：查看服务版本、provider 状态、近期错误及日志位置。无需参数。

```json
{
  "ok": true,
  "appVersion": "0.1.0",
  "apiVersion": "v1",
  "schemaVersion": "1",
  "providers": [],
  "recentErrors": [],
  "logPointers": { "sidecarRuntimeLog": "C:\\logs\\sidecar.jsonl" }
}
```

## 代理路由

### GET `/audio-proxy`

用途：代理通用音频流，保留客户端的 `Range` 请求头。

请求示例：

```text
GET /audio-proxy?url=https%3A%2F%2Fcdn.example.com%2Fsong.mp3
```

成功响应为上游音频二进制流，例如：

```http
HTTP/1.1 206 Partial Content
content-type: audio/mpeg
content-range: bytes 0-1023/1234567
```

`target` 可以作为 `url` 的别名。

### GET `/image-proxy`

用途：代理图片并使用兼容的请求头访问上游。

请求示例：

```text
GET /image-proxy?url=https%3A%2F%2Fexample.com%2Fcover.jpg
```

成功响应为图片二进制流，例如：

```http
HTTP/1.1 200 OK
content-type: image/jpeg
```

### GET `/providers/soda/audio-proxy`

用途：代理 Soda 音频流；可附带播放授权信息。

请求示例：

```text
GET /providers/soda/audio-proxy?url=https%3A%2F%2Fcdn.example.com%2Faudio.m4a&playAuth=example-token
```

参数：`url`（或 `target`）为上游地址，`playAuth` 可选。成功响应为音频二进制流。

## 发现、天气与跨源解析

### GET `/weather/radio`

用途：按天气生成推荐歌曲。

请求示例：

```text
GET /weather/radio?city=Shanghai&timezone=Asia%2FShanghai
```

可用参数：`city`、`q`、`location`、`lat`、`lon`、`timezone`。

```json
{
  "ok": true,
  "data": {
    "ok": true,
    "weather": {
      "provider": "open-meteo",
      "location": { "name": "Shanghai", "latitude": 31.23, "longitude": 121.47 },
      "label": "晴朗",
      "temperature": 28.0,
      "mood": { "key": "sunny", "title": "晴日", "tagline": "适合轻快音乐", "keywords": ["晴天"] }
    },
    "radio": { "title": "天气电台", "subtitle": "晴日推荐", "seedQueries": ["晴天"], "songs": [], "updatedAt": 0 }
  }
}
```

### GET `/discover/home`

用途：返回发现页聚合数据。无需参数。

```json
{
  "ok": true,
  "data": {
    "loggedIn": false,
    "user": null,
    "dailySongs": [],
    "playlists": [],
    "podcasts": [],
    "mode": "starter",
    "updatedAt": 0
  }
}
```

### GET `/search`

用途：跨 provider 搜索歌曲。

请求示例：

```text
GET /search?keyword=%E5%91%A8%E6%9D%B0%E4%BC%A6&provider=qq&limit=10
```

参数：`keyword` 或 `q` 必填；`provider`、`limit` 可选。

```json
{
  "ok": true,
  "data": [
    { "id": "0039MnYb0qxYhV", "provider": "qq", "sourceId": "0039MnYb0qxYhV", "title": "示例歌曲", "artists": ["示例歌手"] }
  ]
}
```

### POST `/song-url`

用途：根据 Track 跨 provider 获取播放地址。

请求体可直接是 Track，或使用带音质的包装对象：

```json
{
  "track": {
    "id": "0039MnYb0qxYhV",
    "provider": "qq",
    "sourceId": "0039MnYb0qxYhV",
    "title": "示例歌曲",
    "artists": ["示例歌手"]
  },
  "quality": "lossless"
}
```

```json
{
  "ok": true,
  "data": {
    "url": "https://example.com/song.flac",
    "proxied": false,
    "provider": "qq",
    "playable": true,
    "quality": "lossless",
    "requestedQuality": "lossless",
    "br": 999000
  }
}
```

### POST `/shared-playlist/import`

用途：从分享链接或分享文本导入歌单。

```json
{ "url": "https://example.com/shared-playlist" }
```

`text` 可替代 `url`。

```json
{
  "ok": true,
  "data": {
    "provider": "qq",
    "playlist": { "provider": "qq", "id": "123", "name": "示例歌单", "trackIds": ["1"] },
    "tracks": [],
    "trackCount": 1,
    "loadedCount": 1,
    "partial": false,
    "partialReason": ""
  }
}
```

## 播客

| 方法与地址 | 用途 | 请求示例 | 成功响应 `data` 示例 |
| --- | --- | --- | --- |
| GET `/podcast/search` | 搜索播客 | `?keywords=科技&limit=18`；`keyword` 可作别名 | `{ "podcasts": [], "total": 0 }` |
| GET `/podcast/hot` | 热门播客 | `?limit=18&offset=0` | `{ "podcasts": [], "more": false }` |
| GET `/podcast/detail` | 播客详情 | `?id=123`；`rid` 可作别名 | `{ "podcast": { "id":"123", "rid":"123", "name":"示例播客" } }` |
| GET `/podcast/programs` | 播客节目 | `?rid=123&limit=30&offset=0`；`id` 可作别名 | `{ "radio": { "id":"123" }, "programs": [], "more": false, "total": 0 }` |
| GET `/podcast/my` | 我的播客收藏 | 无 | `{ "loggedIn": true, "collections": [] }` |
| GET `/podcast/my/items` | 收藏分类项目 | `?key=collect&limit=36&offset=0` | `{ "loggedIn": true, "key":"collect", "title":"我的收藏", "itemType":"radio", "count":0, "items":[] }` |
| GET `/podcast/dj-beatmap` | 分析音频节拍 | `?url=https%3A%2F%2Fexample.com%2Faudio.mp3&duration=180&intro=10` | `{ "ok": true, "map": {} }` |

上述每个输出都包在通用外层，例如：

```json
{ "ok": true, "data": { "podcasts": [], "total": 0 } }
```

## Provider 通用路由

以下路由使用 `/providers/{pid}` 前缀。请求或响应中的 `{pid}` 需替换为实际 provider ID。

### 登录与会话

| 方法与地址 | 用途 | 请求示例 | 成功响应示例 |
| --- | --- | --- | --- |
| GET `/providers/{pid}/login-qr-key` | 创建扫码 key | 无 | `{ "ok":true, "data":{ "provider":"qq", "key":"key-value" } }` |
| GET `/providers/{pid}/login-qr-create` | 创建二维码图片 | `?key=key-value` | `{ "ok":true, "data":{ "provider":"qq", "key":"key-value", "img":"data:image/png;base64,..." } }` |
| GET `/providers/{pid}/login-qr-check` | 轮询扫码状态 | `?key=key-value` | `{ "ok":true, "data":{ "provider":"qq", "key":"key-value", "code":67, "loggedIn":false, "scanned":true, "expired":false, "stored":false } }` |
| POST `/providers/{pid}/session-cookie` | 保存本地 Cookie | 请求体 `{"cookie":"name=value; token=value"}` | `{ "ok":true, "data":{ "provider":"qq", "stored":true } }` |
| DELETE `/providers/{pid}/session-cookie` | 清除本地 Cookie | 无 | `{ "ok":true, "data":{ "provider":"qq", "stored":false } }` |
| POST `/providers/{pid}/session-cookie/clear` | 强制清除本地 Cookie | 无 | `{ "ok":true, "data":{ "provider":"qq", "stored":false } }` |
| GET `/providers/{pid}/login-status` | 查询登录状态 | 无 | `{ "ok":true, "data":{ "provider":"qq", "loggedIn":true, "nickname":"用户", "userId":"10001" } }` |
| POST `/providers/{pid}/logout` | 上游登出并清除本地 Cookie | 无 | `{ "ok":true, "data":{ "provider":"qq", "loggedOut":true } }` |

`session-cookie/clear` 只清理本地保存的 Cookie，不请求上游登出接口；上游登出请使用 `logout`。

### 歌曲、歌词与音质

| 方法与地址 | 用途 | 请求示例 | 成功响应示例 |
| --- | --- | --- | --- |
| GET `/providers/{pid}/search` | 单 provider 搜索 | `?keyword=%E5%91%A8%E6%9D%B0%E4%BC%A6&limit=20`；`q` 可作别名 | `{ "ok":true, "data":[{ "id":"1", "provider":"qq", "sourceId":"1", "title":"示例歌曲", "artists":[] }] }` |
| POST `/providers/{pid}/song-url` | 获取播放地址 | Track，或 `{ "track": Track, "quality":"lossless" }` | `{ "ok":true, "data":{ "url":"https://example.com/song.mp3", "proxied":false, "quality":"standard" } }` |
| POST `/providers/{pid}/qualities` | 查询可用音质 | Track | `{ "ok":true, "data":{ "provider":"qq", "trackId":"1", "defaultQuality":"standard", "qualities":[{ "provider":"qq", "id":"standard", "label":"标准", "requestQuality":"standard", "source":"declared" }] } }` |
| POST `/providers/{pid}/lyric` | 获取歌词 | Track | `{ "ok":true, "data":{ "provider":"qq", "trackId":"1", "lines":[{ "timeMs":0, "text":"歌词" }], "hasTranslation":false, "isWordByWord":false } }` |

Track 请求体示例：

```json
{
  "id": "1",
  "provider": "qq",
  "sourceId": "1",
  "title": "示例歌曲",
  "artists": ["示例歌手"]
}
```

### 歌单与收藏

| 方法与地址 | 用途 | 请求示例 | 成功响应示例 |
| --- | --- | --- | --- |
| GET `/providers/{pid}/playlists` | 当前用户歌单列表 | 无 | `{ "ok":true, "data":[{ "provider":"qq", "id":"123", "name":"示例歌单", "coverUrl":"", "trackIds":[] }] }` |
| GET `/providers/{pid}/playlists/{id}` | 歌单详情 | `/providers/qq/playlists/123` | `{ "ok":true, "data":{ "provider":"qq", "id":"123", "name":"示例歌单", "trackIds":[], "tracks":[] } }` |
| POST `/providers/{pid}/like` | 收藏或取消收藏歌曲 | 请求体 `{"id":"1","liked":true}` | `{ "ok":true, "data":{ "provider":"qq", "id":"1", "liked":true, "code":0 } }` |
| GET `/providers/{pid}/like-check` | 查询歌曲收藏状态 | `?ids=1,2`；`id` 可作别名，至少提供一个有效 ID | `{ "ok":true, "data":{ "provider":"qq", "ids":["1","2"], "liked":{ "1":true, "2":false } } }` |
| POST `/providers/{pid}/playlists/add-song` | 添加歌曲到歌单 | 请求体 `{"playlist_id":"123","track_id":"1"}` | `{ "ok":true, "data":{ "provider":"qq", "playlistId":"123", "trackId":"1", "success":true, "code":0 } }` |

## 常见错误

| HTTP 状态 | 错误码示例 | 含义 |
| --- | --- | --- |
| 400 | `BAD_REQUEST` | 缺少必填参数、请求体格式错误或参数无效。 |
| 401 | `LOGIN_REQUIRED` | 上游操作需要登录态。 |
| 404 | `NOT_FOUND`、`PROVIDER_NOT_FOUND`、`NO_RESULT`、`NO_URL` | 路由、provider 或资源不存在。 |
| 500 | `INTERNAL` | 服务内部错误。 |
| 501 | `NOT_IMPLEMENTED`、`PROVIDER_UNAVAILABLE` | 当前路由目标不可用。 |
| 502 | `UNAVAILABLE`、`VIP_REQUIRED`、`PAID_REQUIRED` | 上游不可用、版权或付费限制。 |

错误响应示例：

```json
{
  "ok": false,
  "error": {
    "code": "BAD_REQUEST",
    "message": "keyword required"
  }
}
```
