//! Test fixtures: wasm modules built from source by this crate's build
//! script (no committed binaries).
#![forbid(unsafe_code)]

/// `guest/test-guest` (raw ABI 1, deliberate attacks).
pub static TEST_GUEST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/test_guest.wasm"));

/// `guest/sdk-test-guest` (built with deckpilot-plugin-sdk: lifecycle,
/// secret handles, number parameters, faults).
pub static SDK_TEST_GUEST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sdk_test_guest.wasm"));

/// `plugins/examples/hello-weather`, the sample plugin.
pub static HELLO_WEATHER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/hello_weather.wasm"));

/// The sample plugin's `manifest.json`.
pub static HELLO_WEATHER_MANIFEST: &str =
    include_str!("../../../../examples/hello-weather/manifest.json");

#[cfg(test)]
mod tests {
    #[test]
    fn guests_are_wasm_modules() {
        for m in [
            super::TEST_GUEST,
            super::SDK_TEST_GUEST,
            super::HELLO_WEATHER,
        ] {
            assert_eq!(&m[..4], b"\0asm");
        }
    }
}
