//! Spike 5: static registry trust.

mod common;

use common::*;
use marketplace_trust_spike::client::*;
use marketplace_trust_spike::registry::RevocationAction;
use marketplace_trust_spike::sig::{self, SigError};
use marketplace_trust_spike::signer::ThrowawayKey;
use time::Duration;

fn client(reg: &Registry, dir: &tempfile::TempDir) -> Client {
    Client::open(reg.embedded_roots(), dir.path().join("trust-state.json")).unwrap()
}

fn setup() -> (Registry, tempfile::TempDir) {
    let mut reg = Registry::new();
    reg.publish(T0, &[plugin("acme.clock", &["1.0.0"])], &[]);
    (reg, tempfile::tempdir().unwrap())
}

// ---------------------------------------------------------------- basics

#[test]
fn valid_index_is_accepted_and_persisted() {
    let (reg, dir) = setup();
    let mut c = client(&reg, &dir);
    assert_eq!(c.freshness(T0), Freshness::NeverVerified);
    assert_eq!(c.refresh(&reg.site, T0), Ok(Refresh::Updated { seq: 1 }));
    assert_eq!(c.freshness(T0), Freshness::Fresh);
    assert_eq!(c.refresh(&reg.site, T0), Ok(Refresh::Unchanged { seq: 1 }));
    // Next run: state comes back from the JSON file.
    let c2 = client(&reg, &dir);
    assert_eq!(c2.state, c.state);
    assert_eq!(c2.state.seq, 1);
    assert_eq!(c2.keys_version(), 1);
    let size = std::fs::metadata(dir.path().join("trust-state.json"))
        .unwrap()
        .len();
    assert!(size < 16 * 1024, "state file is {size} bytes");
}

#[test]
fn tampered_index_cache_on_disk_is_not_trusted() {
    let mut reg = Registry::new();
    reg.publish(
        T0,
        &[plugin("acme.clock", &["1.0.0"])],
        &[revocation("acme.clock", None, RevocationAction::Disable)],
    );
    let dir = tempfile::tempdir().unwrap();
    client(&reg, &dir).refresh(&reg.site, T0).unwrap();
    let cache = dir.path().join("trust-state.index-cache.json");
    let mut bytes = std::fs::read(&cache).unwrap();
    bytes[10] ^= 1;
    std::fs::write(&cache, bytes).unwrap();
    // Offline: the cache does not match the stored timestamp, so there is
    // no index to browse or install from, but revocations (in the trust
    // state) are still enforced.
    reg.site.offline.set(true);
    let mut c = client(&reg, &dir);
    assert!(c.index.is_none());
    assert_eq!(
        c.refresh(&reg.site, T0),
        Ok(Refresh::Offline(Freshness::NeverVerified))
    );
    assert!(matches!(
        c.installed_verdict("acme.clock", "1.0.0"),
        InstalledVerdict::Disable { .. }
    ));
    // Online again: refetched and verified.
    reg.site.offline.set(false);
    assert_eq!(c.refresh(&reg.site, T0), Ok(Refresh::Updated { seq: 1 }));
    assert!(client(&reg, &dir).index.is_some());
}

#[test]
fn new_publish_and_daily_timestamp_are_followed() {
    let (mut reg, dir) = setup();
    let mut c = client(&reg, &dir);
    c.refresh(&reg.site, T0).unwrap();
    let t1 = T0 + Duration::days(3);
    reg.publish(t1, &[plugin("acme.clock", &["1.0.0", "1.1.0"])], &[]);
    assert_eq!(c.refresh(&reg.site, t1), Ok(Refresh::Updated { seq: 2 }));
    // timestamp.yml re-signs daily: stays fresh far past the first expiry.
    for day in 4..60 {
        let t = T0 + Duration::days(day);
        reg.sign_timestamp(t);
        assert_eq!(
            c.refresh(&reg.site, t),
            Ok(Refresh::Unchanged { seq: 2 }),
            "day {day}"
        );
    }
}

#[test]
fn missing_key_set_is_rejected() {
    let (reg, dir) = setup();
    reg.site.files.borrow_mut().remove("keys/1.json");
    let mut c = client(&reg, &dir);
    assert_eq!(c.refresh(&reg.site, T0), Err(TrustError::NoKeyset));
}

