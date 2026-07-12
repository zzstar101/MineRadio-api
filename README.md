# MineRadio API

Mineradio-tauri 的 Rust sidecar HTTP API。它在本机启动一个 `axum` 服务，为桌面端提供音乐 provider、二维码登录、播放与图片代理、播客、天气电台和分享歌单导入能力。

## 快速开始

要求：Rust stable（项目使用 Rust 2024 edition）。

```powershell
cd D:\project\Rust\MineRadio-api
$env:MINERADIO_SIDECAR_PORT = "18080"
cargo run
```

服务默认仅监听 `127.0.0.1`。未设置 `MINERADIO_SIDECAR_PORT` 时使用端口 `0`，由操作系统分配可用端口；启动日志会打印实际地址。

验证服务：

```powershell
Invoke-RestMethod http://127.0.0.1:18080/health
```

示例响应：

```json
{
  "ok": true,
  "appVersion": "0.0.0-dev",
  "apiVersion": "0.1.0",
  "schemaVersion": "0.1.0",
  "providers": ["netease", "qq", "soda"]
}
```

## 主要能力

- 网易云、QQ、Soda provider：搜索、播放地址、歌词、音质、歌单和登录状态。
- QQ、网易云、Soda 二维码登录与本地 Cookie 会话管理。
- 音频、图片及 Soda 音频代理。
- 跨 provider 搜索与播放地址解析。
- 播客、天气电台、发现页和分享歌单导入。
- 酷狗核心请求与签名客户端；尚未作为 HTTP provider 注册。

## API 文档

- [完整路由、参数与响应示例](docs/PROVIDERS_API.md)
- [Provider 能力举证与人工测试 TODO](docs/PROVIDERS_TODO.md)
- [TypeScript 迁移审计](docs/PROVIDER_TS_AUDIT.md)
- [服务迁移待办](docs/SERVICES_PENDING.md)

除 `/health` 与代理类路由外，成功 JSON 响应使用：

```json
{ "ok": true, "data": {} }
```

失败响应使用：

```json
{ "ok": false, "error": { "code": "BAD_REQUEST", "message": "..." } }
```

## 常用接口

```text
GET  /health
GET  /providers/capabilities
GET  /search?keyword=...
POST /song-url

GET  /providers/{pid}/login-qr-key
GET  /providers/{pid}/login-qr-create?key=...
GET  /providers/{pid}/login-qr-check?key=...
POST /providers/{pid}/session-cookie
DELETE /providers/{pid}/session-cookie
POST /providers/{pid}/session-cookie/clear
GET  /providers/{pid}/search?keyword=...
POST /providers/{pid}/song-url
POST /providers/{pid}/lyric
GET  /providers/{pid}/playlists
```

`{pid}` 使用 `netease`、`qq` 或 `soda`。完整接口清单及请求体示例请参阅 [API 文档](docs/PROVIDERS_API.md)。

## 环境变量

| 变量 | 默认值 | 用途 |
| --- | --- | --- |
| `MINERADIO_SIDECAR_PORT` | `0` | 监听端口。 |
| `MINERADIO_APP_VERSION` | `0.0.0-dev` | `/health` 返回的应用版本。 |
| `MINERADIO_API_VERSION` | `0.1.0` | `/health` 返回的 API 版本。 |
| `MINERADIO_SCHEMA_VERSION` | `0.1.0` | `/health` 返回的 schema 版本。 |
| `MINERADIO_SIDECAR_LOG_FILE` | 未设置 | JSONL 运行日志文件路径。 |
| `MINERADIO_SESSION_FILE` | 未设置 | provider Cookie 持久化文件路径。 |
| `MINERADIO_APP_DATA_DIR` | 未设置 | 未设置会话文件路径时，Cookie 保存目录。 |
| `MINERADIO_NETEASE_COOKIE` | 未设置 | 网易云初始 Cookie。 |
| `MINERADIO_QQ_COOKIE` | 未设置 | QQ 初始 Cookie。 |
| `MINERADIO_SODA_COOKIE` | 未设置 | Soda 初始 Cookie。 |

环境变量也可写入项目根目录的 `.env` 文件，程序启动时会加载它。

## 登录与会话

可通过二维码登录：

```powershell
Invoke-RestMethod http://127.0.0.1:18080/providers/qq/login-qr-key
```

或者直接写入已有 Cookie：

```powershell
Invoke-RestMethod `
  -Method Post `
  -Uri http://127.0.0.1:18080/providers/qq/session-cookie `
  -ContentType 'application/json' `
  -Body '{"cookie":"name=value; token=value"}'
```

清除本地 Cookie：

```powershell
Invoke-RestMethod -Method Post http://127.0.0.1:18080/providers/qq/session-cookie/clear
```

该接口只清除本地保存的登录态；如需请求上游平台登出，请调用 `POST /providers/{pid}/logout`。

## 项目结构

```text
src/
├── router.rs       HTTP 路由与参数解析
├── server.rs       服务启动与依赖组装
├── providers/      网易云、QQ、Soda、酷狗 client/adapter
├── services/       登录、代理、播客、天气、导入等业务服务
├── parsers/        歌词等文本解析
└── utils/          加密、音频分析与通用工具
assets/             内嵌 JS 等运行资源
docs/               API、迁移与能力文档
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

## 许可

本项目采用 [LICENSE](LICENSE) 中声明的许可协议。
