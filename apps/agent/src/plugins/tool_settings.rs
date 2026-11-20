//! Where `tool.run`'s user settings come from (`tools::ToolSettings`:
//! `gitExecutable`, `networkTimeoutSeconds`, `dockerExecutable`, `context`,
//! `timeoutSeconds`; `docs/marketplace/module-migration.md` §7 X3): the
//! built-in `git` and `docker` modules' configuration while they are
//! compiled in (where the user edits them today), else the copy the module
//! migration keeps as host-held settings in `plugins.db` (`host_settings`
//! `tools.git` and `tools.docker`, [`super::migration::HostTool`]). So the
//! settings outlive the built-ins (WP-C6) instead of falling back to `PATH`
//! and the default time limits. A plugin never reads either.
//!
//! The service task reports the built-ins whenever it loads its modules
//! ([`builtins`]), the plugin worker the host-held copy after every
//! migration pass ([`held`]); each applies the combination at once.

use std::sync::{PoisonError, RwLock};

use deckpilot_core::{ActionParameters, ParameterValue};
use deckpilot_plugin_store::PluginManager;
use serde_json::Value;

use super::tools::{self, ToolSettings};

/// What each source said last: `None` before it reported.
#[derive(Debug, Clone, Default)]
struct Sources {
    /// The built-ins' configuration (`None` inside: not compiled in).
    builtins: Option<[Option<ActionParameters>; 2]>,
    /// The host-held copies.
    held: [Option<ActionParameters>; 2],
}

static SOURCES: RwLock<Sources> = RwLock::new(Sources {
    builtins: None,
    held: [None, None],
});

/// The modules, in the order of [`Sources`].
const MODULES: [&str; 2] = [
    deckpilot_modules::git::MODULE_ID,
    deckpilot_modules::docker::MODULE_ID,
];

/// The built-in `git` and `docker` modules' configuration, `None` for one
/// that is not compiled in.
pub(crate) fn builtins(git: Option<&ActionParameters>, docker: Option<&ActionParameters>) {
    let mut sources = SOURCES.write().unwrap_or_else(PoisonError::into_inner);
    sources.builtins = Some([git.cloned(), docker.cloned()]);
    apply(&sources);
}

/// The host-held copies in `manager`'s store.
pub(crate) fn held(manager: &PluginManager) {
    let held = MODULES.map(|module| {
        let key = super::migration::plan_for(module)?.extraction?.tool?.key;
        let value = manager.store().host_setting(key).ok()??;
        Some(parameters(&value))
    });
    let mut sources = SOURCES.write().unwrap_or_else(PoisonError::into_inner);
    sources.held = held;
    apply(&sources);
}

/// Each tool's built-in configuration if it is compiled in, else its
/// host-held copy.
fn combined(sources: &Sources) -> ToolSettings {
    let [git, docker] = std::array::from_fn(|i| {
        sources
            .builtins
            .as_ref()
            .and_then(|builtins| builtins[i].clone())
            .or_else(|| sources.held[i].clone())
    });
    ToolSettings::from_modules(git.as_ref(), docker.as_ref())
}

fn apply(sources: &Sources) {
    tools::configure(combined(sources));
}

/// A host-held JSON object as the settings `ToolSettings` reads.
fn parameters(value: &Value) -> ActionParameters {
    let mut parameters = ActionParameters::new();
    for (key, value) in value.as_object().into_iter().flatten() {
        let value = match value {
            Value::String(text) => ParameterValue::Text(text.clone()),
            Value::Bool(flag) => ParameterValue::Boolean(*flag),
            Value::Number(n) => match n.as_i64() {
                Some(n) => ParameterValue::Integer(n),
                None => continue,
            },
            _ => continue,
        };
        parameters = parameters.with(key.clone(), value);
    }
    parameters
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn config(entries: &[(&str, ParameterValue)]) -> ActionParameters {
        entries
            .iter()
            .fold(ActionParameters::new(), |p, (k, v)| p.with(*k, v.clone()))
    }

    /// The built-in's configuration while it is compiled in; the
    /// host-held copy once it is gone; nothing (the defaults) with
    /// neither.
    #[test]
    fn the_builtins_settings_win_while_they_exist_and_the_held_ones_after() {
        let held = parameters(&serde_json::json!({
            "gitExecutable": "/opt/held/git",
            "networkTimeoutSeconds": 45,
            "ignored": [1],
        }));
        let docker = parameters(&serde_json::json!({
            "dockerExecutable": "/opt/held/docker",
            "context": "colima",
            "timeoutSeconds": 900,
        }));
        let builtin = config(&[(
            "gitExecutable",
            ParameterValue::Text("/usr/local/bin/git".into()),
        )]);
        let both = Sources {
            builtins: Some([Some(builtin.clone()), None]),
            held: [Some(held.clone()), Some(docker.clone())],
        };
        let settings = combined(&both);
        assert_eq!(settings.git, Some(PathBuf::from("/usr/local/bin/git")));
        assert_eq!(settings.network_seconds, None);
        // Docker's built-in is gone: its held copy.
        assert_eq!(settings.docker, Some(PathBuf::from("/opt/held/docker")));
        assert_eq!(settings.docker_context.as_deref(), Some("colima"));
        assert_eq!(settings.modify_seconds, Some(900));

        let removed = Sources {
            builtins: Some([None, None]),
            held: [Some(held), None],
        };
        let settings = combined(&removed);
        assert_eq!(settings.git, Some(PathBuf::from("/opt/held/git")));
        assert_eq!(settings.network_seconds, Some(45));
        assert_eq!(settings.docker, None);
        assert_eq!(combined(&Sources::default()), ToolSettings::default());
    }
}