#[test]
fn oversized_documents_are_rejected_before_parsing() {
    let (reg, dir) = setup();
    let mut c = client(&reg, &dir);
    let mut big = reg.site.get_bytes("timestamp.json");
    big.extend(std::iter::repeat_n(b' ', MAX_TIMESTAMP));
    reg.site.put("timestamp.json", big);
    assert_eq!(
        c.refresh(&reg.site, T0),
        Err(TrustError::TooLarge("timestamp.json".into()))
    );
}

// ---------------------------------------------------------------- expiry

#[test]
fn expired_timestamp_is_rejected() {
    let (reg, dir) = setup();
    let mut c = client(&reg, &dir);
    let late = T0 + Duration::days(8);
    assert!(matches!(
        c.refresh(&reg.site, late),
        Err(TrustError::TimestampExpired(_))
    ));
    assert_eq!(c.freshness(late), Freshness::NeverVerified);
    assert_eq!(
        c.can_install(late).unwrap_err(),
        InstallError::NeverVerified
    );
}

#[test]
fn clock_skew_allowance_is_five_minutes() {
    let (reg, _) = setup();
    let expires = T0 + Duration::days(7);
    for (offset, ok) in [(Duration::minutes(4), true), (Duration::minutes(6), false)] {
        let dir = tempfile::tempdir().unwrap();
        let mut c = client(&reg, &dir);
        assert_eq!(
            c.refresh(&reg.site, expires + offset).is_ok(),
            ok,
            "{offset}"
        );
    }
}

#[test]
fn expired_index_is_rejected_even_with_a_fresh_timestamp() {
    // Only timestamp.yml ran for 91 days: the 90-day index expires.
    let (reg, dir) = setup();
    let mut c = client(&reg, &dir);
    c.refresh(&reg.site, T0).unwrap();
    let t = T0 + Duration::days(91);
    reg.sign_timestamp(t);
    assert!(matches!(
        c.refresh(&reg.site, t),
        Err(TrustError::IndexExpired(_))
    ));
    assert_eq!(
        c.freshness(t),
        Freshness::Stale(StaleReason::TimestampExpired)
    );
    // A fresh client hits the same wall.
    let dir2 = tempfile::tempdir().unwrap();
    assert!(matches!(
        client(&reg, &dir2).refresh(&reg.site, t),
        Err(TrustError::IndexExpired(_))
    ));
}

// ---------------------------------------------------------------- rollback

#[test]
fn rollback_to_a_lower_seq_is_rejected_across_runs() {
    let (mut reg, dir) = setup();
    let old = reg.site.snapshot();
    reg.publish(
        T0 + Duration::hours(1),
        &[plugin("acme.clock", &["1.0.0", "1.1.0"])],
        &[],
    );
    {
        let mut c = client(&reg, &dir);
        assert_eq!(
            c.refresh(&reg.site, T0 + Duration::hours(2)),
            Ok(Refresh::Updated { seq: 2 })
        );
    }
    // Restart; a mirror serves the old (still unexpired, validly signed) seq 1.
    reg.site.replay_timestamp(&old);
    let mut c = client(&reg, &dir);
    let before = c.state.clone();
    assert_eq!(
        c.refresh(&reg.site, T0 + Duration::hours(3)),
        Err(TrustError::Rollback {
            stored: 2,
            offered: 1
        })
    );
    assert_eq!(c.state, before, "state untouched");
    assert_eq!(client(&reg, &dir).state.seq, 2, "persisted state untouched");
}

#[test]
fn a_clock_in_the_past_cannot_revive_an_old_index() {
    let (mut reg, dir) = setup();
    let old = reg.site.snapshot();
    reg.publish(
        T0 + Duration::days(2),
        &[plugin("acme.clock", &["1.0.0", "1.1.0"])],
        &[],
    );
    client(&reg, &dir)
        .refresh(&reg.site, T0 + Duration::days(2))
        .unwrap();
    reg.site.replay_timestamp(&old);
    // The clock is set back so that the old timestamp looks valid.
    assert_eq!(
        client(&reg, &dir).refresh(&reg.site, T0),
        Err(TrustError::Rollback {
            stored: 2,
            offered: 1
        })
    );
}

