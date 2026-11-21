//! The privileged kinds of plugin API 1.9 (`docs/marketplace/module-migration.md`
//! §7, X1 to X13): `tools`, `resources`, `projectContext` and `hostViews`,
//! `openUrl.fromParams`, `resource` parameters, the view kinds `workflows`
//! and `projects`, `contributes.statusItems` and `contributes.dashboardCards`.
//!
//! [`check`] applies to every manifest: the shapes, the tool policy, the
//! `^1.9` requirement and the cross-field rules. [`first_party_only`] is the
//! publisher gate (X12): [`Manifest::validate`](crate::Manifest::validate)
//! and [`Manifest::validate_first_party`](crate::Manifest::validate_first_party)
//! add its issues, only
//! [`Manifest::validate_bundled`](crate::Manifest::validate_bundled) does
//! not.

use super::{Checker, Issue, IssueCode, Manifest, ParameterKind, Permission, PermissionKind};
use crate::model::{
    PRIVILEGED_API, ToolDeclaration, ToolExecutable, VariableScope, VariableValueKind,
};
use crate::tools::{self, ToolGrant};

/// The sub-grants an action may name in `requiresPermissions` (plugin API
/// 1.9): each is consented to on its own, so a plugin can do without it.
/// Naming one makes it optional: the plugin is enabled with its other
/// grants, and only the actions that name it wait for it (the docker
/// plugin's Compose changes and `tools:docker:modify`, X3c, off by
/// default).
pub const OPTIONAL_PERMISSIONS: &[&str] = &[
    "tools:git:network",
    "tools:docker:modify",
    "resources:open",
    "resources:paths",
    "projectContext:select",
    "openUrl:from-params",
];

impl crate::Action {
    /// `requiresPermissions`: the optional sub-grants this action needs.
    pub fn requires_permissions(&self) -> &[String] {
        self.requires_permissions.as_deref().unwrap_or_default()
    }

    /// `node.changesContext`: the action selects the run's project.
    pub fn changes_context(&self) -> bool {
        self.node
            .as_ref()
            .and_then(|node| node.changes_context)
            .unwrap_or(false)
    }
}

impl Manifest {
    /// The grant keys that are optional for enabling the plugin: every
    /// sub-grant an action names in `requiresPermissions` (only those the
    /// manifest declares; the validation refuses the others).
    pub fn optional_permissions(&self) -> std::collections::BTreeSet<&str> {
        self.contributes
            .actions()
            .iter()
            .flat_map(crate::Action::requires_permissions)
            .map(String::as_str)
            .filter(|key| declares_sub_grant(self, key))
            .collect()
    }
}

/// Whether `m` declares the sub-grant `key` of [`OPTIONAL_PERMISSIONS`].
fn declares_sub_grant(m: &Manifest, key: &str) -> bool {
    m.permissions
        .iter()
        .any(|permission| match (key, permission) {
            ("tools:git:network", Permission::Tools { tools, .. }) => tools
                .iter()
                .any(|t| t.executable == ToolExecutable::Git && t.network()),
            ("tools:docker:modify", Permission::Tools { tools, .. }) => tools
                .iter()
                .any(|t| t.executable == ToolExecutable::Docker && t.modify()),
            ("resources:open", Permission::Resources { open, .. }) => *open == Some(true),
            ("resources:paths", Permission::Resources { paths, .. }) => *paths == Some(true),
            ("projectContext:select", Permission::ProjectContext { select, .. }) => {
                *select == Some(true)
            }
            ("openUrl:from-params", _) => permission.opens_from_params(),
            _ => false,
        })
}

