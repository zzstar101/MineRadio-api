use std::{env, net::SocketAddr, path::PathBuf};

/// Configuration supplied by a static-library host.
#[derive(Clone, Debug)]
pub struct LibraryConfig {
    pub app_version: String,
    pub api_version: String,
    pub schema_version: String,
    /// A log file path, or a directory in which MineRadio creates a timestamped JSON log.
    pub log_path: Option<PathBuf>,
    /// The JSON file used to persist provider cookies. `None` keeps cookies in memory only.
    pub cookie_file: Option<PathBuf>,
}

impl Default for LibraryConfig {
    fn default() -> Self {
        Self {
            app_version: "0.0.0-dev".to_owned(),
            api_version: "0.1.0".to_owned(),
            schema_version: "0.1.0".to_owned(),
            log_path: None,
            cookie_file: None,
        }
    }
}

impl From<LibraryConfig> for Config {
    fn from(config: LibraryConfig) -> Self {
        Self {
            port: 0,
            app_version: config.app_version,
            api_version: config.api_version,
            schema_version: config.schema_version,
            log_path: config.log_path,
            cookie_file: config.cookie_file,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Config {
    pub port: u16,
    pub app_version: String,
    pub api_version: String,
    pub schema_version: String,
    pub log_path: Option<PathBuf>,
    pub cookie_file: Option<PathBuf>,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            port: read_port("MINERADIO_SIDECAR_PORT").unwrap_or(0),
            app_version: read_string("MINERADIO_APP_VERSION", "0.0.0-dev"),
            api_version: read_string("MINERADIO_API_VERSION", "0.1.0"),
            schema_version: read_string("MINERADIO_SCHEMA_VERSION", "0.1.0"),
            log_path: env::var_os("MINERADIO_SIDECAR_LOG_FILE").map(PathBuf::from),
            cookie_file: env::var_os("MINERADIO_SESSION_FILE").map(PathBuf::from),
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