#[test]
fn two_different_indexes_for_one_seq_are_rejected() {
    let (reg, dir) = setup();
    let mut c = client(&reg, &dir);
    c.refresh(&reg.site, T0).unwrap();
    let plugins = [plugin("acme.clock", &["1.0.0", "6.6.6"])];
    let index = reg.index_doc(1, T0, &plugins, &[]);
    reg.publish_index(&index, &plugins, T0, &reg.index_key);
    reg.sign_timestamp(T0 + Duration::hours(1));
    assert_eq!(
        c.refresh(&reg.site, T0 + Duration::hours(1)),
        Err(TrustError::Equivocation(1))
    );
}

// ---------------------------------------------------------------- freeze

#[test]
fn freeze_attack_is_detected_at_timestamp_expiry() {
    let (mut reg, dir) = setup();
    let mut c = client(&reg, &dir);
    c.refresh(&reg.site, T0).unwrap();
    let frozen = reg.site.snapshot();
    // The registry moves on (e.g. revokes something); the attacker keeps
    // serving the seq-1 timestamp it captured.
    let t1 = T0 + Duration::days(1);
    reg.publish(
        t1,
        &[plugin("acme.clock", &["1.0.0"])],
        &[revocation(
            "acme.clock",
            Some(&["1.0.0"]),
            RevocationAction::Disable,
        )],
    );
    reg.site.replay_timestamp(&frozen);
    // Inside the 7-day window the freeze cannot be told apart from "no news".
    assert_eq!(c.refresh(&reg.site, t1), Ok(Refresh::Unchanged { seq: 1 }));
    assert_eq!(
        c.installed_verdict("acme.clock", "1.0.0"),
        InstalledVerdict::Run
    );
    // After expiry + skew it is: refused, state goes stale, installs stop.
    let t2 = T0 + Duration::days(7) + Duration::minutes(6);
    assert!(matches!(
        c.refresh(&reg.site, t2),
        Err(TrustError::TimestampExpired(_))
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
    reg.sign_timestamp(t2);
    assert_eq!(c.refresh(&reg.site, t2), Ok(Refresh::Updated { seq: 2 }));
    assert!(matches!(
        c.installed_verdict("acme.clock", "1.0.0"),
        InstalledVerdict::Disable { .. }
    ));
}

// ---------------------------------------------------------------- keys

#[test]
fn index_signed_by_a_non_authorized_key_is_rejected() {
    let (reg, _) = setup();
    let stranger = ThrowawayKey::generate();
    let plugins = [plugin("acme.clock", &["1.0.0"])];
    let index = reg.index_doc(1, T0, &plugins, &[]);
    for (name, key) in [
        ("stranger", &stranger),
        ("timestamp key", &reg.timestamp_key),
        ("root key", &reg.roots[0]),
    ] {
        reg.publish_index(&index, &plugins, T0, key);
        reg.sign_timestamp(T0);
        let dir = tempfile::tempdir().unwrap();
        let mut c = client(&reg, &dir);
        assert_eq!(
            c.refresh(&reg.site, T0),
            Err(TrustError::IndexSignature(SigError::BelowThreshold {
                valid: 0,
                threshold: 1
            })),
            "{name}"
        );
        assert_eq!(c.freshness(T0), Freshness::NeverVerified);
    }
}

#[test]
fn timestamp_signed_by_another_role_is_rejected() {
    let (reg, dir) = setup();
    reg.sign_timestamp_with(T0, 1, 1, &reg.index_key);
    assert!(matches!(
        client(&reg, &dir).refresh(&reg.site, T0),
        Err(TrustError::TimestampSignature(_))
    ));
}

#[test]
fn signatures_with_another_comment_are_rejected() {
    let (reg, _) = setup();
    let path = "index/1.json";
    let bytes = reg.site.get_bytes(path);
    for comment in [
        // Another seq, another key set, another role, the updater's format.
        sig::seq_comment("index", 2, 1),
        sig::seq_comment("index", 1, 2),
        sig::seq_comment("timestamp", 1, 1),
        "timestamp:1700000000\tfile:index.json\tversion:1.0.0".to_owned(),
    ] {
        reg.site.put(
            &format!("{path}.minisig"),
            reg.index_key.sign(&bytes, &comment),
        );
        let dir = tempfile::tempdir().unwrap();
        assert!(
            matches!(
                client(&reg, &dir).refresh(&reg.site, T0),
                Err(TrustError::IndexSignature(SigError::WrongComment { .. }))
            ),
            "{comment}"
        );
    }
    // Non-canonical seq in a timestamp comment.
    let ts = reg.site.get_bytes("timestamp.json");
    reg.site.put(
        "timestamp.json.minisig",
        reg.timestamp_key
            .sign(&ts, "deckpilot-registry v1 role:timestamp seq:01 keys:1"),
    );
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        client(&reg, &dir).refresh(&reg.site, T0),
        Err(TrustError::TimestampSignature(SigError::UnparsableComment(
            _
        )))
    ));
}