/// `contributes.actions[].requiresPermissions` (plugin API 1.9): 1 to
/// [`OPTIONAL_PERMISSIONS`]`.len()` unique sub-grants of that list, each
/// declared by the manifest.
fn requires_permissions(c: &mut Checker, m: &Manifest) {
    let required = m.engines.plugin_api;
    for (index, action) in m.contributes.actions().iter().enumerate() {
        let Some(keys) = &action.requires_permissions else {
            continue;
        };
        let path = format!("contributes.actions[{index}].requiresPermissions");
        if required.minimum < PRIVILEGED_API {
            c.error(
                &path,
                IssueCode::Conflict,
                format!(
                    "needs plugin API {PRIVILEGED_API}: set engines.pluginApi to \
                     `^{PRIVILEGED_API}` (it is `{required}`)"
                ),
            );
        }
        c.count(&path, keys.len(), 1, OPTIONAL_PERMISSIONS.len());
        c.unique(&path, keys.iter());
        for (i, key) in keys.iter().enumerate() {
            if !OPTIONAL_PERMISSIONS.contains(&key.as_str()) {
                c.error(
                    format!("{path}[{i}]"),
                    IssueCode::InvalidValue,
                    format!(
                        "`{key}` cannot be optional; an action may name {}",
                        OPTIONAL_PERMISSIONS.join(", ")
                    ),
                );
            } else if !declares_sub_grant(m, key) {
                c.error(
                    format!("{path}[{i}]"),
                    IssueCode::MissingPermission,
                    format!("`{key}` is not a permission the manifest declares"),
                );
            }
        }
    }
}

/// Most executables of one `tools` permission (`git` and `docker`).
const MAX_TOOLS: usize = 2;
/// Most subcommands one tool may declare.
const MAX_SUBCOMMANDS: usize = 16;

/// The shape of one privileged permission.
pub(super) fn permission(c: &mut Checker, path: &str, permission: &Permission) {
    if let Permission::Tools { tools, .. } = permission {
        let field = format!("{path}.tools");
        c.count(&field, tools.len(), 1, MAX_TOOLS);
        c.unique(&field, tools.iter().map(|t| t.executable.as_str()));
        for (index, tool) in tools.iter().enumerate() {
            tool_declaration(c, &format!("{field}[{index}]"), tool);
        }
    }
}

fn tool_declaration(c: &mut Checker, path: &str, tool: &ToolDeclaration) {
    let name = tool.executable.as_str();
    let field = format!("{path}.subcommands");
    c.count(&field, tool.subcommands.len(), 1, MAX_SUBCOMMANDS);
    c.unique(&field, tool.subcommands.iter().map(|s| &s.name));
    let mut grants = Vec::new();
    for (index, declared) in tool.subcommands.iter().enumerate() {
        let at = format!("{field}[{index}]");
        if let Some(reason) = tools::denied_subcommand(tool.executable, &declared.name) {
            c.error(
                format!("{at}.name"),
                IssueCode::InvalidValue,
                format!("`{name} {}` is never allowed: it {reason}", declared.name),
            );
            continue;
        }
        let Some(policy) = tools::subcommand(tool.executable, &declared.name) else {
            let allowed: Vec<&str> = tools::policy_of(tool.executable)
                .iter()
                .map(|p| p.subcommand)
                .collect();
            c.error(
                format!("{at}.name"),
                IssueCode::InvalidValue,
                format!(
                    "`{}` is not a {name} subcommand DeckPilot runs for plugins; the allowed \
                     ones are {}",
                    declared.name,
                    allowed.join(", ")
                ),
            );
            continue;
        };
        grants.push(policy.grant);
        let options = format!("{at}.options");
        c.unique(&options, declared.options());
        for (i, option) in declared.options().iter().enumerate() {
            if let Some(reason) = tools::denied_option(tool.executable, option) {
                c.error(
                    format!("{options}[{i}]"),
                    IssueCode::InvalidValue,
                    format!("`{option}` is never allowed for {name}: it {reason}"),
                );
            } else if !policy.options.contains(&option.as_str()) {
                c.error(
                    format!("{options}[{i}]"),
                    IssueCode::InvalidValue,
                    format!(
                        "`{option}` is not an option DeckPilot allows for `{name} {}`{}",
                        declared.name,
                        if policy.options.is_empty() {
                            " (it takes none)".to_owned()
                        } else {
                            format!(" (allowed: {})", policy.options.join(", "))
                        }
                    ),
                );
            }
        }
    }
    sub_grant(
        c,
        path,
        tool,
        "network",
        ToolExecutable::Git,
        ToolGrant::Network,
        &grants,
    );
    sub_grant(
        c,
        path,
        tool,
        "modify",
        ToolExecutable::Docker,
        ToolGrant::Modify,
        &grants,
    );
}

