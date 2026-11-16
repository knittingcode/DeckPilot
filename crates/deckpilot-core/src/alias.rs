//! Action type aliases: legacy built-in action types that a first-party
//! plugin can serve (`docs/marketplace/module-migration.md` §5.2).
//!
//! When a built-in module moves to a plugin, its action types get new ids
//! in the plugin's namespace (`github.pull-requests` →
//! `deckpilot.github.pull-requests`). Stored documents are **never**
//! rewritten: profiles, bindings, workflow nodes and exports keep the
//! legacy id, so an older app, which still has the built-in, keeps working
//! on the same database. Instead, [`ACTION_ALIASES`] lists each legacy id
//! with the plugin action type that replaces it, and
//! [`ModuleRegistry::action_registry`](crate::ModuleRegistry::action_registry)
//! routes the legacy id when it builds the registry that executes actions:
//!
//! - to the plugin's handler, through an
//!   [`AliasHandler`](crate::registry::AliasHandler) with the plugin's
//!   capabilities, when the agent's [`LegacyRouting`] says the module was
//!   migrated and the plugin is registered, enabled and compatible;
//! - to the built-in handler, as before, otherwise (a fallback when the
//!   plugin is broken, quarantined, incompatible or missing);
//! - to nothing, with a reason, when the user disabled or removed the
//!   plugin, or when there is no built-in to fall back to.
//!
//! The table is checked when the crate compiles ([`validate`]): a legacy id
//! is a built-in id, a target is always an action type of the plugin
//! `deckpilot.<module>` with the same action name, and nothing is listed
//! twice. The first-party publisher [`FIRST_PARTY_PUBLISHER`] and the
//! [`RESERVED_MODULE_IDS`] are part of the same identity rules (§5.1).

use std::collections::{BTreeMap, BTreeSet};

use crate::{ActionType, ActionTypeId, ModuleId, ParameterKind, ParameterSpec};

/// The publisher of first-party plugins. Only first-party plugins can be
/// alias targets; the agent refuses plugins of this publisher from any
/// source but the bundled set and the registry.
pub const FIRST_PARTY_PUBLISHER: &str = "deckpilot";

/// Module ids that are never assigned to anything. Before schema 3,
/// `deckpilot.core.log`, `deckpilot.project.cycle`,
/// `deckpilot.project.select` and `deckpilot.resource.resolve` were stored
/// action types; an old export that still contains them must never resolve
/// to a plugin. [`ModuleRegistry::register`](crate::ModuleRegistry::register)
/// refuses them.
pub const RESERVED_MODULE_IDS: [&str; 3] =
    ["deckpilot.core", "deckpilot.project", "deckpilot.resource"];

/// How a parameter of the legacy type that the plugin cannot declare is
/// translated (§5.3). Applied to the parameters after overrides and
/// mappings, so it covers instances, bindings and workflow nodes alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamRule {
    /// The parameter (a `credential` override) is dropped when it is empty
    /// or equals the legacy module's setting of the same key; the plugin
    /// uses its own credential slot. Any other value cannot be translated:
    /// the run fails, and the agent does not migrate such a module.
    DropIfDefault(&'static str),
}

impl ParamRule {
    pub const fn key(self) -> &'static str {
        match self {
            Self::DropIfDefault(key) => key,
        }
    }
}

/// A legacy built-in action type and the plugin action type that replaces
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionAlias {
    /// The stored id, e.g. `github.pull-requests`.
    pub legacy: &'static str,
    /// The plugin's action type, e.g. `deckpilot.github.pull-requests`.
    pub target: &'static str,
    /// The legacy owner, e.g. `github`.
    pub module: &'static str,
    /// The plugin, e.g. `deckpilot.github`.
    pub plugin: &'static str,
    /// The app version that added the alias.
    pub since: &'static str,
    /// Translations of parameters the plugin cannot declare; usually none.
    pub params: &'static [ParamRule],
}

impl ActionAlias {
    /// # Panics
    ///
    /// Never for an entry that passed [`validate`].
    pub fn legacy_id(&self) -> ActionTypeId {
        ActionTypeId::new(self.legacy).expect("validated alias")
    }

    /// # Panics
    ///
    /// Never for an entry that passed [`validate`].
    pub fn target_id(&self) -> ActionTypeId {
        ActionTypeId::new(self.target).expect("validated alias")
    }

