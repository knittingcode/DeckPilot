//! Static descriptors of the built-in modules that moved to plugins (§7
//! X13, WP-C5, WP-C6): `github`, `gitlab`, `jira`, `git`, `docker`,
//! `workspace` and `projects`. Once a built-in crate is removed, the
//! migration of a configuration saved by 0.9.x must still read it: the
//! module's id and name, its settings (key, kind, default) to read the
//! stored `modules` row with, the capabilities that decide which grants
//! are derived, and its action types with their parameter keys for the
//! contract check. That is all a descriptor holds; nothing of it runs.
//!
//! [`LegacySnapshot::read`](super::LegacySnapshot::read) uses a module's
//! descriptor only when the built-in is not compiled in, so a build with
//! the crates migrates exactly as before. The unit tests below hold every
//! descriptor equal to its built-in while both exist, so they cannot drift
//! before the crate goes.

use deckpilot_core::{
    ActionParameters, ActionType, ActionTypeId, Capability, ModuleId, ModuleManifest,
    ModuleSettings, ParameterKind, ParameterSpec, ParameterValue, Portability,
};

/// A setting's kind, as the built-in declared it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingKind {
    /// One line of at most this many characters.
    Text(usize),
    Integer(i64, i64),
    Boolean,
    Choice(&'static [&'static str]),
    /// The id of a stored credential.
    Credential,
}

/// A setting's default, as the built-in declared it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingDefault {
    None,
    Text(&'static str),
    Integer(i64),
    Boolean(bool),
    Choice(&'static str),
}

/// One setting of a built-in module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LegacySetting {
    pub key: &'static str,
    pub kind: SettingKind,
    pub default: SettingDefault,
    pub required: bool,
}

/// One action type of a built-in module and its parameters' keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LegacyAction {
    pub id: &'static str,
    pub params: &'static [&'static str],
}

/// What the migration knows of a built-in module without its crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LegacyDescriptor {
    pub module: &'static str,
    pub name: &'static str,
    /// Whether a configuration without a `modules` row had it enabled.
    pub enabled_by_default: bool,
    pub capabilities: &'static [Capability],
    pub settings: &'static [LegacySetting],
    pub actions: &'static [LegacyAction],
}

impl LegacySetting {
    fn spec(&self) -> ParameterSpec {
        ParameterSpec {
            key: self.key.to_owned(),
            label: self.key.to_owned(),
            description: None,
            kind: match self.kind {
                SettingKind::Text(max) => ParameterKind::Text {
                    multiline: false,
                    max_length: Some(max),
                },
                SettingKind::Integer(min, max) => ParameterKind::Integer {
                    min: Some(min),
                    max: Some(max),
                },
                SettingKind::Boolean => ParameterKind::Boolean,
                SettingKind::Choice(options) => ParameterKind::Choice {
                    options: options.iter().map(|o| (*o).to_owned()).collect(),
                },
                SettingKind::Credential => ParameterKind::Credential,
            },
            required: self.required,
            default: match self.default {
                SettingDefault::None => None,
                SettingDefault::Text(text) => Some(ParameterValue::Text(text.to_owned())),
                SettingDefault::Integer(n) => Some(ParameterValue::Integer(n)),
                SettingDefault::Boolean(b) => Some(ParameterValue::Boolean(b)),
                SettingDefault::Choice(choice) => Some(ParameterValue::Choice(choice.to_owned())),
            },
        }
    }
}

