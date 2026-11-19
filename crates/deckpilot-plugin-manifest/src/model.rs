//! The typed manifest, field for field as `schema/plugin-manifest.schema.json`.
//!
//! Every struct refuses unknown fields. Optional fields stay `Option` (even
//! where the schema has a default) so a manifest serializes back to the same
//! JSON value it was read from; accessors return the effective default.

use std::collections::{BTreeMap, BTreeSet};

pub use deckpilot_device::{ControlKind, InputCapability};
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::compat::{ApiLevel, ApiRequirement, Platform, VersionRange};
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
    /// How the package's code runs (`docs/marketplace/artifact-contract.md`).
    /// Absent means the #191 JavaScript contract (`plugin.js`), which this
    /// version of DeckPilot reads but refuses to install; see
    /// [`Manifest::runtime_contract`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<Runtime>,
    /// Source entry module the builder compiles or bundles (e.g.
    /// `src/lib.rs`). Informational: the package always holds the
    /// executable artifact under a fixed name ([`Manifest::artifact_file`]).
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

/// The executable artifact a package ships and the runtime contract it
/// implements (`"runtime": {"kind": "wasm", "abi": 1}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Runtime {
    pub kind: RuntimeKind,
    /// The plugin ABI version the module implements (`deckpilot_v1`
    /// imports, `dp_*` exports). Only [`WASM_ABI_VERSION`] exists.
    pub abi: u64,
}

/// Kinds of executable artifact. A JavaScript plugin of the future is also
/// a `wasm` module (`QuickJS` and the bundle compiled into one ABI module),
/// so there is deliberately no `js` kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeKind {
    /// A WebAssembly module, `plugin.wasm`.
    Wasm,
}

/// The plugin ABI version of `"runtime": {"kind": "wasm"}`.
pub const WASM_ABI_VERSION: u64 = 1;
/// The package file of a `wasm` runtime.
pub const WASM_ARTIFACT: &str = "plugin.wasm";
/// The package file of the #191 JavaScript contract (no `runtime`).
pub const LEGACY_JS_ARTIFACT: &str = "plugin.js";

/// What a manifest's `runtime` amounts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeContract {
    /// A WebAssembly module implementing plugin ABI `abi`.
    Wasm { abi: u64 },
    /// No `runtime`: the #191 JavaScript bundle (`plugin.js`). Not
    /// installable by this version of DeckPilot.
    LegacyJs,
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
        /// May be empty only when `user_origins` names at least one slot.
        hosts: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        methods: Option<Vec<HttpMethod>>,
        /// Also hosts the user enters (high tier).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        private_network: Option<bool>,
        /// Origins the user enters in a setting (G2,
        /// `docs/marketplace/module-migration.md` §7): a GitHub Enterprise
        /// server, a self-managed GitLab, a Jira site. Never granted at
        /// install: the user consents to the exact origin they entered
        /// when they save it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_origins: Option<Vec<UserOriginSlot>>,
        justification: String,
    },
    /// Secret slots, injected by the agent; never readable.
    Credentials {
        refs: Vec<String>,
        justification: String,
    },
    /// Notifications, titled with the plugin's name. Without `routing`
    /// they reach the status bar only; with it, the agent shows them where
    /// the user's settings say (G9).
    Notifications {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        routing: Option<NotificationRouting>,
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
    #[serde(rename_all = "camelCase")]
    OpenUrl {
        /// May be empty only when `user_origins` names at least one
        /// setting.
        hosts: Vec<String>,
        /// Settings of the `network` permission's user-origin slots (G2):
        /// pages on the server the user entered there, and allowed, may be
        /// opened too. Never granted at install: it follows the user's
        /// consent to that exact server.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_origins: Option<Vec<String>>,
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

/// Where a plugin's notifications go, as the user sets it in the plugin's
/// settings (G9): each member names one of the manifest's own settings,
/// which the **agent** reads when it shows a notification, never the
/// plugin. A member that is absent keeps what a plugin without routing
/// gets: no desktop notifications, the status bar, every severity.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationRouting {
    /// A boolean setting: also show them as notifications of the
    /// operating system.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop: Option<String>,
    /// A boolean setting: show them in the status bar of an open window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_bar: Option<String>,
    /// A choice setting of severities (`info`, `success`, `warning`,
    /// `error`): less severe notifications are not shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_severity: Option<String>,
}

/// The severities a [`NotificationRouting::minimum_severity`] setting
/// chooses from, least severe first.
pub const NOTIFICATION_SEVERITIES: [&str; 4] = ["info", "success", "warning", "error"];

impl NotificationRouting {
    /// The members that are set, as `(member, setting key)`.
    pub fn members(&self) -> Vec<(&'static str, &str)> {
        [
            ("desktop", self.desktop.as_deref()),
            ("statusBar", self.status_bar.as_deref()),
            ("minimumSeverity", self.minimum_severity.as_deref()),
        ]
        .into_iter()
        .filter_map(|(member, key)| Some((member, key?)))
        .collect()
    }
}

impl Manifest {
    /// The notification routing the `notifications` permission declares.
    pub fn notification_routing(&self) -> Option<&NotificationRouting> {
        self.permissions.iter().find_map(|p| match p {
            Permission::Notifications { routing, .. } => routing.as_ref(),
            _ => None,
        })
    }
}

/// A user-origin slot of the `network` permission: a `text` setting the
/// user enters one origin into (`https://ghe.example.com`). The agent asks
/// for consent to exactly that origin when the user saves it, and the
/// grant ends when the setting changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserOriginSlot {
    /// The key of a `text` setting in `contributes.settings`.
    pub setting: String,
    /// A subset of the permission's methods; absent means all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub methods: Option<Vec<HttpMethod>>,
    /// Whether the user may enter a path prefix with the origin
    /// (`https://ghe.example.com/api/v3`); requests are then limited to
    /// that path and below.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<bool>,
    /// Whether the user may enter a server on a private network (a LAN
    /// address, a `.internal` name, or a name that resolves to a private
    /// address). Each such origin needs its own explicit consent (high
    /// tier); loopback and link-local addresses are never reachable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_network: Option<bool>,
}

