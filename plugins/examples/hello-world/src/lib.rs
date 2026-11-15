//! Hello World: the smallest DeckPilot plugin.
//!
//! One action, `greet`, with one text parameter. It asks the agent for
//! nothing (no capability calls), so the manifest declares no permissions
//! and the plugin can be enabled without granting anything.
#![deny(unsafe_code)]

use deckpilot_plugin_sdk::prelude::*;

/// The plugin definition. `id`, `version`, the action id and its
/// parameters must equal `manifest.json`.
pub fn plugin() -> Plugin {
    Plugin::new("acme.hello-world", "0.1.0")
        .action(Action::new("greet", greet).param(Param::text("name").required().max_length(64)))
}

// The guest ABI 1 exports (`dp_abi_version`, `dp_alloc`, `dp_invoke`, ...).
deckpilot_plugin_sdk::plugin!(plugin);

/// `greet`: "Hello, <name>!". The SDK has already checked that `name` is
/// present, is text and is at most 64 characters.
fn greet(_ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let name: String = args.get("name")?;
    let name = name.trim();
    if name.is_empty() {
        return Err(PluginError::new(
            codes::INVALID_PARAMETERS,
            "parameter `name` must not be blank",
        ));
    }
    let greeting = format!("Hello, {name}!");
    Ok(Output::message(greeting.clone()).with("greeting", greeting))
}

#[cfg(test)]
mod tests {
    use deckpilot_plugin_sdk::testing::{MockHost, manifest_mismatches};
    use deckpilot_plugin_sdk::{ActionOutcome, ParamValue};
    use serde_json::{Value, json};

    use super::*;

    const MANIFEST: &str = include_str!("../manifest.json");
    const SCHEMA: &str = include_str!("../../../schemas/plugin-manifest.schema.json");

    #[test]
    fn definition_is_valid_and_matches_the_manifest() {
        plugin().validate().unwrap();
        let manifest: Value = serde_json::from_str(MANIFEST).unwrap();
        assert_eq!(
            manifest_mismatches(&plugin(), &manifest),
            Vec::<String>::new()
        );
        assert_eq!(manifest["permissions"], json!([]), "no permissions");
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
    fn greet_asks_for_nothing() {
        // Every capability denied: greet must still work.
        let mut host = MockHost::deny_all();
        let out = host.invoke(&plugin(), "greet", json!({"name": "  Ada "}));
        let ActionOutcome::Ok { message, data } = out else {
            panic!("{out:?}")
        };
        assert_eq!(message.as_deref(), Some("Hello, Ada!"));
        assert_eq!(data["greeting"], ParamValue::String("Hello, Ada!".into()));
        assert!(host.requests().is_empty());
    }

    #[test]
    fn bad_names_are_invalid_parameters() {
        let mut host = MockHost::deny_all();
        for params in [
            json!({}),
            json!({"name": "   "}),
            json!({"name": 7}),
            json!({"name": "x".repeat(65)}),
        ] {
            let out = host.invoke(&plugin(), "greet", params);
            assert!(
                matches!(&out, ActionOutcome::Failed { code, .. } if code == codes::INVALID_PARAMETERS),
                "{out:?}"
            );
        }
    }
}