impl LegacyDescriptor {
    /// The module's manifest as far as the migration reads it: id, name,
    /// capabilities, settings and whether it was enabled by default.
    ///
    /// # Panics
    ///
    /// Never for a descriptor of [`DESCRIPTORS`] (the tests parse each).
    pub fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: ModuleId::new(self.module).expect("a descriptor names a module id"),
            version: String::new(),
            name: self.name.to_owned(),
            description: String::new(),
            platforms: Vec::new(),
            pages: Vec::new(),
            capabilities: self.capabilities.to_vec(),
            config: self.settings.iter().map(LegacySetting::spec).collect(),
            dependencies: Vec::new(),
            enabled_by_default: self.enabled_by_default,
            portability: Portability::default(),
            migrations: Vec::new(),
            windows: Vec::new(),
            variables: Vec::new(),
        }
    }

    /// Whether the module was enabled: as its row says, else by default.
    pub fn enabled(&self, row: Option<&ModuleSettings>) -> bool {
        row.map_or(self.enabled_by_default, |row| row.enabled)
    }

    /// The settings the built-in ran with, as the module registry computes
    /// them: the row's values of the settings it declares, checked against
    /// their kinds, and the defaults for the rest; only the defaults when a
    /// stored value does not fit (the registry kept those too).
    pub fn config(&self, row: Option<&ModuleSettings>) -> ActionParameters {
        let manifest = self.manifest();
        let schema = ActionType {
            editor_navigation: None,
            title: None,
            id: ActionTypeId::new(format!("{}.config", self.module)).expect("module ids are valid"),
            name: self.name.to_owned(),
            description: String::new(),
            category: String::new(),
            parameters: manifest.config,
            requires_project: false,
            deprecated: false,
            controls: None,
            node: deckpilot_core::graph::NodeSpec::default(),
        };
        let mut known = ActionParameters::new();
        for (key, value) in row.iter().flat_map(|row| row.config.iter()) {
            if self.settings.iter().any(|s| s.key == key) {
                known = known.with(key.clone(), value.clone());
            }
        }
        schema
            .normalize(&known)
            .or_else(|_| schema.normalize(&ActionParameters::new()))
            .unwrap_or_default()
    }

    /// The parameter keys of the built-in action type `id`.
    pub fn params_of(&self, id: &str) -> Option<&'static [&'static str]> {
        self.actions.iter().find(|a| a.id == id).map(|a| a.params)
    }
}

/// The descriptor of `module`, if it is one of the moved built-ins.
pub(crate) fn of(module: &str) -> Option<&'static LegacyDescriptor> {
    DESCRIPTORS.iter().find(|d| d.module == module)
}

/// Every moved built-in.
pub(crate) const DESCRIPTORS: &[LegacyDescriptor] = &[
    GITHUB_MODULE,
    GITLAB_MODULE,
    JIRA_MODULE,
    GIT_MODULE,
    DOCKER_MODULE,
    WORKSPACE_MODULE,
    PROJECTS_MODULE,
];

const fn setting(
    key: &'static str,
    kind: SettingKind,
    default: SettingDefault,
    required: bool,
) -> LegacySetting {
    LegacySetting {
        key,
        kind,
        default,
        required,
    }
}

