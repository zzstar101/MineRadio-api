use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde_json::{Map, Value};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use crate::config::Config;

const DEFAULT_MAX_BYTES: u64 = 1024 * 1024;
const REDACTED: &str = "[redacted]";
const SENSITIVE_KEY_PATTERNS: [&str; 10] = [
    "cookie",
    "authorization",
    "auth",
    "token",
    "music_u",
    "qm_keyst",
    "qqmusic_key",
    "wxskey",
    "uin",
    "csrf",
];
const SENSITIVE_VALUE_PATTERNS: [&str; 10] = [
    "music_u",
    "qm_keyst",
    "qqmusic_key",
    "wxskey",
    "authorization",
    "bearer ",
    "cookie:",
    "cookie=",
    "access_token",
    "__csrf",
];

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

#[derive(Clone, Debug)]
pub struct SidecarLoggerOptions {
    pub file_path: Option<PathBuf>,
    pub max_bytes: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct SidecarLogger {
    file_path: Option<PathBuf>,
    max_bytes: u64,
}

impl SidecarLogger {
    pub async fn log(&self, entry: Value) {
        let Some(file_path) = &self.file_path else {
            return;
        };
        let _ = append_sidecar_log(file_path, entry, Some(self.max_bytes)).await;
    }

    pub async fn flush(&self) {}

    pub async fn dispose(&self) {
        self.flush().await;
    }
}

pub fn sidecar_log_file() -> Option<String> {
    env::var("MINERADIO_SIDECAR_LOG_FILE")
        .ok()
        .map(|raw| raw.trim().to_owned())
        .filter(|raw| !raw.is_empty())
}

pub fn create_sidecar_logger(opts: SidecarLoggerOptions) -> SidecarLogger {
    let file_path = opts
        .file_path
        .or_else(|| sidecar_log_file().map(PathBuf::from));
    SidecarLogger {
        file_path,
        max_bytes: opts.max_bytes.unwrap_or(DEFAULT_MAX_BYTES).max(1),
    }
}

pub async fn append_sidecar_log(
    file_path: impl AsRef<Path>,
    entry: Value,
    max_bytes: Option<u64>,
) -> std::io::Result<()> {
    let line = format_sidecar_log_line(entry);
    append_sidecar_log_lines(file_path, &[line], max_bytes.unwrap_or(DEFAULT_MAX_BYTES)).await
}

pub fn redact_log_value(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(redact_log_value).collect()),
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, nested) in map {
                if is_sensitive_key(key) {
                    out.insert("redacted".to_owned(), Value::String(REDACTED.to_owned()));
                } else {
                    out.insert(key.clone(), redact_log_value(nested));
                }
            }
            Value::Object(out)
        }
        Value::String(text) if is_sensitive_value(text) => Value::String(REDACTED.to_owned()),
        _ => value.clone(),
    }
}

fn format_sidecar_log_line(entry: Value) -> String {
    let safe_entry = redact_log_value(&entry);
    let mut out = match safe_entry {
        Value::Object(map) => map,
        other => {
            let mut map = Map::new();
            map.insert("value".to_owned(), other);
            map
        }
    };
    out.insert("ts".to_owned(), Value::String(chrono_like_now()));
    format!("{}\n", Value::Object(out))
}

async fn append_sidecar_log_lines(
    file_path: impl AsRef<Path>,
    lines: &[String],
    max_bytes: u64,
) -> std::io::Result<()> {
    if lines.is_empty() {
        return Ok(());
    }

    let file_path = file_path.as_ref();
    if let Some(parent) = file_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut body = if file_path.exists() {
        tokio::fs::read_to_string(file_path)
            .await
            .unwrap_or_default()
    } else {
        String::new()
    };
    body.push_str(&lines.join(""));
    let trimmed = trim_log_text(&body, max_bytes);
    tokio::fs::write(file_path, trimmed).await
}

fn trim_log_text(text: &str, max_bytes: u64) -> String {
    if text.len() as u64 <= max_bytes {
        return text.to_owned();
    }

    let mut kept = Vec::new();
    let mut size = 0_u64;
    for line in text.trim_end().lines().rev() {
        let line_size = line.len() as u64 + 1;
        if !kept.is_empty() && size + line_size > max_bytes {
            break;
        }
        kept.push(line);
        size += line_size;
        if size >= max_bytes {
            break;
        }
    }
    kept.reverse();
    if kept.is_empty() {
        String::new()
    } else {
        format!("{}\n", kept.join("\n"))
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_lowercase();
    SENSITIVE_KEY_PATTERNS
        .iter()
        .any(|pattern| lower.contains(pattern))
}

fn is_sensitive_value(value: &str) -> bool {
    let lower = value.to_lowercase();
    SENSITIVE_VALUE_PATTERNS
        .iter()
        .any(|pattern| lower.contains(pattern))
}

fn chrono_like_now() -> String {
    // TS uses ISO timestamps. Until time formatting is ported, use a stable debug-friendly value.
    format!("{:?}", std::time::SystemTime::now())
}

#[allow(dead_code)]
fn _file_size(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|metadata| metadata.len())
}
