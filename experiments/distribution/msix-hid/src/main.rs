//! Feasibility probe for the Microsoft Store (MSIX) HID question
//! (docs/distribution/capability-matrix.md, section 2.4): prints whether
//! the process has a package identity and every HID device hidapi
//! enumerates, with the result of opening it. run.ps1 runs it unpackaged
//! (control) and from inside a full-trust MSIX package.
//!
//!   msix-hid <label> [<report file>]
//!
//! The report goes to stdout and, when given, also to <report file> (a
//! packaged process started through its execution alias may have no
//! console the runner can read).

use std::fmt::Write as _;

use hidapi::HidApi;

#[cfg(windows)]
fn package_identity() -> String {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentPackageFullName(length: *mut u32, name: *mut u16) -> i32;
    }
    const APPMODEL_ERROR_NO_PACKAGE: i32 = 15700;
    let mut length = 0u32;
    // SAFETY: a null buffer with length 0 asks for the required length.
    let rc = unsafe { GetCurrentPackageFullName(&mut length, std::ptr::null_mut()) };
    if rc == APPMODEL_ERROR_NO_PACKAGE {
        return "none (not packaged)".into();
    }
    let mut buffer = vec![0u16; length as usize];
    // SAFETY: the buffer holds `length` UTF-16 units.
    let rc = unsafe { GetCurrentPackageFullName(&mut length, buffer.as_mut_ptr()) };
    if rc != 0 {
        return format!("error {rc}");
    }
    String::from_utf16_lossy(&buffer[..length.saturating_sub(1) as usize])
}

#[cfg(not(windows))]
fn package_identity() -> String {
    "n/a (not Windows)".into()
}

fn main() {
    let label = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "unlabelled".into());
    let mut out = String::new();
    let _ = writeln!(out, "probe = msix-hid");
    let _ = writeln!(out, "label = {label}");
    let _ = writeln!(out, "package_full_name = {}", package_identity());
    let _ = writeln!(
        out,
        "local_app_data = {}",
        std::env::var("LOCALAPPDATA").unwrap_or_default()
    );
    match HidApi::new() {
        Err(e) => {
            let _ = writeln!(out, "hid_init = failed ({e})");
        }
        Ok(api) => {
            let devices: Vec<_> = api.device_list().cloned().collect();
            let _ = writeln!(out, "hid_init = ok");
            let _ = writeln!(out, "hid_devices = {}", devices.len());
            let elgato = devices.iter().filter(|d| d.vendor_id() == 0x0fd9).count();
            let _ = writeln!(out, "elgato_devices (vendor 0x0fd9) = {elgato}");
            for (index, device) in devices.iter().enumerate() {
                let _ = writeln!(
                    out,
                    "device[{index}] vid=0x{:04x} pid=0x{:04x} usage_page=0x{:04x} usage=0x{:04x} product={:?}",
                    device.vendor_id(),
                    device.product_id(),
                    device.usage_page(),
                    device.usage(),
                    device.product_string().unwrap_or(""),
                );
                let result = match device.open_device(&api) {
                    Ok(_) => "opened".to_owned(),
                    Err(e) => format!("failed ({e})"),
                };
                let _ = writeln!(out, "device[{index}] open = {result}");
            }
        }
    }
    let _ = writeln!(out, "end");
    print!("{out}");
    if let Some(path) = std::env::args().nth(2) {
        if let Err(e) = std::fs::write(&path, &out) {
            eprintln!("cannot write {path}: {e}");
            std::process::exit(1);
        }
    }
}
