use std::collections::HashMap;

use reqwest::{Client, header::HeaderMap};
use serde_json::Value;

use crate::{
    services::auth_session::set_runtime_provider_cookie,
    types::{ProviderLoginQrCheck, ProviderLoginQrImage, ProviderId},
};

#[derive(Clone, Debug, Default)]
pub struct SodaApiResponse {
    pub body: Option<Value>,
}

#[derive(Clone)]
pub struct SodaQrLoginDeps {
    pub client: Client,
    pub qr_code_url: Option<String>,
    pub qr_check_url: Option<String>,
    pub qr_check_referer: Option<String>,
    pub qr_check_user_agent: Option<String>,
}

impl Default for SodaQrLoginDeps {
    fn default() -> Self {
        Self {
            client: Client::new(),
            qr_code_url: None,
            qr_check_url: None,
            qr_check_referer: None,
            qr_check_user_agent: None,
        }
    }
}

pub struct SodaQrLoginService {
    deps: SodaQrLoginDeps,
    image_cache: tokio::sync::Mutex<HashMap<String, ProviderLoginQrImage>>,
}

const SODA_PROVIDER: &str = "soda";
const SODA_QR_CODE_URL: &str = "https://api.qishui.com/passport/web/get_qrcode/?passport_jssdk_version=2.4.13&passport_jssdk_type=normal&is_from_ttaccountsdk=1&aid=386088&language=zh&next=https%3A%2F%2Fapi.qishui.com&need_logo=false&need_short_url=false&is_new_login=1&account_sdk_source=web&account_sdk_source_info=7e276d64776172647760466a6b66707777606b667c273f3637292772606761776c736077273f63646976602927666d776a686061776c736077273f63646976602927766d60696961776c736077273f63646976602927756970626c6b76273f302927756077686c76766c6a6b76273f5e7e276b646860273f276b6a716c636c6664716c6a6b762729277671647160273f276277646b71606127785829276c6b6b60774d606c626d71273f32373529276c6b6b6077526c61716d273f3434353529276a707160774d606c626d71273f32373529276a70716077526c61716d273f34343535292776716a64776260567164717076273f7e276c6b61607d60614147273f7e276c6167273f276a676f6066712729276a75606b273f2763706b66716c6a6b2729276c6b61607d60614147273f276a676f6066712729274c41474e607c57646b6260273f2763706b66716c6a6b2729276a75606b4164716467647660273f27706b6160636c6b60612729276c7656646364776c273f636469766029276d6476436071666d273f6364697660782927696a66646956716a77646260273f7e276c76567075756a77714956716a77646260273f717770602927766c7f60273f363c3c31343c292772776c7160273f7177706078292776716a7764626054706a7164567164717076273f7e277076646260273f34303230313731292774706a7164273f37313637373334323335353529276c7655776c73647160273f6364697660787829277260676269273f7e2773606b616a77273f27426a6a626960254c6b662b252d4b534c414c442c27292777606b6160776077273f27444b424940252d4b534c414c4429254b534c414c44254260436a7766602557515d2530353235252d357d35353535374335312c25416c77606671364134342573765a305a352575765a305a35292541364134342c277829276b6a716c636c6664716c6a6b556077686c76766c6a6b273f276277646b716061272927756077636a7768646b6660273f7e27716c68604a776c626c6b273f34323d373c373c3137353c33312b322927707660614f564d606475566c7f60273f323737353535353529276b64736c6264716c6a6b516c686c6b62273f7e276160666a616061476a617c566c7f60273f37343c312927606b71777c517c7560273f276b64736c6264716c6a6b2729276c6b6c716c64716a77517c7560273f276b64736c6264716c6a6b2729276b646860273f276475753f2a2a7760766a70776660762a68646c6b2b647664772a68646c6b2b6d7168693a62696a6764695a666a6b636c62382032472037377076607741647164203737203644203737462036442030462030465076607776203046203046373034353520304620304644757541647164203046203046576a64686c6b62203046203046566a61644870766c662037372037462037376160736c66604c61203737203644203737303433353c3c33363437373d3c3d3d2037372037462037376c6b76716469694c612037372036442037373432373c3c33353d343033363433363320373720374620373768646c6b55776a6660767646776064716c6a6b516c686020373720364434323d373c373c3134313235352b3c3d3d2037462037376a76203737203644203737526c6b616a72762037372037462037376a765760696064766020373720364420373734352b352b373c303233203737203746203737666a6875707160774b646860203737203644203737465d5534572037372037462037376d7171754d6064616077762037372036442032472032412037462037377360776c637c517764666e4073606b712037372036446364697660203746203737666d646b6b60692037372036442037376a63636c666c6469203737203746203737636a6b71557760636c7d2037372036442037376475752036442037432037437760766a7077666076203743636a6b717620373720324127292777606b61607747696a666e6c6b62567164717076273f276b6a6b2867696a666e6c6b62272927766077736077516c686c6b62273f27272927627069605671647771273f276b6a6b602729276270696041707764716c6a6b273f276b6a6b602778782927776074706076715a6d6a7671273f277760766a7077666076272927776074706076715a7564716d6b646860273f272a68646c6b2b647664772a68646c6b2b6d71686927292767776a72766077273f7e2771273f27363d363137313c373c373d3234272927676c715a75776a716a666a69273f276364697660272927676c715a6d6069756077273f63646976607878&iid=27960026095955&version_code=3.5.1&aid=386088";
const SODA_QR_CHECK_URL: &str = "https://api.qishui.com/passport/web/check_qrconnect/?passport_jssdk_version=2.4.13&passport_jssdk_type=normal&is_from_ttaccountsdk=1&aid=386088&language=zh&account_sdk_source=web&account_sdk_source_info=7e276d64776172647760466a6b66707777606b667c273f3637292772606761776c736077273f63646976602927666d776a686061776c736077273f63646976602927766d60696961776c736077273f63646976602927756970626c6b76273f302927756077686c76766c6a6b76273f5e7e276b646860273f276b6a716c636c6664716c6a6b762729277671647160273f276277646b71606127785829276c6b6b60774d606c626d71273f32373529276c6b6b6077526c61716d273f3434353529276a707160774d606c626d71273f32373529276a70716077526c61716d273f34343535292776716a64776260567164717076273f7e276c6b61607d60614147273f7e276c6167273f276a676f6066712729276a75606b273f2763706b66716c6a6b2729276c6b61607d60614147273f276a676f6066712729274c41474e607c57646b6260273f2763706b66716c6a6b2729276a75606b4164716467647660273f27706b6160636c6b60612729276c7656646364776c273f636469766029276d6476436071666d273f6364697660782927696a66646956716a77646260273f7e276c76567075756a77714956716a77646260273f717770602927766c7f60273f363c3c31343c292772776c7160273f7177706078292776716a7764626054706a7164567164717076273f7e277076646260273f34303230313731292774706a7164273f37313637373334323335353529276c7655776c73647160273f6364697660787829277260676269273f7e2773606b616a77273f27426a6a626960254c6b662b252d4b534c414c442c27292777606b6160776077273f27444b424940252d4b534c414c4429254b534c414c44254260436a7766602557515d2530353235252d357d35353535374335312c25416c77606671364134342573765a305a352575765a305a35292541364134342c277829276b6a716c636c6664716c6a6b556077686c76766c6a6b273f276277646b716061272927756077636a7768646b6660273f7e27716c68604a776c626c6b273f34323d373c373c3137353c33312b322927707660614f564d606475566c7f60273f323737353535353529276b64736c6264716c6a6b516c686c6b62273f7e276160666a616061476a617c566c7f60273f37343c312927606b71777c517c7560273f276b64736c6264716c6a6b2729276c6b6c716c64716a77517c7560273f276b64736c6264716c6a6b2729276b646860273f276475753f2a2a7760766a70776660762a68646c6b2b647664772a68646c6b2b6d7168693a62696a6764695a666a6b636c62382032472037377076607741647164203737203644203737462036442030462030465076607776203046203046373034353520304620304644757541647164203046203046576a64686c6b62203046203046566a61644870766c662037372037462037376160736c66604c61203737203644203737303433353c3c33363437373d3c3d3d2037372037462037376c6b76716469694c612037372036442037373432373c3c33353d343033363433363320373720374620373768646c6b55776a6660767646776064716c6a6b516c686020373720364434323d373c373c3134313235352b3c3d3d2037462037376a76203737203644203737526c6b616a72762037372037462037376a765760696064766020373720364420373734352b352b373c303233203737203746203737666a6875707160774b646860203737203644203737465d5534572037372037462037376d7171754d6064616077762037372036442032472032412037462037377360776c637c517764666e4073606b712037372036446364697660203746203737666d646b6b60692037372036442037376a63636c666c6469203737203746203737636a6b71557760636c7d2037372036442037376475752036442037432037437760766a7077666076203743636a6b717620373720324127292777606b61607747696a666e6c6b62567164717076273f276b6a6b2867696a666e6c6b62272927766077736077516c686c6b62273f27272927627069605671647771273f276b6a6b602729276270696041707764716c6a6b273f276b6a6b602778782927776074706076715a6d6a7671273f277760766a7077666076272927776074706076715a7564716d6b646860273f272a68646c6b2b647664772a68646c6b2b6d71686927292767776a72766077273f7e2771273f27363d363137313c373c373d3234272927676c715a75776a716a666a69273f276364697660272927676c715a6d6069756077273f63646976607878&iid=27960026095955&version_code=3.5.1&aid=386088";
const SODA_QR_CHECK_REFERER: &str = "https://api.qishui.com/";
const SODA_QR_CHECK_USER_AGENT: &str = "LunaPC/3.5.1(408871041)";

