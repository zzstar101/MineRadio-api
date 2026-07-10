use std::{
    env,
    path::{Path, PathBuf},
    sync::OnceLock,
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
static SIDECAR_LOGGER: OnceLock<SidecarLogger> = OnceLock::new();

pub fn init(config: &Config) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_layer = fmt::layer().with_target(true);

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .init();
    let _ = SIDECAR_LOGGER.set(create_sidecar_logger(SidecarLoggerOptions {
        file_path: config.log_file.clone(),
        max_bytes: None,
    }));
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

pub fn global_logger() -> Option<&'static SidecarLogger> {
    SIDECAR_LOGGER.get()
}

pub async fn log_runtime(entry: Value) {
    if let Some(logger) = global_logger() {
        logger.log(entry).await;
    }
}

pub fn spawn_runtime_log(entry: Value) {
    tokio::spawn(async move {
        log_runtime(entry).await;
    });
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_log_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("mineradio-{name}-{nanos}.jsonl"))
    }

    #[test]
    fn redact_log_value_hides_sensitive_keys_and_values() {
        let redacted = redact_log_value(&json!({
            "cookie": "keep-out",
            "nested": {
                "authorizationHeader": "Bearer secret-token",
                "safe": "visible"
            }
        }));

        assert_eq!(redacted["redacted"], REDACTED);
        assert_eq!(redacted["nested"]["redacted"], REDACTED);
        assert_eq!(redacted["nested"]["safe"], "visible");
    }

    #[tokio::test]
    async fn append_sidecar_log_writes_file_and_trims_old_lines() {
        let path = unique_test_log_path("sidecar-log");
        append_sidecar_log(&path, json!({ "event": "first" }), Some(512))
            .await
            .expect("first log write should succeed");
        append_sidecar_log(&path, json!({ "event": "second" }), Some(80))
            .await
            .expect("second log write should succeed");

        let contents = tokio::fs::read_to_string(&path)
            .await
            .expect("log file should exist");

        assert!(contents.contains("\"event\":\"second\""));
        assert!(!contents.contains("\"event\":\"first\""));

        let _ = tokio::fs::remove_file(path).await;
    }
}
