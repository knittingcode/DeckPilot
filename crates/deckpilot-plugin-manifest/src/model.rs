//! The typed manifest, field for field as `schema/plugin-manifest.schema.json`.
//!
//! Every struct refuses unknown fields. Optional fields stay `Option` (even
//! where the schema has a default) so a manifest serializes back to the same
//! JSON value it was read from; accessors return the effective default.

use std::collections::BTreeMap;

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::compat::{ApiRequirement, Platform, VersionRange};
use crate::identity::PluginId;
use crate::unique_map;

/// A plugin manifest, `manifest.json` (`manifestVersion` 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub manifest_version: u64,
    pub id: PluginId,
    pub name: String,
    pub version: Version,
    pub publisher: Publisher,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<Repository>,
    /// SPDX license expression.
    pub license: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keywords: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<Category>>,
    pub engines: Engines,
    /// Source entry module the builder bundles; the package always holds the
    /// bundle as `plugin.js`.
    pub entry: String,
    /// `None` means every platform.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platforms: Option<Vec<Platform>>,
    pub icons: Icons,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets: Option<Assets>,
    /// Everything the plugin may ever ask for. Deny by default: an operation
    /// not covered here is refused whatever the user consents to.
    pub permissions: Vec<Permission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports: Option<Supports>,
    pub contributes: Contributes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Publisher {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepositoryKind {
    Git,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Repository {
    #[serde(rename = "type")]
    pub kind: RepositoryKind,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    DeveloperTools,
    Productivity,
    Communication,
    Monitoring,
    Media,
    SmartHome,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Engines {
    pub deckpilot: VersionRange,
    pub plugin_api: ApiRequirement,
    /// Exact `@deckpilot/plugin-sdk` version used to build. Informational.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk: Option<Version>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Icons {
    #[serde(rename = "32", default, skip_serializing_if = "Option::is_none")]
    pub size32: Option<String>,
    #[serde(rename = "128")]
    pub size128: String,
    #[serde(rename = "256", default, skip_serializing_if = "Option::is_none")]
    pub size256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Assets {
    #[serde(
        default,
        deserialize_with = "unique_map::deserialize",
        skip_serializing_if = "Option::is_none"
    )]
    pub key_icons: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshots: Option<Vec<String>>,
    /// Always `README.md` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readme: Option<String>,
    /// Always `CHANGELOG.md` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changelog: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Head,
    Post,
    Put,
    Patch,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProfileScope {
    Profile,
    Project,
}

/// One declared permission (architecture section 5.2).
///
/// Only these kinds exist in plugin API 1. File system, process execution
/// or inspection, clipboard, raw device access, workflow triggering and
/// custom UI are never grantable, so they have no variant: a manifest that
/// names them does not parse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Permission {
    /// HTTPS to the listed hosts, with the listed methods.
    #[serde(rename_all = "camelCase")]
    Network {
        hosts: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        methods: Option<Vec<HttpMethod>>,
        /// Also hosts the user enters (high tier).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        private_network: Option<bool>,
        justification: String,
    },
    /// Secret slots, injected by the agent; never readable.
    Credentials {
        refs: Vec<String>,
        justification: String,
    },
    Notifications {
        justification: String,
    },
    Profile {
        read: Vec<ProfileScope>,
        justification: String,
    },
    /// Variable ids or `prefix.*` the plugin may read.
    Variables {
        read: Vec<String>,
        justification: String,
    },
    /// A larger key-value storage quota.
    #[serde(rename_all = "camelCase")]
    Storage {
        quota_kb: u64,
        justification: String,
    },
    /// Opening https URLs on the listed hosts in the browser.
    OpenUrl {
        hosts: Vec<String>,
        justification: String,
    },
    /// Timers while the plugin is active.
    #[serde(rename_all = "camelCase")]
    Background {
        min_interval_seconds: u64,
        justification: String,
    },
    /// Button state and key icons for the plugin's own keys.
    Device {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icons: Option<bool>,
        justification: String,
    },
}

/// The kind of a [`Permission`], without its scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PermissionKind {
    Network,
    Credentials,
    Notifications,
    Profile,
    Variables,
    Storage,
    OpenUrl,
    Background,
    Device,
}

impl PermissionKind {
    /// The `kind` text in the manifest.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Credentials => "credentials",
            Self::Notifications => "notifications",
            Self::Profile => "profile",
            Self::Variables => "variables",
            Self::Storage => "storage",
            Self::OpenUrl => "openUrl",
            Self::Background => "background",
            Self::Device => "device",
        }
    }
}