impl SodaQrLoginService {
    pub async fn create_image(&self, key: Option<&str>) -> anyhow::Result<ProviderLoginQrImage> {
        self.load_qr_image(key).await
    }

    pub async fn check(&self, key: &str) -> anyhow::Result<ProviderLoginQrCheck> {
        let normalized_key = key.trim();
        if normalized_key.is_empty() {
            anyhow::bail!("SODA_QR_KEY_REQUIRED");
        }

        let url = ensure_configured_url(
            self.deps
                .qr_check_url
                .as_deref()
                .unwrap_or(SODA_QR_CHECK_URL),
            "SODA_QR_CHECK_URL",
        )?;
        let body = [
            ("need_logo", "false"),
            ("need_short_url", "false"),
            ("is_frontier", "true"),
            ("token", normalized_key),
            ("is_new_login", "1"),
            ("next", "https://api.qishui.com"),
        ];
        let resp = self
            .deps
            .client
            .post(url)
            .header(
                "referer",
                self.deps
                    .qr_check_referer
                    .as_deref()
                    .unwrap_or(SODA_QR_CHECK_REFERER),
            )
            .header(
                "user-agent",
                self.deps
                    .qr_check_user_agent
                    .as_deref()
                    .unwrap_or(SODA_QR_CHECK_USER_AGENT),
            )
            .form(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("SODA_QR_CHECK_HTTP_{}", resp.status().as_u16());
        }

        let headers = resp.headers().clone();
        let parsed = parse_fetch_json(resp).await?;
        let payload = read_soda_qr_check_body(parsed.body)?;
        let confirmed = payload.status == "confirmed";
        let scanned = payload.status == "scanned";
        let expired = payload.status == "expired";
        let mut stored = false;
        let message = if scanned {
            Some(payload.scanned_avatar_url)
        } else if expired {
            Some(payload.expired_qrcode)
        } else {
            Some(String::new())
        };

        if confirmed {
            if let Some(cookie) = cookie_from_set_cookie_headers(&headers) {
                set_runtime_provider_cookie(ProviderId::Soda, cookie)
                    .await
                    .map_err(|err| anyhow::anyhow!(err))?;
                stored = true;
            }
        }

        Ok(ProviderLoginQrCheck {
            provider: ProviderId::Soda,
            key: payload
                .expired_token
                .filter(|_| expired)
                .unwrap_or_else(|| normalized_key.to_owned()),
            code: payload.code,
            message,
            logged_in: stored,
            scanned: Some(scanned),
            expired: Some(expired),
            stored: Some(stored),
        })
    }

