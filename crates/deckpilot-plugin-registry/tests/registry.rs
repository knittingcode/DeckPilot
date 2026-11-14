//! Registry trust: key set chain, timestamp, index, detail, revocations,
//! persisted state and offline behaviour.

mod common;

use common::*;
use deckpilot_plugin_registry::sig::{SigError, index_comment, keys_comment, timestamp_comment};
use deckpilot_plugin_registry::*;

fn setup() -> Registry {
    let mut registry = Registry::new();
    registry.publish(t0(), &[plugin("acme.clock", &["1.0.0"])], &[]);
    registry
}

// ------------------------------------------------------------------ basics

#[test]
fn valid_index_is_accepted_and_persisted() {
    let registry = setup();
    let dir = tempfile::tempdir().unwrap();
    let mut c = file_client(&registry, dir.path());
    assert_eq!(c.freshness(t0()), Freshness::NeverVerified);
    assert_eq!(
        c.refresh(&registry.site, t0()),
        Ok(Refresh::Updated { seq: 1 })
    );
    assert_eq!(c.freshness(t0()), Freshness::Fresh);
    assert_eq!(
        c.refresh(&registry.site, t0()),
        Ok(Refresh::Unchanged { seq: 1 })
    );
    // Next run: state and index cache come back from the files.
    let again = file_client(&registry, dir.path());
    assert_eq!(again.state(), c.state());
    assert_eq!(again.index(), c.index());
    assert_eq!(again.keys_version(), 1);
    assert_eq!(again.freshness(t0()), Freshness::Fresh);
    let size = std::fs::metadata(dir.path().join(FileTrustStore::STATE_FILE))
        .unwrap()
        .len();
    assert!(size < 16 * 1024, "state file is {size} bytes");
    // Nothing but the two files (atomic writes leave no temporaries).
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn tampered_index_cache_is_not_trusted() {
    let mut registry = Registry::new();
    registry.publish(
        t0(),
        &[plugin("acme.clock", &["1.0.0"])],
        &[revocation("acme.clock", None, RevocationAction::Disable)],
    );
    let dir = tempfile::tempdir().unwrap();
    file_client(&registry, dir.path())
        .refresh(&registry.site, t0())
        .unwrap();
    let cache = dir.path().join(FileTrustStore::INDEX_FILE);
    let mut bytes = std::fs::read(&cache).unwrap();
    bytes[10] ^= 1;
    std::fs::write(&cache, bytes).unwrap();
    // Offline: no index to browse or install from, but the revocations (in
    // the trust state) are still enforced.
    registry.site.offline.set(true);
    let mut c = file_client(&registry, dir.path());
    assert!(c.index().is_none());
    assert_eq!(
        c.refresh(&registry.site, t0()),
        Ok(Refresh::Offline(Freshness::Stale(
            StaleReason::IndexUnavailable
        )))
    );
    assert!(matches!(
        c.installed_verdict(&id("acme.clock"), &v("1.0.0")),
        InstalledVerdict::Disable { .. }
    ));
    assert_eq!(
        c.can_install(t0()).unwrap_err(),
        InstallError::Stale(StaleReason::IndexUnavailable)
    );
    // Online again: fetched and verified again.
    registry.site.offline.set(false);
    assert_eq!(
        c.refresh(&registry.site, t0()),
        Ok(Refresh::Updated { seq: 1 })
    );
    assert!(file_client(&registry, dir.path()).index().is_some());
}

#[test]
fn new_publishes_and_daily_timestamps_are_followed() {
    let mut registry = setup();
    let mut c = client(&registry);
    c.refresh(&registry.site, t0()).unwrap();
    let t1 = t0() + days(3);
    registry.publish(t1, &[plugin("acme.clock", &["1.0.0", "1.1.0"])], &[]);
    assert_eq!(
        c.refresh(&registry.site, t1),
        Ok(Refresh::Updated { seq: 2 })
    );
    for day in 4..120 {
        let t = t0() + days(day);
        registry.sign_timestamp(t);
        assert_eq!(
            c.refresh(&registry.site, t),
            Ok(Refresh::Unchanged { seq: 2 }),
            "day {day}"
        );
        assert_eq!(c.freshness(t), Freshness::Fresh, "day {day}");
    }
}

#[test]
fn missing_key_set_is_rejected() {
    let registry = setup();
    registry.site.remove("keys/1.json");
    let mut c = client(&registry);
    assert_eq!(
        c.refresh(&registry.site, t0()),
        Err(RegistryError::NoKeyset)
    );
    assert_eq!(
        c.verify_timestamp(b"{}", b"", t0()).unwrap_err(),
        RegistryError::NoKeyset
    );
}

#[test]
fn oversized_documents_are_rejected_before_parsing() {
    let registry = setup();
    let mut c = client(&registry);
    let big = vec![b' '; MAX_KEYSET_BYTES + 1];
    assert!(matches!(
        c.apply_keyset(&big, b"[]"),
        Err(RegistryError::TooLarge { .. })
    ));
    let many: Vec<String> = vec!["x".into(); MAX_KEYSET_SIGNATURES + 1];
    assert!(matches!(
        c.apply_keyset(
            &registry.site.get("keys/1.json"),
            &serde_json::to_vec(&many).unwrap()
        ),
        Err(RegistryError::TooLarge { .. })
    ));
    c.refresh(&registry.site, t0()).unwrap();
    let mut big = registry.site.get("timestamp.json");
    big.extend(std::iter::repeat_n(b' ', MAX_TIMESTAMP_BYTES));
    assert!(matches!(
        c.verify_timestamp(&big, &registry.site.get("timestamp.json.minisig"), t0()),
        Err(RegistryError::TooLarge { .. })
    ));
    let big_signature = vec![b'A'; MAX_SIGNATURE_BYTES + 1];
    assert!(matches!(
        c.verify_timestamp(&registry.site.get("timestamp.json"), &big_signature, t0()),
        Err(RegistryError::TooLarge { .. })
    ));
}

#[test]
fn a_corrupt_state_is_an_error_not_a_reset() {
    let registry = setup();
    let dir = tempfile::tempdir().unwrap();
    file_client(&registry, dir.path())
        .refresh(&registry.site, t0())
        .unwrap();
    std::fs::write(dir.path().join(FileTrustStore::STATE_FILE), b"{\"format\":").unwrap();
    assert!(matches!(
        RegistryClient::open(
            registry.embedded_roots(),
            FileTrustStore::new(dir.path()),
            Policy::default()
        ),
        Err(RegistryError::Store(StoreError::Corrupt(_)))
    ));
    let mut state = file_client_state(&registry);
    state.format = 2;
    std::fs::write(
        dir.path().join(FileTrustStore::STATE_FILE),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        RegistryClient::open(
            registry.embedded_roots(),
            FileTrustStore::new(dir.path()),
            Policy::default()
        ),
        Err(RegistryError::Store(StoreError::Corrupt(_)))
    ));
}

fn file_client_state(registry: &Registry) -> TrustState {
    let mut c = client(registry);
    c.refresh(&registry.site, t0()).unwrap();
    c.state().clone()
}

#[test]
fn a_failed_save_changes_nothing() {
    let mut registry = setup();
    let mut c = client(&registry);
    c.refresh(&registry.site, t0()).unwrap();
    let before = c.state().clone();
    let mut store = c.store().clone();
    store.fail_saves = true;
    let mut c = RegistryClient::open(registry.embedded_roots(), store, Policy::default()).unwrap();
    registry.publish(
        t0() + days(1),
        &[plugin("acme.clock", &["1.0.0", "2.0.0"])],
        &[],
    );
    assert!(matches!(
        c.refresh(&registry.site, t0() + days(1)),
        Err(RegistryError::Store(_))
    ));
    assert_eq!(c.state(), &before);
    assert_eq!(c.index().unwrap().seq, 1);
}