    /// # Panics
    ///
    /// Never for an entry that passed [`validate`].
    pub fn module_id(&self) -> ModuleId {
        ModuleId::new(self.module).expect("validated alias")
    }

    /// # Panics
    ///
    /// Never for an entry that passed [`validate`].
    pub fn plugin_id(&self) -> ModuleId {
        ModuleId::new(self.plugin).expect("validated alias")
    }

    /// Whether the parameter `key` of the legacy type is translated.
    pub fn translates(&self, key: &str) -> bool {
        self.params.iter().any(|rule| rule.key() == key)
    }
}

/// Every alias, one block per module cut over to its plugin; entries are
/// never removed afterwards, also not when the built-in is.
///
/// Listing a module here changes nothing by itself: the agent's migration
/// journal decides the [`LegacyRouting`], so until a computer's migration
/// of the module is cut over, and in every build without plugins, the
/// legacy ids keep running the built-in.
pub const ACTION_ALIASES: &[ActionAlias] = &[
    // GitHub (P9): `deckpilot.github` (`plugins/first-party/github`), its
    // contract checked by `crates/deckpilot-modules/tests/contracts.rs`.
    // The per-action `credential` override is dropped when it is empty or
    // names the module's credential (the plugin uses its slot `token`);
    // `github.open` has none.
    github("github.current-user", "deckpilot.github.current-user", true),
    github("github.repository", "deckpilot.github.repository", true),
    github(
        "github.pull-requests",
        "deckpilot.github.pull-requests",
        true,
    ),
    github(
        "github.review-requests",
        "deckpilot.github.review-requests",
        true,
    ),
    github("github.checks", "deckpilot.github.checks", true),
    github("github.open", "deckpilot.github.open", false),
    // GitLab (P11): `deckpilot.gitlab` (`plugins/first-party/gitlab`), for
    // GitLab.com or a self-managed GitLab the user enters; its contract
    // checked by `crates/deckpilot-modules/tests/contracts.rs`. As
    // GitHub's, the `credential` override is dropped when it is empty or
    // the module's (the plugin uses its slot `token`); `gitlab.open` has
    // none.
    server(
        "gitlab",
        "deckpilot.gitlab",
        "gitlab.current-user",
        "deckpilot.gitlab.current-user",
        true,
    ),
    server(
        "gitlab",
        "deckpilot.gitlab",
        "gitlab.project",
        "deckpilot.gitlab.project",
        true,
    ),
    server(
        "gitlab",
        "deckpilot.gitlab",
        "gitlab.merge-requests",
        "deckpilot.gitlab.merge-requests",
        true,
    ),
    server(
        "gitlab",
        "deckpilot.gitlab",
        "gitlab.review-requests",
        "deckpilot.gitlab.review-requests",
        true,
    ),
    server(
        "gitlab",
        "deckpilot.gitlab",
        "gitlab.pipeline",
        "deckpilot.gitlab.pipeline",
        true,
    ),
    server(
        "gitlab",
        "deckpilot.gitlab",
        "gitlab.open",
        "deckpilot.gitlab.open",
        false,
    ),
    // Jira (P11): `deckpilot.jira` (`plugins/first-party/jira`), for the
    // Jira site the user enters (the plugin's slot `credential`);
    // `jira.open` has no `credential`.
    server(
        "jira",
        "deckpilot.jira",
        "jira.myself",
        "deckpilot.jira.myself",
        true,
    ),
    server(
        "jira",
        "deckpilot.jira",
        "jira.issue",
        "deckpilot.jira.issue",
        true,
    ),
    server(
        "jira",
        "deckpilot.jira",
        "jira.issues",
        "deckpilot.jira.issues",
        true,
    ),
    server(
        "jira",
        "deckpilot.jira",
        "jira.open",
        "deckpilot.jira.open",
        false,
    ),
    // Notifications (P10): `deckpilot.notifications`
    // (`plugins/first-party/notifications`), the first plugin of the
    // default set; its contract checked by
    // `crates/deckpilot-modules/tests/contracts.rs`. No parameter needs a
    // translation.
    notifications("notifications.show", "deckpilot.notifications.show"),
    notifications(
        "notifications.workflow-summary",
        "deckpilot.notifications.workflow-summary",
    ),
];

/// One alias of the `github` block.
const fn github(legacy: &'static str, target: &'static str, credential: bool) -> ActionAlias {
    ActionAlias {
        legacy,
        target,
        module: "github",
        plugin: "deckpilot.github",
        since: "0.6.0",
        params: if credential {
            &[ParamRule::DropIfDefault("credential")]
        } else {
            &[]
        },
    }
}

