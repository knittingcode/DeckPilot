//! Dependencies between plugins (`dependencies`, plugin API 1.4;
//! `docs/marketplace/dependencies.md`).
//!
//! A manifest names the plugins (or built-in modules) it needs, each with
//! the versions it works with:
//!
//! ```json
//! "dependencies": [
//!   { "id": "acme.projects", "version": ">=1.2.0 <2.0.0" },
//!   { "id": "acme.calendar", "version": ">=1.0.0", "optional": true,
//!     "justification": "Shows your next meeting on the status key." }
//! ]
//! ```
//!
//! A **required** dependency must be installed, compatible and enabled
//! for any of the plugin's actions to run. An **optional** one is never
//! installed by itself: only the actions that name it in
//! `contributes.actions[].requiresDependencies` are unavailable while it is
//! missing. A dependency is only a precondition: it grants nothing. Every
//! plugin, dependency or not, runs with the permissions the user granted
//! to it and to it alone.
//!
//! The field is optional; a manifest without it (every manifest before
//! plugin API 1.4) depends on nothing.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::compat::{ApiLevel, VersionRange};
use crate::identity::PluginId;
use crate::model::{Action, Manifest};

/// The plugin API level that introduced `dependencies` and
/// `contributes.actions[].requiresDependencies`.
pub const DEPENDENCIES_API: ApiLevel = ApiLevel::new(1, 4);
/// Most entries of `dependencies`.
pub const MAX_DEPENDENCIES: usize = 16;
/// Longest `dependencies[].justification`, in characters.
pub const MAX_DEPENDENCY_JUSTIFICATION: usize = 200;
/// Longest id of a built-in module named as a dependency.
pub const MAX_BUILTIN_ID_LEN: usize = 32;

/// What a dependency names: a plugin (`<publisher>.<name>`, from the
/// registry or the application bundle) or a built-in module of DeckPilot
/// (one lowercase word such as `projects`, versioned with the app).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum DependencyTarget {
    Plugin(PluginId),
    Builtin(String),
}

impl DependencyTarget {
    /// A plugin id when the text has a `.`, a built-in module id
    /// otherwise (lowercase ASCII letters, digits and single `-`, starting
    /// with a letter, at most [`MAX_BUILTIN_ID_LEN`] characters).
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.contains('.') {
            return PluginId::parse(text)
                .map(Self::Plugin)
                .map_err(|e| e.to_string());
        }
        if crate::identity::is_kebab_word(text, 2, MAX_BUILTIN_ID_LEN) {
            Ok(Self::Builtin(text.to_owned()))
        } else {
            Err(format!(
                "`{text}` is neither a plugin id (`<publisher>.<name>`) nor a built-in module id \
                 (a lowercase word such as `projects`)"
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Plugin(id) => id.as_str(),
            Self::Builtin(id) => id,
        }
    }

    pub fn is_builtin(&self) -> bool {
        matches!(self, Self::Builtin(_))
    }

    /// The plugin id, unless a built-in module is named.
    pub fn plugin(&self) -> Option<&PluginId> {
        match self {
            Self::Plugin(id) => Some(id),
            Self::Builtin(_) => None,
        }
    }
}

impl fmt::Display for DependencyTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for DependencyTarget {
    type Error = String;

    fn try_from(value: String) -> Result<Self, String> {
        Self::parse(&value)
    }
}

impl From<DependencyTarget> for String {
    fn from(value: DependencyTarget) -> Self {
        value.as_str().to_owned()
    }
}

/// One entry of `dependencies`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Dependency {
    pub id: DependencyTarget,
    /// The versions this plugin works with, in the grammar of
    /// `engines.deckpilot` (e.g. `>=1.2.0 <2.0.0`). For a built-in module,
    /// DeckPilot's version.
    pub version: VersionRange,
    /// `true`: an optional integration, never installed by itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optional: Option<bool>,
    /// What the plugin uses it for, shown before installing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub justification: Option<String>,
}

impl Dependency {
    pub fn is_optional(&self) -> bool {
        self.optional == Some(true)
    }

    pub fn is_required(&self) -> bool {
        !self.is_optional()
    }
}

impl Manifest {
    /// Every declared dependency (none for a manifest without the field).
    pub fn dependencies(&self) -> &[Dependency] {
        self.dependencies.as_deref().unwrap_or_default()
    }

    /// The dependencies that must be installed and enabled.
    pub fn required_dependencies(&self) -> impl Iterator<Item = &Dependency> {
        self.dependencies().iter().filter(|d| d.is_required())
    }

    /// The optional integrations.
    pub fn optional_dependencies(&self) -> impl Iterator<Item = &Dependency> {
        self.dependencies().iter().filter(|d| d.is_optional())
    }

    /// The declared dependency on `id`, if any.
    pub fn dependency(&self, id: &str) -> Option<&Dependency> {
        self.dependencies().iter().find(|d| d.id.as_str() == id)
    }

    /// The dependencies the action needs besides the plugin's required
    /// ones (`requiresDependencies`), resolved against `dependencies`
    /// (an unknown name is a validation error, skipped here).
    pub fn action_dependencies<'a>(
        &'a self,
        action: &'a Action,
    ) -> impl Iterator<Item = &'a Dependency> + 'a {
        action
            .requires_dependencies()
            .iter()
            .filter_map(|id| self.dependency(id))
    }
}

impl Action {
    /// `requiresDependencies`: ids of declared dependencies (usually
    /// optional ones) without which this action cannot run.
    pub fn requires_dependencies(&self) -> &[String] {
        self.requires_dependencies.as_deref().unwrap_or_default()
    }
}
