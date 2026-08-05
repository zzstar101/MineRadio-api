use std::path::PathBuf;

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
