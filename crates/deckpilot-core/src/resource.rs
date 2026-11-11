//! Logical resources and their machine-specific bindings.
//!
//! Portable configuration never contains absolute paths. Instead it refers to
//! a [`ResourceKey`] such as `shop.repository`. Every machine keeps its own
//! [`MachineConfig`] that maps those keys to concrete values (a directory on
//! Linux, an application bundle on macOS, a terminal profile on Windows, ...).

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{IdentifierError, Os, Project};

/// Logical identifier of a machine-specific resource.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ResourceKey(String);

impl ResourceKey {
    pub fn new(key: impl Into<String>) -> Result<Self, IdentifierError> {
        let key = key.into();
        crate::id::validate_dotted("resource key", &key)?;
        Ok(Self(key))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ResourceKey {
    type Error = IdentifierError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ResourceKey> for String {
    fn from(value: ResourceKey) -> Self {
        value.0
    }
}

impl fmt::Display for ResourceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Category of a resource. Used to validate that a binding fits its usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResourceKind {
    Directory,
    Application,
    Terminal,
    DockerEnvironment,
    Service,
    Url,
    Other,
}

/// How a parameter refers to a resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ResourceRef {
    /// Always the same logical resource, regardless of the active project.
    Fixed { key: ResourceKey },
    /// The resource the *active project* assigns to `role`, e.g. `repository`.
    ProjectRole { role: String },
}

/// The concrete value of a resource on one machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineResourceBinding {
    pub key: ResourceKey,
    pub kind: ResourceKind,
    /// Path, application identifier, URL, container name, ... depending on `kind`.
    pub value: String,
}

/// Machine-local configuration. Never exported together with profiles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineConfig {
    pub os: Os,
    pub bindings: Vec<MachineResourceBinding>,
}

impl MachineConfig {
    pub fn new(os: Os) -> Self {
        Self {
            os,
            bindings: Vec::new(),
        }
    }

    pub fn binding(&self, key: &ResourceKey) -> Option<&MachineResourceBinding> {
        self.bindings.iter().find(|binding| &binding.key == key)
    }

    /// Inserts or replaces the binding for `binding.key`.
    pub fn set_binding(&mut self, binding: MachineResourceBinding) {
        match self
            .bindings
            .iter_mut()
            .find(|existing| existing.key == binding.key)
        {
            Some(existing) => *existing = binding,
            None => self.bindings.push(binding),
        }
        self.bindings.sort_by(|a, b| a.key.cmp(&b.key));
    }

    pub fn remove_binding(&mut self, key: &ResourceKey) -> Option<MachineResourceBinding> {
        let index = self
            .bindings
            .iter()
            .position(|binding| &binding.key == key)?;
        Some(self.bindings.remove(index))
    }

