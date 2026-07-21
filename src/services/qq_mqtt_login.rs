// MQTT login protocol adapted from netease-qq-music-api (MIT, Copyright (c) 2026 AstronW).
// See README.md for the acknowledgement and the upstream LICENSE for the full license text.

use std::{
    sync::Once,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow, bail};
use async_tungstenite::tokio::{ConnectStream, connect_async};
use async_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use bytes::BytesMut;
use futures_util::StreamExt;
use rand::RngExt;
use rumqttc::v5::mqttbytes::QoS;
use rumqttc::v5::mqttbytes::v5::{
    Connect, ConnectProperties, ConnectReturnCode, Filter, Packet, PingResp, Publish, Subscribe,
    SubscribeProperties, SubscribeReasonCode,
};
use serde_json::Value;

const MQTT_HOST: &str = "mu.y.qq.com";
const MQTT_PORT: u16 = 443;
const MQTT_PATH: &str = "/ws/handshake";
const MQTT_KEEP_ALIVE: u16 = 45;
const MQTT_MAX_REDIRECTS: usize = 3;
const MQTT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const MQTT_SUBACK_TIMEOUT: Duration = Duration::from_secs(5);
const MQTT_EVENT_WAIT_TIMEOUT: Duration = Duration::from_millis(1500);
const MQTT_CALL_TIMEOUT: Duration = Duration::from_secs(6);
const MQTT_DEFAULT_INTERVAL: Duration = Duration::from_millis(1500);
const MQTT_ERROR_INTERVAL: Duration = Duration::from_secs(3);

static RUSTLS_PROVIDER: Once = Once::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MqttLoginEvent {
    WaitingScan,
    WaitingConfirm,
    QrCodeExpired,
    Canceled,
    LoginFailed,
    Cookies { music_id: String, music_key: String },
}

impl MqttLoginEvent {
    pub(crate) fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::QrCodeExpired | Self::Canceled | Self::LoginFailed | Self::Cookies { .. }
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingLoginState {
    WaitingScan,
    WaitingConfirm,
}

impl PendingLoginState {
    fn as_event(self) -> MqttLoginEvent {
        match self {
            Self::WaitingScan => MqttLoginEvent::WaitingScan,
            Self::WaitingConfirm => MqttLoginEvent::WaitingConfirm,
        }
    }

    fn update(self, event: &MqttLoginEvent) -> Self {
        match event {
            MqttLoginEvent::WaitingScan => Self::WaitingScan,
            MqttLoginEvent::WaitingConfirm => Self::WaitingConfirm,
            _ => self,
        }
    }
}

pub(crate) struct MqttLoginSession {
    qrcode_id: String,
    socket: Option<MqttWebSocket>,
    pending_state: PendingLoginState,
}

impl MqttLoginSession {
    pub(crate) fn new(qrcode_id: &str) -> Self {
        Self {
            qrcode_id: qrcode_id.to_owned(),
            socket: None,
            pending_state: PendingLoginState::WaitingScan,
        }
    }

    fn fallback_event(&self) -> MqttLoginEvent {
        self.pending_state.as_event()
    }

    fn update_pending_state(&mut self, event: &MqttLoginEvent) {
        self.pending_state = self.pending_state.update(event);
        if event.is_terminal() {
            self.pending_state = PendingLoginState::WaitingScan;
        }
    }

    pub(crate) async fn poll_event(&mut self) -> Result<MqttLoginEvent> {
        let deadline = Instant::now() + MQTT_CALL_TIMEOUT;
        let mut retries = 0u32;

        loop {
            match self.poll_event_once().await {
                Ok(event) => return Ok(event),
                Err(err) => {
                    if !is_transient_mqtt_error(&err) {
                        self.socket = None;
                        self.pending_state = PendingLoginState::WaitingScan;
                        return Err(err);
                    }

                    let now = Instant::now();
                    if now >= deadline {
                        self.socket = None;
                        return Ok(self.fallback_event());
                    }

                    self.socket = None;
                    let backoff = retry_backoff(retries);
                    let remain = deadline.saturating_duration_since(now);
                    tokio::time::sleep(backoff.min(remain)).await;
                    retries = retries.saturating_add(1);
                }
            }
        }
    }

