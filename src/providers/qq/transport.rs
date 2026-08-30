use anyhow::Context;
use http::{
    HeaderMap, HeaderName, HeaderValue,
    header::{CONTENT_TYPE, COOKIE, ORIGIN, REFERER, USER_AGENT},
};
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::{
    ProviderId,
    providers::{
        ProviderResult,
        error::{ProviderError, ProviderErrorCode},
    },
    utils::{
        cookie::Cookie,
        cryptors::qq::{QMV, QUA, QV, x4, x5, x7, x9, xj},
    },
};

// client 为 true 时采用客户端算法，否则采用 Web 端算法。
pub(crate) async fn qq_post_model<T: DeserializeOwned>(
    client: Client,
    body: Value,
    referer: Option<&str>,
    cookie: Cookie,
    action: &str,
    is_c: bool,
) -> ProviderResult<T> {
    if is_c {
        qq_post_model_client(client, body, referer, cookie, action).await
    } else {
        qq_post_model_web(client, body, referer, cookie, action).await
    }
}

// QQ 客户端算法。
async fn qq_post_model_client<T: DeserializeOwned>(
    client: Client,
    mut req: Value,
    referer: Option<&str>,
    cookie: Cookie,
    action: &str,
) -> ProviderResult<T> {
    let cookie_keys = ["psrf_qqaccess_token", "psrf_qqopenid", "psrf_qqunionid"];

    let uin = cookie.find_or_default::<String>("uin");
    let guid = cookie.find_or_else("qqmusic_guid", x5);

    // 构建客户端鉴权 comm。
    let mut params: serde_json::Map<String, Value> = serde_json::Map::from_iter([
        ("_channelid".into(), "20".into()),
        ("_os_version".into(), "6.2.9200-2".into()),
        (
            "authst".into(),
            cookie.find_or_default::<String>("qm_keyst").into(),
        ),
        ("ct".into(), "19".into()),
        ("cv".into(), format!("{QV}{QMV}").into()),
        ("guid".into(), json!(&guid)),
        ("patch".into(), "118".into()),
        (
            "psrf_access_token_expiresAt".into(),
            json!(cookie.find_or_default::<u128>("psrf_access_token_expiresAt")),
        ),
        ("tmeAppID".into(), "qqmusic".into()),
        (
            "tmeLoginType".into(),
            json!(cookie.find_or_default::<u128>("tmeLoginType")),
        ),
        ("uin".into(), json!(&uin)),
    ]);

    for key in cookie_keys {
        params.insert(key.to_owned(), json!(cookie.find_or_default::<String>(key)));
    }

    if let Some(obj) = req.as_object_mut() {
        obj.insert("comm".to_owned(), params.into());
    }

    let t = chrono::Utc::now().timestamp();

    let mut headers = build_headers(referer, Some(cookie), false)?;

    let (sign, mask) = x4(&req.to_string(), t as u64);

    headers.insert(
        HeaderName::from_bytes(xj(0x5369_676e).as_bytes()).map_err(internal_error)?,
        HeaderValue::from_str(&sign).map_err(internal_error)?,
    );

    headers.insert(
        HeaderName::from_bytes(xj(0x4d61_736b).as_bytes()).map_err(internal_error)?,
        HeaderValue::from_str(&mask).map_err(internal_error)?,
    );

    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );

    let q = [xj(0x7063_6163_6865_7469), xj(0x6d65)].concat();
    let response = client
        .post("https://u6.y.qq.com/cgi-bin/musics.fcg")
        .query(&[(q.as_str(), t.to_string().as_str())])
        .headers(headers)
        .body(req.to_string())
        .send()
        .await
        .context("send qq upstream client post request")
        .map_err(internal_error)?;

    decode_qq_response(response, action).await
}

