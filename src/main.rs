#[macro_use]
extern crate log;
#[macro_use]
extern crate async_trait;

mod config;
mod http;
#[path = "librespot_audio/lib.rs"]
mod librespot_audio;
#[path = "librespot_core/lib.rs"]
mod librespot_core;
#[path = "librespot_metadata/lib.rs"]
mod librespot_metadata;
#[path = "librespot_protocol/lib.rs"]
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
