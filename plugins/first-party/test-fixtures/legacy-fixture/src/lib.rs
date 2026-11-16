//! TEST FIXTURE, never shipped: a migration-class bundled first-party
//! plugin for the module migration tests (`plugins/first-party/README.md`,
//! `docs/marketplace/module-migration.md` §5.6).
//!
//! It replaces the agent's fixture legacy module `legacy-fixture` (feature
//! `test-hooks`): one action, `whoami`, with the legacy action's
//! parameters (but its `credential` override, which the alias drops) and
//! outputs. It asks the fixture API who the stored token belongs to, with
//! the token of the credential slot `token` when there is one, and greets
//! with the `greeting` setting the migration copied from the built-in.
//! Its permissions cover each consent tier: `notifications` (low),
//! `network` for the API host (medium, derived from the built-in's use),
//! and the credential slot (high, only with the user's consent).
#![deny(unsafe_code)]

use deckpilot_plugin_sdk::prelude::*;

/// The fixture API: the only origin, the built-in's default `apiUrl`.
pub const API: &str = "https://api.example.com";
/// The credential slot (`contributes.credentials[0].key`).
pub const TOKEN_SLOT: &str = "token";
/// The setting copied from the built-in (`contributes.settings`).
pub const GREETING: &str = "greeting";

/// The plugin definition; must equal `manifest.json`.
pub fn plugin() -> Plugin {
    Plugin::new("deckpilot.legacy-fixture", "0.1.0")
        .action(Action::new("whoami", whoami).param(Param::text("prefix").max_length(32)))
}

deckpilot_plugin_sdk::plugin!(plugin);

#[derive(serde::Deserialize)]
struct Who {
    login: String,
}

/// `whoami`: `<prefix><greeting>, <login>` with `who = <login>` and
/// `source = plugin`.
fn whoami(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let prefix: String = if args.has("prefix") {
        args.get("prefix")?
    } else {
        String::new()
    };
    let settings = ctx.settings()?;
    let greeting = settings.text(GREETING).unwrap_or("Hello").to_owned();
    let mut request = Request::get(format!("{API}/whoami"));
    let token = SecretHandle::slot(TOKEN_SLOT)?;
    if ctx.has_credential(&token)? {
        request = request.bearer(&token);
    }
    let response = ctx.fetch(request)?;
    if !response.is_success() {
        return Err(PluginError::new(
            codes::REQUEST_FAILED,
            format!("the fixture API answered HTTP {}", response.status),
        ));
    }
    let who: Who = response.json().map_err(|_| {
        PluginError::new(
            codes::REQUEST_FAILED,
            "the fixture API sent an unexpected answer",
        )
    })?;
    Ok(
        Output::message(format!("{prefix}{greeting}, {}", who.login))
            .with("who", who.login)
            .with("source", "plugin"),
    )
}

#[cfg(test)]
mod tests {
    use deckpilot_plugin_sdk::testing::{MockHost, manifest_mismatches};
    use deckpilot_plugin_sdk::{ActionOutcome, CallResult, CapabilityRequest, ParamValue};
    use serde_json::{Value, json};

    use super::*;

    const MANIFEST: &str = include_str!("../manifest.json");
    const SCHEMA: &str = include_str!("../../../../schemas/plugin-manifest.schema.json");

    #[test]
    fn definition_is_valid_and_matches_the_manifest() {
        plugin().validate().unwrap();
        let manifest: Value = serde_json::from_str(MANIFEST).unwrap();
        assert_eq!(
            manifest_mismatches(&plugin(), &manifest),
            Vec::<String>::new()
        );
        assert!(manifest.get("assets").is_none());
    }

    #[test]
    fn manifest_is_valid_against_the_schema() {
        let schema: Value = serde_json::from_str(SCHEMA).unwrap();
        let validator = jsonschema::draft202012::new(&schema).unwrap();
        let manifest: Value = serde_json::from_str(MANIFEST).unwrap();
        let errors: Vec<String> = validator
            .iter_errors(&manifest)
            .map(|e| format!("{e} at {}", e.instance_path()))
            .collect();
        assert!(errors.is_empty(), "{errors:#?}");
    }

    #[test]
    fn whoami_asks_the_api_with_the_slot_token() {
        let mut host = MockHost::new(|req| match req {
            CapabilityRequest::SettingsGet(_) => CallResult::Ok(json!({"greeting": "Hi"})),
            CapabilityRequest::CredentialHas(c) => {
                assert_eq!(c.credential, TOKEN_SLOT);
                CallResult::Ok(json!(true))
            }
            CapabilityRequest::HttpFetch(f) => {
                assert_eq!(f.url, "https://api.example.com/whoami");
                CallResult::Ok(
                    json!({"status": 200, "headers": [], "body": "{\"login\":\"octo\"}"}),
                )
            }
            _ => CallResult::Ok(Value::Null),
        });
        let out = host.invoke(&plugin(), "whoami", json!({"prefix": "> "}));
        let ActionOutcome::Ok { message, data } = out else {
            panic!("{out:?}")
        };
        assert_eq!(message.as_deref(), Some("> Hi, octo"));
        assert_eq!(data["who"], ParamValue::String("octo".into()));
        assert_eq!(data["source"], ParamValue::String("plugin".into()));
    }
}
