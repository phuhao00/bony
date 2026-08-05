//! Weather tool — Open-Meteo (geocoding + forecast).
//!
//! Dropped into ZeroClaw's managed tree as
//! `crates/zeroclaw-tools/src/weather_tool.rs` before `cargo build`, because
//! upstream's default wttr.in backend frequently mis-resolves Chinese cities
//! (e.g. 深圳 → Ma Tso Lung, Hong Kong).
//!
//! No API key required. Global coverage via open-meteo.com.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use zeroclaw_api::tool::{Tool, ToolOutput, ToolResult};

const GEO_URL: &str = "https://geocoding-api.open-meteo.com/v1/search";
const FORECAST_URL: &str = "https://api.open-meteo.com/v1/forecast";
const TIMEOUT_SECS: u64 = 15;
const CONNECT_TIMEOUT_SECS: u64 = 10;

// Well-known Chinese place names → English query preferred by geocoding.
// Pinning majors avoids ambiguous multi-hits.
fn rewrite_location_query(raw: &str) -> String {
    let t = raw.trim();
    match t {
        "深圳" | "深圳市" => "Shenzhen, Guangdong, China".into(),
        "北京" | "北京市" => "Beijing, China".into(),
        "上海" | "上海市" => "Shanghai, China".into(),
        "广州" | "广州市" => "Guangzhou, Guangdong, China".into(),
        "杭州" | "杭州市" => "Hangzhou, Zhejiang, China".into(),
        "成都" | "成都市" => "Chengdu, Sichuan, China".into(),
        "重庆" | "重庆市" => "Chongqing, China".into(),
        "武汉" | "武汉市" => "Wuhan, Hubei, China".into(),
        "西安" | "西安市" => "Xi'an, Shaanxi, China".into(),
        "南京" | "南京市" => "Nanjing, Jiangsu, China".into(),
        "苏州" | "苏州市" => "Suzhou, Jiangsu, China".into(),
        "天津" | "天津市" => "Tianjin, China".into(),
        "香港" => "Hong Kong".into(),
        "澳门" | "澳門" => "Macau".into(),
        "台北" | "臺北" => "Taipei, Taiwan".into(),
        other => other.to_string(),
    }
}

#[derive(Debug, Deserialize)]
struct GeoResponse {
    results: Option<Vec<GeoHit>>,
}

#[derive(Debug, Deserialize)]
struct GeoHit {
    name: String,
    latitude: f64,
    longitude: f64,
    country: Option<String>,
    #[serde(default)]
    admin1: Option<String>,
    timezone: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ForecastResponse {
    current: Option<CurrentBlock>,
    daily: Option<DailyBlock>,
    hourly: Option<HourlyBlock>,
}

#[derive(Debug, Deserialize)]
struct CurrentBlock {
    time: Option<String>,
    temperature_2m: Option<f64>,
    relative_humidity_2m: Option<f64>,
    apparent_temperature: Option<f64>,
    weather_code: Option<i32>,
    wind_speed_10m: Option<f64>,
    wind_direction_10m: Option<f64>,
    precipitation: Option<f64>,
    cloud_cover: Option<f64>,
    surface_pressure: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct DailyBlock {
    time: Vec<String>,
    temperature_2m_max: Option<Vec<Option<f64>>>,
    temperature_2m_min: Option<Vec<Option<f64>>>,
    weather_code: Option<Vec<Option<i32>>>,
    precipitation_sum: Option<Vec<Option<f64>>>,
    sunrise: Option<Vec<Option<String>>>,
    sunset: Option<Vec<Option<String>>>,
    uv_index_max: Option<Vec<Option<f64>>>,
}

#[derive(Debug, Deserialize)]
struct HourlyBlock {
    time: Vec<String>,
    temperature_2m: Option<Vec<Option<f64>>>,
    weather_code: Option<Vec<Option<i32>>>,
    precipitation_probability: Option<Vec<Option<f64>>>,
    wind_speed_10m: Option<Vec<Option<f64>>>,
}

/// Fetches weather via Open-Meteo — no API key, reliable CJK geocoding.
pub struct WeatherTool;

impl WeatherTool {
    pub fn new() -> Self {
        Self
    }

    fn http_client() -> anyhow::Result<reqwest::Client> {
        let builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_SECS))
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .user_agent("zeroclaw-weather/1.0 (open-meteo)");
        let builder =
            zeroclaw_config::schema::apply_runtime_proxy_to_builder(builder, "tool.weather");
        Ok(builder.build()?)
    }

