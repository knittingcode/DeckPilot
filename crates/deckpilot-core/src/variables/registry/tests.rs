use std::sync::Arc;

use super::*;
use crate::variables::{ValueSource, VariableValue};
use crate::{ProjectId, testing};

fn module(id: &str) -> ModuleId {
    ModuleId::new(id).unwrap()
}

/// Answers every declared variable with `value`.
struct Fixed(VariableValue);

impl VariableSource for Fixed {
    fn resolve(&self, _: &VariableDeclaration, _: &VariableEnvironment<'_>) -> Resolution {
        Resolution::Value {
            value: self.0.clone(),
            source: ValueSource::System,
            sensitive: false,
        }
    }
}

#[allow(clippy::unnecessary_wraps)]
fn fixed(value: VariableValue) -> Option<Arc<dyn VariableSource>> {
    Some(Arc::new(Fixed(value)))
}

fn text(id: &str) -> VariableDeclaration {
    VariableDeclaration::new(id, id, VariableType::String, DeclaredScope::Profile)
}

fn environment<'a>(
    context: &'a ResolutionContext,
    profile: &'a Profile,
) -> VariableEnvironment<'a> {
    VariableEnvironment {
        context,
        profile,
        os: Os::Linux,
        live: &NoLiveState,
    }
}

fn resolve(registry: &VariableRegistry, id: &str, profile: &Profile) -> Resolution {
    let context = ResolutionContext::preview(profile.id);
    registry.resolve(
        &VariableId::new(id).unwrap(),
        &environment(&context, profile),
    )
}

#[test]
fn ids_are_dotted_segments_with_a_namespace() {
    let id = VariableId::new("projects.current.id").unwrap();
    assert_eq!(id.namespace(), "projects");
    assert_eq!(id.name(), "current.id");
    assert!(!id.is_core());
    assert!(VariableId::new("system.platform").unwrap().is_core());
    for bad in [
        "projects",
        "Projects.x",
        "a.b-c",
        "a..b",
        "a.b.c.d.e.f.g",
        "",
    ] {
        assert!(VariableId::new(bad).is_err(), "{bad}");
    }
    assert_eq!(
        serde_json::to_value(&id).unwrap(),
        serde_json::json!("projects.current.id")
    );
}

#[test]
fn module_namespaces_are_lower_camel_case_ids() {
    assert_eq!(module_namespace(&module("projects")), "projects");
    assert_eq!(module_namespace(&module("work-session")), "workSession");
    assert_eq!(module_namespace(&module("a-b-c2")), "aBC2");
}

#[test]
fn the_core_owns_only_genuine_system_variables() {
    let registry = VariableRegistry::new();
    let ids: Vec<String> = registry
        .variables()
        .map(|v| v.declaration.id.to_string())
        .collect();
    assert_eq!(
        ids,
        [
            "system.device.current.id",
            "system.platform",
            "system.profile.current.id",
            "system.profile.current.name",
            "workflow.execution.id",
            "workflow.id",
        ]
    );
    for variable in registry.variables() {
        assert_eq!(*variable.owner, VariableOwner::Core);
        assert!(variable.declaration.read_only);
        assert!(variable.unavailable.is_none());
    }
    assert_eq!(
        registry.namespace_owner("system"),
        Some(VariableOwner::Core)
    );
    assert_eq!(registry.namespace_owner("projects"), None);
}

#[test]
fn a_module_declares_only_inside_its_namespace() {
    let mut registry = VariableRegistry::new();
    let projects = module("projects");
    assert_eq!(
        registry.register_module(
            &projects,
            &[text("projects.current.key"), text("git.branch")],
            fixed(VariableValue::String("x".into())),
        ),
        Err(VariableRegistryError::Namespace {
            module: projects.clone(),
            namespace: "projects".into(),
            id: VariableId::new("git.branch").unwrap(),
        })
    );
    // Nothing of a refused declaration is registered.
    assert!(!registry.has_module(&projects));
    assert!(
        registry
            .get(&VariableId::new("projects.current.key").unwrap())
            .is_none()
    );
    // The core's namespaces and the other roots of the grammar are
    // reserved: a module named after one cannot declare variables.
    for reserved in ["system", "workflow", "steps", "shared"] {
        let id = module(reserved);
        assert_eq!(
            registry.register_module(
                &id,
                &[text(&format!("{reserved}.x"))],
                fixed(VariableValue::String("x".into())),
            ),
            Err(VariableRegistryError::ReservedNamespace {
                module: id,
                namespace: reserved.into(),
            })
        );
    }
    // A module without variables needs no namespace.
    registry
        .register_module(&module("system"), &[], None)
        .unwrap();
}

