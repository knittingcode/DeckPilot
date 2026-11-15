//! The release pipeline writes the package URLs of every update manifest
//! with the download prefix pinned in `tools/release-pipeline/common.sh`.
//! Installed builds accept only URLs under `DOWNLOAD_PREFIX` (compared
//! case-sensitively up to 0.6.0-alpha.3), so the two must stay equal byte
//! for byte, even though the repository is now named
//! `knittingcode/deckpilot`.

use std::path::Path;

fn pinned_prefix() -> String {
    let common =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/release-pipeline/common.sh");
    let text = std::fs::read_to_string(&common)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", common.display()));
    let values: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("DOWNLOAD_PREFIX=\""))
        .map(|rest| {
            rest.strip_suffix('"')
                .expect("DOWNLOAD_PREFIX is a quoted literal")
        })
        .collect();
    assert_eq!(
        values.len(),
        1,
        "common.sh sets DOWNLOAD_PREFIX exactly once: {values:?}"
    );
    values[0].to_owned()
}

#[test]
fn pipeline_prefix_is_the_clients_download_prefix() {
    let pinned = pinned_prefix();
    assert!(
        !pinned.contains('$'),
        "the pinned prefix must be a literal, not derived from the repository name: {pinned}"
    );
    assert_eq!(pinned, deckpilot_update::DOWNLOAD_PREFIX);
}

#[test]
fn client_constants_keep_the_spelling_installed_builds_use() {
    // Byte-identical to the published builds; see lib.rs.
    assert_eq!(deckpilot_update::REPOSITORY, "knittingcode/DeckPilot");
    assert_eq!(
        deckpilot_update::DOWNLOAD_PREFIX,
        "https://github.com/knittingcode/DeckPilot/releases/download/"
    );
    assert_eq!(
        deckpilot_update::STABLE_ENDPOINT,
        "https://github.com/knittingcode/DeckPilot/releases/latest/download/latest.json"
    );
    assert_eq!(
        deckpilot_update::RELEASES_API,
        "https://api.github.com/repos/knittingcode/DeckPilot/releases?per_page=30"
    );
}

#[test]
fn tauri_updater_endpoint_is_the_stable_endpoint() {
    let conf =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/src-tauri/tauri.conf.json");
    let text = std::fs::read_to_string(&conf)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", conf.display()));
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    let endpoints = &json["plugins"]["updater"]["endpoints"];
    assert_eq!(
        endpoints,
        &serde_json::json!([deckpilot_update::STABLE_ENDPOINT]),
        "tauri.conf.json's updater endpoint"
    );
}
