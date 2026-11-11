//! The feature modules built into DeckPilot.
//!
//! This is the only place that lists concrete modules. The agent and the
//! configuration app build their [`ModuleRegistry`] here and then work only
//! with the registry.

use deckpilot_core::{ModuleRegistry, ModuleStates, Os};

pub use deckpilot_module_docker as docker;
pub use deckpilot_module_general as general;
pub use deckpilot_module_git as git;
pub use deckpilot_module_projects as projects;
pub use deckpilot_module_system as system;

/// A registry with every built-in module in its default state.
///
/// # Panics
///
/// Panics if two built-in modules conflict, which is a programming error
/// covered by tests.
pub fn builtin(os: Os) -> ModuleRegistry {
    let mut registry = ModuleRegistry::new(os);
    registry
        .register(Box::new(general::GeneralModule::new()))
        .expect("built-in modules do not conflict");
    registry
        .register(Box::new(projects::ProjectsModule::new()))
        .expect("built-in modules do not conflict");
    registry
        .register(Box::new(system::SystemModule::new()))
        .expect("built-in modules do not conflict");
    registry
        .register(Box::new(git::GitModule::new()))
        .expect("built-in modules do not conflict");
    registry
        .register(Box::new(docker::DockerModule::new()))
        .expect("built-in modules do not conflict");
    registry
}

/// The built-in modules for this machine with persisted `states` applied.
/// Problems with the stored states are returned for logging.
pub fn configured(states: &ModuleStates) -> (ModuleRegistry, Vec<String>) {
    let mut registry = builtin(Os::current());
    let problems = registry.apply_states(states);
    (registry, problems)
}

#[cfg(test)]
mod tests;