/// One alias of a module whose plugin talks to a server the user enters
/// (the `gitlab` and `jira` blocks).
const fn server(
    module: &'static str,
    plugin: &'static str,
    legacy: &'static str,
    target: &'static str,
    credential: bool,
) -> ActionAlias {
    ActionAlias {
        legacy,
        target,
        module,
        plugin,
        since: "0.6.0",
        params: if credential {
            &[ParamRule::DropIfDefault("credential")]
        } else {
            &[]
        },
    }
}

/// One alias of the `notifications` block.
const fn notifications(legacy: &'static str, target: &'static str) -> ActionAlias {
    ActionAlias {
        legacy,
        target,
        module: "notifications",
        plugin: "deckpilot.notifications",
        since: "0.6.0",
        params: &[],
    }
}

const _: () = assert_valid(ACTION_ALIASES);

/// Why an alias table is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AliasError {
    /// The entry's legacy id.
    pub legacy: &'static str,
    pub reason: &'static str,
}

/// Fails to compile (in a `const`) or panics for a table that
/// [`validate`] refuses.
///
/// # Panics
///
/// For an invalid table.
pub const fn assert_valid(aliases: &[ActionAlias]) {
    if let Err(error) = validate(aliases) {
        panic!("{}", error.reason);
    }
}

/// Checks an alias table:
///
/// - `module` is a built-in module id (no `.`), not empty;
/// - `legacy` is `<module>.<name>`, a built-in action type;
/// - `target` is a plugin action type `<publisher>.<plugin>.<name>`, never a
///   built-in id;
/// - `plugin` is `deckpilot.<module>`: only first-party plugins are
///   targets, and never a reserved id;
/// - `target` is `<plugin>.<name>`: the action keeps its legacy name;
/// - `since` is set, parameter rules name distinct keys, and no legacy id
///   is listed twice.
///
/// The ids' characters are checked by the tests that parse every entry
/// with [`ActionTypeId::new`] and [`ModuleId::new`].
pub const fn validate(aliases: &[ActionAlias]) -> Result<(), AliasError> {
    let mut i = 0;
    while i < aliases.len() {
        let alias = &aliases[i];
        macro_rules! error {
            ($reason:expr $(,)?) => {
                Err(AliasError {
                    legacy: alias.legacy,
                    reason: $reason,
                })
            };
        }
        let module = alias.module.as_bytes();
        let legacy = alias.legacy.as_bytes();
        let target = alias.target.as_bytes();
        let plugin = alias.plugin.as_bytes();
        if module.is_empty() || count(module, b'.') != 0 {
            return error!("an alias's module is a built-in module id, without `.`");
        }
        if count(legacy, b'.') != 1 || !starts_with(legacy, module, b'.') {
            return error!("an alias's legacy id is a built-in action type `<module>.<name>`");
        }
        let name = suffix(legacy, module.len() + 1);
        if name.is_empty() {
            return error!("an alias's legacy id is a built-in action type `<module>.<name>`");
        }
        if count(target, b'.') != 2 {
            return error!(
                "an alias's target is a plugin action type `<publisher>.<plugin>.<name>`, never \
                 a built-in id",
            );
        }
        let publisher = FIRST_PARTY_PUBLISHER.as_bytes();
        if !starts_with(plugin, publisher, b'.') {
            return error!("an alias's target is a first-party plugin, `deckpilot.<module>`");
        }
        if !equal(suffix(plugin, publisher.len() + 1), module) {
            return error!("an alias's plugin is `deckpilot.<module>`, named after the module");
        }
        let mut reserved = 0;
        while reserved < RESERVED_MODULE_IDS.len() {
            if equal(plugin, RESERVED_MODULE_IDS[reserved].as_bytes()) {
                return error!("an alias's plugin is a reserved id");
            }
            reserved += 1;
        }
        if !starts_with(target, plugin, b'.') || !equal(suffix(target, plugin.len() + 1), name) {
            return error!("an alias's target is `<plugin>.<name>`, with the legacy action name");
        }
        if alias.since.is_empty() {
            return error!("an alias names the version that added it");
        }
        let mut rule = 0;
        while rule < alias.params.len() {
            let key = alias.params[rule].key().as_bytes();
            if key.is_empty() {
                return error!("a parameter rule names a parameter");
            }
            let mut other = 0;
            while other < rule {
                if equal(alias.params[other].key().as_bytes(), key) {
                    return error!("a parameter has at most one rule");
                }
                other += 1;
            }
            rule += 1;
        }
        let mut earlier = 0;
        while earlier < i {
            if equal(aliases[earlier].legacy.as_bytes(), legacy) {
                return error!("a legacy id has at most one alias");
            }
            earlier += 1;
        }
        i += 1;
    }
    Ok(())
}

