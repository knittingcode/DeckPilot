//! The registry's source data: publishers, plugins, version history and
//! revocations (`registry/`), and the release assets (`releases/`).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use deckpilot_plugin_manifest::{
    FIRST_PARTY_PUBLISHER, Manifest, PackageDigest, PluginId, RESERVED_PUBLISHERS, is_publisher_id,
};
use deckpilot_plugin_package::{
    ExpectedPackage, Limits, MANIFEST, read_archive, sha256, verify_package,
};
use semver::Version;
use serde::{Deserialize, Serialize};

use super::keyset;
use super::layout::{
    Workspace, read, read_json, read_optional, write, write_immutable, write_json,
};
use super::{PublishError, Result, fail};
use crate::model::{
    Deprecated, DetailType, DetailVersion, IndexPublisher, PackageRef, PluginDetail, Revocation,
    RevocationAction, RevokedVersions, SPEC_VERSION, VersionSource, Yanked, release_prefix,
};

/// `registry/plugins/<id>/plugin.json`: what the index lists and the
/// detail describes, maintained by the publisher (`--plugin-detail`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginInfo {
    pub id: PluginId,
    pub name: String,
    /// The index's one-line summary (up to 300 characters).
    pub summary: String,
    pub description: String,
    pub license: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<Deprecated>,
}

/// One plugin as stored: metadata and versions, ascending.
#[derive(Debug, Clone)]
pub(crate) struct StoredPlugin {
    pub info: PluginInfo,
    pub versions: Vec<DetailVersion>,
}

impl StoredPlugin {
    /// The detail document (`plugins/<id>/<sha256>.json`), validated.
    pub fn detail(&self) -> Result<PluginDetail> {
        let detail = PluginDetail {
            kind: DetailType::Value,
            spec_version: SPEC_VERSION,
            id: self.info.id.clone(),
            publisher: self.info.id.publisher().to_owned(),
            name: self.info.name.clone(),
            description: self.info.description.clone(),
            license: self.info.license.clone(),
            homepage: self.info.homepage.clone(),
            source_repository: self.info.source_repository.clone(),
            versions: self.versions.clone(),
        };
        detail
            .validate()
            .map_err(|e| PublishError(format!("{}: {e}", self.info.id)))?;
        Ok(detail)
    }
}

pub(crate) fn publishers(ws: &Workspace) -> Result<Vec<IndexPublisher>> {
    let mut list: Vec<IndexPublisher> = Vec::new();
    for path in json_files(&ws.publishers_dir())? {
        list.push(read_json(&path)?);
    }
    list.sort_by(|a, b| a.namespace.cmp(&b.namespace));
    Ok(list)
}

fn json_files(dir: &std::path::Path) -> Result<Vec<std::path::PathBuf>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return fail(format!("{}: {e}", dir.display())),
    };
    let mut files = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|e| PublishError(format!("{}: {e}", dir.display())))?
            .path();
        if path.extension().is_some_and(|x| x == "json") && path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// Every stored plugin, by id.
pub(crate) fn plugins(ws: &Workspace) -> Result<BTreeMap<PluginId, StoredPlugin>> {
    let mut plugins = BTreeMap::new();
    let dir = ws.plugins_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(plugins),
        Err(e) => return fail(format!("{}: {e}", dir.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|e| PublishError(format!("{}: {e}", dir.display())))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let id = PluginId::parse(name)
            .map_err(|e| PublishError(format!("{}: {e}", entry.path().display())))?;
        let plugin = stored_plugin(ws, &id)?
            .ok_or_else(|| PublishError(format!("{id} has no plugin.json")))?;
        plugins.insert(id, plugin);
    }
    Ok(plugins)
}

fn stored_plugin(ws: &Workspace, id: &PluginId) -> Result<Option<StoredPlugin>> {
    let path = ws.plugin_info_path(id);
    let Some(bytes) = read_optional(&path)? else {
        return Ok(None);
    };
    let info: PluginInfo = super::layout::parse(&path, &bytes)?;
    if info.id != *id {
        return fail(format!("{} names {}", path.display(), info.id));
    }
    let mut versions: Vec<DetailVersion> = Vec::new();
    for file in json_files(&ws.versions_dir(id))? {
        let version: DetailVersion = read_json(&file)?;
        if file.file_stem().and_then(|s| s.to_str()) != Some(version.version.to_string().as_str()) {
            return fail(format!(
                "{} holds version {}",
                file.display(),
                version.version
            ));
        }
        versions.push(version);
    }
    versions.sort_by(|a, b| a.version.cmp(&b.version));
    Ok(Some(StoredPlugin { info, versions }))
}