impl Permission {
    /// The origin settings whose server's pages an `openUrl` permission
    /// may open.
    pub fn open_url_user_origins(&self) -> &[String] {
        match self {
            Self::OpenUrl {
                user_origins: Some(settings),
                ..
            } => settings,
            _ => &[],
        }
    }
}

impl Manifest {
    /// Whether pages on the server the user entered in `setting` may be
    /// opened in the browser (`openUrl.userOrigins`).
    pub fn opens_user_origin(&self, setting: &str) -> bool {
        self.permissions
            .iter()
            .flat_map(Permission::open_url_user_origins)
            .any(|s| s == setting)
    }
}

impl UserOriginSlot {
    pub fn path_prefix(&self) -> bool {
        self.path_prefix == Some(true)
    }

    pub fn private_network(&self) -> bool {
        self.private_network == Some(true)
    }
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

    /// The user-origin slots of a `network` permission.
    pub fn user_origins(&self) -> &[UserOriginSlot] {
        match self {
            Self::Network { user_origins, .. } => user_origins.as_deref().unwrap_or_default(),
            _ => &[],
        }
    }

    pub fn justification(&self) -> &str {
        match self {
            Self::Network { justification, .. }
            | Self::Credentials { justification, .. }
            | Self::Notifications { justification, .. }
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
    /// Toolbar entries (plugin API 1.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub navigation: Option<Vec<NavigationEntry>>,
    /// Host-rendered views the navigation entries open (plugin API 1.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub views: Option<Vec<View>>,
}

impl Contributes {
    pub fn navigation(&self) -> &[NavigationEntry] {
        self.navigation.as_deref().unwrap_or_default()
    }

    pub fn views(&self) -> &[View] {
        self.views.as_deref().unwrap_or_default()
    }

    /// The view a navigation entry opens.
    pub fn view_of(&self, entry: &NavigationEntry) -> Option<&View> {
        self.views().iter().find(|v| v.id == entry.view)
    }

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
    /// The controls and inputs the action can be bound to (plugin API
    /// 1.3). Absent: keys only, with press, release, click, double click
    /// and long press (`ActionControls::legacy` in `deckpilot-core`);
    /// `device.requires` never adds a control kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<ActionControls>,
}

/// Most inputs an action may declare for one control kind (the core's
/// `MAX_INPUTS_PER_KIND`).
pub const MAX_INPUTS_PER_KIND: usize = 8;

/// `contributes.actions[].controls`: per control kind, the inputs the
/// action can be bound to, e.g. `{"key": ["press", "click"], "encoder":
/// ["rotate-clockwise"]}`. A kind that is absent is not supported. The
/// same shape as `deckpilot_core::ActionControls`; the lists keep their
/// order (and any repeated input) so validation can report them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActionControls {
    #[serde(deserialize_with = "unique_map::deserialize_keyed")]
    pub bindings: BTreeMap<ControlKind, Vec<InputCapability>>,
}

impl ActionControls {
    /// The declaration as sets, the form the agent registers.
    pub fn as_sets(&self) -> BTreeMap<ControlKind, BTreeSet<InputCapability>> {
        self.bindings
            .iter()
            .map(|(kind, inputs)| (*kind, inputs.iter().copied().collect()))
            .collect()
    }
}

/// The plugin API level that introduced `controls`, `navigation` and
/// `views`.
pub const NAVIGATION_API: ApiLevel = ApiLevel::new(1, 3);

/// The plugin API level that introduced `contributes.views[].context`.
pub const VIEW_CONTEXT_API: ApiLevel = ApiLevel::new(1, 4);

/// Most `contributes.navigation` entries.
pub const MAX_NAVIGATION_ENTRIES: usize = 4;
/// Most `contributes.views`.
pub const MAX_VIEWS: usize = 8;
/// Longest navigation label, in characters.
pub const MAX_NAVIGATION_LABEL: usize = 24;
/// Longest navigation entry and view id.
pub const MAX_NAVIGATION_ID: usize = 32;

/// One entry the plugin adds to the app's toolbar (plugin API 1.3). It
/// opens a host-rendered view; the plugin ships no UI code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavigationEntry {
    pub id: String,
    pub label: String,
    pub icon: NavigationIcon,
    /// The id of one of `contributes.views`.
    pub view: String,
}