#[test]
fn invalid_embedded_roots_are_refused() {
    let key = ThrowawayKey::generate();
    for roots in [
        Role {
            keys: vec![],
            threshold: 1,
        },
        role(&[&key], 0),
        role(&[&key], 2),
        Role {
            keys: vec![key.public().to_owned(), key.public_with_comment("again")],
            threshold: 1,
        },
        Role {
            keys: vec!["not a key".into()],
            threshold: 1,
        },
    ] {
        assert!(matches!(
            RegistryClient::open(roots, MemoryTrustStore::default(), Policy::default()),
            Err(RegistryError::InvalidRoots(_))
        ));
    }
}

// ------------------------------------------------------------------ expiry

#[test]
fn expired_timestamp_is_rejected() {
    let registry = setup();
    let mut c = client(&registry);
    let late = t0() + days(8);
    assert!(matches!(
        c.refresh(&registry.site, late),
        Err(RegistryError::TimestampExpired { .. })
    ));
    assert_eq!(c.freshness(late), Freshness::NeverVerified);
    assert_eq!(
        c.can_install(late).unwrap_err(),
        InstallError::NeverVerified
    );
}

#[test]
fn clock_skew_allowance_is_five_minutes() {
    let registry = setup();
    let expires = t0() + days(7);
    for (offset, ok) in [
        (chrono::TimeDelta::minutes(4), true),
        (chrono::TimeDelta::minutes(5), false),
        (chrono::TimeDelta::minutes(6), false),
    ] {
        let mut c = client(&registry);
        assert_eq!(
            c.refresh(&registry.site, expires + offset).is_ok(),
            ok,
            "{offset}"
        );
    }
}

#[test]
fn the_timestamp_governs_freshness_not_the_index_expiry() {
    // Only timestamp.yml ran for 200 days: far past the 90-day index
    // expiry. By default the client stays fresh (spike finding 1).
    let registry = setup();
    let mut c = client(&registry);
    c.refresh(&registry.site, t0()).unwrap();
    let t = t0() + days(200);
    registry.sign_timestamp(t);
    assert_eq!(
        c.refresh(&registry.site, t),
        Ok(Refresh::Unchanged { seq: 1 })
    );
    assert_eq!(c.freshness(t), Freshness::Fresh);
    assert!(c.can_install(t).is_ok());
    // A new client accepts the old index under a fresh timestamp too.
    assert_eq!(
        client(&registry).refresh(&registry.site, t),
        Ok(Refresh::Updated { seq: 1 })
    );

    // With the stricter policy the index expiry blocks installs.
    let strict = Policy {
        enforce_index_expiry: true,
        ..Policy::default()
    };
    let mut c = RegistryClient::open(
        registry.embedded_roots(),
        MemoryTrustStore::default(),
        strict,
    )
    .unwrap();
    assert!(matches!(
        c.refresh(&registry.site, t),
        Err(RegistryError::IndexExpired { .. })
    ));
    let mut c = RegistryClient::open(
        registry.embedded_roots(),
        MemoryTrustStore::default(),
        strict,
    )
    .unwrap();
    registry.sign_timestamp(t0() + days(85));
    c.refresh(&registry.site, t0() + days(85)).unwrap();
    assert_eq!(
        c.freshness(t0() + days(91)),
        Freshness::Stale(StaleReason::IndexExpired)
    );
}

#[test]
fn overlong_timestamp_validity_is_rejected() {
    // A stolen timestamp key cannot sign a year-long timestamp to freeze
    // clients for that long.
    let mut registry = setup();
    registry.timestamp_validity = days(365);
    registry.sign_timestamp(t0());
    assert!(matches!(
        client(&registry).refresh(&registry.site, t0()),
        Err(RegistryError::TimestampValidity { .. })
    ));
    registry.timestamp_validity = days(14);
    registry.sign_timestamp(t0());
    assert!(client(&registry).refresh(&registry.site, t0()).is_ok());
}

// --------------------------------------------------------- rollback, clock

#[test]
fn rollback_to_a_lower_seq_is_rejected_across_restarts() {
    let mut registry = setup();
    let old = registry.site.snapshot();
    registry.publish(
        t0() + chrono::TimeDelta::hours(1),
        &[plugin("acme.clock", &["1.0.0", "1.1.0"])],
        &[],
    );
    let mut c = client(&registry);
    assert_eq!(
        c.refresh(&registry.site, t0() + chrono::TimeDelta::hours(2)),
        Ok(Refresh::Updated { seq: 2 })
    );
    // Restart; a mirror serves the old (unexpired, validly signed) seq 1.
    registry.site.replay_timestamp(&old);
    let mut c = restart(&registry, &c);
    let before = c.state().clone();
    assert_eq!(
        c.refresh(&registry.site, t0() + chrono::TimeDelta::hours(3)),
        Err(RegistryError::Rollback {
            stored: 2,
            offered: 1
        })
    );
    assert_eq!(c.state().seq, before.seq);
    assert_eq!(c.state().timestamp, before.timestamp);
    assert_eq!(restart(&registry, &c).state().seq, 2);
}

#[test]
fn a_clock_in_the_past_cannot_revive_an_old_index() {
    let mut registry = setup();
    let old = registry.site.snapshot();
    registry.publish(
        t0() + days(2),
        &[plugin("acme.clock", &["1.0.0", "1.1.0"])],
        &[],
    );
    let mut c = client(&registry);
    c.refresh(&registry.site, t0() + days(2)).unwrap();
    registry.site.replay_timestamp(&old);
    assert_eq!(
        restart(&registry, &c).refresh(&registry.site, t0()),
        Err(RegistryError::Rollback {
            stored: 2,
            offered: 1
        })
    );
}

#[test]
fn a_clock_set_back_cannot_make_an_expired_timestamp_fresh() {
    let registry = setup();
    let mut c = client(&registry);
    c.refresh(&registry.site, t0()).unwrap();
    // Offline for 10 days: stale. The client records that this time passed.
    registry.site.offline.set(true);
    let late = t0() + days(10);
    assert_eq!(
        c.refresh(&registry.site, late),
        Ok(Refresh::Offline(Freshness::Stale(
            StaleReason::TimestampExpired
        )))
    );
    // The clock is set back to day 1 (or the battery died): still stale,
    // also after a restart.
    let back = t0() + days(1);
    assert_eq!(
        c.freshness(back),
        Freshness::Stale(StaleReason::TimestampExpired)
    );
    let mut c = restart(&registry, &c);
    assert_eq!(
        c.refresh(&registry.site, back),
        Ok(Refresh::Offline(Freshness::Stale(
            StaleReason::TimestampExpired
        )))
    );
    assert!(c.can_install(back).is_err());
    // Online with the clock still wrong: the captured old timestamp is
    // refused as expired, a newly signed one is accepted.
    registry.site.offline.set(false);
    assert!(matches!(
        c.refresh(&registry.site, back),
        Err(RegistryError::TimestampExpired { .. })
    ));
    registry.sign_timestamp(late);
    assert_eq!(
        c.refresh(&registry.site, back),
        Ok(Refresh::Unchanged { seq: 1 })
    );
    assert_eq!(c.freshness(back), Freshness::Fresh);
}

#[test]
fn a_clock_far_in_the_future_recovers_at_the_next_refresh() {
    let registry = setup();
    let mut c = client(&registry);
    c.refresh(&registry.site, t0()).unwrap();
    let future = t0() + days(10_000);
    assert!(matches!(
        c.refresh(&registry.site, future),
        Err(RegistryError::TimestampExpired { .. })
    ));
    // The floor is capped at the stored expiry, not year 2054.
    assert_eq!(
        c.state().clock_floor,
        Some(t0() + days(7) + chrono::TimeDelta::minutes(5))
    );
    // Clock corrected; the registry re-signed meanwhile.
    let now = t0() + days(8);
    registry.sign_timestamp(now);
    assert_eq!(
        c.refresh(&registry.site, now),
        Ok(Refresh::Unchanged { seq: 1 })
    );
    assert_eq!(c.freshness(now), Freshness::Fresh);
}

