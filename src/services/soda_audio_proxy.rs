use axum::{body::Body, http::Request, response::Response};
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct SodaAudioProxyRequest {
    pub target: String,
    pub request: Request<Body>,
    pub play_auth: Option<String>,
}

pub struct SodaAudioProxyDeps {}

pub struct SodaAudioProxy {
    deps: SodaAudioProxyDeps,
}

impl SodaAudioProxy {
    pub async fn resolve(&self, _input: SodaAudioProxyRequest) -> anyhow::Result<Response> {
        anyhow::bail!("soda audio proxy is not implemented")
    }

    pub fn deps(&self) -> &SodaAudioProxyDeps {
        &self.deps
    }
}

pub fn create_soda_audio_proxy(deps: SodaAudioProxyDeps) -> SodaAudioProxy {
    SodaAudioProxy { deps }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DecryptDataResult {
    pub data: Vec<u8>,
    pub decrypted: bool,
}

pub fn decode_soda_spade_bytes_for_test(_spade_key_bytes: &[u8]) -> Vec<u8> {
    Vec::new()
}

pub async fn decrypt_soda_audio_data(
    _file_data: Vec<u8>,
    _play_auth: String,
) -> anyhow::Result<DecryptDataResult> {
    anyhow::bail!("soda audio decryption is not implemented")
}