#[test]
fn tampered_documents_are_rejected() {
    let (reg, _) = setup();
    for path in ["keys/1.json", "timestamp.json", "index/1.json"] {
        let original = reg.site.get_bytes(path);
        let mut tampered = original.clone();
        let at = tampered.iter().position(|&b| b == b'1').unwrap();
        tampered[at] = b'2';
        reg.site.put(path, tampered);
        let dir = tempfile::tempdir().unwrap();
        let r = client(&reg, &dir).refresh(&reg.site, T0);
        assert!(r.is_err(), "{path}: {r:?}");
        reg.site.put(path, original);
    }
    let dir = tempfile::tempdir().unwrap();
    assert!(client(&reg, &dir).refresh(&reg.site, T0).is_ok());
}

#[test]
fn index_key_rotation_via_root_signed_key_set() {
    let (mut reg, dir) = setup();
    let mut c = client(&reg, &dir);
    c.refresh(&reg.site, T0).unwrap();
    let old_index_key = std::mem::replace(&mut reg.index_key, ThrowawayKey::generate());
    let root = reg.root_role();
    let doc = reg.keyset_doc(&root, 0, false);
    reg.publish_keyset(&doc, &[&reg.roots[1]]); // either root suffices (1-of-2)
    let t = T0 + Duration::days(1);
    reg.publish(t, &[plugin("acme.clock", &["1.0.0"])], &[]);
    assert_eq!(c.refresh(&reg.site, t), Ok(Refresh::Updated { seq: 2 }));
    assert_eq!(c.keys_version(), 2);
    // The retired index key can no longer sign, even with a correct comment.
    let plugins = [plugin("acme.clock", &["1.0.0", "9.9.9"])];
    let index = reg.index_doc(3, t, &plugins, &[]);
    reg.publish_index(&index, &plugins, t, &old_index_key);
    reg.seq = 3;
    reg.sign_timestamp(t);
    assert!(matches!(
        c.refresh(&reg.site, t),
        Err(TrustError::IndexSignature(_))
    ));
    // A timestamp still naming key set 1 is refused after the rotation.
    reg.sign_timestamp_with(t, 2, 1, &reg.timestamp_key);
    assert_eq!(
        c.refresh(&reg.site, t),
        Err(TrustError::KeysVersion {
            doc: "timestamp",
            found: 1,
            current: 2
        })
    );
}

#[test]
fn key_set_signed_by_a_non_root_key_is_rejected() {
    let (mut reg, dir) = setup();
    let root = reg.root_role();
    let doc = reg.keyset_doc(&root, 0, false);
    let index_key = std::mem::replace(&mut reg.index_key, ThrowawayKey::generate());
    reg.publish_keyset(&doc, &[&index_key]);
    let mut c = client(&reg, &dir);
    assert!(matches!(
        c.refresh(&reg.site, T0),
        Err(TrustError::KeysetSignature(2, _))
    ));
    assert_eq!(c.keys_version(), 1, "keys/1 was accepted, keys/2 was not");
}

