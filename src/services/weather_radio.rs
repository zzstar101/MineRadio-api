use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct WeatherRadioParams {
    pub city: Option<String>,
    pub q: Option<String>,
    pub location: Option<String>,
    pub lat: Option<Value>,
    pub lon: Option<Value>,
    pub timezone: Option<String>,
}

#[derive(Default)]
pub struct WeatherRadioDeps {}

#[derive(Default)]
pub struct WeatherRadioService {
    deps: WeatherRadioDeps,
}

impl WeatherRadioService {
    pub async fn build(&self, params: WeatherRadioParams) -> anyhow::Result<Value> {
        build_weather_radio(params, WeatherRadioDeps::default()).await
    }

    pub fn deps(&self) -> &WeatherRadioDeps {
        &self.deps
    }
}

pub fn create_weather_radio_service(deps: WeatherRadioDeps) -> WeatherRadioService {
    WeatherRadioService { deps }
}

pub async fn build_weather_radio(
    _params: WeatherRadioParams,
    _deps: WeatherRadioDeps,
) -> anyhow::Result<Value> {
    anyhow::bail!("weather radio service is not implemented")
}

pub fn open_meteo_weather_label(code: Value) -> &'static str {
    match code.as_i64() {
        Some(0) => "晴",
        Some(1) | Some(2) => "少云",
        Some(3) => "阴",
        Some(45) | Some(48) => "雾",
        Some(51) | Some(53) | Some(55) => "毛毛雨",
        Some(56) | Some(57) | Some(66) | Some(67) => "冻雨",
        Some(61) | Some(63) | Some(65) => "雨",
        Some(71) | Some(73) | Some(75) | Some(77) => "雪",
        Some(80) | Some(81) | Some(82) => "阵雨",
        Some(85) | Some(86) => "阵雪",
        Some(95) | Some(96) | Some(99) => "雷雨",
        _ => "天气",
    }
}

pub fn build_weather_mood(_weather: Value) -> Value {
    Value::Null
}