#[test]
fn a_replayed_older_signature_does_not_shorten_the_expiry() {
    let registry = setup();
    let first = registry.site.snapshot();
    let mut c = client(&registry);
    c.refresh(&registry.site, t0()).unwrap();
    registry.sign_timestamp(t0() + days(3));
    c.refresh(&registry.site, t0() + days(3)).unwrap();
    let expires = c.state().timestamp.as_ref().unwrap().expires;
    registry.site.replay_timestamp(&first);
    assert_eq!(
        c.refresh(&registry.site, t0() + days(4)),
        Ok(Refresh::Unchanged { seq: 1 })
    );
    assert_eq!(c.state().timestamp.as_ref().unwrap().expires, expires);
}

#[test]
fn two_indexes_for_one_seq_are_rejected() {
    let registry = setup();
    let mut c = client(&registry);
    c.refresh(&registry.site, t0()).unwrap();
    let plugins = [plugin("acme.clock", &["1.0.0", "6.6.6"])];
    let index = registry.index_doc(1, t0(), &plugins, &[]);
    registry.publish_index(&index, &plugins, t0(), &registry.index_key);
    registry.sign_timestamp(t0() + chrono::TimeDelta::hours(1));
    assert_eq!(
        c.refresh(&registry.site, t0() + chrono::TimeDelta::hours(1)),
        Err(RegistryError::Equivocation { seq: 1 })
    );
}

#[test]
fn freeze_attack_is_detected_at_timestamp_expiry() {
    let mut registry = setup();
    let mut c = client(&registry);
    c.refresh(&registry.site, t0()).unwrap();
    let frozen = registry.site.snapshot();
    // The registry revokes something; the attacker keeps serving seq 1.
    let t1 = t0() + days(1);
    registry.publish(
        t1,
        &[plugin("acme.clock", &["1.0.0"])],
        &[revocation(
            "acme.clock",
            Some(&["1.0.0"]),
            RevocationAction::Disable,
        )],
    );
    registry.site.replay_timestamp(&frozen);
    // Within the 7-day window a freeze looks like "no news".
    assert_eq!(
        c.refresh(&registry.site, t1),
        Ok(Refresh::Unchanged { seq: 1 })
    );
    assert_eq!(
        c.installed_verdict(&id("acme.clock"), &v("1.0.0")),
        InstalledVerdict::Run
    );
    // After expiry + skew it is refused; installs stop.
    let t2 = t0() + days(7) + chrono::TimeDelta::minutes(6);
    assert!(matches!(
        c.refresh(&registry.site, t2),
        Err(RegistryError::TimestampExpired { .. })
    ));
    assert_eq!(
        c.freshness(t2),
        Freshness::Stale(StaleReason::TimestampExpired)
    );
    assert_eq!(
        c.can_install(t2).unwrap_err(),
        InstallError::Stale(StaleReason::TimestampExpired)
    );
    // Freeze lifted: the revocation arrives.
    registry.sign_timestamp(t2);
    assert_eq!(
        c.refresh(&registry.site, t2),
        Ok(Refresh::Updated { seq: 2 })
    );
    assert!(matches!(
        c.installed_verdict(&id("acme.clock"), &v("1.0.0")),
        InstalledVerdict::Disable { .. }
    ));
}

// ------------------------------------------------------------ keys, roles

#[test]
fn index_signed_by_a_key_outside_the_index_role_is_rejected() {
    let registry = setup();
    let stranger = ThrowawayKey::generate();
    let plugins = [plugin("acme.clock", &["1.0.0"])];
    let index = registry.index_doc(1, t0(), &plugins, &[]);
    for (name, key) in [
        ("stranger", &stranger),
        ("timestamp key", &registry.timestamp_key),
        ("root key", &registry.roots[0]),
    ] {
        registry.publish_index(&index, &plugins, t0(), key);
        registry.sign_timestamp(t0());
        let mut c = client(&registry);
        assert_eq!(
            c.refresh(&registry.site, t0()),
            Err(RegistryError::IndexSignature(SigError::BelowThreshold {
                valid: 0,
                threshold: 1
            })),
            "{name}"
        );
        assert_eq!(c.freshness(t0()), Freshness::NeverVerified);
    }
}

#[test]
fn timestamp_signed_by_another_role_is_rejected() {
    let registry = setup();
    for key in [&registry.index_key, &registry.roots[0]] {
        registry.sign_timestamp_with(t0(), 1, 1, key);
        assert!(matches!(
            client(&registry).refresh(&registry.site, t0()),
            Err(RegistryError::TimestampSignature(
                SigError::BelowThreshold { .. }
            ))
        ));
    }
}

#[test]
fn key_set_signed_by_a_non_root_key_is_rejected() {
    let mut registry = setup();
    let mut c = client(&registry);
    c.refresh(&registry.site, t0()).unwrap();
    let root = registry.root_role();
    let doc = registry.keyset_doc(&root, 0, false);
    let index_key = std::mem::replace(&mut registry.index_key, ThrowawayKey::generate());
    for signer in [
        &index_key,
        &registry.timestamp_key,
        &ThrowawayKey::generate(),
    ] {
        registry.publish_keyset(&doc, &[signer]);
        registry.keys_version.set(1);
        assert!(matches!(
            c.refresh(&registry.site, t0()),
            Err(RegistryError::KeysetSignature { version: 2, .. })
        ));
        assert_eq!(c.keys_version(), 1);
    }
}

#[test]
fn signatures_with_another_comment_are_rejected() {
    let registry = setup();
    let path = "index/1.json";
    let bytes = registry.site.get(path);
    for comment in [
        index_comment(2, 1),
        index_comment(1, 2),
        timestamp_comment(1, 1),
        keys_comment(1),
        format!("{} ", index_comment(1, 1)),
        index_comment(1, 1).replace("v1", "v2"),
        "timestamp:1700000000\tfile:index.json\tversion:1.0.0".to_owned(),
        String::new(),
    ] {
        registry.site.put(
            &format!("{path}.minisig"),
            registry.index_key.sign(&bytes, &comment),
        );
        assert!(
            matches!(
                client(&registry).refresh(&registry.site, t0()),
                Err(RegistryError::IndexSignature(SigError::WrongComment { .. }))
            ),
            "{comment:?}"
        );
    }
    // Timestamps: the comment must parse canonically, then match.
    let ts = registry.site.get("timestamp.json");
    for comment in [
        "deckpilot-registry v1 role:timestamp seq:01 keys:1".to_owned(),
        "timestamp:1700000000\tfile:timestamp.json\tversion:1.0.0".to_owned(),
        index_comment(1, 1),
    ] {
        registry.site.put(
            "timestamp.json.minisig",
            registry.timestamp_key.sign(&ts, &comment),
        );
        assert!(
            matches!(
                client(&registry).refresh(&registry.site, t0()),
                Err(RegistryError::TimestampSignature(
                    SigError::UnparsableComment(_)
                ))
            ),
            "{comment}"
        );
    }
    // A well-formed comment naming another seq than the content.
    registry.site.put(
        "timestamp.json.minisig",
        registry.timestamp_key.sign(&ts, &timestamp_comment(2, 1)),
    );
    assert!(matches!(
        client(&registry).refresh(&registry.site, t0()),
        Err(RegistryError::Malformed { .. })
    ));
    // Key sets: the version in the comment is the one required.
    let keys = registry.site.get("keys/1.json");
    registry.site.put(
        "keys/1.json.minisig",
        serde_json::to_vec(&[registry.roots[0].sign(&keys, &keys_comment(2))]).unwrap(),
    );
    assert!(matches!(
        client(&registry).refresh(&registry.site, t0()),
        Err(RegistryError::KeysetSignature {
            version: 1,
            error: SigError::WrongComment { .. }
        })
    ));
}

#[test]
fn registry_signatures_never_pass_the_updater_verifier() {
    // And the other way round (above): updater-format comments never pass
    // the registry. Here: the app updater's own check refuses registry
    // signatures, so a registry key can never authenticate an app update.
    let registry = setup();
    for (path, key) in [
        ("keys/1.json", &registry.roots[0]),
        ("index/1.json", &registry.index_key),
        ("timestamp.json", &registry.timestamp_key),
    ] {
        let bytes = registry.site.get(path);
        let signature = if path == "keys/1.json" {
            let list: Vec<String> =
                serde_json::from_slice(&registry.site.get("keys/1.json.minisig")).unwrap();
            list[0].clone()
        } else {
            String::from_utf8(registry.site.get(&format!("{path}.minisig"))).unwrap()
        };
        assert_eq!(
            deckpilot_update::signature::verify_release_artifact(
                key.public(),
                &signature,
                &bytes,
                &v("1.0.0")
            ),
            Err(deckpilot_update::signature::SignatureError::MissingVersion),
            "{path}"
        );
    }
}