#[test]
fn collisions_are_refused() {
    let mut registry = VariableRegistry::new();
    let a = module("a-b");
    registry
        .register_module(
            &a,
            &[text("aB.x")],
            fixed(VariableValue::String("x".into())),
        )
        .unwrap();
    // `a--b` would also be `aB`: one namespace has one owner.
    let b = module("a--b");
    assert_eq!(
        registry.register_module(
            &b,
            &[text("aB.y")],
            fixed(VariableValue::String("y".into()))
        ),
        Err(VariableRegistryError::NamespaceTaken {
            module: b,
            namespace: "aB".into(),
            owner: a.clone(),
        })
    );
    assert_eq!(
        registry.register_module(
            &a,
            &[text("aB.x"), text("aB.x")],
            fixed(VariableValue::String("x".into())),
        ),
        Err(VariableRegistryError::Duplicate(
            VariableId::new("aB.x").unwrap()
        ))
    );
    let writable = VariableDeclaration {
        read_only: false,
        ..text("aB.z")
    };
    assert_eq!(
        registry.register_module(&a, &[writable], fixed(VariableValue::String("z".into()))),
        Err(VariableRegistryError::Writable(
            VariableId::new("aB.z").unwrap()
        ))
    );
    assert_eq!(
        registry.register_module(&a, &[text("aB.x")], None),
        Err(VariableRegistryError::NoSource(a.clone()))
    );
    // The refused attempts left the first registration as it was.
    assert!(registry.get(&VariableId::new("aB.x").unwrap()).is_some());
}

#[test]
fn registration_follows_the_module_and_unavailable_variables_keep_their_references() {
    let profile = testing::with_projects("Work", &["SHOP"]);
    let mut registry = VariableRegistry::new();
    let git = module("git");
    // Before the module is registered, its references are unavailable,
    // not invalid and not deleted.
    assert_eq!(
        resolve(&registry, "git.branch", &profile),
        Resolution::Unavailable {
            owner: "git".into()
        }
    );
    registry
        .register_module(
            &git,
            &[text("git.branch")],
            fixed(VariableValue::String("main".into())),
        )
        .unwrap();
    assert!(matches!(
        resolve(&registry, "git.branch", &profile),
        Resolution::Value { value: VariableValue::String(text), .. } if text == "main"
    ));
    // An id the owner does not declare is unknown.
    assert_eq!(
        resolve(&registry, "git.tag", &profile),
        Resolution::Missing(MissingReason::UnknownVariable)
    );
    registry.set_available(&git, Some("the Git module is disabled".into()));
    let branch = registry
        .get(&VariableId::new("git.branch").unwrap())
        .unwrap();
    assert_eq!(branch.unavailable, Some("the Git module is disabled"));
    assert_eq!(
        *branch.owner,
        VariableOwner::Module {
            module: git.clone()
        }
    );
    assert_eq!(
        resolve(&registry, "git.branch", &profile),
        Resolution::Unavailable {
            owner: "git".into()
        }
    );
    // Registering again keeps the availability.
    registry
        .register_module(
            &git,
            &[text("git.branch")],
            fixed(VariableValue::String("main".into())),
        )
        .unwrap();
    assert!(
        registry
            .get(&VariableId::new("git.branch").unwrap())
            .unwrap()
            .unavailable
            .is_some()
    );
    registry.set_available(&git, None);
    assert_eq!(registry.unregister_module(&git), Some(None));
    assert!(
        registry
            .get(&VariableId::new("git.branch").unwrap())
            .is_none()
    );
    assert_eq!(
        resolve(&registry, "git.branch", &profile),
        Resolution::Unavailable {
            owner: "git".into()
        }
    );
    assert_eq!(registry.unregister_module(&git), None);
}

#[test]
fn values_keep_their_declared_type_and_sensitivity() {
    let profile = testing::with_projects("Work", &["SHOP"]);
    let mut registry = VariableRegistry::new();
    let vault = module("vault");
    let pin = VariableDeclaration {
        sensitive: true,
        ..text("vault.pin")
    };
    registry
        .register_module(&vault, &[pin], fixed(VariableValue::String("1234".into())))
        .unwrap();
    assert!(matches!(
        resolve(&registry, "vault.pin", &profile),
        Resolution::Value {
            sensitive: true,
            ..
        }
    ));
    // A source that answers with another type than declared provides
    // nothing: no implicit conversion.
    let counter = module("counter");
    registry
        .register_module(
            &counter,
            &[VariableDeclaration::new(
                "counter.count",
                "Count",
                VariableType::Integer,
                DeclaredScope::Machine,
            )],
            fixed(VariableValue::String("12".into())),
        )
        .unwrap();
    assert_eq!(
        resolve(&registry, "counter.count", &profile),
        Resolution::Missing(MissingReason::NoValue)
    );
}

