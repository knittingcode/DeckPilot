//! The packaging matrix (`docs/distribution/targets.toml`, README section 8)
//! and the code agree: every target names valid enum values, its channel,
//! provider and format fit together, and the provider the code selects for
//! it is the one the matrix states.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use deckpilot_update::provider::{
    Architecture, BuildFacts, Distribution, DistributionChannel, PackageFormat, Platform,
    ProviderKind, manifest_key, may_construct_tauri_updater, require_manifest_entry,
    select_provider,
};
use toml::{Table, Value};

const VERSION: &str = "0.6.0-alpha.3";

fn docs_matrix() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/distribution/targets.toml")
}

// TODO: once the design (#234) is on main, read only the docs file and
// delete tests/fixtures/targets.toml.
fn fixture_matrix() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/targets.toml")
}

fn matrix() -> Table {
    let docs = docs_matrix();
    let path = if docs.exists() {
        docs
    } else {
        fixture_matrix()
    };
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    text.parse()
        .unwrap_or_else(|e| panic!("{} is not TOML: {e}", path.display()))
}

#[test]
fn the_fixture_is_the_docs_matrix() {
    let docs = docs_matrix();
    if docs.exists() {
        assert_eq!(
            std::fs::read_to_string(fixture_matrix()).unwrap(),
            std::fs::read_to_string(docs).unwrap(),
            "tests/fixtures/targets.toml differs from docs/distribution/targets.toml"
        );
    }
}

fn str_field<'a>(target: &'a Table, key: &str) -> &'a str {
    let id = target.get("id").and_then(Value::as_str).unwrap_or("?");
    target
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("target {id}: `{key}` is missing or not a string"))
}

fn opt_str<'a>(target: &'a Table, key: &str) -> Option<&'a str> {
    target.get(key).map(|v| {
        v.as_str()
            .unwrap_or_else(|| panic!("`{key}` is not a string"))
    })
}

fn strings<'a>(target: &'a Table, key: &str) -> Option<Vec<&'a str>> {
    target.get(key).map(|v| {
        v.as_array()
            .unwrap_or_else(|| panic!("`{key}` is not an array"))
            .iter()
            .map(|item| item.as_str().expect("an array of strings"))
            .collect()
    })
}

fn parse<T: std::str::FromStr>(id: &str, key: &str, name: &str) -> T
where
    T::Err: std::fmt::Display,
{
    name.parse()
        .unwrap_or_else(|e| panic!("target {id}: `{key}`: {e}"))
}

/// The architectures a matrix `arch` value stands for.
fn architectures(id: &str, arch: &str) -> Vec<Architecture> {
    match arch {
        // One package for both (Mac App Store), or one per architecture
        // (the Homebrew cask's two DMGs).
        "universal" | "universal_via_two_dmgs" => Architecture::ALL.to_vec(),
        other => vec![parse(id, "arch", other)],
    }
}

/// `(platform, architecture)` of a Rust target triple.
fn triple(triple: &str) -> Option<(&'static str, &'static str)> {
    Some(match triple {
        "aarch64-apple-darwin" => ("macos", "aarch64"),
        "x86_64-apple-darwin" => ("macos", "x86_64"),
        "x86_64-pc-windows-msvc" => ("windows", "x86_64"),
        "aarch64-pc-windows-msvc" => ("windows", "aarch64"),
        "x86_64-unknown-linux-gnu" => ("linux", "x86_64"),
        "aarch64-unknown-linux-gnu" => ("linux", "aarch64"),
        _ => return None,
    })
}