    /// Resolves a resource reference against this machine and, for
    /// project-relative references, the active project.
    pub fn resolve(
        &self,
        reference: &ResourceRef,
        project: Option<&Project>,
        expected: Option<ResourceKind>,
    ) -> Result<&MachineResourceBinding, ResolveError> {
        let key = match reference {
            ResourceRef::Fixed { key } => key,
            ResourceRef::ProjectRole { role } => {
                let project = project.ok_or(ResolveError::NoActiveProject)?;
                project
                    .resources
                    .get(role)
                    .ok_or_else(|| ResolveError::RoleNotDefined {
                        project: project.key.to_string(),
                        role: role.clone(),
                    })?
            }
        };
        let binding = self
            .binding(key)
            .ok_or_else(|| ResolveError::Unbound(key.clone()))?;
        if let Some(expected) = expected
            && binding.kind != expected
        {
            return Err(ResolveError::KindMismatch {
                key: key.clone(),
                expected,
                actual: binding.kind,
            });
        }
        Ok(binding)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ResolveError {
    #[error("the module is not allowed to read this computer's resource bindings")]
    NotPermitted,
    #[error("no project is active")]
    NoActiveProject,
    #[error("project `{project}` does not define the resource role `{role}`")]
    RoleNotDefined { project: String, role: String },
    #[error("resource `{0}` is not bound on this machine")]
    Unbound(ResourceKey),
    #[error("resource `{key}` is bound as {actual:?}, expected {expected:?}")]
    KindMismatch {
        key: ResourceKey,
        expected: ResourceKind,
        actual: ResourceKind,
    },
}

impl ResolveError {
    /// Stable error code for [`crate::ActionResult::code`].
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotPermitted => "not-permitted",
            Self::NoActiveProject => "no-active-project",
            Self::RoleNotDefined { .. } => "resource-role-missing",
            Self::Unbound(_) => "resource-unbound",
            Self::KindMismatch { .. } => "resource-kind-mismatch",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProjectKey;

    fn key(value: &str) -> ResourceKey {
        ResourceKey::new(value).unwrap()
    }

    fn machine() -> MachineConfig {
        let mut config = MachineConfig::new(Os::Linux);
        config.set_binding(MachineResourceBinding {
            key: key("alpha.repository"),
            kind: ResourceKind::Directory,
            value: "/home/user/src/alpha".into(),
        });
        config
    }

    fn project() -> Project {
        let mut project = Project::new(ProjectKey::new("ALPHA").unwrap(), "Alpha");
        project
            .resources
            .insert("repository".into(), key("alpha.repository"));
        project
    }

    #[test]
    fn resource_keys_reject_paths() {
        assert!(ResourceKey::new("/home/user").is_err());
        assert!(serde_json::from_str::<ResourceKey>(r#""C:\\Users""#).is_err());
        assert!(serde_json::from_str::<ResourceKey>(r#""shop.repo""#).is_ok());
    }

    #[test]
    fn set_binding_replaces_existing_entry() {
        let mut config = machine();
        config.set_binding(MachineResourceBinding {
            key: key("alpha.repository"),
            kind: ResourceKind::Directory,
            value: "/srv/alpha".into(),
        });
        assert_eq!(config.bindings.len(), 1);
        assert_eq!(config.bindings[0].value, "/srv/alpha");
        assert!(config.remove_binding(&key("alpha.repository")).is_some());
        assert_eq!(config.bindings, Vec::<MachineResourceBinding>::new());
    }

    #[test]
    fn resolves_fixed_and_project_relative_references() {
        let config = machine();
        let fixed = ResourceRef::Fixed {
            key: key("alpha.repository"),
        };
        assert_eq!(
            config.resolve(&fixed, None, None).unwrap().value,
            "/home/user/src/alpha"
        );

        let by_role = ResourceRef::ProjectRole {
            role: "repository".into(),
        };
        let project = project();
        let resolved = config.resolve(&by_role, Some(&project), Some(ResourceKind::Directory));
        assert_eq!(resolved.unwrap().value, "/home/user/src/alpha");
    }

    #[test]
    fn reports_resolution_failures() {
        let config = machine();
        let project = project();
        let by_role = ResourceRef::ProjectRole {
            role: "repository".into(),
        };
        assert_eq!(
            config.resolve(&by_role, None, None),
            Err(ResolveError::NoActiveProject)
        );

        let missing_role = ResourceRef::ProjectRole {
            role: "docker".into(),
        };
        assert!(matches!(
            config.resolve(&missing_role, Some(&project), None),
            Err(ResolveError::RoleNotDefined { .. })
        ));

        let unbound = ResourceRef::Fixed {
            key: key("beta.repository"),
        };
        assert!(matches!(
            config.resolve(&unbound, None, None),
            Err(ResolveError::Unbound(_))
        ));

        assert!(matches!(
            config.resolve(&by_role, Some(&project), Some(ResourceKind::Application)),
            Err(ResolveError::KindMismatch { .. })
        ));
    }
}