impl Permission {
    pub fn kind(&self) -> PermissionKind {
        match self {
            Self::Network { .. } => PermissionKind::Network,
            Self::Credentials { .. } => PermissionKind::Credentials,
            Self::Notifications { .. } => PermissionKind::Notifications,
            Self::Profile { .. } => PermissionKind::Profile,
            Self::Variables { .. } => PermissionKind::Variables,
            Self::Storage { .. } => PermissionKind::Storage,
            Self::OpenUrl { .. } => PermissionKind::OpenUrl,
            Self::Background { .. } => PermissionKind::Background,
            Self::Device { .. } => PermissionKind::Device,
        }
    }

    pub fn justification(&self) -> &str {
        match self {
            Self::Network { justification, .. }
            | Self::Credentials { justification, .. }
            | Self::Notifications { justification }
            | Self::Profile { justification, .. }
            | Self::Variables { justification, .. }
            | Self::Storage { justification, .. }
            | Self::OpenUrl { justification, .. }
            | Self::Background { justification, .. }
            | Self::Device { justification, .. } => justification,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceFamily {
    Keys,
    KeyDisplays,
    Dials,
    TouchStrip,
    Brightness,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Supports {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<DeviceSupport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceSupport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<Vec<DeviceFamily>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uses: Option<Vec<DeviceFamily>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Contributes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<Action>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variables: Option<Vec<Variable>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<Settings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials: Option<Vec<Credential>>,
    #[serde(
        default,
        deserialize_with = "unique_map::deserialize",
        skip_serializing_if = "Option::is_none"
    )]
    pub error_codes: Option<BTreeMap<String, String>>,
}

impl Contributes {
    pub fn actions(&self) -> &[Action] {
        self.actions.as_deref().unwrap_or_default()
    }

    pub fn variables(&self) -> &[Variable] {
        self.variables.as_deref().unwrap_or_default()
    }

    pub fn settings(&self) -> &[Setting] {
        self.settings
            .as_ref()
            .and_then(|s| s.fields.as_deref())
            .unwrap_or_default()
    }

    pub fn credentials(&self) -> &[Credential] {
        self.credentials.as_deref().unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairSeparator {
    #[serde(rename = "=")]
    Equals,
    #[serde(rename = ":")]
    Colon,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodeLanguage {
    Json,
    Text,
    Yaml,
    Markdown,
}

/// The value a parameter accepts: `deckpilot-core`'s `ParameterKind` minus
/// `resource` and `credential` (not available to plugins in API 1). The
/// tag and field names match it, so the JSON maps one to one; the types are
/// mirrored rather than imported to keep this crate free of core.
///
/// Variants without fields are empty structs, not unit variants, so unknown
/// fields next to `type` are refused for them too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum ParameterKind {
    #[serde(rename_all = "camelCase")]
    Text {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        multiline: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<u64>,
    },
    Integer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<i64>,
    },
    Boolean {},
    Choice {
        options: Vec<String>,
    },
    Project {},
    #[serde(rename_all = "camelCase")]
    Lines {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_items: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<u64>,
    },
    #[serde(rename_all = "camelCase")]
    Pairs {
        separator: PairSeparator,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_items: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        case_insensitive: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        no_secrets: Option<bool>,
    },
    #[serde(rename_all = "camelCase")]
    Code {
        language: CodeLanguage,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<u64>,
    },
}

/// A literal default value (`$defs/parameterValue`). References and
/// templates are never defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "camelCase",
    try_from = "RawParameterValue"
)]
pub enum ParameterValue {
    Text(String),
    Integer(i64),
    Boolean(bool),
    Choice(String),
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ValueTag {
    Text,
    Integer,
    Boolean,
    Choice,
}

/// Read through a plain struct: serde's adjacently tagged enums word an
/// unknown field as "invalid value: string …", which is no help to authors.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawParameterValue {
    #[serde(rename = "type")]
    tag: ValueTag,
    value: serde_json::Value,
}

impl TryFrom<RawParameterValue> for ParameterValue {
    type Error = String;