// QQ Web 算法。
async fn qq_post_model_web<T: DeserializeOwned>(
    client: Client,
    mut req: Value,
    referer: Option<&str>,
    cookie: Cookie,
    action: &str,
) -> ProviderResult<T> {
    let cookie_keys = ["psrf_qqaccess_token", "psrf_qqopenid", "psrf_qqunionid"];

    let uin = cookie.find_or_default::<String>("uin");
    let guid = cookie.find_or_else("qqmusic_guid", x5);

    // 构建 Web 鉴权 comm。
    let mut params: serde_json::Map<String, Value> = serde_json::Map::from_iter([
        ("_channelid".into(), "20".into()),
        ("_os_version".into(), "6.2.9200-2".into()),
        (
            "authst".into(),
            cookie.find_or_default::<String>("qm_keyst").into(),
        ),
        ("cv".into(), format!("{QV}{QMV}").into()),
        ("guid".into(), json!(&guid)),
        (
            "psrf_access_token_expiresAt".into(),
            json!(cookie.find_or_default::<u128>("psrf_access_token_expiresAt")),
        ),
        ("patch".into(), "118".into()),
        ("tmeAppID".into(), "qqmusic".into()),
        (
            "tmeLoginType".into(),
            json!(cookie.find_or_default::<u128>("tmeLoginType")),
        ),
        ("uin".into(), json!(&uin)),
        ("wid".into(), "7571626021101097984".into()),
    ]);

    for key in cookie_keys {
        params.insert(key.to_owned(), json!(cookie.find_or_default::<String>(key)));
    }

    params.insert("format".into(), json!("json"));
    params.insert("platform".into(), json!("wk_v17"));
    params.insert("inCharset".into(), json!("utf-8"));
    params.insert("outCharset".into(), json!("utf-8"));
    params.insert("notice".into(), json!(0));
    params.insert("needNewCode".into(), json!(1));
    params.insert("ct".into(), json!("20"));

    let g_tk = x7(&cookie.find_or_default::<String>("musickey")).to_string();

    params.insert("g_tk_new_20200303".into(), json!(&g_tk));
    params.insert("g_tk".into(), json!(&g_tk));

    if let Some(obj) = req.as_object_mut() {
        obj.insert("comm".to_owned(), params.into());
    }

    let t = chrono::Utc::now().timestamp_millis().to_string();

    let payload = serde_json::to_string(&req).map_err(internal_error)?;

    let sign = x9(&payload);

    let headers = build_headers(referer, Some(cookie), false)?;

    let response = client
        .post("https://u6.y.qq.com/cgi-bin/musics.fcg")
        .query(&[("sign", sign.as_str()), ("_", t.as_str())])
        .headers(headers)
        .json(&req)
        .send()
        .await
        .context("send qq upstream web post request")
        .map_err(internal_error)?;

    decode_qq_response(response, action).await
}

// 公共响应解码。
async fn decode_qq_response<T: DeserializeOwned>(
    response: reqwest::Response,
    action: &str,
) -> ProviderResult<T> {
    let raw = response
        .bytes()
        .await
        .context("read qq upstream response")
        .map_err(internal_error)?;

    serde_json::from_slice(&raw).map_err(|err| ProviderError {
        code: ProviderErrorCode::InvalidResponse,
        provider: ProviderId::Qq,
        message: format!("decode qq {action} response: {err}"),
        retryable: false,
        action: Some(action.to_owned()),
        raw_message: Some(String::from_utf8_lossy(&raw).into_owned()),
    })
}

fn internal_error(err: impl std::fmt::Display) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::Internal,
        provider: ProviderId::Qq,
        message: err.to_string(),
        retryable: false,
        action: None,
        raw_message: None,
    }
}

fn build_headers(
    referer: Option<&str>,
    cookie: Option<Cookie>,
    with_origin: bool,
) -> ProviderResult<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(QUA));
    if let Some(referer) = referer {
        headers.insert(REFERER, header_value(referer)?);
        if with_origin {
            let origin = reqwest::Url::parse(referer)
                .ok()
                .map(|url| format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default()))
                .unwrap_or_else(|| "https://y.qq.com".to_owned());
            headers.insert(ORIGIN, header_value(&origin)?);
        }
    }
    if let Some(cookie) = cookie {
        let c: String = cookie.into();
        headers.insert(COOKIE, header_value(&c)?);
    }
    Ok(headers)
}

fn header_value(value: &str) -> ProviderResult<HeaderValue> {
    HeaderValue::from_str(value).map_err(internal_error)
}

fn unavailable_error(err: impl std::fmt::Display) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::Unavailable,
        provider: ProviderId::Qq,
        message: err.to_string(),
        retryable: true,
        action: None,
        raw_message: None,
    }
}
