#[macro_use]
extern crate log;
extern crate async_trait;

pub mod api;
pub mod config;
#[path = "vendor/librespot_audio/lib.rs"]
mod librespot_audio;
#[path = "vendor/librespot_core/lib.rs"]
mod librespot_core;
#[path = "vendor/librespot_metadata/lib.rs"]
mod librespot_metadata;
#[path = "vendor/librespot_protocol/lib.rs"]
mod librespot_protocol;
pub(crate) mod parsers;
pub(crate) mod providers;
pub(crate) mod services;
mod types;
pub(crate) mod utils;

pub use api::{Api, ApiError, ApiErrorCode, ApiResult};
pub use config::LibraryConfig;
pub use services::sidecar_log::{log_runtime, spawn_runtime_log};
pub use utils::cryptors::{AudioDecryptResult, decrypt_qq_audio, decrypt_soda_audio};
pub use utils::{
    PodcastAudioFormat, PodcastDjAnalyzerParams, PodcastDjBeat, PodcastDjBeatMap,
    PodcastDjPulseBeat, analyze_podcast_dj_beatmap,
};
