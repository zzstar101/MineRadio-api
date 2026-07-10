use std::{env, net::SocketAddr, path::PathBuf};

#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub app_version: String,
    pub api_version: String,
    pub schema_version: String,
    pub log_file: Option<PathBuf>,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            port: read_port("MINERADIO_SIDECAR_PORT").unwrap_or(0),
            app_version: read_string("MINERADIO_APP_VERSION", "0.0.0-dev"),
            api_version: read_string("MINERADIO_API_VERSION", "0.1.0"),
            schema_version: read_string("MINERADIO_SCHEMA_VERSION", "0.1.0"),
            log_file: env::var_os("MINERADIO_SIDECAR_LOG_FILE").map(PathBuf::from),
        }
    }

    pub fn bind_addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.port))
    }
}

fn read_string(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn read_port(key: &str) -> Option<u16> {
    env::var(key).ok()?.parse().ok()
}
