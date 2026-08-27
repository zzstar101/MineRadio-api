//! ## 设计准则
//! - adpater 若有伴生接口需考虑若参与跨源是否会产生问题
//!     例如: SongUrl的相关信息必须并入响应体SongUrlResult，因为自动换源无法溯源, 且溯源会复杂接口,伴生接口的响应体会作废
//! - adpater 之间不得互相调用, 封装接口的信息整合应该由客户端进行
//!     例如: SongUrlResult不应该包含VIP信息，是否登录，该由客户端的 login_status 和 Track的PlayableState 来判断
//! - adpater 有参数未透传时需要设计缓存模型, 不得静默降级
//! - model 建模名称规范: providerId + 接口在 adapter 里的名字 + 修饰(如fallback, V1) + Resp
//! - model impl只负责标准化和简单信息整合例如歌词model调用解析器

//! # Provider 分层架构
//!
//! ## 项目定位（先读这里）
//!
//! 本 crate 是「客户端配 API 的中转层」：嵌在音乐客户端旁边的私有取数后端，
//! 不是对外的服务端，更不是分布式服务。由此推论：
//! - 缓存与并发控制的量级按「单个用户实例」估算，不存在多租户 / 水平扩展诉求；
//! - 我们对上游必须表现得像**一个正常客户端**，风控规避的优先级高于吞吐；
//! - 契约的终点是把上游给的诚实一手事实交给客户端，而不是交付替它做好的结论。
//!
//! ## 分层结构
//!
//! 每个音源 provider（qq / netease / kugou / soda / spotify）内部按三层划分职责：
//!
//! ```text
//! client.rs   →  纯 HTTP 传输层
//! model.rs    →  反序列化 & 标准化映射层
//! adapter.rs  →  编排决策层（实现 ProviderAdapter trait）
//! lyric.rs    →  歌词解析器
//! ```
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
//!   （命名规范：providerId + adapter 内方法名 + 修饰词（V1/fallback…）+ Resp）
//! - 每个顶层响应结构体提供 `fn standardize(self) -> …` 映射为领域类型
//! - 只负责标准化与字段级简单整合（如歌词 model 调用解析器），不做跨接口决策
//!
//! **standardize() 判空规则**：
//! - **与 Vec 相关** → 当且仅当返回结构和传入结构的必需项目被 vec 包含时需要判空。
//!   注1：`standardize()` 的返回值以 `Vec<T>` 进入最终标准化结果；
//!   注2：必需项来源于某个 vec 内部结构体。
//! - **需适配层协调决策的中间态** → 按原始字段返回，model 不判空，由 adapter 定策略。
//!
//! ### adapter.rs — 编排决策层
//! - 实现 `ProviderAdapter` trait，是外部调用的唯一入口
//! - 调用 client → 调 model::standardize → 判空 → 兜底 → 错误映射
//! - 需要两边协调决策的逻辑（如歌词解析策略选择）在此层完成
//! - 尚在使用的 `map.rs`（Value → 领域类型的临时映射函数）随建模移植逐步移除
//!
//! ```text
//! 外部 → adapter  →  client  →  HTTP
//!              ↘  model::standardize()  →  领域类型
//!              ↘  判空 / 兜底 / 错误映射
//! ```
//!
//! adapter 不直接返回 model 的原始响应结构体给外部，所有返回值必须是领域类型或
//! `ProviderResult<T>`。
//!
//! ## 缓存设计：读穿缓存为什么和单飞闸门绑定
//!
//! 以网易歌单详情为例，缓存在本层解决四件事：
//!
//! 1. **切片落在本地是刚需**。能透传分页参数的切片本来该由服务器干，但确实存在
//!    不透传的接口形态（一次回全量 / 固定窗口）。这种情况下数据不接在手里，
//!    就没法按上层需要自由切片。
//! 3. **缓存扩容有窗口期**，平均全量数据拉取在400ms左右，如果用户短期快速翻页
//!    则会触发并发。所以引入单飞闸门, 虽然其真实触发机会少，但采取的单飞闸门对
//!    比普通互斥锁缓存以及不带缓存，并没有什么额外的巨大开销。
//! 4. **陈旧性由守卫兜底**：过期条目由后台守卫（janitor）周期清扫，避免超时后
//!    读到错误信息；后续计划提供强制刷新选项配合它。
//!
//! ## 并行控制：不是过度设计
//!
//! 单实例场景下乍看几个互斥锁就能完事，但这些闸门各对应一类真实的事故序列：
//!
//! - **翻页放大合并**：排队者的容量需求归并（demand 合并），持锁者一次拉满喂饱队列；
//! - **同 key 单飞**：同一份上游响应被多方消费（播放地址 / 试听区间 / 品质表）时，
//!   同 key 的并发命中只会真正打一次上游，短窗内复用结果；
//! - **清扫竞态防护**：有人在排队时守卫不得清空槽位（pending/demand 计数保护），
//!   否则新旧两把闸门并存，单飞会被清扫窗口打穿。
//!
//! 统一目标只有一个：让上游眼中的我们是正常使用产品的客户端，而不是一个高并发
//! 爬虫。删掉其中任何一层，都可能会在某类操作序列上放大请求量。
//!
//! ## 设计准则
//!
//! 1. **客户端的事由客户端来搞，API 不准替客户端整合信息、不替它操心**。
//!    adapter 之间不得互相调用；想把多个来源拼成一个好写的结论时，答案是
//!    让客户端自己做判断，而不是服务端先揉在一起。
//!    例：SongUrlResult 不携带 VIP / 登录态回声 —— 登录态问 login_status，
//!    可播性提示看 Track.playableState。
//! 2. **伴生事实并入主响应体，不开伴生端点**。与取址绑定的信息（如试听区间）
//!    必须放进 SongUrlResult 本体：跨源自动换源之后溯源不可行，而溯源路径
//!    复杂化了接口还让伴生端点拿到的数据和实际播放对不上。
//! 3. **adapter 有参数未透传时必须显式设计**（如上文缓存模型），禁止静默降级。
//! 4. **判断后置**：服务端只交一手事实不下结论；失败走类型化错误码；
//!    字段允许缺席胜过必填逼出来的硬编数据。
//! 5. 允许的例外只有一种：**必经链路上的多上游调用**（soda 取址的两跳、
//!    qq 取址时 cdn+detail 并发）——因为该事实本身拆不开，不是为了省事。
//! 6. 尽可能地还原客户端的行为，而不是能过就行。部分参数目前是非强校验的，
//!    后期可能会产生强校验，例如汽水的二神校验。