#[test]
fn root_rotation_needs_old_and_new_roots() {
    let (mut reg, dir) = setup();
    let mut c = client(&reg, &dir);
    c.refresh(&reg.site, T0).unwrap();
    let new_roots = [ThrowawayKey::generate(), ThrowawayKey::generate()];
    let new_root_role = role(&[&new_roots[0], &new_roots[1]], 1);
    let doc = reg.keyset_doc(&new_root_role, 0, false);

    // Old root only: the new roots never agreed.
    reg.publish_keyset(&doc, &[&reg.roots[0]]);
    reg.keys_version.set(1);
    assert!(matches!(
        c.refresh(&reg.site, T0),
        Err(TrustError::KeysetSignature(2, _))
    ));
    // New root only: not authorized by the trusted (old) roots.
    reg.publish_keyset(&doc, &[&new_roots[0]]);
    reg.keys_version.set(1);
    assert!(matches!(
        c.refresh(&reg.site, T0),
        Err(TrustError::KeysetSignature(2, _))
    ));
    // Both: accepted.
    reg.publish_keyset(&doc, &[&reg.roots[0], &new_roots[0]]);
    let t = T0 + Duration::hours(1);
    reg.publish(t, &[plugin("acme.clock", &["1.0.0"])], &[]);
    assert_eq!(c.refresh(&reg.site, t), Ok(Refresh::Updated { seq: 2 }));
    assert_eq!(c.state.keyset.as_ref().unwrap().root, new_root_role);

    // From now on the old roots are powerless.
    let old_roots = std::mem::replace(&mut reg.roots, new_roots.into());
    let attacker_doc = reg.keyset_doc(&role(&[&old_roots[0]], 1), 0, false);
    reg.publish_keyset(&attacker_doc, &[&old_roots[0], &old_roots[1]]);
    assert!(matches!(
        c.refresh(&reg.site, t),
        Err(TrustError::KeysetSignature(3, _))
    ));
}

#[test]
fn two_of_two_root_threshold() {
    // U-10 alternative: threshold 2 of 2 root keys.
    let mut reg = Registry::new();
    reg.root_threshold = 2;
    reg.keys_version.set(0);
    let root = reg.root_role();
    let doc = reg.keyset_doc(&root, 0, false);
    reg.keys_version.set(1);
    reg.publish(T0, &[plugin("acme.clock", &["1.0.0"])], &[]);
    for (name, signers, ok) in [
        ("one root", vec![&reg.roots[0]], false),
        ("same root twice", vec![&reg.roots[0], &reg.roots[0]], false),
        ("both roots", vec![&reg.roots[0], &reg.roots[1]], true),
    ] {
        let bytes = serde_json::to_vec_pretty(&doc).unwrap();
        let sigs: Vec<String> = signers
            .iter()
            .map(|k| k.sign(&bytes, &sig::keys_comment(1)))
            .collect();
        reg.site.put("keys/1.json", bytes);
        reg.site
            .put("keys/1.json.minisig", serde_json::to_vec(&sigs).unwrap());
        let dir = tempfile::tempdir().unwrap();
        let r = client(&reg, &dir).refresh(&reg.site, T0);
        if ok {
            assert_eq!(r, Ok(Refresh::Updated { seq: 1 }), "{name}");
        } else {
            assert_eq!(
                r,
                Err(TrustError::KeysetSignature(
                    1,
                    SigError::BelowThreshold {
                        valid: 1,
                        threshold: 2
                    }
                )),
                "{name}"
            );
        }
    }
}