pub(crate) fn revocations(ws: &Workspace) -> Result<Vec<Revocation>> {
    match read_optional(&ws.revocations_path())? {
        Some(bytes) => super::layout::parse(&ws.revocations_path(), &bytes),
        None => Ok(Vec::new()),
    }
}

/// `publisher`: adds or updates `registry/publishers/<login>.json`.
///
/// A reserved namespace is refused (no client installs its plugins), except
/// DeckPilot's own `deckpilot` with `first_party` (`--first-party`).
pub(crate) fn publisher(ws: &Workspace, entry: &IndexPublisher, first_party: bool) -> Result<()> {
    if !is_publisher_id(&entry.namespace) {
        return fail(format!(
            "`{}` is not a publisher namespace",
            entry.namespace
        ));
    }
    let is_first_party = entry.namespace == FIRST_PARTY_PUBLISHER;
    if first_party && !is_first_party {
        return fail(format!(
            "--first-party is for the namespace {FIRST_PARTY_PUBLISHER}, not {}",
            entry.namespace
        ));
    }
    if is_first_party && !first_party {
        return fail(format!(
            "the namespace {FIRST_PARTY_PUBLISHER} is DeckPilot's own: give --first-party"
        ));
    }
    if !is_first_party && RESERVED_PUBLISHERS.contains(&entry.namespace.as_str()) {
        return fail(format!(
            "the namespace {} is reserved for DeckPilot; no client installs its plugins",
            entry.namespace
        ));
    }
    if entry.github_id == 0 {
        return fail("--github-id must be at least 1");
    }
    if !(1..=39).contains(&entry.login.len())
        || !entry
            .login
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return fail(format!("`{}` is not a GitHub login", entry.login));
    }
    if let Some(other) = publishers(ws)?
        .into_iter()
        .find(|p| p.namespace == entry.namespace && p.login != entry.login)
    {
        return fail(format!(
            "namespace {} already belongs to {}",
            entry.namespace, other.login
        ));
    }
    write_json(
        &ws.publishers_dir().join(format!("{}.json", entry.login)),
        entry,
    )
}

/// Inputs of `add-version` that the package does not carry.
#[derive(Debug, Clone)]
pub(crate) struct NewVersion {
    pub source: VersionSource,
    pub builder: Version,
    pub attestation: Option<String>,
    pub published_at: DateTime<Utc>,
}