    fn try_from(raw: RawParameterValue) -> Result<Self, String> {
        let value = match (raw.tag, raw.value) {
            (ValueTag::Text, serde_json::Value::String(s)) => Self::Text(s),
            (ValueTag::Choice, serde_json::Value::String(s)) => Self::Choice(s),
            (ValueTag::Boolean, serde_json::Value::Bool(b)) => Self::Boolean(b),
            (ValueTag::Integer, serde_json::Value::Number(n)) if n.as_i64().is_some() => {
                Self::Integer(n.as_i64().unwrap_or_default())
            }
            (tag, _) => {
                let expected = match tag {
                    ValueTag::Text | ValueTag::Choice => "a string",
                    ValueTag::Integer => "a whole number",
                    ValueTag::Boolean => "true or false",
                };
                return Err(format!(
                    "a default of type `{}` needs {expected} as its value",
                    format!("{tag:?}").to_lowercase()
                ));
            }
        };
        Ok(value)
    }
}

/// One parameter of an action (`ParameterSpec`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Parameter {
    pub key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub kind: ParameterKind,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<ParameterValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Portability {
    Portable,
    #[default]
    MachineSpecific,
    Sensitive,
}

/// One plugin setting: a parameter plus how it travels in exports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Setting {
    pub key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub kind: ParameterKind,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<ParameterValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portability: Option<Portability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portability_version: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<Setting>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputKind {
    #[default]
    Text,
    Integer,
    Boolean,
    Path,
    Url,
    ProjectKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Output {
    pub key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<OutputKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub always: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitive: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Effect {
    #[default]
    ReadOnly,
    Opens,
    Modifies,
}

/// Workflow node contract (`NodeSpec` without `changesContext`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Node {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outputs: Option<Vec<Output>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_safe: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotent: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancellable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<Effect>,
}

impl Node {
    pub fn effect(&self) -> Effect {
        self.effect.unwrap_or_default()
    }

    /// Schema default: 30 seconds.
    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms.unwrap_or(30_000)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RefreshMode {
    OnEvents,
    Every,
}

/// Button-state refresh for keys bound to the action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StateRefresh {
    pub refresh: RefreshMode,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionDevice {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<Vec<DeviceFamily>>,
}

/// An action type the plugin contributes; registered as
/// `<plugin id>.<action id>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Action {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub parameters: Vec<Parameter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_project: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaced_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<Node>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<StateRefresh>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<ActionDevice>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VariableValueKind {
    String,
    Integer,
    Boolean,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VariableValueType {
    #[serde(rename = "type")]
    pub kind: VariableValueKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VariableScope {
    Machine,
    Profile,
    Project,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VariableRequirement {
    Project,
}

/// A variable the plugin publishes, as `plugins.<ns>.<id>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Variable {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub value_type: VariableValueType,
    pub scope: VariableScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<Vec<VariableRequirement>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitive: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CredentialScheme {
    Bearer,
    Basic,
    Header,
    Query,
}

/// A secret slot the user fills; the plugin only ever holds a handle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Credential {
    pub key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The only hosts the secret is ever sent to.
    pub hosts: Vec<String>,
    pub schemes: Vec<CredentialScheme>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_url: Option<String>,
}