/// `network` (git) and `modify` (docker): declared exactly when a
/// subcommand needs it, and only on their own tool.
fn sub_grant(
    c: &mut Checker,
    path: &str,
    tool: &ToolDeclaration,
    member: &str,
    owner: ToolExecutable,
    grant: ToolGrant,
    used: &[ToolGrant],
) {
    let declared = match grant {
        ToolGrant::Network => tool.network(),
        ToolGrant::Modify => tool.modify(),
        ToolGrant::Read => return,
    };
    let needed = used.contains(&grant);
    let name = tool.executable.as_str();
    if tool.executable != owner {
        if declared {
            c.error(
                format!("{path}.{member}"),
                IssueCode::Conflict,
                format!(
                    "`{member}` exists only for {}, not for {name}",
                    owner.as_str()
                ),
            );
        }
        return;
    }
    if needed && !declared {
        c.error(
            format!("{path}.{member}"),
            IssueCode::MissingPermission,
            format!(
                "a declared {name} subcommand needs `{member}: true` (`tools:{name}:{member}`, \
                 asked for on its own)"
            ),
        );
    } else if declared && !needed {
        c.error(
            format!("{path}.{member}"),
            IssueCode::Conflict,
            format!("no declared {name} subcommand needs `{member}`: leave it out"),
        );
    }
}

/// Every place a manifest uses a privileged kind, with its JSON path and
/// what it is.
fn usages(m: &Manifest) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for (index, permission) in m.permissions.iter().enumerate() {
        if permission.kind().is_first_party_only() {
            found.push((
                format!("permissions[{index}]"),
                format!("the `{}` permission", permission.kind().as_str()),
            ));
        } else if permission.opens_from_params() {
            found.push((
                format!("permissions[{index}].fromParams"),
                "`openUrl.fromParams`".to_owned(),
            ));
        }
    }
    for (a, action) in m.contributes.actions().iter().enumerate() {
        if action.changes_context() {
            found.push((
                format!("contributes.actions[{a}].node.changesContext"),
                "`node.changesContext`".to_owned(),
            ));
        }
        for (p, parameter) in action.parameters.iter().enumerate() {
            if matches!(parameter.kind, ParameterKind::Resource { .. }) {
                found.push((
                    format!("contributes.actions[{a}].parameters[{p}].kind"),
                    "a `resource` parameter".to_owned(),
                ));
            }
        }
    }
    for (index, variable) in m.contributes.variables().iter().enumerate() {
        if let Some(source) = variable.source {
            found.push((
                format!("contributes.variables[{index}].source"),
                format!("the host source `{}`", source.as_str()),
            ));
        }
    }
    for (index, view) in m.contributes.views().iter().enumerate() {
        if view.kind.is_host_view() {
            found.push((
                format!("contributes.views[{index}].kind"),
                format!("the `{}` view", view.kind.as_str()),
            ));
        }
    }
    if m.contributes.status_items.is_some() {
        found.push(("contributes.statusItems".into(), "`statusItems`".into()));
    }
    if m.contributes.dashboard_cards.is_some() {
        found.push((
            "contributes.dashboardCards".into(),
            "`dashboardCards`".into(),
        ));
    }
    found
}

/// The rules every manifest is checked against.
pub(super) fn check(c: &mut Checker, m: &Manifest) {
    let required = m.engines.plugin_api;
    if required.minimum < PRIVILEGED_API {
        for (path, what) in usages(m) {
            c.error(
                path,
                IssueCode::Conflict,
                format!(
                    "{what} needs plugin API {PRIVILEGED_API}: set engines.pluginApi to \
                     `^{PRIVILEGED_API}` (it is `{required}`)"
                ),
            );
        }
    }
    for (index, setting) in m.contributes.settings().iter().enumerate() {
        if matches!(setting.kind, ParameterKind::Resource { .. }) {
            c.error(
                format!("contributes.settings.fields[{index}].kind"),
                IssueCode::InvalidValue,
                "a setting cannot be a resource: resources are bound per action or project",
            );
        }
    }
    host_views(c, m);
    host_variables(c, m);
    changes_context(c, m);
    requires_permissions(c, m);
    // Common rule 6: no exfiltration channel next to process authority.
    if m.permission(PermissionKind::Tools).is_some() {
        for (index, permission) in m.permissions.iter().enumerate() {
            let kind = permission.kind();
            if matches!(
                kind,
                PermissionKind::Network | PermissionKind::Credentials | PermissionKind::OpenUrl
            ) {
                c.error(
                    format!("permissions[{index}]"),
                    IssueCode::Conflict,
                    format!(
                        "a plugin with `tools` cannot also declare `{}`: its results leave it \
                         only through the user's own configuration",
                        kind.as_str()
                    ),
                );
            }
        }
    }
}