const fn count(bytes: &[u8], byte: u8) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == byte {
            n += 1;
        }
        i += 1;
    }
    n
}

const fn equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// `bytes` is `prefix`, `separator`, and something more.
const fn starts_with(bytes: &[u8], prefix: &[u8], separator: u8) -> bool {
    if bytes.len() <= prefix.len() + 1 || bytes[prefix.len()] != separator {
        return false;
    }
    let mut i = 0;
    while i < prefix.len() {
        if bytes[i] != prefix[i] {
            return false;
        }
        i += 1;
    }
    true
}

const fn suffix(bytes: &[u8], from: usize) -> &[u8] {
    if from >= bytes.len() {
        return &[];
    }
    bytes.split_at(from).1
}

/// The alias of `legacy` in `aliases`, if it has one.
pub fn lookup<'a>(aliases: &'a [ActionAlias], legacy: &ActionTypeId) -> Option<&'a ActionAlias> {
    aliases.iter().find(|alias| alias.legacy == legacy.as_str())
}

/// The alias of `legacy` in [`ACTION_ALIASES`], if it has one.
pub fn alias_of(legacy: &ActionTypeId) -> Option<&'static ActionAlias> {
    lookup(ACTION_ALIASES, legacy)
}

/// Where the legacy action types of one built-in module go, decided by the
/// agent from its migration records. The core never reads the plugin
/// store.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LegacyRoute {
    /// The built-in handles them: today's behaviour, and the default.
    #[default]
    Builtin,
    /// The module was migrated: the plugin handles them while it can, the
    /// built-in (if compiled in) when the plugin is broken, quarantined,
    /// incompatible or missing because of a fault.
    Plugin,
    /// The module was migrated and the user then disabled or uninstalled
    /// the plugin. That choice is respected: no fallback to the built-in.
    PluginWithdrawn,
}

/// The [`LegacyRoute`] of every migrated module. Modules without an entry
/// are routed to their built-in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LegacyRouting(BTreeMap<ModuleId, LegacyRoute>);

impl LegacyRouting {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with(mut self, module: ModuleId, route: LegacyRoute) -> Self {
        self.0.insert(module, route);
        self
    }

    pub fn route(&self, module: &ModuleId) -> LegacyRoute {
        self.0.get(module).copied().unwrap_or_default()
    }
}

/// How a legacy action type is served right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AliasOutcome {
    /// By the built-in handler, as before any migration.
    Builtin,
    /// By the plugin's handler, under the legacy id.
    Plugin,
    /// By the built-in handler because the plugin cannot serve it; shown
    /// as a warning in the Activity view.
    Fallback { reason: String },
    /// By nothing.
    Unavailable { reason: String },
}

/// One alias and how it is routed, for reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasStatus {
    pub legacy: ActionTypeId,
    pub target: ActionTypeId,
    pub plugin: ModuleId,
    pub outcome: AliasOutcome,
}

