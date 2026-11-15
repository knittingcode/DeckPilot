// The accepted compile-time values, as plain data. `build.rs` includes this
// file to validate `DECKPILOT_DISTRIBUTION_CHANNEL` and
// `DECKPILOT_PACKAGE_FORMAT` with readable errors; the library's unit tests
// check it against the enums in `lib.rs`, so the two cannot drift apart.
// The library itself rejects invalid values a second time during constant
// evaluation.

/// `(distribution_channel, platforms it builds for, package formats)`.
pub const CHANNELS: &[(&str, &[&str], &[&str])] = &[
    (
        "github",
        &["macos", "windows", "linux"],
        &["app", "msi", "nsis", "deb", "rpm", "appimage"],
    ),
    ("homebrew", &["macos"], &["app"]),
    ("winget", &["windows"], &["msi", "nsis"]),
    ("apt", &["linux"], &["deb"]),
    ("flathub", &["linux"], &["flatpak"]),
    ("mac_app_store", &["macos"], &["mas_pkg"]),
    ("microsoft_store", &["windows"], &["msix"]),
];

/// `(package_format, platform)`.
pub const FORMATS: &[(&str, &str)] = &[
    ("app", "macos"),
    ("msi", "windows"),
    ("nsis", "windows"),
    ("deb", "linux"),
    ("rpm", "linux"),
    ("appimage", "linux"),
    ("flatpak", "linux"),
    ("mas_pkg", "macos"),
    ("msix", "windows"),
];

/// `(target_os, platform)`: the compilation targets a build may have.
pub const PLATFORMS: &[(&str, &str)] = &[
    ("macos", "macos"),
    ("windows", "windows"),
    ("linux", "linux"),
];

/// The compilation target architectures a build may have.
pub const ARCHITECTURES: &[&str] = &["aarch64", "x86_64"];