/// Flips two bits of every byte of `original` and requires `check` to
/// refuse each result.
fn flip_every_byte(original: &[u8], what: &str, mut accepts: impl FnMut(&[u8]) -> bool) {
    for at in 0..original.len() {
        for bit in [0u8, 6] {
            let mut flipped = original.to_vec();
            flipped[at] ^= 1 << bit;
            assert!(!accepts(&flipped), "{what} byte {at} bit {bit}");
        }
    }
    assert!(accepts(original), "{what} unchanged");
}

#[test]
fn every_bit_flip_of_a_signed_document_is_rejected() {
    let registry = setup();
    let site = &registry.site;
    let keys = site.get("keys/1.json");
    let keys_signatures = site.get("keys/1.json.minisig");
    flip_every_byte(&keys, "keys/1.json", |bytes| {
        client(&registry)
            .apply_keyset(bytes, &keys_signatures)
            .is_ok()
    });

    let mut c = client(&registry);
    c.apply_keyset(&keys, &keys_signatures).unwrap();
    let timestamp = site.get("timestamp.json");
    let timestamp_signature = site.get("timestamp.json.minisig");
    flip_every_byte(&timestamp, "timestamp.json", |bytes| {
        c.verify_timestamp(bytes, &timestamp_signature, t0())
            .is_ok()
    });

    let verified = c
        .verify_timestamp(&timestamp, &timestamp_signature, t0())
        .unwrap();
    let index = site.get("index/1.json");
    let index_signature = site.get("index/1.json.minisig");
    flip_every_byte(&index, "index/1.json", |bytes| {
        let mut fresh = client(&registry);
        fresh.apply_keyset(&keys, &keys_signatures).unwrap();
        fresh
            .commit(verified.clone(), Some((bytes, &index_signature)), t0())
            .is_ok()
    });
}

/// The signed parts of a minisign signature: everything but the untrusted
/// comment line, which minisign does not sign.
fn signed_lines(signature_b64: &[u8]) -> Option<Vec<String>> {
    use base64::Engine as _;
    let text = base64::engine::general_purpose::STANDARD
        .decode(std::str::from_utf8(signature_b64).ok()?.trim())
        .ok()?;
    let text = String::from_utf8(text).ok()?;
    Some(text.lines().skip(1).map(str::to_owned).collect())
}

#[test]
fn every_bit_flip_of_a_signature_is_rejected_unless_unsigned() {
    let registry = setup();
    let site = &registry.site;
    let mut c = client(&registry);
    c.apply_keyset(&site.get("keys/1.json"), &site.get("keys/1.json.minisig"))
        .unwrap();
    let timestamp = site.get("timestamp.json");
    let index = site.get("index/1.json");
    let verified = c
        .verify_timestamp(&timestamp, &site.get("timestamp.json.minisig"), t0())
        .unwrap();
    let mut rejected = 0;
    for path in ["timestamp.json.minisig", "index/1.json.minisig"] {
        let original = site.get(path);
        let signed = signed_lines(&original).unwrap();
        for at in 0..original.len() {
            for bit in [0u8, 3] {
                let mut flipped = original.clone();
                flipped[at] ^= 1 << bit;
                let accepted = if path.starts_with("timestamp") {
                    c.verify_timestamp(&timestamp, &flipped, t0()).is_ok()
                } else {
                    let mut fresh = client(&registry);
                    fresh
                        .apply_keyset(&site.get("keys/1.json"), &site.get("keys/1.json.minisig"))
                        .unwrap();
                    fresh
                        .commit(verified.clone(), Some((&index, &flipped)), t0())
                        .is_ok()
                };
                if accepted {
                    // Only the untrusted comment changed.
                    assert_eq!(
                        signed_lines(&flipped).as_ref(),
                        Some(&signed),
                        "{path} byte {at} bit {bit}"
                    );
                } else {
                    rejected += 1;
                }
            }
        }
    }
    assert!(rejected > 1000, "{rejected}");
}

#[test]
fn truncated_documents_are_rejected() {
    let registry = setup();
    for path in [
        "keys/1.json",
        "keys/1.json.minisig",
        "timestamp.json",
        "timestamp.json.minisig",
        "index/1.json",
        "index/1.json.minisig",
    ] {
        let original = registry.site.get(path);
        let step = (original.len() / 60).max(1);
        for len in (0..original.len()).step_by(step) {
            registry.site.put(path, original[..len].to_vec());
            assert!(
                client(&registry).refresh(&registry.site, t0()).is_err(),
                "{path} truncated to {len}"
            );
        }
        registry.site.put(path, original);
    }
}

#[test]
fn index_key_rotation_through_a_root_signed_key_set() {
    let mut registry = setup();
    let mut c = client(&registry);
    c.refresh(&registry.site, t0()).unwrap();
    let old_index_key = std::mem::replace(&mut registry.index_key, ThrowawayKey::generate());
    let root = registry.root_role();
    let doc = registry.keyset_doc(&root, 0, false);
    registry.publish_keyset(&doc, &[&registry.roots[1]]); // either root (1 of 2)
    let t = t0() + days(1);
    registry.publish(t, &[plugin("acme.clock", &["1.0.0"])], &[]);
    assert_eq!(
        c.refresh(&registry.site, t),
        Ok(Refresh::Updated { seq: 2 })
    );
    assert_eq!(c.keys_version(), 2);
    // The retired index key cannot sign any more.
    let plugins = [plugin("acme.clock", &["1.0.0", "9.9.9"])];
    let index = registry.index_doc(3, t, &plugins, &[]);
    registry.publish_index(&index, &plugins, t, &old_index_key);
    registry.seq = 3;
    registry.sign_timestamp(t);
    assert!(matches!(
        c.refresh(&registry.site, t),
        Err(RegistryError::IndexSignature(_))
    ));
    // A timestamp still naming key set 1 is refused after the rotation.
    registry.sign_timestamp_with(t, 2, 1, &registry.timestamp_key);
    assert!(matches!(
        c.refresh(&registry.site, t),
        Err(RegistryError::KeysVersion {
            document: "timestamp",
            found: 1,
            current: 2
        })
    ));
}

#[test]
fn a_replayed_key_set_is_not_a_new_link() {
    // keys/1.json served as keys/2.json: signed for version 1, not 2.
    let registry = setup();
    let mut c = client(&registry);
    c.refresh(&registry.site, t0()).unwrap();
    registry
        .site
        .put("keys/2.json", registry.site.get("keys/1.json"));
    registry.site.put(
        "keys/2.json.minisig",
        registry.site.get("keys/1.json.minisig"),
    );
    assert!(matches!(
        c.refresh(&registry.site, t0()),
        Err(RegistryError::KeysetSignature {
            version: 2,
            error: SigError::WrongComment { .. } | SigError::BelowThreshold { .. }
        })
    ));
    // Correctly signed for version 2 but saying version 1 inside.
    let bytes = registry.site.get("keys/1.json");
    registry.site.put(
        "keys/2.json.minisig",
        serde_json::to_vec(&[registry.roots[0].sign(&bytes, &keys_comment(2))]).unwrap(),
    );
    assert!(matches!(
        c.refresh(&registry.site, t0()),
        Err(RegistryError::Malformed { .. })
    ));
    assert_eq!(c.keys_version(), 1);
}