#[test]
fn the_channel_provider_table_matches_the_code() {
    let matrix = matrix();
    assert_eq!(matrix.get("schema").and_then(Value::as_integer), Some(1));
    let table = matrix
        .get("provider_for_channel")
        .and_then(Value::as_table)
        .expect("[provider_for_channel]");
    let listed: BTreeSet<&str> = table.keys().map(String::as_str).collect();
    let channels: BTreeSet<&str> = DistributionChannel::ALL.iter().map(|c| c.name()).collect();
    assert_eq!(listed, channels, "every channel, and only those");
    for (name, provider) in table {
        let channel: DistributionChannel = parse("provider_for_channel", name, name);
        let provider: ProviderKind =
            parse("provider_for_channel", name, provider.as_str().unwrap());
        assert_eq!(channel.update_provider(), provider, "{channel}");
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn every_target_maps_to_a_valid_provider() {
    let matrix = matrix();
    let targets = matrix
        .get("target")
        .and_then(Value::as_array)
        .expect("[[target]]");
    assert_ne!(targets.len(), 0);
    let mut ids = BTreeSet::new();
    let mut shipping_formats = BTreeSet::new();
    for target in targets {
        let target = target.as_table().expect("[[target]] is a table");
        let id = str_field(target, "id");
        assert!(ids.insert(id), "duplicate target {id}");
        let status = str_field(target, "status");
        assert!(
            ["shipping", "stable_only", "planned", "feasibility"].contains(&status),
            "{id}: status {status}"
        );

        let channel: DistributionChannel = parse(
            id,
            "distribution_channel",
            str_field(target, "distribution_channel"),
        );
        let provider: ProviderKind =
            parse(id, "update_provider", str_field(target, "update_provider"));
        let format: PackageFormat =
            parse(id, "package_format", str_field(target, "package_format"));
        let platform: Platform = parse(id, "os", str_field(target, "os"));
        assert_ne!(
            provider,
            ProviderKind::None,
            "{id}: every target is updated somehow"
        );
        assert_eq!(
            channel.update_provider(),
            provider,
            "{id}: provider follows the channel"
        );
        assert!(
            channel.allows_format(format),
            "{id}: {format} is not a {channel} format"
        );
        assert_eq!(
            format.platform(),
            platform,
            "{id}: {format} is not a {platform} format"
        );

        if let Some(rust_target) = opt_str(target, "rust_target") {
            let (os, arch) =
                triple(rust_target).unwrap_or_else(|| panic!("{id}: unknown target {rust_target}"));
            assert_eq!(os, platform.name(), "{id}: rust_target vs os");
            assert_eq!(arch, str_field(target, "arch"), "{id}: rust_target vs arch");
        }
        // A reserved dedicated channel (Homebrew, WinGet) must accept the
        // same format on the same platform once it gets its own build.
        if let Some(reserved) = opt_str(target, "reserved_channel") {
            let reserved: DistributionChannel = parse(id, "reserved_channel", reserved);
            assert!(reserved.allows_format(format), "{id}: reserved {reserved}");
            assert!(reserved.builds_for(platform), "{id}: reserved {reserved}");
        }
        if let Some(reuses) = strings(target, "reuses") {
            for other in reuses {
                assert!(ids.contains(other), "{id} reuses {other}, listed earlier");
            }
        }

        // Builds without the Tauri updater (stores) never have a GitHub
        // provider; a GitHub provider always has an updater.
        let updater_compiled = target
            .get("updater_compiled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let updater_key = target
            .get("updater_key")
            .and_then(Value::as_bool)
            .unwrap_or(provider == ProviderKind::GithubRelease);
        if provider == ProviderKind::GithubRelease {
            assert!(
                updater_compiled && updater_key,
                "{id}: a GitHub build updates itself"
            );
        } else {
            assert!(
                !updater_key,
                "{id}: only GitHub builds carry the updater key (R1)"
            );
        }
        if provider == ProviderKind::StoreManaged {
            assert!(!updater_compiled, "{id}: stores ship without the updater");
        }

        let keys = strings(target, "manifest_keys").unwrap_or_default();
        if provider != ProviderKind::GithubRelease {
            assert!(
                keys.is_empty(),
                "{id}: only GitHub builds are in the manifest"
            );
        }

        for architecture in architectures(id, str_field(target, "arch")) {
            // Channels with their own build compile the format in; a GitHub
            // build gets it from the bundler.
            let compiled = (channel != DistributionChannel::Github).then_some(format);
            let distribution =
                Distribution::new(channel, compiled, VERSION, platform, architecture)
                    .unwrap_or_else(|e| panic!("{id}: {e}"));
            let selection = select_provider(BuildFacts {
                distribution: &distribution,
                bundled_format: Some(format),
                updater_key,
            });
            assert_eq!(selection.kind, provider, "{id} ({architecture})");
            assert_eq!(selection.package_format, Some(format), "{id}");
            assert_eq!(selection.not_configured, None, "{id}");
            assert_eq!(
                may_construct_tauri_updater(&distribution, &selection),
                provider == ProviderKind::GithubRelease,
                "{id}: R1"
            );
            // R3: a target the pipeline already builds lists its exact key.
            if provider == ProviderKind::GithubRelease && opt_str(target, "rust_target").is_some() {
                let key = manifest_key(&distribution, &selection).unwrap();
                assert_eq!(
                    require_manifest_entry(&distribution, &selection, keys.iter().copied()),
                    Ok(key),
                    "{id}: manifest_keys"
                );
            }
        }
        if ["shipping", "stable_only"].contains(&status) {
            assert_eq!(channel, DistributionChannel::Github, "{id}: shipping today");
            shipping_formats.insert(format.name());
        }
    }
    // Today's GitHub release covers every format the GitHub updater installs.
    let updatable: BTreeSet<&str> = PackageFormat::ALL
        .iter()
        .filter(|f| f.updater_suffix().is_some())
        .map(|f| f.name())
        .collect();
    assert_eq!(shipping_formats, updatable);
}

#[test]
fn capability_verdicts_name_known_channels() {
    let matrix = matrix();
    let capabilities = matrix
        .get("capabilities")
        .and_then(Value::as_table)
        .expect("[capabilities.*]");
    let verdicts = ["supported", "adaptation", "blocked", "unverified"];
    for (name, row) in capabilities {
        let channel: DistributionChannel = parse("capabilities", name, name);
        let row = row.as_table().expect("a table per channel");
        for (capability, verdict) in row {
            let verdict = verdict.as_str().unwrap();
            assert!(
                verdicts.contains(&verdict),
                "{name}.{capability} = {verdict}"
            );
        }
        let self_update = row.get("self_update").and_then(Value::as_str);
        // Store builds and the APT variant never update themselves (R2).
        // Homebrew and WinGet ship the GitHub build in phase 1, so their
        // verdict describes that build, not the reserved channel.
        if channel.update_provider() == ProviderKind::StoreManaged
            || channel == DistributionChannel::Apt
        {
            assert_eq!(self_update, Some("blocked"), "{name}");
        }
        if channel == DistributionChannel::Github {
            assert_eq!(self_update, Some("supported"));
        }
    }
}
