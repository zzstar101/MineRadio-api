#[macro_use]
extern crate log;
#[macro_use]
extern crate async_trait;

mod config;
mod http;
#[path = "vendor/librespot_audio/lib.rs"]
mod librespot_audio;
#[path = "vendor/librespot_core/lib.rs"]
mod librespot_core;
#[path = "vendor/librespot_metadata/lib.rs"]
mod librespot_metadata;
#[path = "vendor/librespot_protocol/lib.rs"]
mod librespot_protocol;
mod parsers;
mod providers;
mod router;
mod server;
mod services;
mod types;
mod utils;

use crate::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let config = Config::from_env();
    services::sidecar_log::init(&config);

    server::serve(config).await
}
