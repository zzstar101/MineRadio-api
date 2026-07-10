use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use futures::future::{BoxFuture, FutureExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::types::Track;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const OPEN_METEO_FORECAST_URL: &str = "https://api.open-meteo.com/v1/forecast";
const OPEN_METEO_GEOCODE_URL: &str = "https://geocoding-api.open-meteo.com/v1/search";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct WeatherRadioParams {
    pub city: Option<String>,
    pub q: Option<String>,
    pub location: Option<String>,
    pub lat: Option<Value>,
    pub lon: Option<Value>,
    pub timezone: Option<String>,
}

pub type WeatherNow = Arc<dyn Fn() -> i64 + Send + Sync>;
pub type WeatherFetch = Arc<
    dyn Fn(WeatherRadioParams) -> BoxFuture<'static, anyhow::Result<WeatherSnapshot>> + Send + Sync,
>;
pub type WeatherSearch =
    Arc<dyn Fn(String, u32) -> BoxFuture<'static, anyhow::Result<Vec<Track>>> + Send + Sync>;

#[derive(Clone)]
pub struct WeatherRadioDeps {
    pub now: WeatherNow,
    pub fetch_weather: WeatherFetch,
    pub search_tracks: WeatherSearch,
}

impl Default for WeatherRadioDeps {
    fn default() -> Self {
        Self {
            now: Arc::new(now_millis),
            fetch_weather: Arc::new(|params| {
                async move { fetch_open_meteo_weather(params).await }.boxed()
            }),
            search_tracks: Arc::new(|_keyword, _limit| async move { Ok(Vec::new()) }.boxed()),
        }
    }
}

#[derive(Clone, Default)]
pub struct WeatherRadioService {
    deps: WeatherRadioDeps,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherLocation {
    pub name: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub admin1: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    #[serde(default)]
    pub timezone: String,
    #[serde(default)]
    pub fallback: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WeatherMood {
    pub key: String,
    pub title: String,
    pub tagline: String,
    pub energy: f64,
    pub warmth: f64,
    pub focus: f64,
    pub melancholy: f64,
    #[serde(default)]
    pub keywords: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherSnapshot {
    pub provider: String,
    pub location: WeatherLocation,
    pub label: String,
    pub weather_code: Option<f64>,
    pub temperature: Option<f64>,
    pub apparent_temperature: Option<f64>,
    pub humidity: Option<f64>,
    pub precipitation: Option<f64>,
    pub cloud_cover: Option<f64>,
    pub wind_speed: Option<f64>,
    pub wind_gusts: Option<f64>,
    pub is_day: Option<f64>,
    #[serde(default)]
    pub time: String,
    pub updated_at: i64,
    #[serde(default)]
    pub error: String,
    pub mood: WeatherMood,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WeatherRadio {
    title: String,
    subtitle: String,
    seed_queries: Vec<String>,
    songs: Vec<Track>,
    updated_at: i64,
}

#[derive(Clone, Debug, Serialize)]
struct WeatherRadioResponse {
    ok: bool,
    weather: WeatherSnapshot,
    radio: WeatherRadio,
}

impl WeatherRadioService {
    pub async fn build(&self, params: WeatherRadioParams) -> anyhow::Result<Value> {
        build_weather_radio(params, self.deps.clone()).await
    }

}

pub fn create_weather_radio_service(deps: WeatherRadioDeps) -> WeatherRadioService {
    WeatherRadioService { deps }
}

pub async fn build_weather_radio(
    params: WeatherRadioParams,
    deps: WeatherRadioDeps,
) -> anyhow::Result<Value> {
    let now = deps.now.clone();
    let weather = match (deps.fetch_weather)(params.clone()).await {
        Ok(weather) => weather,
        Err(err) => fallback_weather_for_radio(&params, err, (now)()),
    };

    let queries = weather_radio_seed_queries(&weather.mood);
    let mut songs = Vec::new();
    for query in queries.iter().take(4) {
        if let Ok(mut found) = (deps.search_tracks)(query.clone(), 6).await {
            songs.append(&mut found);
        }
    }
    if songs.len() < 10 && !weather.mood.keywords.is_empty() {
        for query in weather.mood.keywords.iter().take(2) {
            if let Ok(mut found) = (deps.search_tracks)(query.clone(), 6).await {
                songs.append(&mut found);
            }
        }
    }

    let response = WeatherRadioResponse {
        ok: true,
        radio: WeatherRadio {
            title: weather.mood.title.clone(),
            subtitle: weather.mood.tagline.clone(),
            seed_queries: queries.into_iter().take(4).collect(),
            songs: order_weather_songs(songs, &weather.mood)
                .into_iter()
                .take(18)
                .collect(),
            updated_at: (now)(),
        },
        weather,
    };
    Ok(serde_json::to_value(response)?)
}

pub fn open_meteo_weather_label(code: Value) -> &'static str {
    match code.as_f64().map(|value| value as i64) {
        Some(0) => "鏅?",
        Some(1) | Some(2) => "灏戜簯",
        Some(3) => "闃?",
        Some(45) | Some(48) => "闆?",
        Some(51) | Some(53) | Some(55) => "姣涙瘺闆?",
        Some(56) | Some(57) | Some(66) | Some(67) => "鍐婚洦",
        Some(61) | Some(63) | Some(65) => "闆?",
        Some(71) | Some(73) | Some(75) | Some(77) => "闆?",
        Some(80) | Some(81) | Some(82) => "闃甸洦",
        Some(85) | Some(86) => "闃甸洩",
        Some(95) | Some(96) | Some(99) => "闆烽洦",
        _ => "澶╂皵",
    }
}

fn build_weather_mood_inner(weather: &Value, hour: u32) -> WeatherMood {
    let code = number_field(weather, "weatherCode");
    let temp = number_field(weather, "temperature");
    let apparent = number_field(weather, "apparentTemperature");
    let rain = number_field(weather, "precipitation").unwrap_or(0.0);
    let humidity = number_field(weather, "humidity").unwrap_or(0.0);
    let wind = number_field(weather, "windSpeed").unwrap_or(0.0);
    let is_day = number_field(weather, "isDay");
    let is_night = is_day == Some(0.0) || hour < 6 || hour >= 20;
    let is_morning = (5..11).contains(&hour);
    let is_dusk = (17..20).contains(&hour);
    let code_i = code.map(|value| value as i64).unwrap_or(i64::MIN);
    let is_rain = rain > 0.0
        || [
            51, 53, 55, 56, 57, 61, 63, 65, 66, 67, 80, 81, 82, 95, 96, 99,
        ]
        .contains(&code_i);
    let is_snow = [71, 73, 75, 77, 85, 86].contains(&code_i);
    let is_cloud = [2, 3, 45, 48].contains(&code_i);
    let is_storm = [95, 96, 99].contains(&code_i);
    let feels = apparent.or(temp).unwrap_or(f64::NAN);

    let mut mood = WeatherMood {
        key: "clear".to_owned(),
        title: "鏅存湕鐢靛彴".to_owned(),
        tagline: "璁╄妭濂忎寒涓€鐐癸紝鍍忕獥杈圭殑鍏?".to_owned(),
        energy: 0.62,
        warmth: 0.58,
        focus: 0.48,
        melancholy: 0.24,
        keywords: strings(&[
            "杞诲揩 鍗庤",
            "city pop",
            "indie pop",
            "chill pop",
            "闃冲厜 姝屽崟",
        ]),
    };
    if is_storm {
        mood = WeatherMood {
            key: "storm".to_owned(),
            title: "闆烽洦鐢靛彴".to_owned(),
            tagline: "浣庨鏇村帤锛岄€傚悎鎶婁笘鐣屽叧灏忎竴鐐?".to_owned(),
            energy: 0.46,
            warmth: 0.34,
            focus: 0.66,
            melancholy: 0.62,
            keywords: strings(&[
                "鏆楄壊 R&B",
                "trip hop",
                "澶滄櫄 鐢靛瓙",
                "姘涘洿 鎽囨粴",
                "闆ㄥ 姝屽崟",
            ]),
        };
    } else if is_rain {
        mood = WeatherMood {
            key: "rain".to_owned(),
            title: "闆ㄥぉ鐢靛彴".to_owned(),
            tagline: "鐣欎竴鐐规疆婀跨殑绌洪棿缁欐棆寰?".to_owned(),
            energy: 0.38,
            warmth: 0.42,
            focus: 0.64,
            melancholy: 0.66,
            keywords: strings(&[
                "闆ㄥぉ R&B",
                "lofi rainy",
                "鍗庤 鎱㈡瓕",
                "dream pop",
                "闆ㄥ 姝屽崟",
            ]),
        };
    } else if is_snow || feels <= 3.0 {
        mood = WeatherMood {
            key: "snow".to_owned(),
            title: "鍐风┖姘旂數鍙?".to_owned(),
            tagline: "骞插噣銆佹參閫熴€佸甫涓€鐐瑰啲澶╃殑棰楃矑鎰?".to_owned(),
            energy: 0.34,
            warmth: 0.28,
            focus: 0.72,
            melancholy: 0.54,
            keywords: strings(&[
                "鍐ぉ 姘戣埃",
                "ambient piano",
                "鏃ョ郴 鍐ぉ",
                "indie folk",
                "瀹夐潤 姝屽崟",
            ]),
        };
    } else if feels >= 31.0 || humidity >= 78.0 {
        mood = WeatherMood {
            key: "humid".to_owned(),
            title: "闂风儹鐢靛彴".to_owned(),
            tagline: "闄嶄綆瀵嗗害锛岀暀鍑轰竴鐐瑰懠鍚?".to_owned(),
            energy: 0.48,
            warmth: 0.76,
            focus: 0.46,
            melancholy: 0.30,
            keywords: strings(&[
                "澶忔棩 chill",
                "bossa nova",
                "city pop 澶忓ぉ",
                "杞荤數瀛?",
                "娴疯竟 姝屽崟",
            ]),
        };
    } else if is_cloud {
        mood = WeatherMood {
            key: "cloudy".to_owned(),
            title: "闃村ぉ鐢靛彴".to_owned(),
            tagline: "涓嶆€ョ潃鏄庝寒锛屽厛璁╁０闊冲彉杞?".to_owned(),
            energy: 0.40,
            warmth: 0.46,
            focus: 0.58,
            melancholy: 0.52,
            keywords: strings(&[
                "闃村ぉ 鍗庤",
                "indie rock mellow",
                "neo soul",
                "chillhop",
                "鐙珛 姘戣埃",
            ]),
        };
    }

    if is_night {
        mood.key.push_str("-night");
        mood.title = if mood.key.starts_with("clear") {
            "澶滆壊鐢靛彴".to_owned()
        } else {
            mood.title.replace("鐢靛彴", "澶滃惉")
        };
        mood.tagline = "闊抽噺鏀句綆涓€鐐癸紝璁╁鑹插弬涓庣紪鏇?".to_owned();
        mood.energy = mood.energy.min(0.42);
        mood.focus = mood.focus.max(0.68);
        mood.melancholy = mood.melancholy.max(0.52);
        mood.keywords = prepend_unique(
            strings(&[
                "澶滄櫄 R&B",
                "late night jazz",
                "ambient",
                "lofi sleep",
                "澶滆窇 姝屽崟",
            ]),
            mood.keywords,
            3,
        );
    } else if is_morning {
        mood.title = if mood.key.starts_with("rain") {
            "闆ㄦ櫒鐢靛彴"
        } else {
            "鏃╂櫒鐢靛彴"
        }
        .to_owned();
        mood.energy = mood.energy.max(0.52);
        mood.keywords = prepend_unique(
            strings(&[
                "鏃╂櫒 閫氬嫟",
                "morning acoustic",
                "娓呮櫒 indie",
                "杞诲揩 鍗庤",
            ]),
            mood.keywords,
            3,
        );
    } else if is_dusk {
        mood.title = if mood.key.starts_with("rain") {
            "榛勬槒闆ㄥ０"
        } else {
            "榛勬槒鐢靛彴"
        }
        .to_owned();
        mood.melancholy = mood.melancholy.max(0.48);
        mood.keywords = prepend_unique(
            strings(&[
                "榛勬槒 city pop",
                "鏃ヨ惤 姝屽崟",
                "钀芥棩椋炶溅",
                "soul pop",
            ]),
            mood.keywords,
            3,
        );
    }

    if wind >= 28.0 {
        mood.energy = mood.energy.max(0.56);
        mood.keywords = prepend_unique(
            strings(&["鍏矾 鎽囨粴", "windy day playlist"]),
            mood.keywords,
            4,
        );
    }
    mood.keywords = unique_strings(mood.keywords).into_iter().take(7).collect();
    mood
}

async fn fetch_open_meteo_weather(params: WeatherRadioParams) -> anyhow::Result<WeatherSnapshot> {
    let location = resolve_open_meteo_location(&params).await?;
    let mut url = url::Url::parse(OPEN_METEO_FORECAST_URL)?;
    url.query_pairs_mut()
        .append_pair("latitude", &location.latitude.unwrap_or_default().to_string())
        .append_pair("longitude", &location.longitude.unwrap_or_default().to_string())
        .append_pair("current", "temperature_2m,relative_humidity_2m,apparent_temperature,is_day,precipitation,rain,showers,snowfall,weather_code,cloud_cover,wind_speed_10m,wind_gusts_10m")
        .append_pair("hourly", "precipitation_probability,weather_code,temperature_2m")
        .append_pair("forecast_days", "1")
        .append_pair("timezone", if location.timezone.is_empty() { "auto" } else { &location.timezone });
    let body = request_json(url.as_str()).await?;
    let cur = body
        .get("current")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut weather = WeatherSnapshot {
        provider: "open-meteo".to_owned(),
        location,
        label: open_meteo_weather_label(cur.get("weather_code").cloned().unwrap_or(Value::Null))
            .to_owned(),
        weather_code: finite_or_null(cur.get("weather_code")),
        temperature: finite_or_null(cur.get("temperature_2m")),
        apparent_temperature: finite_or_null(cur.get("apparent_temperature")),
        humidity: finite_or_null(cur.get("relative_humidity_2m")),
        precipitation: finite_or_null(Some(&json!(
            number_from_value(cur.get("precipitation")).unwrap_or(0.0)
                + number_from_value(cur.get("rain")).unwrap_or(0.0)
                + number_from_value(cur.get("showers")).unwrap_or(0.0)
                + number_from_value(cur.get("snowfall")).unwrap_or(0.0)
        ))),
        cloud_cover: finite_or_null(cur.get("cloud_cover")),
        wind_speed: finite_or_null(cur.get("wind_speed_10m")),
        wind_gusts: finite_or_null(cur.get("wind_gusts_10m")),
        is_day: finite_or_null(cur.get("is_day")),
        time: cur
            .get("time")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        updated_at: now_millis(),
        error: String::new(),
        mood: WeatherMood {
            key: String::new(),
            title: String::new(),
            tagline: String::new(),
            energy: 0.0,
            warmth: 0.0,
            focus: 0.0,
            melancholy: 0.0,
            keywords: Vec::new(),
        },
    };
    weather.mood = build_weather_mood_inner(&serde_json::to_value(&weather)?, chrono_like_hour());
    Ok(weather)
}

async fn resolve_open_meteo_location(
    params: &WeatherRadioParams,
) -> anyhow::Result<WeatherLocation> {
    let lat = clamp_number(params.lat.as_ref(), -90.0, 90.0, f64::NAN);
    let lon = clamp_number(params.lon.as_ref(), -180.0, 180.0, f64::NAN);
    if lat.is_finite() && lon.is_finite() {
        return Ok(WeatherLocation {
            name: first_param(params).unwrap_or_else(|| "褰撳墠浣嶇疆".to_owned()),
            country: String::new(),
            admin1: String::new(),
            latitude: Some(lat),
            longitude: Some(lon),
            timezone: params.timezone.clone().unwrap_or_else(|| "auto".to_owned()),
            fallback: false,
        });
    }

    let raw = first_param(params).unwrap_or_default();
    if raw.is_empty() {
        return Ok(default_location(false));
    }
    let mut url = url::Url::parse(OPEN_METEO_GEOCODE_URL)?;
    url.query_pairs_mut()
        .append_pair("name", &raw)
        .append_pair("count", "1")
        .append_pair("language", "zh")
        .append_pair("format", "json");
    let body = request_json(url.as_str()).await?;
    let first = body
        .get("results")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_object);
    let Some(first) = first else {
        return Ok(default_location(true));
    };
    Ok(WeatherLocation {
        name: first
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(&raw)
            .to_owned(),
        country: first
            .get("country")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        admin1: first
            .get("admin1")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        latitude: finite_or_null(first.get("latitude")),
        longitude: finite_or_null(first.get("longitude")),
        timezone: first
            .get("timezone")
            .and_then(Value::as_str)
            .unwrap_or("auto")
            .to_owned(),
        fallback: false,
    })
}

async fn request_json(target: &str) -> anyhow::Result<Value> {
    let response = reqwest::Client::new()
        .get(target)
        .header("user-agent", UA)
        .send()
        .await?;
    if !response.status().is_success() {
        anyhow::bail!("weather request failed: {}", response.status().as_u16());
    }
    Ok(response.json().await?)
}

fn fallback_weather_for_radio(
    params: &WeatherRadioParams,
    err: anyhow::Error,
    now: i64,
) -> WeatherSnapshot {
    let mut location = default_location(true);
    location.name = first_param(params).unwrap_or(location.name);
    location.timezone = params.timezone.clone().unwrap_or(location.timezone);
    WeatherSnapshot {
        provider: "open-meteo".to_owned(),
        location,
        label: "澶╂皵鏆備笉鍙敤".to_owned(),
        weather_code: None,
        temperature: None,
        apparent_temperature: None,
        humidity: None,
        precipitation: None,
        cloud_cover: None,
        wind_speed: None,
        wind_gusts: None,
        is_day: None,
        time: String::new(),
        updated_at: now,
        error: err.to_string(),
        mood: WeatherMood {
            key: "fallback".to_owned(),
            title: "涓存椂鐢靛彴".to_owned(),
            tagline: "澶╂皵鏆傛椂娌℃湁鍥炴潵锛屽厛鏀句竴缁勭ǔ濡ョ殑姝?".to_owned(),
            energy: 0.54,
            warmth: 0.55,
            focus: 0.55,
            melancholy: 0.35,
            keywords: strings(&[
                "鍗庤 娴佽",
                "indie pop",
                "city pop",
                "杞诲揩 姝屽崟",
                "chill pop",
            ]),
        },
    }
}

fn weather_radio_seed_queries(mood: &WeatherMood) -> Vec<String> {
    let key = &mood.key;
    if key.contains("rain") || key.contains("storm") {
        return strings(&[
            "闄堝杩?闃村ぉ蹇箰",
            "鍛ㄦ澃浼?闆ㄤ笅涓€鏁存櫄",
            "瀛欑嚂濮?閬囪",
            "鏋楀鍢?璇磋皫",
            "姣涗笉鏄?娑堟剚",
        ]);
    }
    if key.contains("snow") || key.contains("cloudy") {
        return strings(&[
            "闄堝杩?濂戒箙涓嶈",
            "鑾枃钄?闃村ぉ",
            "鏉庡仴 璐濆姞灏旀箹鐣?",
            "鏈存爲 骞冲嚒涔嬭矾",
            "钄″仴闆?杈惧皵鏂?",
        ]);
    }
    if key.contains("humid") {
        return strings(&[
            "钀芥棩椋炶溅 My Jinji",
            "鍛婁簲浜?鐖变汉閿欒繃",
            "澶忔棩鍏ヤ镜浼佺敾 鎯冲幓娴疯竟",
            "闄堢划璐?鏃呰鐨勬剰涔?",
            "鐜嬭嫢鐞?Lost in Paradise",
        ]);
    }
    if key.contains("night") {
        return strings(&[
            "鏂瑰ぇ鍚?鐗瑰埆鐨勪汉",
            "闄跺枂 鐖卞緢绠€鍗?",
            "Frank Ocean Pink + White",
            "鏋楀繂鑾?澶滃お榛?",
            "Norah Jones Don't Know Why",
        ]);
    }
    strings(&[
        "瀛欑嚂濮?澶╅粦榛?",
        "鍛ㄦ澃浼?鏅村ぉ",
        "浜旀湀澶?娓╂煍",
        "闄堝杩?绋崇ǔ鐨勫垢绂?",
        "鐜嬭彶",
    ])
}

fn order_weather_songs(songs: Vec<Track>, mood: &WeatherMood) -> Vec<Track> {
    let mut sorted = unique_songs_by_key(songs)
        .into_iter()
        .filter(|song| {
            !song.title.is_empty() && !song.id.is_empty() && !is_low_signal_weather_song(song)
        })
        .collect::<Vec<_>>();
    sorted.sort_by(|a, b| score_weather_song(b, mood).cmp(&score_weather_song(a, mood)));
    diversify_weather_songs(unique_weather_titles(sorted), 2)
}

fn unique_songs_by_key(songs: Vec<Track>) -> Vec<Track> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for song in songs {
        let fallback = format!("{}|{}", song.title, song.artists.join("/"));
        let key = if song.id.is_empty() {
            fallback
        } else {
            song.id.clone()
        };
        if key.trim().is_empty() || seen.contains(&key) {
            continue;
        }
        seen.insert(key);
        out.push(song);
    }
    out
}

fn is_low_signal_weather_song(song: &Track) -> bool {
    let text = format!("{} {} {}", song.title, song.artists.join(" "), song.album).to_lowercase();
    if text.trim().is_empty() {
        return true;
    }
    let patterns = [
        r"(?i)(^|[\s\-_/锛?])ai(?:\s*(姝寍姝屾洸|闊充箰|cover|缈诲敱|鐢熸垚|浣滄洸|婕斿敱|濂冲０|鐢峰０)|$|[\s\-_/锛?])",
        r"(?i)suno|udio|浜哄伐鏅鸿兘|鐢熸垚姝屾洸|ai姝屾洸|铏氭嫙姝屾墜|娴嬭瘯闊抽|demo|beat\s*maker",
        r"(?i)缈昏嚜|缈诲敱|cover|remix|浼村|绾煶涔恷閽㈢惔|dj|live\s*鐗坾live鐗坾鍞編閽㈢惔|karaoke|instrumental",
        r"(?i)鐧藉櫔闊硘闆ㄥ０|鐫＄湢|鍔╃湢|鍐ユ兂|鐤楁剤棰戠巼|鐜闊硘鑷劧澹伴煶|asmr",
        r"(?i)[锛?](r&b|lofi|jazz|dj|edm|trap|remix|浼村|绾煶涔恷閽㈢惔|鐢靛瓙|娌绘剤|鍙ら|濂冲０|鐢峰０|鑻辨枃|涓枃鐗坾鎶栭煶|ai)[锛?]",
    ];
    patterns.iter().any(|pattern| regex::Regex::new(pattern).unwrap().is_match(&text))
        || regex::Regex::new(r"(?i)^(绾煶涔恷杞婚煶涔恷娌绘剤绯粅鏀炬澗|鐫＄湢|闆ㄥぉ|闃村ぉ|澶滄櫄|澶忔棩|娴疯竟)$")
            .unwrap()
            .is_match(song.title.trim())
}

fn score_weather_song(song: &Track, mood: &WeatherMood) -> i32 {
    let text = format!("{} {} {}", song.title, song.artists.join(" "), song.album).to_lowercase();
    let mut score = 0;
    if !song.cover_url.is_empty() {
        score += 4;
    }
    if song.duration_ms.is_some() {
        score += 2;
    }
    if regex::Regex::new(r"鍛ㄦ澃浼闄堝杩厊瀛欑嚂濮縷浜旀湀澶﹟鐜嬭彶|闄跺枂|鏂瑰ぇ鍚寍鏋楀鍢墊钄″仴闆厊鑾枃钄殀鏉庡仴|姣涗笉鏄搢鍛婁簲浜簗钀芥棩椋炶溅|闄堢划璐瀨鏈存爲").unwrap().is_match(&text) {
        score += 10;
    }
    let key = &mood.key;
    if key.contains("rain")
        && regex::Regex::new(r"闆▅闃磡澶渱鎱r&b|soul|闄堝杩厊鏋楀鍢墊瀛欑嚂濮?")
            .unwrap()
            .is_match(&text)
    {
        score += 5;
    }
    if key.contains("humid")
        && regex::Regex::new(r"澶弢娴穦city|pop|钀芥棩|鍛婁簲浜簗鏂瑰ぇ鍚寍闄跺枂")
            .unwrap()
            .is_match(&text)
    {
        score += 5;
    }
    if key.contains("night")
        && regex::Regex::new(r"澶渱moon|jazz|soul|r&b|鏂瑰ぇ鍚寍闄跺枂|鐜嬭彶")
            .unwrap()
            .is_match(&text)
    {
        score += 5;
    }
    if key.contains("cloudy")
        && regex::Regex::new(r"闃磡姘戣埃|indie|闄堢划璐瀨鏈存爲|鏉庡仴")
            .unwrap()
            .is_match(&text)
    {
        score += 5;
    }
    score
}

fn unique_weather_titles(sorted: Vec<Track>) -> Vec<Track> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for song in sorted {
        let key = weather_title_key(&song);
        if !key.is_empty() && seen.contains(&key) {
            continue;
        }
        if !key.is_empty() {
            seen.insert(key);
        }
        out.push(song);
    }
    out
}

fn weather_title_key(song: &Track) -> String {
    let lower = song.title.to_lowercase();
    let no_brackets = regex::Regex::new(r"[锛?][^锛?]*[锛?]")
        .unwrap()
        .replace_all(&lower, "");
    regex::Regex::new(r#"[\s._\-路'鈥?鈥溾€濄€屻€嶃€娿€?锛?\\|]+"#)
        .unwrap()
        .replace_all(&no_brackets, "")
        .trim()
        .to_owned()
}

fn diversify_weather_songs(sorted: Vec<Track>, artist_limit: usize) -> Vec<Track> {
    let mut primary = Vec::new();
    let mut deferred = Vec::new();
    let mut counts = HashMap::<String, usize>::new();
    for song in sorted {
        let key = weather_artist_key(&song);
        let count = *counts.get(&key).unwrap_or(&0);
        if count < artist_limit {
            primary.push(song);
            counts.insert(key, count + 1);
        } else {
            deferred.push(song);
        }
    }
    if primary.len() >= 8 {
        primary
    } else {
        let needed = 8 - primary.len();
        primary.extend(deferred.into_iter().take(needed));
        primary
    }
}

fn weather_artist_key(song: &Track) -> String {
    let raw = song.artists.first().unwrap_or(&song.title);
    let first = regex::Regex::new(r"\s*/\s*|銆亅,|&")
        .unwrap()
        .split(raw)
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    if first.is_empty() {
        "unknown".to_owned()
    } else {
        first
    }
}

fn default_location(fallback: bool) -> WeatherLocation {
    WeatherLocation {
        name: "涓婃捣".to_owned(),
        country: "China".to_owned(),
        admin1: String::new(),
        latitude: Some(31.2304),
        longitude: Some(121.4737),
        timezone: "Asia/Shanghai".to_owned(),
        fallback,
    }
}

fn first_param(params: &WeatherRadioParams) -> Option<String> {
    [&params.city, &params.q, &params.location]
        .into_iter()
        .filter_map(|value| value.as_ref())
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .map(str::to_owned)
}

fn clamp_number(value: Option<&Value>, min: f64, max: f64, fallback: f64) -> f64 {
    let Some(value) = value.and_then(|value| number_from_value(Some(value))) else {
        return fallback;
    };
    value.max(min).min(max)
}

fn finite_or_null(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(|value| number_from_value(Some(value)))
        .filter(|value| value.is_finite())
}

fn number_from_value(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn number_field(value: &Value, field: &str) -> Option<f64> {
    value
        .get(field)
        .and_then(|value| number_from_value(Some(value)))
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn prepend_unique(mut prefix: Vec<String>, rest: Vec<String>, rest_take: usize) -> Vec<String> {
    prefix.extend(rest.into_iter().take(rest_take));
    unique_strings(prefix)
}

fn unique_strings(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn chrono_like_hour() -> u32 {
    12
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str, title: &str) -> Track {
        Track {
            provider: "netease".to_owned(),
            id: id.to_owned(),
            source_id: id.to_owned(),
            media_mid: None,
            title: title.to_owned(),
            artists: vec!["闄堝杩?".to_owned()],
            album: String::new(),
            cover_url: String::new(),
            quality_hints: vec!["standard".to_owned()],
            playable_state: "playable".to_owned(),
            duration_ms: Some(180_000),
            artwork_url: None,
        }
    }

    #[test]
    fn open_meteo_weather_label_preserves_baseline_weather_code_labels() {
        assert_eq!(open_meteo_weather_label(json!(0)), "鏅?");
        assert_eq!(open_meteo_weather_label(json!(61)), "闆?");
        assert_eq!(open_meteo_weather_label(json!(95)), "闆烽洦");
    }

    #[test]
    fn build_weather_mood_maps_rainy_weather_to_baseline_rainy_radio_copy() {
        let mood = build_weather_mood_inner(
            &json!({
                "weatherCode": 61,
                "temperature": 22,
                "apparentTemperature": 21,
                "precipitation": 1,
                "humidity": 88,
                "windSpeed": 6,
                "isDay": 1
            }),
            13,
        );

        assert_eq!(mood.key, "rain");
        assert_eq!(mood.title, "闆ㄥぉ鐢靛彴");
        assert!(mood.keywords.contains(&"闆ㄥぉ R&B".to_owned()));
    }

    #[tokio::test]
    async fn build_weather_radio_falls_back_to_temporary_radio_when_weather_provider_fails() {
        let searched = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let searched_for_dep = Arc::clone(&searched);
        let result = build_weather_radio(
            WeatherRadioParams {
                city: Some("涓婃捣".to_owned()),
                ..Default::default()
            },
            WeatherRadioDeps {
                now: Arc::new(|| 123),
                fetch_weather: Arc::new(|_| async move { anyhow::bail!("network down") }.boxed()),
                search_tracks: Arc::new(move |keyword, limit| {
                    let searched = Arc::clone(&searched_for_dep);
                    async move {
                        searched.lock().unwrap().push(format!("{keyword}:{limit}"));
                        Ok(vec![track(&keyword, &keyword)])
                    }
                    .boxed()
                }),
            },
        )
        .await
        .unwrap();

        assert_eq!(result["ok"], true);
        assert_eq!(result["weather"]["label"], "澶╂皵鏆備笉鍙敤");
        assert_eq!(result["radio"]["title"], "涓存椂鐢靛彴");
        assert_eq!(result["radio"]["songs"].as_array().unwrap().len(), 6);
        assert_eq!(searched.lock().unwrap()[0], "瀛欑嚂濮?澶╅粦榛?:6");
    }

    #[tokio::test]
    async fn build_weather_radio_orders_and_filters_weather_songs_like_baseline_pool_cleanup() {
        let mood = build_weather_mood_inner(
            &json!({
                "weatherCode": 61,
                "temperature": 22,
                "apparentTemperature": 21,
                "precipitation": 1,
                "humidity": 88,
                "windSpeed": 6,
                "isDay": 1
            }),
            13,
        );
        let result = build_weather_radio(
            WeatherRadioParams {
                city: Some("涓婃捣".to_owned()),
                ..Default::default()
            },
            WeatherRadioDeps {
                now: Arc::new(|| 123),
                fetch_weather: Arc::new(move |_| {
                    let mood = mood.clone();
                    async move {
                        Ok(WeatherSnapshot {
                            provider: "open-meteo".to_owned(),
                            location: WeatherLocation {
                                name: "涓婃捣".to_owned(),
                                country: "涓浗".to_owned(),
                                admin1: String::new(),
                                latitude: Some(31.23),
                                longitude: Some(121.47),
                                timezone: "Asia/Shanghai".to_owned(),
                                fallback: false,
                            },
                            label: "闆?".to_owned(),
                            weather_code: Some(61.0),
                            temperature: Some(22.0),
                            apparent_temperature: Some(21.0),
                            humidity: Some(88.0),
                            precipitation: Some(1.0),
                            cloud_cover: Some(90.0),
                            wind_speed: Some(6.0),
                            wind_gusts: Some(10.0),
                            is_day: Some(1.0),
                            time: String::new(),
                            updated_at: 100,
                            error: String::new(),
                            mood,
                        })
                    }
                    .boxed()
                }),
                search_tracks: Arc::new(|keyword, _limit| {
                    async move {
                        Ok(vec![
                            track("same", "闃村ぉ蹇箰"),
                            track("ai", "AI 缈诲敱 demo"),
                            track(&format!("id-{keyword}"), &keyword),
                        ])
                    }
                    .boxed()
                }),
            },
        )
        .await
        .unwrap();

        assert_eq!(result["weather"]["mood"]["title"], "闆ㄥぉ鐢靛彴");
        assert_eq!(result["radio"]["seedQueries"][0], "闄堝杩?闃村ぉ蹇箰");
        let titles = result["radio"]["songs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|song| song["title"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert!(!titles.contains(&"AI 缈诲敱 demo".to_owned()));
        let same_count = result["radio"]["songs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|song| song["id"] == "same")
            .count();
        assert_eq!(same_count, 1);
    }
}
