//! TEST FIXTURE, never shipped: a default-class bundled first-party
//! plugin for the bundling, pinning and seeding tests
//! (`plugins/first-party/README.md`).
//!
//! One action, `greet`. It declares one low-tier permission
//! (`notifications`, which seeding may grant without asking) and uses none.
#![deny(unsafe_code)]

use deckpilot_plugin_sdk::prelude::*;

/// The plugin definition; must equal `manifest.json`.
pub fn plugin() -> Plugin {
    Plugin::new("deckpilot.example-default", "0.1.0")
        .action(Action::new("greet", greet).param(Param::text("name").required().max_length(64)))
}

deckpilot_plugin_sdk::plugin!(plugin);

/// `greet`: "Hello from the bundle, <name>!".
fn greet(_ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let name: String = args.get("name")?;
    let name = name.trim();
    if name.is_empty() {
        return Err(PluginError::new(
            codes::INVALID_PARAMETERS,
            "parameter `name` must not be blank",
        ));
    }
    let greeting = format!("Hello from the bundle, {name}!");
    Ok(Output::message(greeting.clone()).with("greeting", greeting))
}

#[cfg(test)]
mod tests {
    use deckpilot_plugin_sdk::testing::{MockHost, manifest_mismatches};
    use deckpilot_plugin_sdk::{ActionOutcome, ParamValue};
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
        // A bundled plugin is manifest.json, plugin.wasm, LICENSE and its
        // icons: no other assets.
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
    fn greet_asks_for_nothing() {
        let mut host = MockHost::deny_all();
        let out = host.invoke(&plugin(), "greet", json!({"name": " Ada "}));
        let ActionOutcome::Ok { message, data } = out else {
            panic!("{out:?}")
        };
        assert_eq!(message.as_deref(), Some("Hello from the bundle, Ada!"));
        assert_eq!(
            data["greeting"],
            ParamValue::String("Hello from the bundle, Ada!".into())
        );
        assert!(host.requests().is_empty());
    }
}