#[test]
fn root_rotation_needs_old_and_new_roots() {
    let mut registry = setup();
    let mut c = client(&registry);
    c.refresh(&registry.site, t0()).unwrap();
    let new_roots = [ThrowawayKey::generate(), ThrowawayKey::generate()];
    let new_root_role = role(&[&new_roots[0], &new_roots[1]], 1);
    let doc = registry.keyset_doc(&new_root_role, 0, false);

    // Old root only: the new roots never agreed.
    registry.publish_keyset(&doc, &[&registry.roots[0]]);
    registry.keys_version.set(1);
    assert!(matches!(
        c.refresh(&registry.site, t0()),
        Err(RegistryError::KeysetSignature { version: 2, .. })
    ));
    // New root only: not authorized by the trusted roots.
    registry.publish_keyset(&doc, &[&new_roots[0]]);
    registry.keys_version.set(1);
    assert!(matches!(
        c.refresh(&registry.site, t0()),
        Err(RegistryError::KeysetSignature { version: 2, .. })
    ));
    // Both: accepted.
    registry.publish_keyset(&doc, &[&registry.roots[0], &new_roots[0]]);
    let t = t0() + chrono::TimeDelta::hours(1);
    registry.publish(t, &[plugin("acme.clock", &["1.0.0"])], &[]);
    assert_eq!(
        c.refresh(&registry.site, t),
        Ok(Refresh::Updated { seq: 2 })
    );
    assert_eq!(c.state().keyset.as_ref().unwrap().root, new_root_role);

    // The old roots are powerless from now on.
    let old_roots = std::mem::replace(&mut registry.roots, new_roots.into());
    let attacker = registry.keyset_doc(&role(&[&old_roots[0]], 1), 0, false);
    registry.publish_keyset(&attacker, &[&old_roots[0], &old_roots[1]]);
    assert!(matches!(
        c.refresh(&registry.site, t),
        Err(RegistryError::KeysetSignature { version: 3, .. })
    ));
}

#[test]
fn a_threshold_counts_distinct_keys() {
    let mut registry = Registry::new();
    registry.root_threshold = 2;
    registry.keys_version.set(0);
    let root = registry.root_role();
    let doc = registry.keyset_doc(&root, 0, false);
    registry.keys_version.set(1);
    registry.publish(t0(), &[plugin("acme.clock", &["1.0.0"])], &[]);
    let bytes = serde_json::to_vec_pretty(&doc).unwrap();
    let sign = |k: &ThrowawayKey| k.sign(&bytes, &keys_comment(1));
    for (name, signatures, ok) in [
        ("one root", vec![sign(&registry.roots[0])], false),
        (
            "same root twice",
            vec![sign(&registry.roots[0]), sign(&registry.roots[0])],
            false,
        ),
        (
            "both roots",
            vec![sign(&registry.roots[0]), sign(&registry.roots[1])],
            true,
        ),
    ] {
        registry.site.put("keys/1.json", bytes.clone());
        registry.site.put(
            "keys/1.json.minisig",
            serde_json::to_vec(&signatures).unwrap(),
        );
        let r = RegistryClient::open(root.clone(), MemoryTrustStore::default(), Policy::default())
            .unwrap()
            .refresh(&registry.site, t0());
        if ok {
            assert_eq!(r, Ok(Refresh::Updated { seq: 1 }), "{name}");
        } else {
            assert_eq!(
                r,
                Err(RegistryError::KeysetSignature {
                    version: 1,
                    error: SigError::BelowThreshold {
                        valid: 1,
                        threshold: 2
                    }
                }),
                "{name}"
            );
        }
    }
    // One key listed twice under different untrusted comments is one key:
    // a 2-of-2 role made of it is refused outright.
    let key = &registry.roots[0];
    let doubled = Role {
        keys: vec![key.public().to_owned(), key.public_with_comment("copy")],
        threshold: 2,
    };
    assert!(matches!(
        RegistryClient::open(
            doubled.clone(),
            MemoryTrustStore::default(),
            Policy::default()
        ),
        Err(RegistryError::InvalidRoots(_))
    ));
    assert_eq!(
        sig::verify_role(&doubled, &[sign(key)], &bytes, &keys_comment(1)),
        Err(SigError::BadRole)
    );
}

#[test]
fn a_key_set_may_not_share_keys_between_roles() {
    let mut registry = setup();
    let mut c = client(&registry);
    c.refresh(&registry.site, t0()).unwrap();
    let root = registry.root_role();
    let mut doc = registry.keyset_doc(&root, 0, false);
    doc.timestamp = doc.index.clone();
    registry.publish_keyset(&doc, &[&registry.roots[0]]);
    assert!(matches!(
        c.refresh(&registry.site, t0()),
        Err(RegistryError::Malformed { .. })
    ));
    let mut doc = registry.keyset_doc(&root, 0, false);
    doc.version = 2;
    doc.index = role(&[&registry.roots[1]], 1);
    registry.keys_version.set(1);
    registry.publish_keyset(&doc, &[&registry.roots[0]]);
    assert!(matches!(
        c.refresh(&registry.site, t0()),
        Err(RegistryError::Malformed { .. })
    ));
    registry.index_key = ThrowawayKey::generate();
    assert_eq!(c.keys_version(), 1);
}

#[test]
fn seq_floor_reset_recovers_from_an_inflated_seq() {
    let mut registry = setup();
    let mut c = client(&registry);
    // A compromised index and timestamp key pushed seq to 1,000,000.
    registry.seq = 999_999;
    registry.publish(t0(), &[plugin("acme.clock", &["1.0.0"])], &[]);
    assert_eq!(
        c.refresh(&registry.site, t0()),
        Ok(Refresh::Updated { seq: 1_000_000 })
    );
    let compromised = c.store().clone();

    // Recovery without the reset flag: the real registry at seq 5 is a
    // rollback, and seq_floor 5 changes nothing (it is a floor, a max).
    let t = t0() + chrono::TimeDelta::hours(1);
    registry.index_key = ThrowawayKey::generate();
    registry.timestamp_key = ThrowawayKey::generate();
    let root = registry.root_role();
    let mut doc = registry.keyset_doc(&root, 5, false);
    registry.publish_keyset(&doc, &[&registry.roots[0]]);
    registry.seq = 4;
    registry.publish(t, &[plugin("acme.clock", &["1.0.0"])], &[]);
    let mut other = RegistryClient::open(
        registry.embedded_roots(),
        compromised.clone(),
        Policy::default(),
    )
    .unwrap();
    assert_eq!(
        other.refresh(&registry.site, t),
        Err(RegistryError::Rollback {
            stored: 1_000_000,
            offered: 5
        })
    );

    // With seq_floor_reset the client accepts the re-signed content.
    doc.seq_floor_reset = true;
    registry.keys_version.set(1);
    registry.publish_keyset(&doc, &[&registry.roots[0]]);
    registry.seq = 4;
    registry.publish(t, &[plugin("acme.clock", &["1.0.0"])], &[]);
    assert_eq!(
        c.refresh(&registry.site, t),
        Ok(Refresh::Updated { seq: 5 })
    );
    // ... and seq_floor itself is a floor for a fresh client.
    registry.seq = 3;
    registry.publish(t, &[plugin("acme.clock", &["1.0.0"])], &[]);
    assert_eq!(
        client(&registry).refresh(&registry.site, t),
        Err(RegistryError::Rollback {
            stored: 5,
            offered: 4
        })
    );
}

#[test]
fn new_embedded_roots_restart_the_chain() {
    // Root compromise: an app release ships new roots (section 9.4) and the
    // registry re-publishes its chain from keys/1.json under them.
    let mut registry = setup();
    let mut c = client(&registry);
    registry.publish(
        t0(),
        &[plugin("acme.clock", &["1.0.0"])],
        &[revocation(
            "acme.clock",
            Some(&["1.0.0"]),
            RevocationAction::Disable,
        )],
    );
    c.refresh(&registry.site, t0()).unwrap();
    let store = c.store().clone();

    let mut fresh = Registry::new();
    fresh.seq = registry.seq;
    fresh.publish(
        t0() + days(1),
        &[plugin("acme.clock", &["1.0.0"])],
        &[revocation(
            "acme.clock",
            Some(&["1.0.0"]),
            RevocationAction::Disable,
        )],
    );
    let mut c = RegistryClient::open(fresh.embedded_roots(), store, Policy::default()).unwrap();
    assert_eq!(c.keys_version(), 0);
    assert_eq!(c.freshness(t0() + days(1)), Freshness::NeverVerified);
    // Rollback floor and revocations survive the change.
    assert_eq!(c.state().seq, 2);
    assert!(matches!(
        c.installed_verdict(&id("acme.clock"), &v("1.0.0")),
        InstalledVerdict::Disable { .. }
    ));
    assert_eq!(
        c.refresh(&fresh.site, t0() + days(1)),
        Ok(Refresh::Updated { seq: 3 })
    );
    // The old registry's documents are worthless now.
    assert!(c.refresh(&registry.site, t0() + days(1)).is_err());
}

