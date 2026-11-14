//! Hello Weather: a sample DeckPilot plugin.
//!
//! * `greet` is pure: it only reads its parameters.
//! * `current-temperature` asks the agent to fetch
//!   `https://api.open-meteo.com/v1/forecast`. The plugin itself has no
//!   network: the request goes through the permission broker, which allows
//!   it only if the user approved the `network` permission for exactly
//!   that origin and method (see `manifest.json`).
//!
//! Every error is typed: shared codes from `deckpilot_plugin_sdk::codes`
//! or the plugin's own codes, declared in `manifest.json`
//! `contributes.errorCodes` (see [`PLUGIN_CODES`]).
#![deny(unsafe_code)]

use deckpilot_plugin_sdk::prelude::*;
use serde::Deserialize;

/// The only origin this plugin talks to. Must match the manifest's
/// `network` permission (`hosts`, `methods`).
pub const WEATHER_ORIGIN: &str = "https://api.open-meteo.com";

/// Codes this plugin returns besides the shared ones. Each one is declared
/// in `manifest.json` `contributes.errorCodes`.
pub const PLUGIN_CODES: &[&str] = &[WEATHER_SERVICE_ERROR, INVALID_RESPONSE];
const WEATHER_SERVICE_ERROR: &str = "weather-service-error";
const INVALID_RESPONSE: &str = "invalid-response";

/// Cities with known coordinates (the `city` choice options).
const CITIES: &[(&str, &str, f64, f64)] = &[
    ("berlin", "Berlin", 52.52, 13.41),
    ("london", "London", 51.51, -0.13),
    ("new-york", "New York", 40.71, -74.01),
    ("tokyo", "Tokyo", 35.68, 139.69),
];

/// The plugin definition; `deckpilot_plugin_sdk::plugin!` exports it.
pub fn plugin() -> Plugin {
    let cities: Vec<&str> = CITIES.iter().map(|c| c.0).collect();
    Plugin::new("acme.hello-weather", "0.1.0")
        .on_init(check_cities)
        .action(
            Action::new("greet", greet)
                .param(Param::text("name").required().max_length(64))
                .param(Param::choice("style", &["plain", "excited"]).default("plain")),
        )
        .action(
            Action::new("current-temperature", current_temperature)
                .param(Param::choice("city", &cities).required())
                .param(Param::choice("units", &["celsius", "fahrenheit"]).default("celsius")),
        )
}

deckpilot_plugin_sdk::plugin!(plugin);

/// Init hook: runs once when the host loads the module. No capabilities
/// here; a failure refuses the load.
fn check_cities() -> Result<(), PluginError> {
    let valid = CITIES
        .iter()
        .all(|(_, _, lat, lon)| (-90.0..=90.0).contains(lat) && (-180.0..=180.0).contains(lon));
    if valid {
        Ok(())
    } else {
        Err(PluginError::new(codes::PLUGIN_ERROR, "invalid city table"))
    }
}

/// Pure action: no capability calls at all.
fn greet(_ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let name: String = args.get("name")?;
    let name = name.trim();
    if name.is_empty() {
        return Err(PluginError::new(
            codes::INVALID_PARAMETERS,
            "parameter `name` must not be blank",
        ));
    }
    let style: String = args.get("style")?;
    let greeting = match style.as_str() {
        "excited" => format!("Hello, {name}!"),
        _ => format!("Hello, {name}."),
    };
    Ok(Output::message(greeting.clone()).with("greeting", greeting))
}

#[derive(Deserialize)]
struct Forecast {
    current: Current,
    #[serde(default)]
    current_units: Option<Units>,
}

#[derive(Deserialize)]
struct Current {
    temperature_2m: f64,
    #[serde(default)]
    time: Option<String>,
}

#[derive(Deserialize)]
struct Units {
    temperature_2m: String,
}

/// Brokered action: one GET to [`WEATHER_ORIGIN`].
fn current_temperature(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let city: String = args.get("city")?;
    let units: String = args.get("units")?;
    // The SDK has already checked `city` against the declared options.
    let Some(&(_, label, lat, lon)) = CITIES.iter().find(|c| c.0 == city) else {
        return Err(PluginError::new(
            codes::PLUGIN_ERROR,
            "city table out of sync",
        ));
    };
    let url = format!(
        "{WEATHER_ORIGIN}/v1/forecast?latitude={lat}&longitude={lon}\
         &current=temperature_2m&temperature_unit={units}"
    );
    ctx.debug(format!("fetching the temperature for {label}"));

    // A denial (permission not approved, or revoked) comes back as a
    // CapabilityError; `?` turns it into `not-permitted` with a message
    // that names the origin and the missing permission.
    let response = ctx.fetch(Request::get(url).header("Accept", "application/json"))?;

    match response.status {
        200..=299 => {}
        429 => {
            return Err(PluginError::new(
                codes::THROTTLED,
                "the weather service is rate limiting requests",
            )
            .retryable());
        }
        s @ 500..=599 => {
            return Err(PluginError::new(
                WEATHER_SERVICE_ERROR,
                format!("the weather service failed (HTTP {s})"),
            )
            .retryable());
        }
        s => {
            return Err(PluginError::new(
                WEATHER_SERVICE_ERROR,
                format!("the weather service refused the request (HTTP {s})"),
            ));
        }
    }
    let forecast: Forecast = response.json().map_err(|_| {
        PluginError::new(
            INVALID_RESPONSE,
            "the weather service sent an unexpected answer",
        )
    })?;
    if !forecast.current.temperature_2m.is_finite() {
        return Err(PluginError::new(
            INVALID_RESPONSE,
            "the temperature is not a number",
        ));
    }
    let unit = forecast
        .current_units
        .map(|u| u.temperature_2m)
        .unwrap_or_else(|| if units == "fahrenheit" { "°F" } else { "°C" }.to_owned());
    let temperature = format!("{:.1}", forecast.current.temperature_2m);
    let mut out = Output::message(format!("{label}: {temperature} {unit}"))
        .with("temperature", temperature)
        .with("unit", unit)
        .with("city", label);
    if let Some(time) = forecast.current.time {
        out = out.with("observedAt", time);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