/// Why the plugin action type `target` cannot replace the built-in
/// `builtin` under `alias` (§5.3); empty when it can. The plugin must
/// declare:
///
/// - every parameter of the built-in, except those `alias` translates,
///   with a compatible kind (the same choices, ranges and lengths at least
///   as wide) and not required where the built-in's is optional; new
///   parameters only when optional;
/// - every output of the built-in, with the same kind and `sensitive`
///   flag;
/// - the same `effect`, `retrySafe`, `idempotent` and `changesContext`, a
///   timeout at least as long, and `cancellable` wherever the built-in is
///   (the agent makes every plugin action cancellable: it can always stop
///   the plugin, so a plugin replacing an action that cannot be
///   interrupted, such as `github.open`, only stops sooner).
pub fn incompatibilities(
    alias: &ActionAlias,
    builtin: &ActionType,
    target: &ActionType,
) -> Vec<String> {
    let mut problems = Vec::new();
    for spec in &builtin.parameters {
        if alias.translates(&spec.key) {
            continue;
        }
        let Some(other) = target.parameters.iter().find(|p| p.key == spec.key) else {
            problems.push(format!("parameter `{}` is missing", spec.key));
            continue;
        };
        if let Err(reason) = kind_compatible(&spec.kind, &other.kind) {
            problems.push(format!("parameter `{}`: {reason}", spec.key));
        }
        if !spec.required && is_required(other) {
            problems.push(format!("parameter `{}` became required", spec.key));
        }
    }
    for spec in &target.parameters {
        if !builtin.parameters.iter().any(|p| p.key == spec.key) && is_required(spec) {
            problems.push(format!("new parameter `{}` is required", spec.key));
        }
    }
    for output in &builtin.node.outputs {
        match target.node.outputs.iter().find(|o| o.key == output.key) {
            None => problems.push(format!("output `{}` is missing", output.key)),
            Some(other) if other.kind != output.kind => {
                problems.push(format!("output `{}` has another kind", output.key));
            }
            Some(other) if other.sensitive != output.sensitive => {
                problems.push(format!(
                    "output `{}` has another `sensitive` flag",
                    output.key
                ));
            }
            Some(_) => {}
        }
    }
    let (b, t) = (&builtin.node, &target.node);
    for (field, same) in [
        ("effect", b.effect == t.effect),
        ("retrySafe", b.retry_safe == t.retry_safe),
        ("idempotent", b.idempotent == t.idempotent),
        ("cancellable", !b.cancellable || t.cancellable),
        ("changesContext", b.changes_context == t.changes_context),
    ] {
        if !same {
            problems.push(format!("the node's `{field}` differs"));
        }
    }
    if t.timeout_ms < b.timeout_ms {
        problems.push("the node's timeout is shorter".to_owned());
    }
    problems
}

fn is_required(spec: &ParameterSpec) -> bool {
    spec.required && spec.default.is_none()
}

/// `None` (no limit) is the widest.
fn at_least<T: Ord>(builtin: Option<T>, plugin: Option<T>) -> bool {
    match (builtin, plugin) {
        (_, None) => true,
        (None, Some(_)) => false,
        (Some(b), Some(p)) => p >= b,
    }
}

fn at_most(builtin: Option<i64>, plugin: Option<i64>) -> bool {
    match (builtin, plugin) {
        (_, None) => true,
        (None, Some(_)) => false,
        (Some(b), Some(p)) => p <= b,
    }
}

fn kind_compatible(builtin: &ParameterKind, plugin: &ParameterKind) -> Result<(), &'static str> {
    use ParameterKind as K;
    let ok = match (builtin, plugin) {
        (
            K::Text {
                multiline: bm,
                max_length: bl,
            },
            K::Text {
                multiline: pm,
                max_length: pl,
            },
        ) => (!bm || *pm) && at_least(*bl, *pl),
        (K::Integer { min: bn, max: bx }, K::Integer { min: pn, max: px }) => {
            at_most(*bn, *pn) && at_least(*bx, *px)
        }
        (K::Choice { options: b }, K::Choice { options: p }) => {
            b.iter().collect::<BTreeSet<_>>() == p.iter().collect::<BTreeSet<_>>()
        }
        (K::Resource { kind: b }, K::Resource { kind: p }) => b == p,
        (
            K::Lines {
                max_items: bi,
                max_length: bl,
                ..
            },
            K::Lines {
                max_items: pi,
                max_length: pl,
                ..
            },
        ) => at_least(*bi, *pi) && at_least(*bl, *pl),
        (
            K::Pairs {
                separator: bs,
                max_items: bi,
                case_insensitive: bc,
                no_secrets: bn,
                ..
            },
            K::Pairs {
                separator: ps,
                max_items: pi,
                case_insensitive: pc,
                no_secrets: pn,
                ..
            },
        ) => bs == ps && bc == pc && (*bn || !pn) && at_least(*bi, *pi),
        (
            K::Code {
                language: bl,
                max_length: bm,
            },
            K::Code {
                language: pl,
                max_length: pm,
            },
        ) => bl == pl && at_least(*bm, *pm),
        (K::Boolean, K::Boolean) | (K::Project, K::Project) | (K::Credential, K::Credential) => {
            true
        }
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err("its kind or constraints are narrower")
    }
}