    /// Parse bare "lat,lon" / "lat, lon" if present.
    fn parse_coords(raw: &str) -> Option<(f64, f64)> {
        let parts: Vec<&str> = raw.split(',').map(str::trim).collect();
        if parts.len() != 2 {
            return None;
        }
        let lat: f64 = parts[0].parse().ok()?;
        let lon: f64 = parts[1].parse().ok()?;
        if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) {
            Some((lat, lon))
        } else {
            None
        }
    }

    async fn geocode(client: &reqwest::Client, query: &str) -> anyhow::Result<GeoHit> {
        if let Some((lat, lon)) = Self::parse_coords(query) {
            return Ok(GeoHit {
                name: format!("{lat:.4},{lon:.4}"),
                latitude: lat,
                longitude: lon,
                country: None,
                admin1: None,
                timezone: None,
            });
        }

        let url = format!(
            "{GEO_URL}?name={}&count=5&language=en&format=json",
            urlencoding::encode(query)
        );
        let resp = client.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("geocoding failed HTTP {} for '{query}'", resp.status());
        }
        let body: GeoResponse = resp.json().await?;
        let mut hits = body.results.unwrap_or_default();
        if hits.is_empty() {
            anyhow::bail!(
                "Could not resolve location '{query}'. Try a city name (English or local), \
                 or GPS coordinates like '22.54,114.06'."
            );
        }

        // Prefer China mainland for Chinese queries / Guangdong cities.
        let q_lower = query.to_ascii_lowercase();
        if q_lower.contains("china")
            || query.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
            || q_lower.contains("shenzhen")
            || q_lower.contains("guangdong")
        {
            if let Some(idx) = hits.iter().position(|h| {
                h.country
                    .as_deref()
                    .is_some_and(|c| c.eq_ignore_ascii_case("China") || c == "中国")
            }) {
                return Ok(hits.swap_remove(idx));
            }
        }

        Ok(hits.swap_remove(0))
    }

    async fn forecast(
        client: &reqwest::Client,
        hit: &GeoHit,
        days: u8,
        metric: bool,
    ) -> anyhow::Result<ForecastResponse> {
        let temp_unit = if metric { "celsius" } else { "fahrenheit" };
        let wind_unit = if metric { "kmh" } else { "mph" };
        let precip_unit = if metric { "mm" } else { "inch" };
        let tz = hit
            .timezone
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("auto");
        let day_count = days.max(1).min(3);

        let url = format!(
            "{FORECAST_URL}?latitude={lat}&longitude={lon}\
             &current=temperature_2m,relative_humidity_2m,apparent_temperature,\
weather_code,wind_speed_10m,wind_direction_10m,precipitation,cloud_cover,surface_pressure\
             &daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_sum,\
sunrise,sunset,uv_index_max\
             &hourly=temperature_2m,weather_code,precipitation_probability,wind_speed_10m\
             &forecast_days={day_count}\
             &temperature_unit={temp_unit}\
             &wind_speed_unit={wind_unit}\
             &precipitation_unit={precip_unit}\
             &timezone={tz}",
            lat = hit.latitude,
            lon = hit.longitude,
        );

        let resp = client.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!(
                "forecast failed HTTP {} for {},{}",
                resp.status(),
                hit.latitude,
                hit.longitude
            );
        }
        Ok(resp.json().await?)
    }

    fn weather_code_label(code: i32) -> &'static str {
        match code {
            0 => "Clear",
            1 => "Mainly clear",
            2 => "Partly cloudy",
            3 => "Overcast",
            45 | 48 => "Fog",
            51 | 53 | 55 => "Drizzle",
            56 | 57 => "Freezing drizzle",
            61 | 63 | 65 => "Rain",
            66 | 67 => "Freezing rain",
            71 | 73 | 75 => "Snow",
            77 => "Snow grains",
            80 | 81 | 82 => "Rain showers",
            85 | 86 => "Snow showers",
            95 => "Thunderstorm",
            96 | 99 => "Thunderstorm with hail",
            _ => "Unknown",
        }
    }

    fn wind_dir_label(deg: f64) -> &'static str {
        static DIRS: [&str; 16] = [
            "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
            "NW", "NNW",
        ];
        let idx = ((deg / 22.5).round() as usize) % 16;
        DIRS[idx]
    }

    fn place_label(hit: &GeoHit) -> String {
        match (
            hit.admin1.as_deref().filter(|s| !s.is_empty()),
            hit.country.as_deref().filter(|s| !s.is_empty()),
        ) {
            (Some(a), Some(c)) => format!("{}, {}, {}", hit.name, a, c),
            (None, Some(c)) => format!("{}, {}", hit.name, c),
            (Some(a), None) => format!("{}, {}", hit.name, a),
            _ => hit.name.clone(),
        }
    }

    fn format_output(hit: &GeoHit, data: &ForecastResponse, metric: bool, days: u8) -> String {
        let place = Self::place_label(hit);
        let cur = match &data.current {
            Some(c) => c,
            None => return format!("No current conditions for {place}."),
        };

        let temp_u = if metric { "°C" } else { "°F" };
        let wind_u = if metric { "km/h" } else { "mph" };
        let precip_u = if metric { "mm" } else { "in" };

        let temp = cur
            .temperature_2m
            .map(|t| format!("{t:.0}{temp_u}"))
            .unwrap_or_else(|| "—".into());
        let feels = cur
            .apparent_temperature
            .map(|t| format!("{t:.0}{temp_u}"))
            .unwrap_or_else(|| "—".into());
        let humidity = cur
            .relative_humidity_2m
            .map(|h| format!("{h:.0}%"))
            .unwrap_or_else(|| "—".into());
        let wind = match (cur.wind_speed_10m, cur.wind_direction_10m) {
            (Some(s), Some(d)) => format!("{s:.0} {wind_u} {}", Self::wind_dir_label(d)),
            (Some(s), None) => format!("{s:.0} {wind_u}"),
            _ => "—".into(),
        };
        let precip = cur
            .precipitation
            .map(|p| format!("{p:.1} {precip_u}"))
            .unwrap_or_else(|| "—".into());
        let cloud = cur
            .cloud_cover
            .map(|c| format!("{c:.0}%"))
            .unwrap_or_else(|| "—".into());
        let pressure = cur
            .surface_pressure
            .map(|p| format!("{p:.0} hPa"))
            .unwrap_or_else(|| "—".into());
        let desc = cur
            .weather_code
            .map(Self::weather_code_label)
            .unwrap_or("Unknown");
        let obs = cur.time.as_deref().unwrap_or("—");

        let mut out = format!(
            "Weather for {place} (as of {obs})\n\
             ─────────────────────────────────────────\n\
             Conditions : {desc}\n\
             Temperature: {temp} (feels like {feels})\n\
             Humidity   : {humidity}\n\
             Wind       : {wind}\n\
             Precipitation: {precip}\n\
             Pressure   : {pressure}\n\
             Cloud Cover: {cloud}"
        );

        if days == 0 {
            return out;
        }

        if let Some(daily) = &data.daily {
            out.push_str("\n\nForecast\n────────");
            let n = daily.time.len().min(days as usize);
            for i in 0..n {
                let date = &daily.time[i];
                let hi = daily
                    .temperature_2m_max
                    .as_ref()
                    .and_then(|v| v.get(i).copied().flatten())
                    .map(|t| format!("{t:.0}{temp_u}"))
                    .unwrap_or_else(|| "—".into());
                let lo = daily
                    .temperature_2m_min
                    .as_ref()
                    .and_then(|v| v.get(i).copied().flatten())
                    .map(|t| format!("{t:.0}{temp_u}"))
                    .unwrap_or_else(|| "—".into());
                let code = daily
                    .weather_code
                    .as_ref()
                    .and_then(|v| v.get(i).copied().flatten())
                    .map(Self::weather_code_label)
                    .unwrap_or("—");
                let rain = daily
                    .precipitation_sum
                    .as_ref()
                    .and_then(|v| v.get(i).copied().flatten())
                    .map(|p| format!("{p:.1}{precip_u}"))
                    .unwrap_or_else(|| "—".into());
                let uv = daily
                    .uv_index_max
                    .as_ref()
                    .and_then(|v| v.get(i).copied().flatten())
                    .map(|u| format!("{u:.0}"))
                    .unwrap_or_else(|| "—".into());
                let sunrise = daily
                    .sunrise
                    .as_ref()
                    .and_then(|v| v.get(i).and_then(|s| s.as_ref()))
                    .map(|s| s.as_str())
                    .unwrap_or("—");
                let sunset = daily
                    .sunset
                    .as_ref()
                    .and_then(|v| v.get(i).and_then(|s| s.as_ref()))
                    .map(|s| s.as_str())
                    .unwrap_or("—");
                out.push_str(&format!(
                    "\n  {date}: {code} | High {hi} / Low {lo} | Rain {rain} | UV {uv} | \
                     Sunrise {sunrise} | Sunset {sunset}"
                ));
            }
        }

        if days <= 2 {
            if let Some(hourly) = &data.hourly {
                out.push_str("\n\nToday (selected hours)\n────────────────────");
                let take = hourly.time.len().min(24);
                for i in (0..take).step_by(6) {
                    let t = &hourly.time[i];
                    let temp_h = hourly
                        .temperature_2m
                        .as_ref()
                        .and_then(|v| v.get(i).copied().flatten())
                        .map(|t| format!("{t:.0}{temp_u}"))
                        .unwrap_or_else(|| "—".into());
                    let code = hourly
                        .weather_code
                        .as_ref()
                        .and_then(|v| v.get(i).copied().flatten())
                        .map(Self::weather_code_label)
                        .unwrap_or("—");
                    let rain_p = hourly
                        .precipitation_probability
                        .as_ref()
                        .and_then(|v| v.get(i).copied().flatten())
                        .map(|p| format!("{p:.0}%"))
                        .unwrap_or_else(|| "—".into());
                    let wind_h = hourly
                        .wind_speed_10m
                        .as_ref()
                        .and_then(|v| v.get(i).copied().flatten())
                        .map(|s| format!("{s:.0} {wind_u}"))
                        .unwrap_or_else(|| "—".into());
                    out.push_str(&format!(
                        "\n    {t}: {temp_h} — {code} | Wind {wind_h} | Rain chance {rain_p}"
                    ));
                }
            }
        }

        out
    }
}

