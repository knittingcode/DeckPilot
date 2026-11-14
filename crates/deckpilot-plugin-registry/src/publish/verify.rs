//! `verify-site`: the generated site checked end to end by the CLIENT's
//! verifier ([`RegistryClient`]), as a CI gate before deploy (and, with a
//! stored state, as an existing client would see the update).

use std::path::Path;

use chrono::{DateTime, TimeDelta, Utc};
use deckpilot_plugin_package::{Limits, verify_package};

use super::layout::{SiteSource, Workspace, read, read_optional};
use super::{PublishError, Result, fail};
use crate::model::{RevocationAction, Role};
use crate::{
    FileTrustStore, Freshness, MemoryTrustStore, Policy, Refresh, RegistryClient, TrustStore,
};

/// What `verify-site` checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteReport {
    pub keys_version: u64,
    pub seq: u64,
    pub updated: bool,
    pub expires: DateTime<Utc>,
    pub plugins: usize,
    pub installable_versions: usize,
    pub packages_verified: usize,
    pub revocations: usize,
}

/// Verifies `site` as a client trusting `roots` would, from `store` (a
/// fresh client unless a stored state is given), at `now`:
///
/// * the key set chain, the timestamp and the index (`refresh`);
/// * fresh, with at least `min_remaining` before the timestamp expires;
/// * every listed plugin's detail against the index;
/// * every version a client would install (`resolve_install`), and, when
///   its release asset is in `releases`, the package itself with
///   `verify_package` against the pins.
#[allow(clippy::too_many_lines)] // one pass over every document
pub fn verify_site<S: TrustStore>(
    ws: &Workspace,
    roots: Role,
    store: S,
    now: DateTime<Utc>,
    min_remaining: TimeDelta,
    require_packages: bool,
) -> Result<SiteReport> {
    let site = ws.site();
    let source = SiteSource::new(&site);
    let mut client = RegistryClient::open(roots, store, Policy::default())
        .map_err(|e| PublishError(format!("client: {e}")))?;
    let refresh = client
        .refresh(&source, now)
        .map_err(|e| PublishError(format!("the client refuses the site: {e}")))?;
    let updated = match refresh {
        Refresh::Updated { .. } => true,
        Refresh::Unchanged { .. } => false,
        Refresh::Offline(_) => return fail("the site could not be read"),
    };
    if client.freshness(now) != Freshness::Fresh {
        return fail(format!(
            "the client considers the site {:?}",
            client.freshness(now)
        ));
    }
    let timestamp = client
        .state()
        .timestamp
        .clone()
        .ok_or_else(|| PublishError("no timestamp stored".into()))?;
    if timestamp.expires - now < min_remaining {
        return fail(format!(
            "timestamp.json expires at {}, sooner than required; run `timestamp`",
            timestamp.expires
        ));
    }
    let index = client
        .index()
        .cloned()
        .ok_or_else(|| PublishError("no index stored".into()))?;
    let limits = Limits::default();
    let mut installable = 0;
    let mut packages = 0;
    for entry in &index.plugins {
        let detail_bytes = read(&site.join(&entry.detail.path))?;
        let detail = client
            .verify_detail(&entry.id, &detail_bytes)
            .map_err(|e| PublishError(format!("the client refuses {}: {e}", entry.detail.path)))?;
        for version in &detail.versions {
            let revoked = index
                .revocations
                .iter()
                .any(|r| r.matches(&entry.id, &version.version));
            if revoked || version.yanked.is_some() {
                let refused = client
                    .resolve_install(&entry.id, &version.version, &detail_bytes, None, now)
                    .is_err();
                if !refused {
                    return fail(format!(
                        "{} {} is withdrawn but a client would install it",
                        entry.id, version.version
                    ));
                }
                continue;
            }
            let target = client
                .resolve_install(&entry.id, &version.version, &detail_bytes, None, now)
                .map_err(|e| PublishError(format!("{} {}: {e}", entry.id, version.version)))?;
            installable += 1;
            let file = ws.release_file(&target.url);
            match file.as_deref().map(read_optional).transpose()?.flatten() {
                Some(bytes) => {
                    verify_package(&bytes, &target.expected, &limits).map_err(|e| {
                        PublishError(format!(
                            "{} {}: the release asset fails verification: {e}",
                            entry.id, version.version
                        ))
                    })?;
                    packages += 1;
                }
                None if require_packages => {
                    return fail(format!("the release asset {} is missing", target.url));
                }
                None => {}
            }
        }
    }
    for revocation in &index.revocations {
        let disabled = revocation.action == RevocationAction::Disable;
        if let crate::RevokedVersions::List(list) = &revocation.versions {
            for version in list {
                let verdict = client.installed_verdict(&revocation.id, version);
                let enforced = match verdict {
                    crate::InstalledVerdict::Disable { .. } => true,
                    crate::InstalledVerdict::RunWithNotice { .. } => !disabled,
                    crate::InstalledVerdict::Run => false,
                };
                if !enforced {
                    return fail(format!(
                        "the revocation of {} {version} is not enforced",
                        revocation.id
                    ));
                }
            }
        }
    }
    Ok(SiteReport {
        keys_version: client.keys_version(),
        seq: index.seq,
        updated,
        expires: timestamp.expires,
        plugins: index.plugins.len(),
        installable_versions: installable,
        packages_verified: packages,
        revocations: index.revocations.len(),
    })
}

/// `verify-site` from the command line: roots from `roots` (or
/// `roots.json`), a fresh client or the state kept in `state_dir`.
pub(crate) fn run(
    ws: &Workspace,
    roots: Option<&Path>,
    state_dir: Option<&Path>,
    now: DateTime<Utc>,
    min_remaining: TimeDelta,
    require_packages: bool,
) -> Result<SiteReport> {
    let roots: Role = match roots {
        Some(path) => super::layout::read_json(path)?,
        None => super::keyset::roots(ws)?,
    };
    match state_dir {
        Some(dir) => verify_site(
            ws,
            roots,
            FileTrustStore::new(dir),
            now,
            min_remaining,
            require_packages,
        ),
        None => verify_site(
            ws,
            roots,
            MemoryTrustStore::default(),
            now,
            min_remaining,
            require_packages,
        ),
    }
}
