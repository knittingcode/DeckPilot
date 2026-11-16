//! User-entered origins (G2): the slot is a declared grant consented at
//! install; the exact origin is recorded per plugin and setting only with
//! the user's consent, within what the slot allows, and goes with a
//! revocation, a changed setting (the caller revokes it) and the plugin.

#![cfg(feature = "sqlcipher")]

mod common;

use common::{Env, grants_of, id, package_edited};
use deckpilot_plugin_manifest::HttpMethod;
use deckpilot_plugin_store::{
    Consent, GrantScope, InstallSource, PermissionDiff, PluginManager, StoreError, Tier,
    UpdateOutcome, UserOriginConsent, user_origin_key,
};
use serde_json::{Value, json};

const SETTING: &str = "defaultRepository";

/// The fixture's permissions with an origin slot in `defaultRepository`
/// (GET and POST; a private network if `private`).
fn permissions(private: bool) -> Vec<Value> {
    let mut slot = json!({"setting": SETTING, "pathPrefix": true});
    if private {
        slot["privateNetwork"] = json!(true);
    }
    vec![
        json!({"kind": "network", "hosts": ["api.github.com"], "methods": ["GET", "POST"],
               "userOrigins": [slot],
               "justification": "Reads pull requests from GitHub or your server."}),
        json!({"kind": "credentials", "refs": ["token"],
               "justification": "Sends your token to GitHub or your server only."}),
    ]
}

/// The token is also sent to the origin in the setting.
fn bind_token(manifest: &mut Value) {
    manifest["contributes"]["credentials"][0]["userOrigin"] = json!(SETTING);
}

fn install(manager: &mut PluginManager, private: bool) {
    let package = package_edited("1.0.0", &permissions(private), bind_token);
    manager
        .install(
            &package,
            &InstallSource::Sideload,
            &Consent::all(&grants_of(&package), "user"),
        )
        .unwrap();
}

fn consent(origin: &str) -> UserOriginConsent {
    UserOriginConsent {
        setting: SETTING.into(),
        origin: origin.into(),
        path_prefix: Some("/api/v3".into()),
        methods: vec![HttpMethod::Get],
        private_network: false,
        credentials: vec!["token".into()],
    }
}

#[test]
fn the_slot_is_a_declared_grant_of_its_own_tier() {
    for (private, tier) in [(false, Tier::Medium), (true, Tier::High)] {
        let package = package_edited("1.0.0", &permissions(private), bind_token);
        let grants = grants_of(&package);
        let scope = grants.get(&user_origin_key(SETTING)).unwrap();
        assert_eq!(
            *scope,
            GrantScope::UserOriginSlot {
                methods: vec![HttpMethod::Get, HttpMethod::Post],
                path_prefix: true,
                private_network: private,
            }
        );
        assert_eq!(scope.tier(), tier);
        assert_eq!(
            user_origin_key(SETTING).to_string(),
            "network:user-origin:defaultRepository"
        );
        // The credential records its binding, so binding it later is an
        // escalation.
        assert!(matches!(
            grants.get(&"credentials:token".parse().unwrap()),
            Some(GrantScope::Credential { user_origin: Some(s), .. }) if s == SETTING
        ));
    }
}

#[test]
fn an_origin_is_recorded_only_with_the_users_consent_within_the_slot() {
    let env = Env::new();
    let mut manager = env.open();
    install(&mut manager, false);
    assert!(manager.store().user_origins(&id()).unwrap().is_empty());

    let ok = consent("https://ghe.example.com");
    manager.grant_user_origin(&id(), &ok, "user").unwrap();
    let rows = manager.store().user_origins(&id()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].consent, ok);
    assert_eq!(rows[0].decided_by, "user");
    assert_eq!(rows[0].granted_for_version, "1.0.0");
    let audit = manager.store().audit(&id()).unwrap();
    let last = audit.last().unwrap();
    assert_eq!(last.op, "grant");
    assert_eq!(
        last.target.as_deref(),
        Some("network:user-origin:defaultRepository")
    );
    assert_eq!(
        last.reason.as_deref(),
        Some("https://ghe.example.com/api/v3")
    );

    // Anything the slot does not allow is refused and records nothing.
    let refused: Vec<UserOriginConsent> = vec![
        UserOriginConsent {
            setting: "pollMinutes".into(),
            ..ok.clone()
        },
        UserOriginConsent {
            methods: vec![HttpMethod::Delete],
            ..ok.clone()
        },
        UserOriginConsent {
            methods: vec![],
            ..ok.clone()
        },
        UserOriginConsent {
            private_network: true,
            ..ok.clone()
        },
        UserOriginConsent {
            credentials: vec!["other".into()],
            ..ok.clone()
        },
        UserOriginConsent {
            origin: "http://ghe.example.com".into(),
            ..ok.clone()
        },
        UserOriginConsent {
            origin: "https://ghe.example.com/api".into(),
            ..ok.clone()
        },
        UserOriginConsent {
            origin: "https://u@ghe.example.com".into(),
            ..ok.clone()
        },
        UserOriginConsent {
            path_prefix: Some("/api/v3/".into()),
            ..ok.clone()
        },
        UserOriginConsent {
            path_prefix: Some("api".into()),
            ..ok.clone()
        },
    ];
    for bad in refused {
        let other = UserOriginConsent {
            origin: bad.origin.replace("ghe.", "other."),
            ..bad
        };
        assert!(
            matches!(
                manager.grant_user_origin(&id(), &other, "user"),
                Err(StoreError::InvalidData(_))
            ),
            "{other:?}"
        );
    }
    assert_eq!(manager.store().user_origins(&id()).unwrap()[0].consent, ok);
}