pub mod error;
pub mod kugou;
pub(crate) mod lyric;
pub mod netease;
pub mod qq;
pub mod soda;
pub mod spotify;

use async_trait::async_trait;

use crate::types::{
    AlbumDetail, AlbumSummary, LyricPayload, PlaylistAddSongAck, PlaylistDetail, PlaylistSummary,
    ProviderLoginStatus, RecommendationPage, SearchType, SongLikeAck, SongLikeCheckAck,
    SongUrlOptions, SongUrlResult, Track, TrackQualityAvailability,
};

pub type ProviderResult<T> = std::result::Result<T, error::ProviderError>;
pub use crate::types::ProviderId;
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn id(&self) -> ProviderId;

    /// 搜索单曲
    async fn search_track(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<Vec<Track>>;

    /// 搜索专辑
    async fn search_album(
        &self,
        _keyword: &str,
        _offset: u32,
        _limit: u32,
    ) -> ProviderResult<Vec<AlbumSummary>> {
        Err(error::ProviderError::not_implemented(
            self.id(),
            "search_album",
        ))
    }

    /// 搜索歌单
    async fn search_playlist(
        &self,
        _keyword: &str,
        _offset: u32,
        _limit: u32,
    ) -> ProviderResult<Vec<PlaylistSummary>> {
        Err(error::ProviderError::not_implemented(
            self.id(),
            "search_playlist",
        ))
    }

    /// 统一搜索入口（向后兼容）：按 search_type 分发到 search_track / search_album / search_playlist
    /// 注意：Album / Playlist 的返回会丢弃类型信息，外部应优先调用具体的 search_* 方法
    #[allow(dead_code)]
    async fn search(
        &self,
        keyword: &str,
        search_type: SearchType,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<Vec<Track>> {
        match search_type {
            SearchType::Track | SearchType::Artist => {
                self.search_track(keyword, offset, limit).await
            }
            SearchType::Album => Err(error::ProviderError::not_implemented(
                self.id(),
                "search (album type — use search_album instead)",
            )),
            SearchType::Playlist => Err(error::ProviderError::not_implemented(
                self.id(),
                "search (playlist type — use search_playlist instead)",
            )),
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
    async fn playlist_detail(
        &self,
        id: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<PlaylistDetail>;
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

    async fn update_song_in_playlist(
        &self,
        _playlist_id: &str,
        _track_id: &str,
        _adding: bool,
    ) -> ProviderResult<PlaylistAddSongAck> {
        Err(error::ProviderError::not_implemented(
            self.id(),
            "update_playlist_song",
        ))
    }

    async fn album_list(&self) -> ProviderResult<Vec<AlbumSummary>> {
        Err(error::ProviderError::not_implemented(
            self.id(),
            "album_list",
        ))
    }

    async fn album_detail(
        &self,
        _id: &str,
        _offset: u32,
        _limit: u32,
    ) -> ProviderResult<AlbumDetail> {
        Err(error::ProviderError::not_implemented(
            self.id(),
            "album_list",
        ))
    }

    async fn stream_next(&self, _id: &str) -> ProviderResult<Track> {
        Err(error::ProviderError::not_implemented(
            self.id(),
            "stream_next",
        ))
    }

    async fn recommendation_page(&self, _refresh: bool) -> ProviderResult<RecommendationPage> {
        Err(error::ProviderError::not_implemented(
            self.id(),
            "recommend_page",
        ))
    }
}