#[test]
fn foreign_download_prefix_is_rejected_even_if_signed() {
    let registry = setup();
    let plugins = [plugin("acme.clock", &["1.0.0"])];
    let mut index = registry.index_doc(1, t0(), &plugins, &[]);
    index.download_prefix = "https://github.com/evil/releases/download/".into();
    registry.publish_index(&index, &plugins, t0(), &registry.index_key);
    registry.sign_timestamp(t0());
    assert!(matches!(
        client(&registry).refresh(&registry.site, t0()),
        Err(RegistryError::BadDownloadPrefix(_))
    ));
}

#[test]
fn signed_but_invalid_indexes_are_rejected() {
    let registry = setup();
    let plugins = [plugin("acme.clock", &["1.0.0"])];
    let mutations: [fn(&mut Index); 6] = [
        |i| i.seq = 2,
        |i| i.keys_version = 2,
        |i| i.source_commit = "xyz".into(),
        |i| i.plugins[0].detail.path = "plugins/acme.clock/elsewhere.json".into(),
        |i| i.plugins[0].publisher = "evil".into(),
        |i| i.expires = i.generated_at,
    ];
    for (n, mutate) in mutations.iter().enumerate() {
        let mut index = registry.index_doc(1, t0(), &plugins, &[]);
        mutate(&mut index);
        let bytes = serde_json::to_vec_pretty(&index).unwrap();
        registry.site.put("index/1.json", bytes.clone());
        registry.site.put(
            "index/1.json.minisig",
            registry.index_key.sign(&bytes, &index_comment(1, 1)),
        );
        registry.sign_timestamp(t0());
        assert!(
            matches!(
                client(&registry).refresh(&registry.site, t0()),
                Err(RegistryError::Malformed { .. }
                    | RegistryError::IndexMismatch(_)
                    | RegistryError::KeysVersion { .. })
            ),
            "mutation {n}"
        );
    }
}

#[test]
fn a_timestamp_must_pin_the_index_of_its_seq() {
    let mut registry = setup();
    registry.publish(t0(), &[plugin("acme.clock", &["1.0.0", "1.1.0"])], &[]);
    // A timestamp for seq 2 pointing at index/1.json.
    let mut ts: Timestamp = serde_json::from_slice(&registry.timestamp_bytes(t0(), 1, 1)).unwrap();
    ts.seq = 2;
    let bytes = serde_json::to_vec(&ts).unwrap();
    registry.site.put("timestamp.json", bytes.clone());
    registry.site.put(
        "timestamp.json.minisig",
        registry
            .timestamp_key
            .sign(&bytes, &timestamp_comment(2, 1)),
    );
    assert!(matches!(
        client(&registry).refresh(&registry.site, t0()),
        Err(RegistryError::IndexMismatch(_))
    ));
}

#[test]
fn the_step_by_step_api_matches_refresh() {
    let registry = setup();
    let mut c = client(&registry);
    let site = &registry.site;
    assert_eq!(
        c.apply_keyset(&site.get("keys/1.json"), &site.get("keys/1.json.minisig")),
        Ok(1)
    );
    let ts = c
        .verify_timestamp(
            &site.get("timestamp.json"),
            &site.get("timestamp.json.minisig"),
            t0(),
        )
        .unwrap();
    assert!(c.needs_index(&ts));
    assert_eq!(ts.index_ref().path, "index/1.json");
    assert_eq!(
        c.commit(ts.clone(), None, t0()),
        Err(RegistryError::IndexRequired)
    );
    assert_eq!(
        c.commit(
            ts.clone(),
            Some((&site.get("index/1.json"), &site.get("index/1.json.minisig"))),
            t0()
        ),
        Ok(Refresh::Updated { seq: 1 })
    );
    assert!(!c.needs_index(&ts));
    assert_eq!(c.commit(ts, None, t0()), Ok(Refresh::Unchanged { seq: 1 }));
}

// -------------------------------------------------------------- revocation

fn revoked_registry() -> (Registry, Client) {
    let mut registry = Registry::new();
    let mut spec = plugin("acme.clock", &["1.0.0", "1.1.0", "1.2.0", "1.3.0"]);
    spec.versions[3].yanked = Some("broken on Windows".into());
    registry.publish(
        t0(),
        &[spec, plugin("evil.miner", &["0.1.0", "0.2.0"])],
        &[
            revocation("acme.clock", Some(&["1.0.0"]), RevocationAction::Disable),
            revocation("acme.clock", Some(&["1.1.0"]), RevocationAction::Yank),
            revocation("evil.miner", None, RevocationAction::Disable),
        ],
    );
    let mut c = client(&registry);
    c.refresh(&registry.site, t0()).unwrap();
    (registry, c)
}

fn detail_of(registry: &Registry, c: &Client, plugin_id: &str) -> Vec<u8> {
    registry
        .site
        .get(&c.detail_ref(&id(plugin_id)).unwrap().path)
}

#[test]
fn revoked_and_yanked_versions_are_refused_for_install() {
    let (registry, c) = revoked_registry();
    let install = |plugin_id: &str, version: &str| {
        c.resolve_install(
            &id(plugin_id),
            &v(version),
            &registry
                .site
                .get(
                    &c.detail_ref(&id(plugin_id))
                        .map_or_else(|| "missing".to_owned(), |r| r.path.clone()),
                )
                .clone(),
            None,
            t0(),
        )
    };
    assert!(matches!(
        install("acme.clock", "1.0.0"),
        Err(InstallError::Revoked {
            advisory: Some(_),
            ..
        })
    ));
    assert!(matches!(
        install("acme.clock", "1.1.0"),
        Err(InstallError::Yanked { .. })
    ));
    assert!(matches!(
        install("acme.clock", "1.3.0"),
        Err(InstallError::Yanked { .. })
    ));
    assert!(matches!(
        install("evil.miner", "0.2.0"),
        Err(InstallError::Revoked { .. })
    ));
    assert!(matches!(
        install("acme.clock", "9.0.0"),
        Err(InstallError::UnknownVersion { .. })
    ));
    assert!(matches!(
        c.resolve_install(&id("acme.other"), &v("1.0.0"), b"", None, t0()),
        Err(InstallError::UnknownPlugin(_))
    ));
    let ok = install("acme.clock", "1.2.0").unwrap();
    assert_eq!(ok.url, release_url("acme.clock", "1.2.0"));
}

#[test]
fn revoked_installed_versions_are_flagged_offline_and_stale() {
    let (registry, c) = revoked_registry();
    let check = |c: &Client| {
        assert!(matches!(
            c.installed_verdict(&id("acme.clock"), &v("1.0.0")),
            InstalledVerdict::Disable {
                advisory: Some(_),
                ..
            }
        ));
        assert!(matches!(
            c.installed_verdict(&id("acme.clock"), &v("1.1.0")),
            InstalledVerdict::RunWithNotice { .. }
        ));
        assert_eq!(
            c.installed_verdict(&id("acme.clock"), &v("1.2.0")),
            InstalledVerdict::Run
        );
        assert!(matches!(
            c.installed_verdict(&id("evil.miner"), &v("0.1.0")),
            InstalledVerdict::Disable { .. }
        ));
    };
    check(&c);
    // Next start, a month later, offline: stale, revocations still enforced.
    registry.site.offline.set(true);
    let mut c = restart(&registry, &c);
    let late = t0() + days(30);
    assert_eq!(
        c.refresh(&registry.site, late),
        Ok(Refresh::Offline(Freshness::Stale(
            StaleReason::TimestampExpired
        )))
    );
    check(&c);
    // Even with the index cache gone.
    let mut store = c.store().clone();
    store.index = None;
    let c = RegistryClient::open(registry.embedded_roots(), store, Policy::default()).unwrap();
    check(&c);
}