/// `add-version`: verifies the package with `deckpilot-plugin-package`,
/// records the version (sha256, size, manifest digest, permissions and
/// engines taken from the package itself) and stores the release asset.
/// Returns the new entry. Re-adding the identical package is a no-op.
///
/// A `deckpilot.*` package needs `first_party` (`--first-party`), a current
/// key set that authorizes the `deckpilot` namespace and a verified
/// `deckpilot` publisher: what a client requires before it installs one.
#[allow(clippy::too_many_lines)] // verification, then one record
pub(crate) fn add_version(
    ws: &Workspace,
    package: &[u8],
    info: Option<PluginInfo>,
    new: NewVersion,
    first_party: bool,
) -> Result<(DetailVersion, bool)> {
    let limits = Limits::default();
    let files = read_archive(package, &limits)
        .map_err(|e| PublishError(format!("the package is invalid: {e}")))?;
    let manifest_bytes = files
        .get(MANIFEST)
        .ok_or_else(|| PublishError("the package has no manifest.json".into()))?;
    let manifest = Manifest::from_slice(manifest_bytes)
        .map_err(|e| PublishError(format!("manifest.json: {e}")))?;
    let raw: serde_json::Value = serde_json::from_slice(manifest_bytes)
        .map_err(|e| PublishError(format!("manifest.json: {e}")))?;
    check_first_party(ws, &manifest.id, first_party)?;
    let permissions = match raw.get("permissions") {
        Some(serde_json::Value::Array(list)) => list.clone(),
        Some(_) => return fail("manifest.json: permissions is not an array"),
        None => Vec::new(),
    };
    let digest = PackageDigest {
        sha256: sha256(package),
        size: u64::try_from(package.len()).unwrap_or(u64::MAX),
    };
    // The same verification an installer runs, against the values recorded.
    let expected = ExpectedPackage {
        id: manifest.id.clone(),
        version: manifest.version.clone(),
        package: digest.clone(),
        manifest_sha256: sha256(manifest_bytes),
        permissions: Some(permissions.clone()),
        first_party,
    };
    let verified = verify_package(package, &expected, &limits)
        .map_err(|e| PublishError(format!("the package is invalid: {e}")))?;
    let manifest = verified.manifest();
    let id = manifest.id.clone();

    let namespace = id.publisher();
    if !publishers(ws)?.iter().any(|p| p.namespace == namespace) {
        return fail(format!(
            "no publisher owns the namespace {namespace}; add one with `publisher`"
        ));
    }
    let stored = stored_plugin(ws, &id)?;
    let info = match (info, &stored) {
        (Some(info), _) => {
            if info.id != id {
                return fail(format!(
                    "--plugin-detail is for {}, the package is {id}",
                    info.id
                ));
            }
            info
        }
        (None, Some(stored)) => stored.info.clone(),
        (None, None) => {
            return fail(format!(
                "{id} is new: give its listing with --plugin-detail <plugin.json>"
            ));
        }
    };
    let mut versions = stored.map(|s| s.versions).unwrap_or_default();
    if let Some(existing) = versions.iter().find(|v| v.version == manifest.version) {
        if existing.package.sha256 == digest.sha256 && existing.package.size == digest.size {
            return Ok((existing.clone(), false));
        }
        return fail(format!(
            "{id} {} is already published with another package (versions are immutable)",
            manifest.version
        ));
    }
    if let Some(last) = versions.last()
        && last.version >= manifest.version
    {
        return fail(format!(
            "{id} {} is not greater than the latest published {} (versions are append-only)",
            manifest.version, last.version
        ));
    }
    let url = format!(
        "{}{id}-{}.dpplugin",
        release_prefix(&id, &manifest.version),
        manifest.version
    );
    let entry = DetailVersion {
        version: manifest.version.clone(),
        published_at: new.published_at,
        engines: manifest.engines.clone(),
        platforms: manifest.platforms.clone(),
        permissions,
        dependencies: manifest.dependencies.clone(),
        manifest_sha256: verified.integrity().manifest_sha256,
        source: new.source,
        package: PackageRef {
            url: url.clone(),
            sha256: digest.sha256,
            size: digest.size,
        },
        builder: new.builder,
        attestation: new.attestation,
        yanked: None,
    };
    versions.push(entry.clone());
    let plugin = StoredPlugin {
        info: info.clone(),
        versions,
    };
    let detail = plugin.detail()?;
    let detail_len = super::to_json(&detail)?.len();
    if u64::try_from(detail_len).unwrap_or(u64::MAX) > crate::MAX_DETAIL_BYTES {
        return fail(format!("the detail of {id} would exceed 256 KiB"));
    }
    let release = ws
        .release_file(&url)
        .ok_or_else(|| PublishError(format!("{url} is not a release asset URL")))?;
    write_immutable(&release, package)?;
    write_json(&ws.plugin_info_path(&id), &info)?;
    write_json(&ws.version_path(&id, &entry.version), &entry)?;
    Ok((entry, true))
}

/// `add-version`'s first-party rules ([`add_version`]).
fn check_first_party(ws: &Workspace, id: &PluginId, first_party: bool) -> Result<()> {
    let is_first_party = id.publisher() == FIRST_PARTY_PUBLISHER;
    if first_party && !is_first_party {
        return fail(format!(
            "--first-party is for {FIRST_PARTY_PUBLISHER}.* packages, not {id}"
        ));
    }
    if !is_first_party {
        return Ok(());
    }
    if !first_party {
        return fail(format!(
            "{id} is a DeckPilot first-party plugin: add it with --first-party"
        ));
    }
    if !keyset::current(ws)?.authorizes_first_party() {
        return fail(format!(
            "the current key set does not authorize the namespace {FIRST_PARTY_PUBLISHER} \
             (`keyset rotate --first-party`, signed by the roots); clients would refuse {id}"
        ));
    }
    if !publishers(ws)?
        .iter()
        .any(|p| p.namespace == FIRST_PARTY_PUBLISHER && p.verified)
    {
        return fail(format!(
            "no verified publisher owns {FIRST_PARTY_PUBLISHER} (`publisher --namespace \
             {FIRST_PARTY_PUBLISHER} --verified --first-party`); clients would refuse {id}"
        ));
    }
    Ok(())
}

fn check_reason(reason: &str) -> Result<()> {
    let count = reason.chars().count();
    if count == 0 || count > 500 {
        return fail("--reason needs 1 to 500 characters");
    }
    Ok(())
}

