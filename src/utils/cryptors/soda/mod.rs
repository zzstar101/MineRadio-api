use md5::{Digest, Md5};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use url::Url;

pub mod audio;
pub use audio::decrypt_soda_audio;

pub fn q9v(url: &str, headers: &mut HeaderMap) -> bool {
    sign_with_body(url, &[], headers).is_ok()
}

pub fn q9v_with_body(url: &str, body: &[u8], headers: &mut HeaderMap) -> bool {
    sign_with_body(url, body, headers).is_ok()
}

fn sign_with_body(url: &str, body: &[u8], headers: &mut HeaderMap) -> Result<(), String> {
    ensure_required_headers(headers, body);
    validate(url, headers)?;
    let header_text = format_headers(headers)?;
    merge_signature(headers, &sign(url, &header_text)?)
}

fn sign(url: &str, headers: &str) -> Result<String, String> {
    crate::utils::cryptors::csigner::real_soda_sign(url, headers)
}

fn ensure_required_headers(headers: &mut HeaderMap, body: &[u8]) {
    if !headers.contains_key("content-type") {
        headers.insert(
            "content-type",
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
    }
    if !headers.contains_key("X-SS-STUB") {
        headers.insert(
            "X-SS-STUB",
            HeaderValue::from_str(&hex::encode_upper(Md5::digest(body))).unwrap(),
        );
    }
}

fn validate(url: &str, headers: &HeaderMap) -> Result<(), String> {
    let parsed = Url::parse(url).map_err(|err| format!("invalid Soda signing URL: {err}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Soda signing URL must use http or https".to_owned());
    }
    for (key, value) in headers {
        if key.as_str().is_empty() || value.to_str().is_err() {
            return Err("Soda signing headers contain an invalid name or value".to_owned());
        }
    }
    Ok(())
}

fn format_headers(headers: &HeaderMap) -> Result<String, String> {
    let mut entries = headers
        .iter()
        .map(|(key, value)| {
            Ok((
                key.as_str().to_owned(),
                value
                    .to_str()
                    .map_err(|_| "Soda signing header value is not valid UTF-8".to_owned())?
                    .to_owned(),
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    entries.sort_unstable_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    Ok(entries
        .into_iter()
        .flat_map(|(key, value)| [key, value])
        .collect::<Vec<_>>()
        .join("\r\n"))
}

fn merge_signature(headers: &mut HeaderMap, signature: &str) -> Result<(), String> {
    let parts = signature
        .split("\r\n")
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() || parts.len() % 2 != 0 {
        return Err("Soda signer returned malformed headers".to_owned());
    }
    for pair in parts.chunks_exact(2) {
        if pair[0].contains(['\r', '\n', '\0']) || pair[1].contains(['\r', '\n', '\0']) {
            return Err("Soda signer returned invalid header data".to_owned());
        }
        let name = HeaderName::from_bytes(pair[0].as_bytes())
            .map_err(|_| "Soda signer returned an invalid header name".to_owned())?;
        let value = HeaderValue::from_str(pair[1])
            .map_err(|_| "Soda signer returned an invalid header value".to_owned())?;
        headers.insert(name, value);
    }
    Ok(())
}