#[test]
fn seq_floor_reset_recovers_from_an_inflated_seq() {
    let (mut reg, dir) = setup();
    let mut c = client(&reg, &dir);
    // A compromised index + timestamp key pushed seq to 1,000,000.
    reg.seq = 999_999;
    reg.publish(T0, &[plugin("acme.clock", &["1.0.0"])], &[]);
    assert_eq!(
        c.refresh(&reg.site, T0),
        Ok(Refresh::Updated { seq: 1_000_000 })
    );

    // Recovery without the reset flag: the real registry at seq 5 is a rollback.
    let t = T0 + Duration::hours(1);
    reg.index_key = ThrowawayKey::generate();
    reg.timestamp_key = ThrowawayKey::generate();
    let root = reg.root_role();
    let mut doc = reg.keyset_doc(&root, 5, false);
    reg.publish_keyset(&doc, &[&reg.roots[0]]);
    reg.seq = 4;
    reg.publish(t, &[plugin("acme.clock", &["1.0.0"])], &[]);
    let dir2 = tempfile::tempdir().unwrap();
    std::fs::copy(
        dir.path().join("trust-state.json"),
        dir2.path().join("trust-state.json"),
    )
    .unwrap();
    assert_eq!(
        client(&reg, &dir2).refresh(&reg.site, t),
        Err(TrustError::Rollback {
            stored: 1_000_000,
            offered: 5
        })
    );

    // With seq_floor_reset the client accepts the re-signed content.
    doc.seq_floor_reset = true;
    reg.keys_version.set(1);
    reg.publish_keyset(&doc, &[&reg.roots[0]]);
    reg.seq = 4;
    reg.publish(t, &[plugin("acme.clock", &["1.0.0"])], &[]);
    assert_eq!(c.refresh(&reg.site, t), Ok(Refresh::Updated { seq: 5 }));
    // ... and seq_floor itself is a floor.
    reg.seq = 3;
    reg.publish(t, &[plugin("acme.clock", &["1.0.0"])], &[]);
    let dir3 = tempfile::tempdir().unwrap();
    let mut fresh = client(&reg, &dir3);
    assert_eq!(
        fresh.refresh(&reg.site, t),
        Err(TrustError::Rollback {
            stored: 5,
            offered: 4
        })
    );
}

#[test]
fn foreign_download_prefix_is_rejected_even_if_signed() {
    let (reg, dir) = setup();
    let plugins = [plugin("acme.clock", &["1.0.0"])];
    let mut index = reg.index_doc(1, T0, &plugins, &[]);
    index.download_prefix = "https://github.com/evil/releases/download/".into();
    reg.publish_index(&index, &plugins, T0, &reg.index_key);
    reg.sign_timestamp(T0);
    assert!(matches!(
        client(&reg, &dir).refresh(&reg.site, T0),
        Err(TrustError::BadDownloadPrefix(_))
    ));
}

// ---------------------------------------------------------------- revocation

fn revoked_registry() -> (Registry, tempfile::TempDir, Client) {
    let mut reg = Registry::new();
    let mut spec = plugin("acme.clock", &["1.0.0", "1.1.0", "1.2.0", "1.3.0"]);
    spec.versions[3].yanked = Some("broken on Windows".into());
    reg.publish(
        T0,
        &[spec, plugin("evil.miner", &["0.1.0", "0.2.0"])],
        &[
            revocation("acme.clock", Some(&["1.0.0"]), RevocationAction::Disable),
            revocation("acme.clock", Some(&["1.1.0"]), RevocationAction::Yank),
            revocation("evil.miner", None, RevocationAction::Disable),
        ],
    );
    let dir = tempfile::tempdir().unwrap();
    let mut c = client(&reg, &dir);
    c.refresh(&reg.site, T0).unwrap();
    (reg, dir, c)
}

#[test]
fn revoked_versions_are_refused_for_install() {
    let (reg, _dir, c) = revoked_registry();
    let install = |id: &str, v: &str| c.resolve_install(&reg.site, id, v, T0);
    assert!(matches!(
        install("acme.clock", "1.0.0"),
        Err(InstallError::Revoked(..))
    ));
    assert!(matches!(
        install("acme.clock", "1.1.0"),
        Err(InstallError::Yanked(..))
    ));
    assert!(matches!(
        install("acme.clock", "1.3.0"),
        Err(InstallError::Yanked(..))
    ));
    assert!(matches!(
        install("evil.miner", "0.2.0"),
        Err(InstallError::Revoked(..))
    ));
    assert!(matches!(
        install("acme.clock", "9.0.0"),
        Err(InstallError::UnknownVersion(_))
    ));
    assert!(matches!(
        install("acme.other", "1.0.0"),
        Err(InstallError::UnknownPlugin(_))
    ));
    let ok = install("acme.clock", "1.2.0").unwrap();
    assert_eq!(ok.url, release_url("acme.clock", "1.2.0"));
}