/// `node.changesContext` (X6): only with a `projectContext` permission
/// that may `select`, whose switch the action's run then applies to the
/// run's project instead of the active one.
fn changes_context(c: &mut Checker, m: &Manifest) {
    let select = m
        .permissions
        .iter()
        .any(|p| matches!(p, Permission::ProjectContext { select, .. } if *select == Some(true)));
    if select {
        return;
    }
    for (index, action) in m.contributes.actions().iter().enumerate() {
        if action.changes_context() {
            c.error(
                format!("contributes.actions[{index}].node.changesContext"),
                IssueCode::MissingPermission,
                "an action that changes the run's project needs the `projectContext` \
                 permission with `select`",
            );
        }
    }
}

/// Host-published variables (X7): project scope, a string, and only with
/// the `projectContext` permission, whose values they are.
fn host_variables(c: &mut Checker, m: &Manifest) {
    let granted = m.permission(PermissionKind::ProjectContext).is_some();
    for (index, variable) in m.contributes.variables().iter().enumerate() {
        let Some(source) = variable.source else {
            continue;
        };
        let path = format!("contributes.variables[{index}]");
        if variable.scope != VariableScope::Project {
            c.error(
                format!("{path}.scope"),
                IssueCode::InvalidValue,
                format!(
                    "`{}` is a project value: its scope is `project`",
                    source.as_str()
                ),
            );
        }
        if variable.value_type.kind != VariableValueKind::String {
            c.error(
                format!("{path}.valueType"),
                IssueCode::InvalidValue,
                format!("`{}` is a string", source.as_str()),
            );
        }
        if !granted {
            c.error(
                format!("{path}.source"),
                IssueCode::MissingPermission,
                format!(
                    "`{}` needs the `{}` permission",
                    source.as_str(),
                    PermissionKind::ProjectContext.as_str()
                ),
            );
        }
    }
}

/// `hostViews` is needed for, and only meaningful with, the host views,
/// status items and dashboard cards.
fn host_views(c: &mut Checker, m: &Manifest) {
    let declared = m.permission(PermissionKind::HostViews).is_some();
    let mut used = false;
    let mut kinds = Vec::new();
    for (index, view) in m.contributes.views().iter().enumerate() {
        if view.kind.is_host_view() {
            used = true;
            if kinds.contains(&view.kind) {
                c.error(
                    format!("contributes.views[{index}].kind"),
                    IssueCode::Duplicate,
                    format!("the `{}` view is declared twice", view.kind.as_str()),
                );
            }
            kinds.push(view.kind);
        }
    }
    if let Some(items) = &m.contributes.status_items {
        used = true;
        c.count("contributes.statusItems", items.len(), 1, 4);
        c.unique(
            "contributes.statusItems",
            items.iter().map(|i| format!("{i:?}")),
        );
    }
    if let Some(cards) = &m.contributes.dashboard_cards {
        used = true;
        c.count("contributes.dashboardCards", cards.len(), 1, 4);
        c.unique(
            "contributes.dashboardCards",
            cards.iter().map(|i| format!("{i:?}")),
        );
    }
    if used && !declared {
        c.error(
            "permissions",
            IssueCode::MissingPermission,
            "host views, status items and dashboard cards need the `hostViews` permission",
        );
    } else if declared && !used {
        c.warn(
            "permissions",
            "`hostViews` grants nothing without a `workflows` or `projects` view, \
             `statusItems` or `dashboardCards`",
        );
    }
}

/// The publisher gate (X12): an issue for every privileged kind, for a
/// manifest that is not a first-party plugin shipped inside the
/// application bundle.
pub(crate) fn first_party_only(m: &Manifest) -> Vec<Issue> {
    usages(m)
        .into_iter()
        .map(|(path, what)| Issue {
            path,
            code: IssueCode::FirstPartyOnly,
            message: format!(
                "{what} is only for DeckPilot's own plugins shipped inside the app: a \
                 registry, sideloaded or developer package cannot declare it"
            ),
        })
        .collect()
}