    async fn load_qr_image(&self, key: Option<&str>) -> anyhow::Result<ProviderLoginQrImage> {
        let normalized_key = key.map(str::trim).unwrap_or_default();
        if !normalized_key.is_empty() {
            return self
                .image_cache
                .lock()
                .await
                .get(normalized_key)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("SODA_QR_IMAGE_MISSING"));
        }

        let url = ensure_configured_url(
            self.deps.qr_code_url.as_deref().unwrap_or(SODA_QR_CODE_URL),
            "SODA_QR_CODE_URL",
        )?;
        let resp = self.deps.client.get(url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("SODA_QR_CODE_HTTP_{}", resp.status().as_u16());
        }
        let payload = read_soda_qr_code_body(resp.json::<Value>().await?)?;
        let image = ProviderLoginQrImage {
            provider: ProviderId::Soda,
            key: payload.token,
            img: payload.qrcode,
            url: None,
        };
        self.image_cache
            .lock()
            .await
            .insert(image.key.clone(), image.clone());
        Ok(image)
    }
}

pub fn create_soda_qr_login_service(deps: SodaQrLoginDeps) -> SodaQrLoginService {
    SodaQrLoginService {
        deps,
        image_cache: tokio::sync::Mutex::new(HashMap::new()),
    }
}