#[test]
fn revocations_cannot_be_dropped_or_weakened() {
    let t = t0() + days(1);
    let cases: [(&str, Vec<Revocation>); 4] = [
        ("dropped", vec![]),
        (
            "narrowed from * to a version",
            vec![
                revocation("acme.clock", Some(&["1.0.0"]), RevocationAction::Disable),
                revocation("acme.clock", Some(&["1.1.0"]), RevocationAction::Yank),
                revocation("evil.miner", Some(&["0.1.0"]), RevocationAction::Disable),
            ],
        ),
        (
            "weakened from disable to yank",
            vec![
                revocation("acme.clock", Some(&["1.0.0"]), RevocationAction::Yank),
                revocation("acme.clock", Some(&["1.1.0"]), RevocationAction::Yank),
                revocation("evil.miner", None, RevocationAction::Disable),
            ],
        ),
        (
            "moved to another version",
            vec![
                revocation("acme.clock", Some(&["1.2.0"]), RevocationAction::Disable),
                revocation("acme.clock", Some(&["1.1.0"]), RevocationAction::Yank),
                revocation("evil.miner", None, RevocationAction::Disable),
            ],
        ),
    ];
    for (name, revocations) in cases {
        let (mut registry, mut c) = revoked_registry();
        registry.publish(
            t,
            &[plugin("acme.clock", &["1.0.0", "1.2.0"])],
            &revocations,
        );
        assert!(
            matches!(
                c.refresh(&registry.site, t),
                Err(RegistryError::RevocationDropped { .. })
            ),
            "{name}"
        );
        assert!(matches!(
            c.installed_verdict(&id("acme.clock"), &v("1.0.0")),
            InstalledVerdict::Disable { .. }
        ));
    }
    // Strengthening, widening, merging and rewording are fine.
    let (mut registry, mut c) = revoked_registry();
    let mut reworded = revocation(
        "acme.clock",
        Some(&["1.0.0", "1.1.0"]),
        RevocationAction::Disable,
    );
    reworded.reason = "A clearer reason".into();
    reworded.advisory = None;
    registry.publish(
        t,
        &[plugin("acme.clock", &["1.0.0", "1.2.0"])],
        &[
            reworded,
            revocation("evil.miner", None, RevocationAction::Disable),
        ],
    );
    assert_eq!(
        c.refresh(&registry.site, t),
        Ok(Refresh::Updated { seq: 2 })
    );
    assert!(matches!(
        c.installed_verdict(&id("acme.clock"), &v("1.1.0")),
        InstalledVerdict::Disable { .. }
    ));
}

#[test]
fn a_revocation_is_lifted_only_through_a_root_signed_reset() {
    let (mut registry, mut c) = revoked_registry();
    let t = t0() + days(1);
    // The maintainers decide acme.clock 1.0.0 was revoked by mistake. A new
    // index alone cannot lift it...
    let kept = [
        revocation("acme.clock", Some(&["1.1.0"]), RevocationAction::Yank),
        revocation("evil.miner", None, RevocationAction::Disable),
    ];
    registry.publish(t, &[plugin("acme.clock", &["1.0.0", "1.2.0"])], &kept);
    assert!(matches!(
        c.refresh(&registry.site, t),
        Err(RegistryError::RevocationDropped { .. })
    ));
    // ... a key set signed offline by a root, with seq_floor_reset, can.
    let root = registry.root_role();
    let doc = registry.keyset_doc(&root, registry.seq + 1, true);
    registry.publish_keyset(&doc, &[&registry.roots[0]]);
    registry.publish(t, &[plugin("acme.clock", &["1.0.0", "1.2.0"])], &kept);
    assert_eq!(
        c.refresh(&registry.site, t),
        Ok(Refresh::Updated { seq: 3 })
    );
    assert_eq!(
        c.installed_verdict(&id("acme.clock"), &v("1.0.0")),
        InstalledVerdict::Run
    );
    assert!(matches!(
        c.installed_verdict(&id("evil.miner"), &v("0.1.0")),
        InstalledVerdict::Disable { .. }
    ));
    // The reset is one-shot: the next index is cumulative again.
    registry.publish(t, &[plugin("acme.clock", &["1.0.0", "1.2.0"])], &[]);
    assert!(matches!(
        c.refresh(&registry.site, t),
        Err(RegistryError::RevocationDropped { .. })
    ));
    // And it survives a restart while pending: the flag is persisted.
    let (mut registry, c) = revoked_registry();
    let root = registry.root_role();
    let doc = registry.keyset_doc(&root, registry.seq + 1, true);
    registry.publish_keyset(&doc, &[&registry.roots[0]]);
    registry.sign_timestamp(t); // still names key set 1: refused
    let mut c = restart(&registry, &c);
    assert!(c.refresh(&registry.site, t).is_err());
    assert!(c.state().revocations_resettable);
    let mut c = restart(&registry, &c);
    registry.publish(t, &[plugin("acme.clock", &["1.0.0"])], &kept);
    assert_eq!(
        c.refresh(&registry.site, t),
        Ok(Refresh::Updated { seq: 2 })
    );
    assert!(!c.state().revocations_resettable);
}

// ------------------------------------------------------------------ detail

#[test]
fn every_bit_flip_of_a_detail_is_rejected() {
    let (registry, c) = revoked_registry();
    let detail = detail_of(&registry, &c, "acme.clock");
    assert!(c.verify_detail(&id("acme.clock"), &detail).is_ok());
    for at in 0..detail.len() {
        let mut flipped = detail.clone();
        flipped[at] ^= 0x20;
        assert!(
            matches!(
                c.verify_detail(&id("acme.clock"), &flipped),
                Err(RegistryError::DetailMismatch(_))
            ),
            "byte {at}"
        );
    }
    for len in 0..detail.len() {
        assert!(c.verify_detail(&id("acme.clock"), &detail[..len]).is_err());
    }
}

#[test]
fn a_detail_must_be_the_pinned_one_for_its_plugin() {
    let (registry, c) = revoked_registry();
    let other = detail_of(&registry, &c, "evil.miner");
    assert!(matches!(
        c.verify_detail(&id("acme.clock"), &other),
        Err(RegistryError::DetailMismatch(_))
    ));
    assert!(matches!(
        c.verify_detail(&id("acme.nothing"), &other),
        Err(RegistryError::UnknownPlugin(_))
    ));
    assert!(matches!(
        client(&registry).verify_detail(&id("acme.clock"), &other),
        Err(RegistryError::NoIndex)
    ));
}

#[test]
fn signed_but_inconsistent_details_are_rejected() {
    let mutations: [fn(&mut PluginDetail); 7] = [
        |d| d.id = id("evil.clock"),
        |d| d.versions.reverse(),
        |d| d.versions[0].package.url = "https://evil.example/x.dpplugin".into(),
        |d| {
            d.versions[0].package.url = release_url("acme.clock", "9.9.9");
        },
        |d| d.versions[0].package.size = 6 * 1024 * 1024,
        |d| {
            d.versions[1].yanked = Some(Yanked {
                at: t0(),
                reason: "latest is yanked".into(),
            });
        },
        |d| d.versions[0].permissions = vec![serde_json::json!({"kind": "filesystem"})],
    ];
    for (n, mutate) in mutations.iter().enumerate() {
        let mut registry = Registry::new();
        let spec = plugin("acme.clock", &["1.0.0", "1.1.0"]);
        let mut detail = registry.detail_doc(&spec, t0());
        mutate(&mut detail);
        let bytes = serde_json::to_vec_pretty(&detail).unwrap();
        let mut index = registry.index_doc(1, t0(), &[spec], &[]);
        let digest = deckpilot_plugin_package::sha256(&bytes);
        index.plugins[0].detail = FileRef {
            path: format!("plugins/acme.clock/{digest}.json"),
            sha256: digest,
            length: bytes.len() as u64,
        };
        registry.publish_index(&index, &[], t0(), &registry.index_key);
        registry.seq = 1;
        registry.sign_timestamp(t0());
        let mut c = client(&registry);
        c.refresh(&registry.site, t0()).unwrap();
        assert!(
            matches!(
                c.verify_detail(&id("acme.clock"), &bytes),
                Err(RegistryError::Malformed { .. } | RegistryError::DetailMismatch(_))
            ),
            "mutation {n}"
        );
    }
}