    async fn poll_event_once(&mut self) -> Result<MqttLoginEvent> {
        if self.socket.is_none() {
            let mut mqtt = MqttWebSocket::connect(&self.qrcode_id).await?;
            mqtt.subscribe(&self.qrcode_id).await?;
            self.socket = Some(mqtt);
        }

        let event = {
            let mqtt = self.socket.as_mut().expect("socket should be initialized");
            match mqtt.next_login_event(MQTT_EVENT_WAIT_TIMEOUT).await? {
                Some(event) => event,
                None => self.fallback_event(),
            }
        };
        self.update_pending_state(&event);

        if event.is_terminal() {
            self.socket = None;
        }
        Ok(event)
    }
}

fn is_transient_mqtt_error(err: &anyhow::Error) -> bool {
    let message = err.to_string();
    [
        "timed out",
        "websocket connect failed",
        "mqtt read frame failed",
        "send mqtt packet failed",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

fn retry_backoff(retries: u32) -> Duration {
    let factor = 2f64.powi(retries.min(10) as i32);
    let secs =
        (MQTT_DEFAULT_INTERVAL.as_secs_f64() * factor).min(MQTT_ERROR_INTERVAL.as_secs_f64());
    Duration::from_secs_f64(secs)
}

struct MqttWebSocket {
    stream: async_tungstenite::WebSocketStream<ConnectStream>,
    pending_event: Option<MqttLoginEvent>,
}

enum ConnectHandshake {
    Connected,
    Redirect(String),
}

impl MqttWebSocket {
    async fn connect(qrcode_id: &str) -> Result<Self> {
        let mut server_reference: Option<String> = None;

        for redirect_count in 0..=MQTT_MAX_REDIRECTS {
            let mut socket = Self::open(&handshake_path(server_reference.as_deref())).await?;
            match socket.connect_mqtt(qrcode_id).await? {
                ConnectHandshake::Connected => return Ok(socket),
                ConnectHandshake::Redirect(next_server_reference) => {
                    if redirect_count == MQTT_MAX_REDIRECTS {
                        bail!("QQ_MQTT_TOO_MANY_REDIRECTS");
                    }
                    server_reference = Some(next_server_reference);
                }
            }
        }
        bail!("QQ_MQTT_UNREACHABLE_REDIRECT_STATE")
    }

    async fn open(path: &str) -> Result<Self> {
        install_rustls_provider();
        let url = format!("wss://{MQTT_HOST}:{MQTT_PORT}{path}");
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|err| anyhow!("QQ_MQTT_BUILD_REQUEST: {err}"))?;
        let headers = request.headers_mut();
        headers.insert(
            "Sec-WebSocket-Protocol",
            "mqtt".parse().expect("valid header"),
        );
        headers.insert("Origin", "https://y.qq.com".parse().expect("valid header"));
        headers.insert(
            "Referer",
            "https://y.qq.com/".parse().expect("valid header"),
        );
        headers.insert(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/123.0.0.0 Safari/537.36"
                .parse()
                .expect("valid header"),
        );

        let (stream, _) = tokio::time::timeout(MQTT_CONNECT_TIMEOUT, connect_async(request))
            .await
            .map_err(|_| anyhow!("QQ_MQTT_WEBSOCKET_CONNECT_TIMED_OUT"))?
            .map_err(|err| anyhow!("QQ_MQTT_WEBSOCKET_CONNECT_FAILED: {err}"))?;
        Ok(Self {
            stream,
            pending_event: None,
        })
    }

    async fn connect_mqtt(&mut self, qrcode_id: &str) -> Result<ConnectHandshake> {
        let mut properties = ConnectProperties::new();
        properties.authentication_method = Some("pass".to_owned());
        properties.user_properties = vec![
            ("tmeAppID".to_owned(), "qqmusic".to_owned()),
            ("business".to_owned(), "management".to_owned()),
            ("hashTag".to_owned(), qrcode_id.to_owned()),
            ("clientTag".to_owned(), "management.user".to_owned()),
            ("userID".to_owned(), qrcode_id.to_owned()),
        ];
        self.send_packet(Packet::Connect(
            Connect {
                keep_alive: MQTT_KEEP_ALIVE,
                client_id: build_client_id(),
                clean_start: true,
                properties: Some(properties),
            },
            None,
            None,
        ))
        .await?;

        let connack = match self.next_packet(MQTT_CONNECT_TIMEOUT, false).await? {
            Some(Packet::ConnAck(connack)) => connack,
            Some(_) => bail!("QQ_MQTT_EXPECTED_CONNACK"),
            None => bail!("QQ_MQTT_CONNACK_TIMED_OUT"),
        };
        match connack.code {
            ConnectReturnCode::Success => Ok(ConnectHandshake::Connected),
            ConnectReturnCode::UseAnotherServer | ConnectReturnCode::ServerMoved => {
                let reference = connack
                    .properties
                    .and_then(|properties| properties.server_reference)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow!("QQ_MQTT_REDIRECT_REFERENCE_MISSING"))?;
                Ok(ConnectHandshake::Redirect(reference))
            }
            code => bail!("QQ_MQTT_CONNECT_REJECTED: {code:?}"),
        }
    }

    async fn subscribe(&mut self, qrcode_id: &str) -> Result<()> {
        let mut subscribe = Subscribe::new(
            Filter::new(
                format!("management.qrcode_login/{qrcode_id}"),
                QoS::AtMostOnce,
            ),
            Some(SubscribeProperties {
                id: None,
                user_properties: vec![
                    ("authorization".to_owned(), "tmelogin".to_owned()),
                    ("pubsub".to_owned(), "unicast".to_owned()),
                ],
            }),
        );
        subscribe.pkid = 1;
        self.send_packet(Packet::Subscribe(subscribe)).await?;

        loop {
            match self.next_packet(MQTT_SUBACK_TIMEOUT, false).await? {
                Some(Packet::SubAck(suback)) if suback.pkid == 1 => {
                    if suback
                        .return_codes
                        .iter()
                        .any(|code| !matches!(code, SubscribeReasonCode::Success(_)))
                    {
                        bail!("QQ_MQTT_SUBSCRIBE_REJECTED");
                    }
                    return Ok(());
                }
                Some(Packet::Publish(publish)) => {
                    if self.pending_event.is_none() {
                        self.pending_event = parse_publish_event(&publish);
                    }
                }
                Some(_) => {}
                None => bail!("QQ_MQTT_SUBACK_TIMED_OUT"),
            }
        }
    }

    async fn next_login_event(&mut self, timeout: Duration) -> Result<Option<MqttLoginEvent>> {
        if let Some(event) = self.pending_event.take() {
            return Ok(Some(event));
        }
        loop {
            let Some(packet) = self.next_packet(timeout, true).await? else {
                return Ok(None);
            };
            match packet {
                Packet::Publish(publish) => {
                    if let Some(event) = parse_publish_event(&publish) {
                        return Ok(Some(event));
                    }
                }
                Packet::PingReq(_) => self.send_packet(Packet::PingResp(PingResp)).await?,
                Packet::Disconnect(_) => return Ok(Some(MqttLoginEvent::LoginFailed)),
                _ => {}
            }
        }
    }

    async fn next_packet(
        &mut self,
        timeout: Duration,
        timeout_as_none: bool,
    ) -> Result<Option<Packet>> {
        loop {
            let frame = match tokio::time::timeout(timeout, self.stream.next()).await {
                Ok(frame) => frame,
                Err(_) if timeout_as_none => return Ok(None),
                Err(_) => bail!("QQ_MQTT_PACKET_READ_TIMED_OUT"),
            };
            let Some(frame) = frame else {
                return Ok(None);
            };
            let frame = frame.map_err(|err| anyhow!("QQ_MQTT_READ_FRAME_FAILED: {err}"))?;
            match frame {
                Message::Binary(payload) => {
                    let mut bytes = BytesMut::from(payload.as_ref());
                    return Packet::read(&mut bytes, None)
                        .map(Some)
                        .map_err(|err| anyhow!("QQ_MQTT_DECODE_PACKET_FAILED: {err}"));
                }
                Message::Close(_) => return Ok(None),
                Message::Ping(_) | Message::Pong(_) | Message::Text(_) | Message::Frame(_) => {}
            }
        }
    }

    async fn send_packet(&mut self, packet: Packet) -> Result<()> {
        let mut bytes = BytesMut::new();
        packet
            .write(&mut bytes, None)
            .map_err(|err| anyhow!("QQ_MQTT_ENCODE_PACKET_FAILED: {err}"))?;
        self.stream
            .send(Message::Binary(bytes.freeze()))
            .await
            .map_err(|err| anyhow!("QQ_MQTT_SEND_PACKET_FAILED: {err}"))
    }
}

fn install_rustls_provider() {
    RUSTLS_PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn build_client_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{millis}{}", rand::rng().random_range(1000..=9999))
}

fn parse_publish_event(publish: &Publish) -> Option<MqttLoginEvent> {
    let event_type = publish
        .properties
        .as_ref()?
        .user_properties
        .iter()
        .find_map(|(key, value)| (key == "type").then_some(value.as_str()))?;
    match event_type {
        "scanned" => Some(MqttLoginEvent::WaitingConfirm),
        "canceled" => Some(MqttLoginEvent::Canceled),
        "timeout" => Some(MqttLoginEvent::QrCodeExpired),
        "loginFailed" => Some(MqttLoginEvent::LoginFailed),
        "cookies" => parse_cookies_event(publish.payload.as_ref()),
        _ => None,
    }
}

fn parse_cookies_event(payload: &[u8]) -> Option<MqttLoginEvent> {
    let cookies = serde_json::from_slice::<Value>(payload)
        .ok()?
        .get("cookies")?
        .as_object()?
        .clone();
    let music_id = extract_cookie_value(&cookies, "qqmusic_uin")?;
    let music_key = extract_cookie_value(&cookies, "qqmusic_key")?;
    (!music_id.is_empty() && !music_key.is_empty()).then_some(MqttLoginEvent::Cookies {
        music_id,
        music_key,
    })
}

fn extract_cookie_value(cookies: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    let value = cookies.get(key)?;
    value
        .as_str()
        .map(ToOwned::to_owned)
        .or_else(|| value.as_u64().map(|value| value.to_string()))
        .or_else(|| {
            value
                .as_object()?
                .get("value")?
                .as_str()
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            value
                .as_object()?
                .get("value")?
                .as_u64()
                .map(|value| value.to_string())
        })
}

fn handshake_path(server_reference: Option<&str>) -> String {
    match server_reference {
        Some(server_reference) => format!("{MQTT_PATH}/{server_reference}"),
        None => MQTT_PATH.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use rumqttc::v5::mqttbytes::v5::PublishProperties;

    use super::*;

    fn publish(event_type: &str, payload: &[u8]) -> Publish {
        let mut properties = PublishProperties::default();
        properties.user_properties = vec![("type".to_owned(), event_type.to_owned())];
        Publish::new(
            "management.qrcode_login/test",
            QoS::AtMostOnce,
            payload.to_vec(),
            Some(properties),
        )
    }

    #[test]
    fn parses_login_events() {
        assert_eq!(
            parse_publish_event(&publish("scanned", b"{}")),
            Some(MqttLoginEvent::WaitingConfirm)
        );
        assert_eq!(
            parse_publish_event(&publish("timeout", b"{}")),
            Some(MqttLoginEvent::QrCodeExpired)
        );
        assert_eq!(
            parse_publish_event(&publish("canceled", b"{}")),
            Some(MqttLoginEvent::Canceled)
        );
    }

    #[test]
    fn parses_cookie_values_in_supported_shapes() {
        let event = parse_publish_event(&publish(
            "cookies",
            br#"{"cookies":{"qqmusic_uin":{"value":10001},"qqmusic_key":"Q_H_L_test"}}"#,
        ));
        assert_eq!(
            event,
            Some(MqttLoginEvent::Cookies {
                music_id: "10001".to_owned(),
                music_key: "Q_H_L_test".to_owned(),
            })
        );
    }

    #[test]
    fn rejects_incomplete_cookie_events() {
        assert_eq!(
            parse_publish_event(&publish("cookies", br#"{"cookies":{}}"#)),
            None
        );
    }
}
