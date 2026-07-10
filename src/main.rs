mod config;
mod http;
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