#[derive(Debug)]
struct SodaQrCodeBody {
    qrcode: String,
    token: String,
}

#[derive(Debug)]
struct SodaQrCheckBody {
    status: String,
    code: i64,
    scanned_avatar_url: String,
    expired_token: Option<String>,
    expired_qrcode: String,
}

fn as_obj(value: Option<&Value>) -> Option<&serde_json::Map<String, Value>> {
    value.and_then(Value::as_object)
}

fn read_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn ensure_configured_url(url: &str, name: &str) -> anyhow::Result<String> {
    let normalized = url.trim();
    if normalized.is_empty() {
        anyhow::bail!("{name}_MISSING");
    }
    Ok(normalized.to_owned())
}

fn read_soda_qr_code_body(body: Value) -> anyhow::Result<SodaQrCodeBody> {
    let root = as_obj(Some(&body));
    let data = as_obj(root.and_then(|root| root.get("data")));
    let message = read_string(root.and_then(|root| root.get("message"))).unwrap_or_default();
    if message != "success" {
        anyhow::bail!("SODA_QR_CODE_REQUEST_FAILED");
    }
    let qrcode = read_string(data.and_then(|data| data.get("qrcode"))).unwrap_or_default();
    let token = read_string(data.and_then(|data| data.get("token"))).unwrap_or_default();
    if qrcode.is_empty() || token.is_empty() {
        anyhow::bail!("SODA_QR_CODE_DATA_MISSING");
    }
    Ok(SodaQrCodeBody { qrcode, token })
}

fn read_soda_qr_check_body(body: Option<Value>) -> anyhow::Result<SodaQrCheckBody> {
    let root = as_obj(body.as_ref());
    let data = as_obj(root.and_then(|root| root.get("data")));
    let message = read_string(root.and_then(|root| root.get("message"))).unwrap_or_default();
    if message != "success" {
        anyhow::bail!("SODA_QR_CHECK_REQUEST_FAILED");
    }
    let scan_user_info = as_obj(data.and_then(|data| data.get("scan_user_info")));
    Ok(SodaQrCheckBody {
        status: read_string(data.and_then(|data| data.get("status"))).unwrap_or_default(),
        code: data
            .and_then(|data| data.get("error_code"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
        scanned_avatar_url: read_string(scan_user_info.and_then(|info| info.get("avatar_url")))
            .unwrap_or_default(),
        expired_token: read_string(data.and_then(|data| data.get("token"))),
        expired_qrcode: read_string(data.and_then(|data| data.get("qrcode"))).unwrap_or_default(),
    })
}

fn split_combined_set_cookie_header(header: &str) -> Vec<String> {
    header
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn read_set_cookie_headers(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(split_combined_set_cookie_header)
        .collect()
}

fn cookie_from_set_cookie_headers(headers: &HeaderMap) -> Option<String> {
    let cookie = read_set_cookie_headers(headers)
        .into_iter()
        .filter_map(|header| {
            header
                .split(';')
                .next()
                .map(str::trim)
                .map(ToOwned::to_owned)
        })
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    if cookie.trim().is_empty() {
        None
    } else {
        Some(cookie)
    }
}

async fn parse_fetch_json(resp: reqwest::Response) -> anyhow::Result<SodaApiResponse> {
    let text = resp.text().await?;
    if text.trim().is_empty() {
        return Ok(SodaApiResponse {
            body: Some(serde_json::json!({})),
        });
    }
    Ok(SodaApiResponse {
        body: serde_json::from_str(&text).ok(),
    })
}