fn check_advisory(advisory: Option<&String>) -> Result<()> {
    if let Some(advisory) = advisory
        && (!advisory.starts_with("https://") || advisory.contains(char::is_whitespace))
    {
        return fail("--advisory must be an https URL");
    }
    Ok(())
}

/// `revoke`: adds a revocation (or updates the reason, advisory and date
/// of one with the same plugin, versions and action). Nothing here can
/// remove or narrow a revocation.
pub(crate) fn revoke(ws: &Workspace, revocation: Revocation) -> Result<String> {
    check_reason(&revocation.reason)?;
    check_advisory(revocation.advisory.as_ref())?;
    let stored = stored_plugin(ws, &revocation.id)?
        .ok_or_else(|| PublishError(format!("{} is not in the registry", revocation.id)))?;
    if let RevokedVersions::List(list) = &revocation.versions {
        if list.is_empty() {
            return fail("--versions is empty");
        }
        for version in list {
            if !stored.versions.iter().any(|v| v.version == *version) {
                return fail(format!("{} {version} was never published", revocation.id));
            }
        }
    }
    let mut list = revocations(ws)?;
    let message = if let Some(existing) = list.iter_mut().find(|r| {
        r.id == revocation.id && r.versions == revocation.versions && r.action == revocation.action
    }) {
        existing.reason.clone_from(&revocation.reason);
        existing.advisory.clone_from(&revocation.advisory);
        existing.at = revocation.at;
        "updated"
    } else {
        list.push(revocation);
        "added"
    };
    write_json(&ws.revocations_path(), &list)?;
    Ok(message.to_owned())
}

/// `yank`: marks the version withdrawn in its detail (no longer offered)
/// and adds a `yank` revocation (the notice installed copies show).
pub(crate) fn yank(
    ws: &Workspace,
    id: &PluginId,
    version: &Version,
    reason: String,
    advisory: Option<String>,
    at: DateTime<Utc>,
) -> Result<()> {
    check_reason(&reason)?;
    let path = ws.version_path(id, version);
    if !path.is_file() {
        return fail(format!("{id} {version} was never published"));
    }
    let mut entry: DetailVersion = read_json(&path)?;
    if entry.yanked.is_none() {
        entry.yanked = Some(Yanked {
            at,
            reason: reason.clone(),
        });
        write(&path, &super::to_json(&entry)?)?;
    }
    revoke(
        ws,
        Revocation {
            id: id.clone(),
            versions: RevokedVersions::List(vec![version.clone()]),
            action: RevocationAction::Yank,
            reason,
            advisory,
            at,
        },
    )?;
    Ok(())
}

/// Whether the next index may drop revocations: only after a root-signed
/// key set with `seq_floor_reset` and before any index was published under
/// it (the client allows it exactly once, section 9.4 recovery).
pub(crate) fn lifting_allowed(ws: &Workspace) -> Result<bool> {
    let newest = ws.newest_keyset();
    let chain = keyset::verified_chain(ws, newest)?;
    let last_index_keys = match ws.index_seqs()?.last() {
        Some(seq) => {
            let path = ws.index_path(*seq);
            let index: crate::Index = read_json(&path)?;
            index.keys_version
        }
        None => 0,
    };
    Ok(chain
        .iter()
        .any(|k| k.version > last_index_keys && k.seq_floor_reset))
}

/// `lift-revocation`: removes the revocations of `id` (all, or those
/// naming exactly `versions`). Refused unless [`lifting_allowed`].
pub(crate) fn lift(
    ws: &Workspace,
    id: &PluginId,
    versions: Option<&RevokedVersions>,
) -> Result<usize> {
    if !lifting_allowed(ws)? {
        return fail(
            "revocations are cumulative: lifting one needs a root-signed key set with \
             seq_floor_reset (`keyset rotate --seq-floor-reset`) and no index published \
             under it yet",
        );
    }
    let mut list = revocations(ws)?;
    let before = list.len();
    list.retain(|r| !(r.id == *id && versions.is_none_or(|v| r.versions == *v)));
    let removed = before - list.len();
    if removed == 0 {
        return fail(format!("no matching revocation of {id}"));
    }
    write_json(&ws.revocations_path(), &list)?;
    Ok(removed)
}

/// Reads a package file for `add-version`.
pub(crate) fn read_package(path: &std::path::Path) -> Result<Vec<u8>> {
    read(path)
}
