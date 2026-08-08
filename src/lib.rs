#[macro_use]
extern crate log;
extern crate async_trait;

pub mod api;
pub(crate) mod auth_session;
pub mod config;
pub(crate) mod cross_source;
pub(crate) mod error;
#[path = "vendor/librespot_audio/lib.rs"]
mod librespot_audio;
#[path = "vendor/librespot_core/lib.rs"]
mod librespot_core;
#[path = "vendor/librespot_metadata/lib.rs"]
mod librespot_metadata;
#[path = "vendor/librespot_protocol/lib.rs"]
mod librespot_protocol;
pub(crate) mod podcast;
pub(crate) mod provider;
pub(crate) mod providers;
pub(crate) mod qr_login;
pub(crate) mod qr_login_api;
pub(crate) mod sidecar_log;
pub mod types;
pub(crate) mod utils;
pub(crate) mod weather_radio;

pub use api::{Api, ApiError, ApiErrorCode, ApiResult, ProviderApi};
pub use config::LibraryConfig;
pub use qr_login::QrLoginKind;
/// 将结构化事件写入已配置的运行时日志，并可通过后台任务异步提交。
pub use sidecar_log::{log_runtime, spawn_runtime_log};
/// 解密 QQ 音乐或汽水音乐的音频二进制数据，返回解密后的内容及 MIME 类型。
pub use utils::cryptors::{AudioDecryptResult, decrypt_qq_audio, decrypt_soda_audio};
/// 基于音频二进制数据和指定格式生成播客 DJ 节拍图。
pub use utils::{
    PodcastAudioFormat, PodcastDjAnalyzerParams, PodcastDjBeat, PodcastDjBeatMap,
    PodcastDjPulseBeat, analyze_podcast_dj_beatmap,
};
