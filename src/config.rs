use std::path::PathBuf;

/// Configuration supplied by a static-library host.
#[derive(Clone, Debug)]
pub struct LibraryConfig {
    pub app_version: String,
    pub api_version: String,
    pub schema_version: String,
    /// Directory used for persistent data. Defaults to the current user's
    /// application-data directory plus `MineRadio-Tauri`.
    pub data_dir: Option<PathBuf>,
}

impl Default for LibraryConfig {
    fn default() -> Self {
        Self {
            app_version: "0.0.0-dev".to_owned(),
            api_version: "0.1.0".to_owned(),
            schema_version: "0.1.0".to_owned(),
            data_dir: None,
        }
    }
}

impl LibraryConfig {
    pub(crate) fn persistent_data_dir(&self) -> Result<PathBuf, &'static str> {
        self.data_dir
            .clone()
            .or_else(default_data_dir)
            .ok_or("could not determine the current user's application-data directory")
    }
}

fn default_data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|path| path.join("MineRadio-Tauri"))
}
