use std::collections::HashMap;

use anyhow::{Result, bail};
use serde_json::Value;

use crate::types::{ProviderId, ProviderLoginQrCheck};

pub(crate) const QQ_LOGIN_COOKIE_REMAPS: &[(&str, &str)] = &[
    ("access_token", "psrf_qqaccess_token"),
    ("openid", "psrf_qqopenid"),
    ("unionid", "psrf_qqunionid"),
    ("refresh_token", "psrf_qqrefresh_token"),
    ("expired_at", "psrf_access_token_expiresAt"),
    ("musickey", "qm_keyst"),
    ("encryptUin", "euin"),
];

pub(crate) fn qq_music_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .map(|name| format!("{name}-MRT"))
        .unwrap_or_else(|| "MineRadio-MRT".to_owned())
}

pub(crate) fn normalize_login_cookie(
    data: &Value,
    guid: &str,
    is_wechat: bool,
    empty_error: &'static str,
) -> Result<String> {
    let mut data_map = flatten_data_to_map(data);
    remap_qq_login_data_map(&mut data_map, is_wechat);
    let cookie = cookie_from_data_map(&data_map, empty_error)?;
    Ok(cookie_with_qqmusic_guid(cookie, guid))
}

pub(crate) fn required_key(key: &str, error: &'static str) -> Result<String> {
    let key = key.trim();
    if key.is_empty() {
        bail!(error);
    }
    Ok(key.to_owned())
}

pub(crate) fn flatten_data_to_map(data: &Value) -> HashMap<String, Value> {
    let mut map = HashMap::new();
    if let Value::Object(obj) = data {
        for (key, value) in obj {
            match value {
                Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => {
                    map.insert(key.clone(), value.clone());
                }
                _ => {}
            }
        }
    }
    map
}

pub(crate) fn remap_qq_login_data_map(data_map: &mut HashMap<String, Value>, is_wechat: bool) {
    for (source, target) in QQ_LOGIN_COOKIE_REMAPS {
        remap_qq_login_data_key(data_map, source, target);
    }
    remap_qq_login_data_key(data_map, "musicid", "uin");
    data_map.insert(
        "tmeLoginType".to_owned(),
        Value::from(if is_wechat { 1 } else { 2 }),
    );
    if is_wechat {
        remap_qq_login_data_key(data_map, "musicid", "wxuin");
    }
}

pub(crate) fn cookie_from_data_map(
    data_map: &HashMap<String, Value>,
    empty_error: &'static str,
) -> Result<String> {
    let mut parts = Vec::new();
    for (key, value) in data_map {
        let s = match value {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => continue,
            _ => continue,
        };
        if !s.is_empty() {
            parts.push(format!("{key}={s}"));
        }
    }
    if parts.is_empty() {
        bail!(empty_error);
    }
    Ok(parts.join("; "))
}

pub(crate) fn check_response(
    key: &str,
    code: i64,
    message: &str,
    logged_in: bool,
    scanned: bool,
    expired: bool,
    stored: bool,
) -> ProviderLoginQrCheck {
    ProviderLoginQrCheck {
        provider: ProviderId::Qq,
        key: key.to_owned(),
        code,
        message: Some(message.to_owned()),
        logged_in,
        scanned: Some(scanned),
        expired: Some(expired),
        stored: Some(stored),
    }
}

pub(crate) fn check_qq_login_error(value: &Value) -> Result<()> {
    let Some(err_tip) = find_err_tip(value) else {
        return Ok(());
    };
    if err_tip.contains("限制") && err_tip.contains("超出登录") {
        bail!("QQ_LOGIN_DEVICE_LIMIT: {err_tip}");
    }
    Ok(())
}

pub(crate) fn cookie_with_qqmusic_guid(cookie: String, guid: &str) -> String {
    format!("{cookie}; qqmusic_guid={guid}")
}

fn remap_qq_login_data_key(data_map: &mut HashMap<String, Value>, source: &str, target: &str) {
    let Some(value) = data_map.get(source).cloned() else {
        return;
    };
    if matches!(&value, Value::Null) || value.as_str().is_some_and(|text| text.trim().is_empty()) {
        return;
    }
    data_map.insert(target.to_owned(), value);
}

fn find_err_tip(value: &Value) -> Option<&str> {
    match value {
        Value::Object(object) => {
            if let Some(err_tip) = object.get("errTip").and_then(Value::as_str) {
                return Some(err_tip);
            }
            object.values().find_map(find_err_tip)
        }
        Value::Array(values) => values.iter().find_map(find_err_tip),
        _ => None,
    }
}