// ----------------------------------------------------------------- install

#[test]
fn install_end_to_end_through_the_package_reader() {
    let mut registry = Registry::new();
    let spec = plugin("acme.github-prs", &["1.0.0", "1.1.0"]);
    let served = spec.versions[1].built.bytes.clone();
    let older = spec.versions[0].built.bytes.clone();
    registry.publish(t0(), &[spec], &[]);
    let mut c = client(&registry);
    c.refresh(&registry.site, t0()).unwrap();
    let detail = detail_of(&registry, &c, "acme.github-prs");
    let target = c
        .resolve_install(&id("acme.github-prs"), &v("1.1.0"), &detail, None, t0())
        .unwrap();
    assert!(
        target
            .url
            .ends_with("acme.github-prs--v1.1.0/acme.github-prs-1.1.0.dpplugin")
    );
    assert_eq!(target.permissions.len(), 4);
    let limits = deckpilot_plugin_package::Limits::default();
    let verified =
        deckpilot_plugin_package::verify_package(&served, &target.expected, &limits).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("plugins/acme.github-prs/1.1.0");
    verified
        .unpack(&dir.path().join("plugins/.staging"), &plugin_dir)
        .unwrap();
    deckpilot_plugin_package::verify_installed(&plugin_dir, verified.integrity()).unwrap();
    // The previous version served instead is refused before unpacking.
    assert!(matches!(
        deckpilot_plugin_package::verify_package(&older, &target.expected, &limits),
        Err(deckpilot_plugin_package::PackageError::SizeMismatch { .. }
            | deckpilot_plugin_package::PackageError::HashMismatch { .. })
    ));
}

#[test]
fn install_checks_the_host() {
    use deckpilot_plugin_manifest::{ApiLevel, HostEnvironment, Platform};
    let mut registry = Registry::new();
    let mut spec = plugin("acme.clock", &["1.0.0"]);
    spec.versions[0].platforms = Some(vec![Platform::Macos]);
    let plugins = [spec];
    let mut index = registry.index_doc(1, t0(), &plugins, &[]);
    index.min_app_version = Some(v("0.8.0"));
    registry.publish_index(&index, &plugins, t0(), &registry.index_key);
    registry.seq = 1;
    registry.sign_timestamp(t0());
    let mut c = client(&registry);
    c.refresh(&registry.site, t0()).unwrap();
    let detail = detail_of(&registry, &c, "acme.clock");
    let host = |app: &str, platform| HostEnvironment {
        app_version: v(app),
        plugin_api: ApiLevel::new(1, 0),
        platform,
    };
    let resolve = |h: &HostEnvironment| {
        c.resolve_install(&id("acme.clock"), &v("1.0.0"), &detail, Some(h), t0())
    };
    assert!(resolve(&host("0.9.0", Platform::Macos)).is_ok());
    assert!(matches!(
        resolve(&host("0.7.0", Platform::Macos)),
        Err(InstallError::AppTooOld { .. })
    ));
    match resolve(&host("1.2.0", Platform::Windows)) {
        Err(InstallError::Incompatible { reasons, .. }) => assert_eq!(reasons.len(), 2),
        other => panic!("{other:?}"),
    }
    let old_api = HostEnvironment {
        plugin_api: ApiLevel::new(0, 9),
        ..host("0.9.0", Platform::Macos)
    };
    assert!(matches!(
        resolve(&old_api),
        Err(InstallError::Incompatible { .. })
    ));
}

#[test]
fn deprecated_plugins_can_still_be_installed() {
    let mut registry = Registry::new();
    let mut spec = plugin("acme.clock", &["1.0.0"]);
    spec.deprecated = true;
    registry.publish(t0(), &[spec], &[]);
    let mut c = client(&registry);
    c.refresh(&registry.site, t0()).unwrap();
    let detail = detail_of(&registry, &c, "acme.clock");
    let target = c
        .resolve_install(&id("acme.clock"), &v("1.0.0"), &detail, None, t0())
        .unwrap();
    assert_eq!(
        target.deprecated.unwrap().replacement,
        Some(id("acme.other"))
    );
}

// ----------------------------------------------------------------- offline

#[test]
fn offline_behaviour() {
    let registry = setup();

    // Never verified and offline: nothing to browse or install.
    registry.site.offline.set(true);
    let mut c = client(&registry);
    assert_eq!(
        c.refresh(&registry.site, t0()),
        Ok(Refresh::Offline(Freshness::NeverVerified))
    );
    assert_eq!(
        c.can_install(t0()).unwrap_err(),
        InstallError::NeverVerified
    );

    registry.site.offline.set(false);
    c.refresh(&registry.site, t0()).unwrap();

    // Offline, cache unexpired: browsing and installing from the verified
    // cache work (the package still has to come from somewhere, and is
    // checked against the cached pins).
    registry.site.offline.set(true);
    let t = t0() + days(3);
    let mut c = restart(&registry, &c);
    assert_eq!(
        c.refresh(&registry.site, t),
        Ok(Refresh::Offline(Freshness::Fresh))
    );
    assert!(c.can_install(t).is_ok());
    let detail = registry
        .site
        .files
        .borrow()
        .iter()
        .find(|(p, _)| p.starts_with("plugins/"))
        .map(|(_, b)| b.clone())
        .unwrap();
    assert!(
        c.resolve_install(&id("acme.clock"), &v("1.0.0"), &detail, None, t)
            .is_ok()
    );

    // Offline past expiry: stale; installs blocked; installed plugins run;
    // browsing still works from the cache.
    let t = t0() + days(8);
    assert_eq!(
        c.refresh(&registry.site, t),
        Ok(Refresh::Offline(Freshness::Stale(
            StaleReason::TimestampExpired
        )))
    );
    assert_eq!(
        c.can_install(t).unwrap_err(),
        InstallError::Stale(StaleReason::TimestampExpired)
    );
    assert!(matches!(
        c.resolve_install(&id("acme.clock"), &v("1.0.0"), &detail, None, t),
        Err(InstallError::Stale(_))
    ));
    assert!(c.verify_detail(&id("acme.clock"), &detail).is_ok());
    assert_eq!(
        c.installed_verdict(&id("acme.clock"), &v("1.0.0")),
        InstalledVerdict::Run
    );

    // Back online after timestamp.yml ran again: fresh.
    registry.site.offline.set(false);
    registry.sign_timestamp(t);
    assert_eq!(
        c.refresh(&registry.site, t),
        Ok(Refresh::Unchanged { seq: 1 })
    );
    assert_eq!(c.freshness(t), Freshness::Fresh);
}

/// Serves everything but indexes, which time out.
struct Flaky<'a>(&'a Site);

impl RegistrySource for Flaky<'_> {
    fn fetch(&self, path: &str, max: usize) -> Result<Vec<u8>, FetchError> {
        if path.starts_with("index/") {
            return Err(FetchError::Unavailable("timeout".into()));
        }
        self.0.fetch(path, max)
    }
}

#[test]
fn offline_part_way_keeps_the_stored_state() {
    let mut registry = setup();
    let mut c = client(&registry);
    c.refresh(&registry.site, t0()).unwrap();
    registry.publish(
        t0() + days(1),
        &[plugin("acme.clock", &["1.0.0", "2.0.0"])],
        &[],
    );
    // The timestamp arrives, the index does not.
    assert_eq!(
        c.refresh(&Flaky(&registry.site), t0() + days(1)),
        Ok(Refresh::Offline(Freshness::Fresh))
    );
    assert_eq!(c.state().seq, 1);
    registry.site.remove("index/2.json");
    // A missing index (404) is an error, not "offline".
    assert!(matches!(
        c.refresh(&registry.site, t0() + days(1)),
        Err(RegistryError::Fetch {
            error: FetchError::NotFound,
            ..
        })
    ));
}