impl Default for WeatherTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WeatherTool {
    fn name(&self) -> &str {
        "weather"
    }

    fn description(&self) -> &str {
        "Get current weather conditions and up to 3-day forecast for any location worldwide \
         using Open-Meteo. Supports city names (any language, Chinese included — e.g. 深圳, \
         Shanghai), GPS coordinates ('22.54,114.06'). Units default to metric (°C, km/h, mm)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "location": {
                    "type": "string",
                    "description": "City name in any language (prefer 'Shenzhen, China' for \
                                    Chinese cities if ambiguous), or GPS lat,lon."
                },
                "units": {
                    "type": "string",
                    "enum": ["metric", "imperial"],
                    "description": "Unit system. 'metric' = °C, km/h, mm (default). \
                                    'imperial' = °F, mph, inches."
                },
                "days": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 3,
                    "description": "Forecast days (0–3). 0 = current only. Default: 1."
                }
            },
            "required": ["location"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let location = match args.get("location").and_then(|v| v.as_str()) {
            Some(loc) if !loc.trim().is_empty() => rewrite_location_query(loc),
            _ => {
                return Ok(ToolResult {
                    success: false,
                    output: ToolOutput::default(),
                    error: Some("Missing required parameter 'location'".into()),
                });
            }
        };

        let metric = args
            .get("units")
            .and_then(|v| v.as_str())
            .map(|u| u.to_lowercase() != "imperial")
            .unwrap_or(true);

        let days: u8 = args
            .get("days")
            .and_then(|v| v.as_u64())
            .map(|d| d.min(3) as u8)
            .unwrap_or(1);

        let client = match Self::http_client() {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: ToolOutput::default(),
                    error: Some(e.to_string()),
                });
            }
        };

        let hit = match Self::geocode(&client, &location).await {
            Ok(h) => h,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: ToolOutput::default(),
                    error: Some(e.to_string()),
                });
            }
        };

        match Self::forecast(&client, &hit, days, metric).await {
            Ok(data) => {
                let output = Self::format_output(&hit, &data, metric, days);
                Ok(ToolResult {
                    success: true,
                    output: output.into(),
                    error: None,
                })
            }
            Err(e) => Ok(ToolResult {
                success: false,
                output: ToolOutput::default(),
                error: Some(e.to_string()),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_weather() {
        assert_eq!(WeatherTool::new().name(), "weather");
    }

    #[test]
    fn rewrite_maps_shenzhen() {
        assert!(rewrite_location_query("深圳").contains("Shenzhen"));
        assert_eq!(rewrite_location_query("London"), "London");
    }

    #[test]
    fn parse_coords_ok() {
        assert_eq!(
            WeatherTool::parse_coords("22.54, 114.06"),
            Some((22.54, 114.06))
        );
        assert!(WeatherTool::parse_coords("深圳").is_none());
    }

    #[test]
    fn weather_code_labels() {
        assert_eq!(WeatherTool::weather_code_label(0), "Clear");
        assert_eq!(WeatherTool::weather_code_label(2), "Partly cloudy");
    }

    #[tokio::test]
    async fn execute_missing_location_returns_error() {
        let result = WeatherTool::new().execute(json!({})).await.unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("location"));
    }
}
