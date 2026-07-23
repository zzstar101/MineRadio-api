//! # Provider 分层架构
//!
//! 每个音源 provider（qq / netease / kugou / soda）内部按三层划分职责：
//!
//! ```text
//! client.rs   →  纯 HTTP 传输层
//! model.rs    →  反序列化 & 标准化映射层
//! adapter.rs  →  编排决策层（实现 ProviderAdapter trait）
//! ```
//!
//! ## 各层职责
//!
//! ### client.rs — HTTP 传输层
//! - 封装 HTTP 请求（URL、Header、Cookie、签名、UA）
//! - 反序列化为 provider 专属的原始响应结构体（`QqXxxResp` / `NeteaseXxxResp`）
//! - 或返回未定型的 `serde_json::Value`（仅用于无需进一步建模的一次性接口）
//! - **不**接触领域类型（`Track` / `PlaylistSummary` / …）
//! - **不**做判空、兜底、标准化
//!
//! ### model.rs — 反序列化 & 标准化映射层
//! - 定义上游 API JSON 对应的 `#[derive(Deserialize)]` 结构体
//! - 每个顶层响应结构体提供 `fn standardize(self) -> …` 映射为领域类型
//! - **standardize() 判空规则**：
//!   - **列表型最终结果** → 返回 `Option<Vec<T>>`，在 model 层完成判空
//!   - **单体型最终结果** → 直接返回 `T`，不判空
//!   - **需适配层协调决策的中间态** → 按原始字段返回，model 不判空，由 adapter 决定解析策略
//!
//!   "*列表型*" 指：`standardize()` 的返回值以 `Vec<T>` 进入最终标准化的结构体，
//!   或其关键数据为列表形式且无法在反序列化时被检测为空。
//!
//! - **禁止**跨层 standardize 互相调用（字段级 standardize 调用除外）例如歌词model调用解析器
//!
//! ### adapter.rs — 编排决策层
//! - 实现 `ProviderAdapter` trait，是外部调用的唯一入口
//! - 调用 client → 调 model::standardize → 判空 → 兜底 → 错误映射
//! - 需要两边协调决策的逻辑（如歌词解析策略选择）在此层完成
//! - 当前部分 adapter 仍在使用的 `map.rs`（Value → 领域类型的临时映射函数）
//!   将在 model 整体建模移植完成后移除
//!
//! ## 调用流向
//!
//! ```text
//! 外部 → adapter  →  client  →  HTTP
//!              ↘  model::standardize()  →  领域类型
//!              ↘  判空 / 兜底 / 错误映射
//! ```
//!
//! adapter 不直接返回 model 的原始响应结构体给外部，所有返回值必须是领域类型
//! 或 `ProviderResult<T>`。

pub mod error;
pub mod kugou;
pub mod netease;
pub mod qq;
pub mod registry;
pub mod soda;

use async_trait::async_trait;

use crate::types::{
    AlbumDetail, AlbumSummary, LyricPayload, PlaylistAddSongAck, PlaylistDetail, PlaylistSummary,
    ProviderLoginStatus, SearchType, SongLikeAck, SongLikeCheckAck, SongUrlOptions, SongUrlResult,
    Track, TrackQualityAvailability,
};

pub type ProviderResult<T> = std::result::Result<T, error::ProviderError>;
pub use crate::types::ProviderId;
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn id(&self) -> ProviderId;

    /// 搜索单曲
    async fn search_track(&self, keyword: &str, offset: u32, limit: u32) -> ProviderResult<Vec<Track>>;

    /// 搜索专辑
    async fn search_album(&self, _keyword: &str, _offset: u32, _limit: u32) -> ProviderResult<Vec<AlbumSummary>> {
        Err(error::ProviderError::not_implemented(self.id(), "search_album"))
    }

    /// 搜索歌单
    async fn search_playlist(&self, _keyword: &str, _offset: u32, _limit: u32) -> ProviderResult<Vec<PlaylistSummary>> {
        Err(error::ProviderError::not_implemented(self.id(), "search_playlist"))
    }

    /// 统一搜索入口（向后兼容）：按 search_type 分发到 search_track / search_album / search_playlist
    /// 注意：Album / Playlist 的返回会丢弃类型信息，外部应优先调用具体的 search_* 方法
    #[allow(dead_code)]
    async fn search(&self, keyword: &str, search_type: SearchType, offset: u32, limit: u32) -> ProviderResult<Vec<Track>> {
        match search_type {
            SearchType::Track | SearchType::Artist => self.search_track(keyword, offset, limit).await,
            SearchType::Album => {
                Err(error::ProviderError::not_implemented(self.id(), "search (album type — use search_album instead)"))
            }
            SearchType::Playlist => {
                Err(error::ProviderError::not_implemented(self.id(), "search (playlist type — use search_playlist instead)"))
            }
        }
    }

    async fn song_url(
        &self,
        track: &Track,
        opts: Option<SongUrlOptions>,
    ) -> ProviderResult<SongUrlResult>;
    async fn track_qualities(&self, track: &Track) -> ProviderResult<TrackQualityAvailability>;
    async fn lyric(&self, track: &Track) -> ProviderResult<LyricPayload>;
    async fn playlist_list(&self) -> ProviderResult<Vec<PlaylistSummary>>;
    async fn playlist_detail(&self, id: &str, offset: u32, limit: u32) -> ProviderResult<PlaylistDetail>;
    async fn login_status(&self) -> ProviderResult<ProviderLoginStatus>;
    async fn logout(&self) -> ProviderResult<()>;

    async fn like_song(&self, _id: &str, _liked: bool) -> ProviderResult<SongLikeAck> {
        Err(error::ProviderError::not_implemented(self.id(), "like"))
    }

    async fn check_song_likes(&self, _ids: &[String]) -> ProviderResult<SongLikeCheckAck> {
        Err(error::ProviderError::not_implemented(
            self.id(),
            "check_likes",
        ))
    }

    async fn add_song_to_playlist(
        &self,
        _playlist_id: &str,
        _track_id: &str,
    ) -> ProviderResult<PlaylistAddSongAck> {
        Err(error::ProviderError::not_implemented(
            self.id(),
            "add_to_playlist",
        ))
    }

    async fn album_list(&self) -> ProviderResult<Vec<AlbumSummary>> {
        Err(error::ProviderError::not_implemented(
            self.id(),
            "album_list",
        ))
    }

    async fn album_detail(&self, _id: &str, _offset: u32, _limit: u32) -> ProviderResult<AlbumDetail> {
        Err(error::ProviderError::not_implemented(
            self.id(),
            "album_list",
        ))
    }
}