/// A navigation icon: one file for both themes, or one per theme (at least
/// one of them). Files are package-relative PNG, WebP or SVG under
/// `assets/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NavigationIcon {
    Single(String),
    Themed(ThemedIcon),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThemedIcon {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dark: Option<String>,
}

/// The theme an icon is shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IconTheme {
    Light,
    Dark,
}

impl NavigationIcon {
    /// The file for `theme`: the theme's own, else the other theme's.
    pub fn file(&self, theme: IconTheme) -> Option<&str> {
        match self {
            Self::Single(path) => Some(path),
            Self::Themed(ThemedIcon { light, dark }) => match theme {
                IconTheme::Light => light.as_deref().or(dark.as_deref()),
                IconTheme::Dark => dark.as_deref().or(light.as_deref()),
            },
        }
    }

    /// Every file the icon names, with its field (`""` for a single file,
    /// `light`, `dark`).
    pub fn files(&self) -> Vec<(&'static str, &str)> {
        match self {
            Self::Single(path) => vec![("", path.as_str())],
            Self::Themed(ThemedIcon { light, dark }) => [("light", light), ("dark", dark)]
                .into_iter()
                .filter_map(|(field, path)| Some((field, path.as_deref()?)))
                .collect(),
        }
    }
}

/// What a view shows. Host-rendered only: no HTML, URLs or plugin code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ViewKind {
    /// The plugin's variables, live.
    Variables,
    /// The Action Library filtered to the plugin's actions.
    Actions,
    /// The plugin's configuration page.
    Settings,
}

/// What a view works on (plugin API 1.4), and so which of the app's
/// toolbar selectors it shows: the device, the profile being edited, both
/// or neither. It changes only the app's own chrome: the plugin is given
/// no device or profile data for it, and it grants no permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ViewContext {
    /// Neither (the default).
    #[default]
    #[serde(rename = "none")]
    None,
    #[serde(rename = "device")]
    Device,
    #[serde(rename = "profile")]
    Profile,
    #[serde(rename = "device+profile")]
    DeviceProfile,
}

impl ViewContext {
    /// The name in manifests and over IPC.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Device => "device",
            Self::Profile => "profile",
            Self::DeviceProfile => "device+profile",
        }
    }
}

/// A view a navigation entry opens (plugin API 1.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct View {
    pub id: String,
    pub kind: ViewKind,
    /// Which toolbar selectors the view shows (plugin API 1.4); `none`
    /// when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<ViewContext>,
}

impl View {
    /// [`View::context`], `none` when absent.
    pub fn context(&self) -> ViewContext {
        self.context.unwrap_or_default()
    }
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
    /// The only hosts the secret is ever sent to (besides the user origin
    /// below). May be empty only with `user_origin`.
    pub hosts: Vec<String>,
    /// The setting of a user-origin slot (G2): the secret is also sent to
    /// the origin the user entered there, once they consented to it, and
    /// nowhere else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_origin: Option<String>,
    pub schemes: Vec<CredentialScheme>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_url: Option<String>,
}