#[test]
fn a_private_network_is_only_what_the_slot_allows_and_the_user_said() {
    let env = Env::new();
    let mut manager = env.open();
    install(&mut manager, true);
    let private = UserOriginConsent {
        private_network: true,
        ..consent("https://10.0.0.5")
    };
    manager.grant_user_origin(&id(), &private, "user").unwrap();
    let rows = manager.store().user_origins(&id()).unwrap();
    assert!(rows[0].consent.private_network);
    assert!(
        manager
            .store()
            .audit(&id())
            .unwrap()
            .last()
            .unwrap()
            .reason
            .as_deref()
            .unwrap()
            .ends_with("(private network)")
    );
}

#[test]
fn without_the_slot_grant_no_origin_is_recorded() {
    let env = Env::new();
    let mut manager = env.open();
    install(&mut manager, false);
    manager
        .revoke(&id(), &user_origin_key(SETTING), "user")
        .unwrap();
    let result = manager.grant_user_origin(&id(), &consent("https://ghe.example.com"), "user");
    assert!(
        matches!(&result, Err(StoreError::ConsentRequired { missing, .. })
            if missing == &[user_origin_key(SETTING)]),
        "{result:?}"
    );
    assert!(manager.store().user_origins(&id()).unwrap().is_empty());
}

#[test]
fn a_revocation_or_a_changed_setting_deletes_it_and_so_does_an_uninstall() {
    let env = Env::new();
    let mut manager = env.open();
    install(&mut manager, false);
    manager
        .grant_user_origin(&id(), &consent("https://ghe.example.com"), "user")
        .unwrap();
    // A new origin for the same setting replaces the old one.
    manager
        .grant_user_origin(&id(), &consent("https://ghe2.example.com"), "user")
        .unwrap();
    let rows = manager.store().user_origins(&id()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].consent.origin, "https://ghe2.example.com");

    assert_eq!(
        manager
            .revoke_user_origin(&id(), SETTING, "the setting changed")
            .unwrap()
            .as_deref(),
        Some("https://ghe2.example.com/api/v3")
    );
    assert!(manager.store().user_origins(&id()).unwrap().is_empty());
    let last = manager.store().audit(&id()).unwrap().pop().unwrap();
    assert_eq!(
        (last.op.as_str(), last.decision.as_str()),
        ("revoke", "revoked")
    );
    assert_eq!(
        last.reason.as_deref(),
        Some("https://ghe2.example.com/api/v3: the setting changed")
    );
    // Nothing left to revoke.
    assert_eq!(
        manager
            .revoke_user_origin(&id(), SETTING, "revoked")
            .unwrap(),
        None
    );

    manager
        .grant_user_origin(&id(), &consent("https://ghe.example.com"), "user")
        .unwrap();
    manager.uninstall(&id()).unwrap();
    install(&mut manager, false);
    assert!(manager.store().user_origins(&id()).unwrap().is_empty());
}

#[test]
fn allowing_a_private_network_in_an_update_is_a_high_tier_escalation() {
    let env = Env::new();
    let mut manager = env.open();
    install(&mut manager, false);
    let update = package_edited("1.1.0", &permissions(true), bind_token);
    let outcome = manager.update(&update, &InstallSource::Sideload).unwrap();
    let UpdateOutcome::AwaitingConsent { diff, .. } = outcome else {
        panic!("{outcome:?}")
    };
    let PermissionDiff { broadened, .. } = &diff;
    assert_eq!(broadened.len(), 1);
    assert_eq!(broadened[0].key, user_origin_key(SETTING));
    assert_eq!(diff.highest_tier(), Some(Tier::High));

    // Binding the token to the origin setting in an update is one too.
    let env = Env::new();
    let mut manager = env.open();
    let package = package_edited("1.0.0", &permissions(false), |_| {});
    manager
        .install(
            &package,
            &InstallSource::Sideload,
            &Consent::all(&grants_of(&package), "user"),
        )
        .unwrap();
    let update = package_edited("1.1.0", &permissions(false), bind_token);
    let outcome = manager.update(&update, &InstallSource::Sideload).unwrap();
    let UpdateOutcome::AwaitingConsent { diff, .. } = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(diff.escalations(), ["credentials:token".parse().unwrap()]);
}