#[test]
fn core_variables_resolve_from_the_context() {
    let profile = testing::with_projects("Work", &["SHOP"]);
    let registry = VariableRegistry::new();
    assert!(matches!(
        resolve(&registry, "system.profile.current.name", &profile),
        Resolution::Value { value: VariableValue::String(name), .. } if name == "Work"
    ));
    assert!(matches!(
        resolve(&registry, "system.platform", &profile),
        Resolution::Value { value: VariableValue::String(os), .. } if os == "linux"
    ));
    assert_eq!(
        resolve(&registry, "system.device.current.id", &profile),
        Resolution::Missing(MissingReason::NoDevice)
    );
    assert_eq!(
        resolve(&registry, "workflow.execution.id", &profile),
        Resolution::UnavailableUntilRuntime
    );
    // Another profile than the context's resolves nothing.
    let other = testing::with_projects("Home", &[]);
    let context = ResolutionContext::preview(profile.id);
    assert_eq!(
        registry.resolve(
            &VariableId::new("system.profile.current.name").unwrap(),
            &environment(&context, &other)
        ),
        Resolution::Missing(MissingReason::OtherProfile)
    );
}

#[test]
fn the_environment_gives_the_project_and_device_of_the_context() {
    let profile = testing::with_projects("Work", &["SHOP"]);
    let shop = profile.projects[0].id;
    let context = ResolutionContext::preview(profile.id);
    assert_eq!(
        environment(&context, &profile).project().unwrap_err(),
        Resolution::Missing(MissingReason::NoProject)
    );
    let active = context
        .clone()
        .with_project(Some(shop), ProjectSource::Application);
    assert_eq!(environment(&active, &profile).project().unwrap().id, shop);
    // The active project never stands in for a run's project.
    let run = context
        .clone()
        .with_project(Some(shop), ProjectSource::Execution);
    assert_eq!(
        environment(&run, &profile).project().unwrap_err(),
        Resolution::UnavailableUntilRuntime
    );
    let foreign = context.with_project(Some(ProjectId::new()), ProjectSource::Explicit);
    assert_eq!(
        environment(&foreign, &profile).project().unwrap_err(),
        Resolution::Missing(MissingReason::OtherProfile)
    );
    let device = DeviceId {
        model: deckpilot_device::DeviceModelId::new("elgato.mk2"),
        serial: "A1".into(),
    };
    let with_device = active.with_device(Some(device));
    let environment = environment(&with_device, &profile);
    let descriptor = environment.device().unwrap();
    assert_eq!(descriptor.device_id(), "elgato.mk2#A1");
    assert_eq!(descriptor.display_name(), None);
}

#[test]
fn every_alias_names_a_module_owned_id_and_is_deprecated() {
    let mut aliases: Vec<&str> = Vec::new();
    for alias in ALIASES {
        let canonical = alias.alias.canonical();
        assert_eq!(canonical.as_str(), alias.canonical);
        assert!(!canonical.is_core(), "{}", alias.canonical);
        assert_eq!(
            canonical.namespace(),
            module_namespace(&module(alias.owner)),
            "{}",
            alias.canonical
        );
        let deprecation = alias.alias.deprecation().unwrap();
        assert_eq!(deprecation.canonical, canonical);
        assert_eq!(deprecation.since, "0.6.0");
        aliases.push(alias.alias.path());
    }
    assert_eq!(
        aliases,
        [
            "system.project.current.id",
            "system.project.current.key",
            "system.project.current.name",
            "system.work.session.id",
            "system.work.session.state",
            "system.work.session.startedAt",
            "system.work.session.elapsedSeconds",
            "system.work.session.projectId",
            "system.work.session.canFinish",
        ]
    );
    // Core variables are their own canonical id.
    assert_eq!(
        SystemVariable::Platform.canonical().as_str(),
        "system.platform"
    );
    assert!(SystemVariable::Platform.deprecation().is_none());
    // Every legacy id is either a core variable or an alias.
    for variable in SystemVariable::ALL {
        let core = VariableRegistry::new()
            .get(&VariableId::new(variable.path()).unwrap())
            .is_some();
        assert!(core != variable.alias().is_some(), "{variable:?}");
    }
}

#[test]
fn declarations_serialize_for_the_app() {
    let declaration = VariableDeclaration::new(
        "projects.current.key",
        "Project key",
        VariableType::String,
        DeclaredScope::Project,
    )
    .describe("Key of the project in the context")
    .requires(Requirement::Project);
    let json = serde_json::to_value(&declaration).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "id": "projects.current.key",
            "name": "Project key",
            "description": "Key of the project in the context",
            "valueType": {"type": "string"},
            "scope": "project",
            "requires": ["project"],
            "readOnly": true,
        })
    );
    // Read-only unless said otherwise.
    let mut minimal = json;
    minimal.as_object_mut().unwrap().remove("readOnly");
    assert_eq!(
        serde_json::from_value::<VariableDeclaration>(minimal).unwrap(),
        declaration
    );
    assert_eq!(
        serde_json::to_value(VariableOwner::Module {
            module: module("projects")
        })
        .unwrap(),
        serde_json::json!({"kind": "module", "module": "projects"})
    );
}