#[test]
fn revoked_installed_versions_are_flagged_also_offline_and_stale() {
    let (reg, dir, c) = revoked_registry();
    let check = |c: &Client| {
        assert!(matches!(
            c.installed_verdict("acme.clock", "1.0.0"),
            InstalledVerdict::Disable {
                advisory: Some(_),
                ..
            }
        ));
        assert!(matches!(
            c.installed_verdict("acme.clock", "1.1.0"),
            InstalledVerdict::RunWithNotice { .. }
        ));
        assert_eq!(
            c.installed_verdict("acme.clock", "1.2.0"),
            InstalledVerdict::Run
        );
        assert!(matches!(
            c.installed_verdict("evil.miner", "0.1.0"),
            InstalledVerdict::Disable { .. }
        ));
    };
    check(&c);
    // Next start, a month later, offline: stale, revocations still enforced.
    reg.site.offline.set(true);
    let mut c = client(&reg, &dir);
    let late = T0 + Duration::days(30);
    assert_eq!(
        c.refresh(&reg.site, late),
        Ok(Refresh::Offline(Freshness::Stale(
            StaleReason::TimestampExpired
        )))
    );
    check(&c);
}

#[test]
fn dropping_a_revocation_is_rejected() {
    let (mut reg, _dir, mut c) = revoked_registry();
    let t = T0 + Duration::days(1);
    reg.publish(t, &[plugin("acme.clock", &["1.0.0", "1.2.0"])], &[]);
    assert!(matches!(
        c.refresh(&reg.site, t),
        Err(TrustError::RevocationDropped(_))
    ));
    assert!(matches!(
        c.installed_verdict("acme.clock", "1.0.0"),
        InstalledVerdict::Disable { .. }
    ));
}

#[test]
fn tampered_detail_file_is_rejected() {
    let (reg, _dir, c) = revoked_registry();
    let path = c.index.as_ref().unwrap().plugins[0].detail.path.clone();
    let mut bytes = reg.site.get_bytes(&path);
    let at = bytes.len() / 2;
    bytes[at] ^= 1;
    reg.site.put(&path, bytes);
    assert!(matches!(
        c.resolve_install(&reg.site, "acme.clock", "1.2.0", T0),
        Err(InstallError::Detail(_))
    ));
}

// ---------------------------------------------------------------- offline

#[test]
fn offline_behavior() {
    let (reg, dir) = setup();

    // Never verified and offline: nothing to browse or install.
    reg.site.offline.set(true);
    let mut c = client(&reg, &dir);
    assert_eq!(
        c.refresh(&reg.site, T0),
        Ok(Refresh::Offline(Freshness::NeverVerified))
    );
    assert_eq!(c.can_install(T0).unwrap_err(), InstallError::NeverVerified);

    reg.site.offline.set(false);
    c.refresh(&reg.site, T0).unwrap();

    // Offline, cache still valid: browsing and installing from the verified
    // cache are allowed (the download itself needs the network or a staged
    // package; its bytes are checked against the cached pins).
    reg.site.offline.set(true);
    let t = T0 + Duration::days(3);
    let mut c = client(&reg, &dir);
    assert_eq!(
        c.refresh(&reg.site, t),
        Ok(Refresh::Offline(Freshness::Fresh))
    );
    assert!(c.can_install(t).is_ok());

    // Offline past expiry: stale; installs and updates blocked; installed
    // plugins keep running (verdict from stored revocations).
    let t = T0 + Duration::days(8);
    assert_eq!(
        c.refresh(&reg.site, t),
        Ok(Refresh::Offline(Freshness::Stale(
            StaleReason::TimestampExpired
        )))
    );
    assert_eq!(
        c.can_install(t).unwrap_err(),
        InstallError::Stale(StaleReason::TimestampExpired)
    );
    assert_eq!(
        c.installed_verdict("acme.clock", "1.0.0"),
        InstalledVerdict::Run
    );

    // Back online after timestamp.yml ran again: fresh.
    reg.site.offline.set(false);
    reg.sign_timestamp(t);
    assert_eq!(c.refresh(&reg.site, t), Ok(Refresh::Unchanged { seq: 1 }));
    assert_eq!(c.freshness(t), Freshness::Fresh);
}
