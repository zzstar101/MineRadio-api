use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use crate::config::Config;

pub fn init(config: &Config) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_layer = fmt::layer().with_target(true);

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .init();

    if let Some(path) = &config.log_file {
        tracing::warn!(
            log_file = %path.display(),
            "file logging is configured but not implemented yet"
        );
    }
}