const fn action(id: &'static str, params: &'static [&'static str]) -> LegacyAction {
    LegacyAction { id, params }
}

/// The settings every server module (`github`, `gitlab`, `jira`) has
/// besides its origin.
const fn timeout() -> LegacySetting {
    setting(
        "timeoutSeconds",
        SettingKind::Integer(1, 120),
        SettingDefault::Integer(20),
        true,
    )
}

const fn private_host() -> LegacySetting {
    setting(
        "allowPrivateHost",
        SettingKind::Boolean,
        SettingDefault::Boolean(false),
        true,
    )
}

const SERVER: &[Capability] = &[
    Capability::Network,
    Capability::Credentials,
    Capability::OpenExternal,
];

/// `github` (removed by WP-C5).
pub(crate) mod github {
    pub const MODULE_ID: &str = "github";
    pub const CREDENTIAL: &str = "credential";
    pub const API_URL: &str = "apiUrl";
    pub const DEFAULT_API_URL: &str = "https://api.github.com";
    pub const ALLOW_PRIVATE: &str = "allowPrivateHost";
}

const GITHUB_MODULE: LegacyDescriptor = LegacyDescriptor {
    module: github::MODULE_ID,
    name: "GitHub",
    enabled_by_default: true,
    capabilities: SERVER,
    settings: &[
        setting(
            github::API_URL,
            SettingKind::Text(512),
            SettingDefault::Text(github::DEFAULT_API_URL),
            true,
        ),
        setting(
            github::CREDENTIAL,
            SettingKind::Credential,
            SettingDefault::Text("github.token"),
            true,
        ),
        timeout(),
        private_host(),
    ],
    actions: &[
        action("github.current-user", &["credential"]),
        action("github.repository", &["repository", "credential"]),
        action(
            "github.pull-requests",
            &["repository", "filter", "limit", "credential"],
        ),
        action(
            "github.review-requests",
            &["repository", "limit", "credential"],
        ),
        action(
            "github.checks",
            &["repository", "ref", "pullRequest", "credential"],
        ),
        action("github.open", &["url", "repository", "pullRequest"]),
    ],
};

/// `gitlab` (removed by WP-C5).
pub(crate) mod gitlab {
    pub const MODULE_ID: &str = "gitlab";
    pub const CREDENTIAL: &str = "credential";
    pub const API_URL: &str = "apiUrl";
    pub const DEFAULT_API_URL: &str = "https://gitlab.com/api/v4";
    pub const ALLOW_PRIVATE: &str = "allowPrivateHost";
}

const GITLAB_MODULE: LegacyDescriptor = LegacyDescriptor {
    module: gitlab::MODULE_ID,
    name: "GitLab",
    enabled_by_default: true,
    capabilities: SERVER,
    settings: &[
        setting(
            gitlab::API_URL,
            SettingKind::Text(512),
            SettingDefault::Text(gitlab::DEFAULT_API_URL),
            true,
        ),
        setting(
            gitlab::CREDENTIAL,
            SettingKind::Credential,
            SettingDefault::Text("gitlab.token"),
            true,
        ),
        timeout(),
        private_host(),
    ],
    actions: &[
        action("gitlab.current-user", &["credential"]),
        action("gitlab.project", &["project", "credential"]),
        action(
            "gitlab.merge-requests",
            &["project", "filter", "limit", "credential"],
        ),
        action(
            "gitlab.review-requests",
            &["project", "limit", "credential"],
        ),
        action(
            "gitlab.pipeline",
            &["project", "ref", "mergeRequest", "credential"],
        ),
        action("gitlab.open", &["url", "project", "mergeRequest"]),
    ],
};

/// `jira` (removed by WP-C5).
pub(crate) mod jira {
    pub const MODULE_ID: &str = "jira";
    pub const CREDENTIAL: &str = "credential";
    pub const SITE_URL: &str = "siteUrl";
    pub const ALLOW_PRIVATE: &str = "allowPrivateHost";
}

const JIRA_MODULE: LegacyDescriptor = LegacyDescriptor {
    module: jira::MODULE_ID,
    name: "Jira",
    enabled_by_default: true,
    capabilities: SERVER,
    settings: &[
        setting(
            jira::SITE_URL,
            SettingKind::Text(512),
            SettingDefault::None,
            false,
        ),
        setting(
            "deployment",
            SettingKind::Choice(&["cloud", "data-center"]),
            SettingDefault::Choice("cloud"),
            true,
        ),
        setting(
            jira::CREDENTIAL,
            SettingKind::Credential,
            SettingDefault::Text("jira.token"),
            true,
        ),
        timeout(),
        private_host(),
    ],
    actions: &[
        action("jira.myself", &["credential"]),
        action("jira.issue", &["issue", "credential"]),
        action(
            "jira.issues",
            &[
                "source",
                "filterId",
                "project",
                "includeDone",
                "limit",
                "credential",
            ],
        ),
        action("jira.open", &["url", "issue"]),
    ],
};

/// `git` (removed by WP-C6).
pub(crate) mod git {
    pub const MODULE_ID: &str = "git";
    pub const EXECUTABLE: &str = "gitExecutable";
    pub const NETWORK_TIMEOUT: &str = "networkTimeoutSeconds";
}

const GIT_MODULE: LegacyDescriptor = LegacyDescriptor {
    module: git::MODULE_ID,
    name: "Git",
    enabled_by_default: true,
    capabilities: &[
        Capability::ProfileRead,
        Capability::MachineResources,
        Capability::ProcessExecution,
        Capability::OpenExternal,
    ],
    settings: &[
        setting(
            git::EXECUTABLE,
            SettingKind::Text(1024),
            SettingDefault::None,
            false,
        ),
        setting(
            "repositoryRole",
            SettingKind::Text(64),
            SettingDefault::Text("repository"),
            true,
        ),
        setting(
            git::NETWORK_TIMEOUT,
            SettingKind::Integer(5, 120),
            SettingDefault::Integer(20),
            true,
        ),
    ],
    actions: &[
        action("git.status", &["repository", "project_id"]),
        action("git.branch", &["repository", "project_id"]),
        action("git.changes", &["repository", "project_id", "limit"]),
        action("git.last-commit", &["repository", "project_id"]),
        action(
            "git.remote-status",
            &["repository", "project_id", "refresh"],
        ),
        action(
            "git.open-repository",
            &["repository", "project_id", "application"],
        ),
    ],
};

/// `docker` (removed by WP-C6).
pub(crate) mod docker {
    pub const MODULE_ID: &str = "docker";
    pub const EXECUTABLE: &str = "dockerExecutable";
    pub const CONTEXT: &str = "context";
    pub const ALLOW_STATE_CHANGES: &str = "allowStateChanges";
    pub const TIMEOUT: &str = "timeoutSeconds";
}

const DOCKER_MODULE: LegacyDescriptor = LegacyDescriptor {
    module: docker::MODULE_ID,
    name: "Docker",
    enabled_by_default: true,
    capabilities: &[
        Capability::ProfileRead,
        Capability::MachineResources,
        Capability::ProcessExecution,
    ],
    settings: &[
        setting(
            docker::EXECUTABLE,
            SettingKind::Text(1024),
            SettingDefault::None,
            false,
        ),
        setting(
            docker::CONTEXT,
            SettingKind::Text(1024),
            SettingDefault::None,
            false,
        ),
        setting(
            docker::ALLOW_STATE_CHANGES,
            SettingKind::Boolean,
            SettingDefault::Boolean(false),
            true,
        ),
        setting(
            "composeRole",
            SettingKind::Text(1024),
            SettingDefault::Text("compose"),
            true,
        ),
        setting(
            docker::TIMEOUT,
            SettingKind::Integer(10, 1800),
            SettingDefault::Integer(300),
            true,
        ),
    ],
    actions: &[
        action("docker.status", &["context"]),
        action("docker.containers", &["all", "name", "context"]),
        action(
            "docker.compose-status",
            &["compose", "project_id", "projectName", "context"],
        ),
        action(
            "docker.compose-up",
            &[
                "compose",
                "project_id",
                "projectName",
                "context",
                "services",
            ],
        ),
        action(
            "docker.compose-down",
            &[
                "compose",
                "project_id",
                "projectName",
                "context",
                "removeOrphans",
            ],
        ),
        action(
            "docker.compose-restart",
            &[
                "compose",
                "project_id",
                "projectName",
                "context",
                "services",
            ],
        ),
        action(
            "docker.logs",
            &[
                "container",
                "service",
                "lines",
                "since",
                "compose",
                "project_id",
                "projectName",
                "context",
            ],
        ),
    ],
};

/// `workspace` (removed by WP-C6).
pub(crate) mod workspace {
    pub const MODULE_ID: &str = "workspace";
}

const WORKSPACE_MODULE: LegacyDescriptor = LegacyDescriptor {
    module: workspace::MODULE_ID,
    name: "Workspace",
    enabled_by_default: true,
    capabilities: &[
        Capability::ProfileRead,
        Capability::MachineResources,
        Capability::OpenExternal,
    ],
    settings: &[],
    actions: &[
        action(
            "workspace.open-in-editor",
            &[
                "application",
                "editor",
                "location",
                "path",
                "line",
                "column",
            ],
        ),
        action("workspace.open-urls", &["urls", "browser"]),
        action(
            "workspace.open-project",
            &[
                "editor",
                "editorRole",
                "repositoryRole",
                "docsRole",
                "urlRoles",
                "browser",
            ],
        ),
    ],
};

/// `projects` (removed by WP-C6; its `ProjectCatalog` moves to the core).
pub(crate) mod projects {
    pub const MODULE_ID: &str = "projects";
}

const PROJECTS_MODULE: LegacyDescriptor = LegacyDescriptor {
    module: projects::MODULE_ID,
    name: "Projects",
    enabled_by_default: true,
    capabilities: &[
        Capability::ProfileRead,
        Capability::ProjectContext,
        Capability::MachineResources,
    ],
    settings: &[setting(
        "wrapAround",
        SettingKind::Boolean,
        SettingDefault::Boolean(true),
        true,
    )],
    actions: &[
        action("projects.cycle", &["direction"]),
        action("projects.select", &["project_id"]),
        action("projects.current", &[]),
        action("projects.repository", &["project_id"]),
        action("projects.documentation", &["project_id"]),
        action("projects.resolve-resource", &["project_id", "resource"]),
        action("projects.next", &[]),
        action("projects.previous", &[]),
        action("projects.active", &[]),
    ],
};

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde::Deserialize;

    use super::*;

    /// The frozen contract of a removed built-in: its manifest and action
    /// types as 0.9.0 declared them (`crates/deckpilot-modules/legacy`,
    /// written from the crates before they were removed).
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Frozen {
        manifest: ModuleManifest,
        action_types: Vec<ActionType>,
    }

    macro_rules! frozen {
        ($id:literal) => {
            (
                $id,
                include_str!(concat!(
                    "../../../../../crates/deckpilot-modules/legacy/",
                    $id,
                    ".json"
                )),
            )
        };
    }

    fn builtins() -> Vec<Frozen> {
        [
            frozen!("github"),
            frozen!("gitlab"),
            frozen!("jira"),
            frozen!("git"),
            frozen!("docker"),
            frozen!("workspace"),
            frozen!("projects"),
        ]
        .into_iter()
        .map(|(id, text)| {
            let frozen: Frozen = serde_json::from_str(text).expect("a frozen contract parses");
            assert_eq!(frozen.manifest.id.as_str(), id);
            frozen
        })
        .collect()
    }

    /// Every descriptor is exactly what the migration read of its
    /// built-in, as frozen when the crate was removed: name, default
    /// state, capabilities, platforms (all), every setting's key, kind,
    /// default and `required`, every action type with its parameter keys.
    #[test]
    fn every_descriptor_is_its_builtin() {
        let modules = builtins();
        assert_eq!(modules.len(), DESCRIPTORS.len());
        for module in modules {
            let builtin = &module.manifest;
            let descriptor = of(builtin.id.as_str())
                .unwrap_or_else(|| panic!("no descriptor of {}", builtin.id));
            let described = descriptor.manifest();
            assert_eq!(described.id, builtin.id);
            assert_eq!(described.name, builtin.name, "{}", builtin.id);
            assert_eq!(
                described.enabled_by_default, builtin.enabled_by_default,
                "{}",
                builtin.id
            );
            assert!(builtin.platforms.is_empty(), "{}", builtin.id);
            assert_eq!(
                described.capabilities, builtin.capabilities,
                "{}",
                builtin.id
            );
            let settings = |config: &[ParameterSpec]| {
                config
                    .iter()
                    .map(|s| (s.key.clone(), s.kind.clone(), s.default.clone(), s.required))
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                settings(&described.config),
                settings(&builtin.config),
                "{}",
                builtin.id
            );
            let defaults = descriptor.config(None);
            let config = builtin
                .config
                .iter()
                .filter_map(|s| s.default.clone().map(|d| (s.key.clone(), d)))
                .fold(ActionParameters::new(), |p, (k, v)| p.with(k, v));
            let actions: Vec<(String, Vec<String>)> = module
                .action_types
                .iter()
                .map(|d| {
                    (
                        d.id.to_string(),
                        d.parameters.iter().map(|p| p.key.clone()).collect(),
                    )
                })
                .collect();
            let declared: Vec<(String, Vec<String>)> = descriptor
                .actions
                .iter()
                .map(|a| {
                    (
                        a.id.to_owned(),
                        a.params.iter().map(|p| (*p).to_owned()).collect(),
                    )
                })
                .collect();
            assert_eq!(declared, actions, "{}", builtin.id);
            assert_eq!(defaults, config, "{}", builtin.id);
        }
    }

    /// The keys the plans and the host's tool settings name are settings
    /// the built-ins declared.
    #[test]
    fn the_named_keys_are_the_builtins() {
        let modules = builtins();
        let declares = |module: &str, key: &str| {
            let frozen = modules
                .iter()
                .find(|m| m.manifest.id.as_str() == module)
                .unwrap_or_else(|| panic!("{module} is frozen"));
            assert!(
                frozen.manifest.config.iter().any(|s| s.key == key),
                "{module} declares `{key}`"
            );
        };
        for (module, keys) in [
            (
                github::MODULE_ID,
                &[github::CREDENTIAL, github::API_URL, github::ALLOW_PRIVATE][..],
            ),
            (
                gitlab::MODULE_ID,
                &[gitlab::CREDENTIAL, gitlab::API_URL, gitlab::ALLOW_PRIVATE],
            ),
            (
                jira::MODULE_ID,
                &[jira::CREDENTIAL, jira::SITE_URL, jira::ALLOW_PRIVATE],
            ),
            (git::MODULE_ID, &[git::EXECUTABLE, git::NETWORK_TIMEOUT]),
            (
                docker::MODULE_ID,
                &[
                    docker::EXECUTABLE,
                    docker::CONTEXT,
                    docker::ALLOW_STATE_CHANGES,
                    docker::TIMEOUT,
                ],
            ),
        ] {
            for key in keys {
                declares(module, key);
            }
        }
        let default = |module: &str, key: &str| {
            modules
                .iter()
                .find(|m| m.manifest.id.as_str() == module)
                .and_then(|m| m.manifest.config.iter().find(|s| s.key == key))
                .and_then(|s| s.default.clone())
        };
        assert_eq!(
            default(github::MODULE_ID, github::API_URL),
            Some(ParameterValue::Text(github::DEFAULT_API_URL.into()))
        );
        assert_eq!(
            default(gitlab::MODULE_ID, gitlab::API_URL),
            Some(ParameterValue::Text(gitlab::DEFAULT_API_URL.into()))
        );
        assert!(
            modules
                .iter()
                .any(|m| m.manifest.id.as_str() == workspace::MODULE_ID)
        );
        assert!(
            modules
                .iter()
                .any(|m| m.manifest.id.as_str() == projects::MODULE_ID)
        );
    }

    /// One descriptor per module, each with a plan and an alias block,
    /// and every aliased legacy id described.
    #[test]
    fn every_aliased_module_but_notifications_is_described() {
        let aliased: BTreeSet<&str> = deckpilot_core::alias::ACTION_ALIASES
            .iter()
            .map(|a| a.module)
            .filter(|m| *m != "notifications")
            .collect();
        let described: BTreeSet<&str> = DESCRIPTORS.iter().map(|d| d.module).collect();
        assert_eq!(described, aliased);
        for alias in deckpilot_core::alias::ACTION_ALIASES {
            if let Some(descriptor) = of(alias.module) {
                assert!(
                    descriptor.params_of(alias.legacy).is_some(),
                    "{}",
                    alias.legacy
                );
                assert!(super::super::plan_for(alias.module).is_some());
            }
        }
    }

    /// The settings of a stored row: its values where they fit, defaults
    /// for the rest; settings the module never had are not read; the
    /// stored enabled state, else enabled.
    #[test]
    fn a_row_is_read_as_the_registry_read_it() {
        let docker = of("docker").unwrap();
        let row = ModuleSettings {
            enabled: false,
            config: ActionParameters::new()
                .with("timeoutSeconds", ParameterValue::Integer(900))
                .with(
                    "dockerExecutable",
                    ParameterValue::Text("/opt/docker".into()),
                )
                .with("gone", ParameterValue::Boolean(true)),
        };
        let config = docker.config(Some(&row));
        assert_eq!(
            config.get("timeoutSeconds"),
            Some(&ParameterValue::Integer(900))
        );
        assert_eq!(config.text("dockerExecutable"), Some("/opt/docker"));
        assert_eq!(config.text("composeRole"), Some("compose"));
        assert_eq!(config.get("gone"), None);
        assert!(!docker.enabled(Some(&row)));
        assert!(docker.enabled(None));
        // A value that does not fit: the defaults, as the registry kept.
        let wrong = ModuleSettings {
            enabled: true,
            config: ActionParameters::new().with("timeoutSeconds", ParameterValue::Integer(1)),
        };
        assert_eq!(
            docker.config(Some(&wrong)).get("timeoutSeconds"),
            Some(&ParameterValue::Integer(300))
        );
    }
}
