//! Feasibility probe for the Mac App Store HID question
//! (docs/distribution/capability-matrix.md, section 2.4).
//!
//! Prints a plain-text report: whether the process is in the App Sandbox,
//! the Input Monitoring (ListenEvent) access state, and every HID device
//! hidapi enumerates, with the result of opening it exclusively (as the
//! DeckPilot agent does through elgato-streamdeck) and shared. run.sh runs it
//! unsandboxed and sandboxed with different entitlements and keeps the
//! reports under results/.

use std::ffi::CStr;
use std::fmt::Write as _;

use hidapi::HidApi;

const ELGATO_VENDOR: u16 = 0x0fd9;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    /// `IOHIDAccessType IOHIDCheckAccess(IOHIDRequestType)`, macOS 10.15+.
    fn IOHIDCheckAccess(request: u32) -> u32;
}

/// `kIOHIDRequestTypeListenEvent`: the Input Monitoring permission.
const LISTEN_EVENT: u32 = 1;

fn access_name(value: u32) -> &'static str {
    match value {
        0 => "granted",
        1 => "denied",
        2 => "unknown (never asked)",
        _ => "unexpected value",
    }
}

/// The user's real home directory from the password database: inside the
/// sandbox `HOME` points into the container.
fn real_home() -> Option<String> {
    // SAFETY: getpwuid returns a pointer into static storage or null; it is
    // read at once, on one thread.
    unsafe {
        let entry = libc::getpwuid(libc::getuid());
        if entry.is_null() || (*entry).pw_dir.is_null() {
            return None;
        }
        Some(
            CStr::from_ptr((*entry).pw_dir)
                .to_string_lossy()
                .into_owned(),
        )
    }
}

fn sandbox_section(out: &mut String) {
    let container = std::env::var("APP_SANDBOX_CONTAINER_ID").ok();
    let _ = writeln!(
        out,
        "app_sandbox_container_id = {}",
        container.as_deref().unwrap_or("(not set)")
    );
    let _ = writeln!(out, "home = {}", std::env::var("HOME").unwrap_or_default());
    // A control that the sandbox is enforced: the real ~/Library/Preferences
    // is outside the container. (Not ~/Documents: outside the sandbox that
    // one is TCC-protected and blocks on a consent prompt.)
    if let Some(home) = real_home() {
        let preferences = format!("{home}/Library/Preferences");
        let result = match std::fs::read_dir(&preferences) {
            Ok(_) => "readable".to_owned(),
            Err(e) => format!("denied ({e})"),
        };
        let _ = writeln!(out, "read_dir(~/Library/Preferences) = {result}");
    }
    // SAFETY: plain call without arguments.
    let access = unsafe { IOHIDCheckAccess(LISTEN_EVENT) };
    let _ = writeln!(
        out,
        "input_monitoring (IOHIDCheckAccess ListenEvent) = {access} {}",
        access_name(access)
    );
}

fn hid_section(out: &mut String) {
    let api = match HidApi::new() {
        Ok(api) => api,
        Err(e) => {
            let _ = writeln!(out, "hid_init = failed ({e})");
            return;
        }
    };
    let devices: Vec<_> = api.device_list().cloned().collect();
    let _ = writeln!(out, "hid_init = ok");
    let _ = writeln!(out, "hid_devices = {}", devices.len());
    let stream_decks = devices
        .iter()
        .filter(|d| d.vendor_id() == ELGATO_VENDOR)
        .count();
    let _ = writeln!(out, "elgato_devices (vendor 0x0fd9) = {stream_decks}");
    for (index, device) in devices.iter().enumerate() {
        let _ = writeln!(
            out,
            "device[{index}] vid=0x{:04x} pid=0x{:04x} usage_page=0x{:04x} usage=0x{:04x} interface={} product={:?} manufacturer={:?}",
            device.vendor_id(),
            device.product_id(),
            device.usage_page(),
            device.usage(),
            device.interface_number(),
            device.product_string().unwrap_or(""),
            device.manufacturer_string().unwrap_or(""),
        );
        for (mode, exclusive) in [("exclusive", true), ("shared", false)] {
            api.set_open_exclusive(exclusive);
            let path: &CStr = device.path();
            let result = match api.open_path(path) {
                Ok(handle) => {
                    // A read with a zero timeout proves the handle works
                    // without waiting for input.
                    let mut buffer = [0u8; 64];
                    match handle.read_timeout(&mut buffer, 0) {
                        Ok(n) => format!("opened, read_timeout(0) = {n} bytes"),
                        Err(e) => format!("opened, read_timeout(0) failed ({e})"),
                    }
                }
                Err(e) => format!("failed ({e})"),
            };
            let _ = writeln!(out, "device[{index}] open {mode} = {result}");
        }
    }
    api.set_open_exclusive(true);
}

fn main() {
    let mut out = String::new();
    let label = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "unlabelled".into());
    let _ = writeln!(out, "probe = macos-sandbox-hid");
    let _ = writeln!(out, "label = {label}");
    let _ = writeln!(out, "pid = {}", std::process::id());
    sandbox_section(&mut out);
    hid_section(&mut out);
    // run.sh checks for this marker: a report without it was cut short.
    let _ = writeln!(out, "end");
    print!("{out}");
}