/// Routing through [`crate::ModuleRegistry`], with fixture aliases.
#[cfg(test)]
mod routing;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FIXTURE_ALIASES, legacy_fetch, plugin_fetch};

    fn alias(legacy: &'static str, target: &'static str, module: &'static str) -> ActionAlias {
        ActionAlias {
            legacy,
            target,
            module,
            plugin: "deckpilot.github",
            since: "0.9.0",
            params: &[],
        }
    }

    fn check(entries: &[ActionAlias]) -> Result<(), &'static str> {
        validate(entries).map_err(|e| e.reason)
    }

    /// The shipped table: GitHub's block (P9) and the notifications
    /// block (P10), nothing else.
    #[test]
    fn the_shipped_table_is_the_github_and_notifications_blocks() {
        assert_eq!(validate(ACTION_ALIASES), Ok(()));
        let legacy: Vec<&str> = ACTION_ALIASES.iter().map(|a| a.legacy).collect();
        assert_eq!(
            legacy,
            [
                "github.current-user",
                "github.repository",
                "github.pull-requests",
                "github.review-requests",
                "github.checks",
                "github.open",
                "gitlab.current-user",
                "gitlab.project",
                "gitlab.merge-requests",
                "gitlab.review-requests",
                "gitlab.pipeline",
                "gitlab.open",
                "jira.myself",
                "jira.issue",
                "jira.issues",
                "jira.open",
                "notifications.show",
                "notifications.workflow-summary",
            ]
        );
        for alias in ACTION_ALIASES {
            let (plugin, params): (&str, &[ParamRule]) = match alias.module {
                "github" if alias.legacy == "github.open" => ("deckpilot.github", &[]),
                "github" => (
                    "deckpilot.github",
                    &[ParamRule::DropIfDefault("credential")],
                ),
                "gitlab" if alias.legacy == "gitlab.open" => ("deckpilot.gitlab", &[]),
                "gitlab" => (
                    "deckpilot.gitlab",
                    &[ParamRule::DropIfDefault("credential")],
                ),
                "jira" if alias.legacy == "jira.open" => ("deckpilot.jira", &[]),
                "jira" => ("deckpilot.jira", &[ParamRule::DropIfDefault("credential")]),
                "notifications" => ("deckpilot.notifications", &[]),
                other => panic!("an unexpected block: {other}"),
            };
            assert_eq!(alias.plugin, plugin, "{}", alias.legacy);
            assert_eq!(alias.params, params, "{}", alias.legacy);
        }
        let found = alias_of(&ActionTypeId::new("github.pull-requests").unwrap()).unwrap();
        assert_eq!(found.target, "deckpilot.github.pull-requests");
        let found = alias_of(&ActionTypeId::new("notifications.show").unwrap()).unwrap();
        assert_eq!(found.target, "deckpilot.notifications.show");
        let found = alias_of(&ActionTypeId::new("gitlab.merge-requests").unwrap()).unwrap();
        assert_eq!(found.target, "deckpilot.gitlab.merge-requests");
        let found = alias_of(&ActionTypeId::new("jira.issues").unwrap()).unwrap();
        assert_eq!(found.target, "deckpilot.jira.issues");
        assert_eq!(alias_of(&ActionTypeId::new("http.request").unwrap()), None);
    }

    /// Every entry, shipped or fixture, parses as the ids it stands for.
    #[test]
    fn entries_are_valid_ids() {
        for alias in ACTION_ALIASES.iter().chain(FIXTURE_ALIASES) {
            assert!(!alias.module_id().is_plugin(), "{}", alias.legacy);
            assert!(alias.plugin_id().is_plugin(), "{}", alias.legacy);
            assert!(!alias.plugin_id().is_reserved(), "{}", alias.legacy);
            assert_eq!(
                ModuleId::owner_of(&alias.legacy_id()),
                Some(alias.module_id())
            );
            assert_eq!(
                ModuleId::owner_of(&alias.target_id()),
                Some(alias.plugin_id())
            );
        }
    }

    #[test]
    fn a_valid_entry_maps_a_builtin_to_its_first_party_plugin() {
        assert_eq!(
            check(&[alias(
                "github.pull-requests",
                "deckpilot.github.pull-requests",
                "github"
            )]),
            Ok(())
        );
        assert_eq!(validate(FIXTURE_ALIASES), Ok(()));
    }

    #[test]
    fn a_target_is_never_a_builtin_id() {
        for target in ["github.pull-requests", "issues", "gitlab.pull-requests"] {
            assert_eq!(
                check(&[alias("github.pull-requests", target, "github")]),
                Err(
                    "an alias's target is a plugin action type `<publisher>.<plugin>.<name>`, \
                     never a built-in id"
                ),
                "{target}"
            );
        }
    }

    #[test]
    fn a_target_is_always_a_first_party_plugin() {
        let mut entry = alias(
            "github.pull-requests",
            "acme.github.pull-requests",
            "github",
        );
        entry.plugin = "acme.github";
        assert_eq!(
            check(&[entry]),
            Err("an alias's target is a first-party plugin, `deckpilot.<module>`")
        );
        // A publisher that only starts like the first-party one.
        let mut entry = alias(
            "github.pull-requests",
            "deckpilotx.github.pull-requests",
            "github",
        );
        entry.plugin = "deckpilotx.github";
        assert_eq!(
            check(&[entry]),
            Err("an alias's target is a first-party plugin, `deckpilot.<module>`")
        );
        // The plugin is named after the module, and the target is its.
        let mut entry = alias(
            "github.pull-requests",
            "deckpilot.gitlab.pull-requests",
            "github",
        );
        entry.plugin = "deckpilot.gitlab";
        assert_eq!(
            check(&[entry]),
            Err("an alias's plugin is `deckpilot.<module>`, named after the module")
        );
        assert_eq!(
            check(&[alias(
                "github.pull-requests",
                "deckpilot.gitlab.pull-requests",
                "github"
            )]),
            Err("an alias's target is `<plugin>.<name>`, with the legacy action name")
        );
        assert_eq!(
            check(&[alias(
                "github.pull-requests",
                "deckpilot.github.prs",
                "github"
            )]),
            Err("an alias's target is `<plugin>.<name>`, with the legacy action name")
        );
    }

    #[test]
    fn reserved_names_are_never_targets() {
        for module in ["core", "project", "resource"] {
            let legacy: &'static str = Box::leak(format!("{module}.log").into_boxed_str());
            let target: &'static str =
                Box::leak(format!("deckpilot.{module}.log").into_boxed_str());
            let plugin: &'static str = Box::leak(format!("deckpilot.{module}").into_boxed_str());
            let mut entry = alias(legacy, target, module);
            entry.plugin = plugin;
            assert_eq!(
                check(&[entry]),
                Err("an alias's plugin is a reserved id"),
                "{module}"
            );
        }
    }

    #[test]
    fn malformed_entries_are_refused() {
        let ok = alias(
            "github.pull-requests",
            "deckpilot.github.pull-requests",
            "github",
        );
        assert_eq!(
            check(&[alias(
                "deckpilot.github.pull-requests",
                "deckpilot.github.pull-requests",
                "github"
            )]),
            Err("an alias's legacy id is a built-in action type `<module>.<name>`")
        );
        assert_eq!(
            check(&[alias(
                "gitlab.pull-requests",
                "deckpilot.github.pull-requests",
                "github"
            )]),
            Err("an alias's legacy id is a built-in action type `<module>.<name>`")
        );
        assert_eq!(
            check(&[ActionAlias {
                module: "deckpilot.github",
                ..ok
            }]),
            Err("an alias's module is a built-in module id, without `.`")
        );
        assert_eq!(
            check(&[ActionAlias { since: "", ..ok }]),
            Err("an alias names the version that added it")
        );
        assert_eq!(
            check(&[ActionAlias {
                params: &[ParamRule::DropIfDefault("")],
                ..ok
            }]),
            Err("a parameter rule names a parameter")
        );
        assert_eq!(
            check(&[ActionAlias {
                params: &[
                    ParamRule::DropIfDefault("credential"),
                    ParamRule::DropIfDefault("credential")
                ],
                ..ok
            }]),
            Err("a parameter has at most one rule")
        );
        assert_eq!(check(&[ok, ok]), Err("a legacy id has at most one alias"));
    }

    #[test]
    #[should_panic(expected = "an alias's target is a first-party plugin")]
    fn an_invalid_table_does_not_pass_the_assertion() {
        let mut entry = alias(
            "github.pull-requests",
            "acme.github.pull-requests",
            "github",
        );
        entry.plugin = "acme.github";
        assert_valid(&[entry]);
    }

    #[test]
    fn lookup_finds_only_listed_legacy_ids() {
        let fetch = ActionTypeId::new("legacy.fetch").unwrap();
        assert_eq!(
            lookup(FIXTURE_ALIASES, &fetch).map(|a| a.target),
            Some("deckpilot.legacy.fetch")
        );
        // Neither the target nor other types of the module.
        assert!(
            lookup(
                FIXTURE_ALIASES,
                &ActionTypeId::new("deckpilot.legacy.fetch").unwrap()
            )
            .is_none()
        );
        assert!(lookup(FIXTURE_ALIASES, &ActionTypeId::new("legacy.other").unwrap()).is_none());
        assert!(alias_of(&fetch).is_none());
    }

    #[test]
    fn routing_defaults_to_the_builtin() {
        let github = ModuleId::new("github").unwrap();
        assert_eq!(LegacyRouting::new().route(&github), LegacyRoute::Builtin);
        let routing = LegacyRouting::new().with(github.clone(), LegacyRoute::Plugin);
        assert_eq!(routing.route(&github), LegacyRoute::Plugin);
        assert_eq!(
            routing.route(&ModuleId::new("gitlab").unwrap()),
            LegacyRoute::Builtin
        );
    }

    #[test]
    fn compatibility_follows_the_contract() {
        let alias = &FIXTURE_ALIASES[0];
        let builtin = legacy_fetch();
        let plugin = plugin_fetch();
        assert_eq!(
            incompatibilities(alias, &builtin, &plugin),
            Vec::<String>::new()
        );

        // The translated `credential` is not required of the plugin, but
        // only because of the rule.
        let strict = ActionAlias {
            params: &[],
            ..*alias
        };
        assert_eq!(
            incompatibilities(&strict, &builtin, &plugin),
            ["parameter `credential` is missing"]
        );

        let mut narrower = plugin.clone();
        narrower.parameters[0].kind = ParameterKind::Text {
            multiline: false,
            max_length: Some(10),
        };
        narrower.parameters.push(ParameterSpec {
            required: true,
            ..narrower.parameters[0].clone()
        });
        narrower.parameters[1].key = "extra".into();
        narrower.node.outputs[0].sensitive = true;
        narrower.node.timeout_ms = 1;
        narrower.node.effect = crate::graph::ActionEffect::Modifies;
        narrower.node.cancellable = !builtin.node.cancellable;
        let mut expected = vec![
            "parameter `message`: its kind or constraints are narrower",
            "new parameter `extra` is required",
            "output `body` has another `sensitive` flag",
            "the node's `effect` differs",
        ];
        if builtin.node.cancellable {
            expected.push("the node's `cancellable` differs");
        }
        expected.push("the node's timeout is shorter");
        assert_eq!(incompatibilities(alias, &builtin, &narrower), expected);

        // A plugin may be cancellable where the built-in is not (the agent
        // makes every plugin action cancellable), never the other way.
        let mut stubborn = builtin.clone();
        stubborn.node.cancellable = false;
        let mut cancellable = plugin.clone();
        cancellable.node.cancellable = true;
        assert_eq!(
            incompatibilities(alias, &stubborn, &cancellable),
            Vec::<String>::new()
        );
        let mut eager = builtin.clone();
        eager.node.cancellable = true;
        let mut uninterruptible = plugin.clone();
        uninterruptible.node.cancellable = false;
        assert_eq!(
            incompatibilities(alias, &eager, &uninterruptible),
            ["the node's `cancellable` differs"]
        );

        let mut missing = plugin;
        missing.parameters.clear();
        missing.node.outputs.clear();
        assert_eq!(
            incompatibilities(alias, &builtin, &missing),
            ["parameter `message` is missing", "output `body` is missing"]
        );
    }

    #[test]
    fn wider_kinds_are_compatible() {
        use ParameterKind as K;
        let int = |min, max| K::Integer { min, max };
        assert!(kind_compatible(&int(Some(1), Some(10)), &int(Some(0), None)).is_ok());
        assert!(kind_compatible(&int(Some(1), Some(10)), &int(Some(2), Some(10))).is_err());
        assert!(kind_compatible(&int(None, None), &int(None, Some(5))).is_err());
        let choice = |options: &[&str]| K::Choice {
            options: options.iter().map(|o| (*o).to_owned()).collect(),
        };
        assert!(kind_compatible(&choice(&["a", "b"]), &choice(&["b", "a"])).is_ok());
        assert!(kind_compatible(&choice(&["a", "b"]), &choice(&["a"])).is_err());
        assert!(kind_compatible(&choice(&["a"]), &choice(&["a", "b"])).is_err());
        assert!(kind_compatible(&K::Boolean, &K::Boolean).is_ok());
        assert!(kind_compatible(&K::Boolean, &K::Credential).is_err());
    }
}
