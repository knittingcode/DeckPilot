//! Resource handles (plugin API 1.9, X1 and X2 in
//! `docs/marketplace/module-migration.md` §7): `resource` parameters and
//! `resource.resolve`.
//!
//! * The engine resolves an invocation's `resource` parameters, the run's
//!   project roles and the roles of a project the action's own project
//!   parameter names, on the agent's thread, through the core's
//!   `ResourceRef::{Fixed, ProjectRole}` and the machine bindings
//!   ([`InvocationResources::of`]). A `Fixed` reference names a binding,
//!   never a path, so nothing a stored parameter says reaches past one.
//! * The executor of that one invocation keeps the result in its handle
//!   table. A handle is 128 random bits from the operating system's
//!   CSPRNG (32 lowercase hex characters), bound to the plugin and to this
//!   executor, which serves exactly one invocation: a handle of another
//!   invocation, of another plugin, or one that was never issued is
//!   `not-permitted`. Nothing about a handle outlives the invocation.
//! * A `resource` parameter that resolved to a binding of one of its
//!   declared kinds reaches the guest as such a handle
//!   ([`Executor::with_resources`]); one that did not is absent, and
//!   `resource.resolve {param}` says why.
//! * `resource.resolve` answers `{handle, kind, label}` (the label is the
//!   binding's key, which the user chose), plus `displayPath` only with
//!   the `resources:paths` grant. It resolves only the named parameter or
//!   role, only to kinds the request (and, for a parameter, the manifest)
//!   accepts; there is no list. A role resolves in the run's project, or
//!   in another project of the profile only when the action's own project
//!   parameter names it. Without an active project, or with the Projects
//!   feature off, a role is `no-active-project`. 20 resolves per
//!   invocation, every call audited ([`super::PrivilegedRecord`]): the
//!   parameter or role, the kind, the label and the outcome, never a path.
//! * Paths inside a handle ([`Executor::bound_inside`], for `tool.run` and
//!   `resource.open`) are checked by the core's
//!   [`existing_inside`](deckpilot_core::paths::existing_inside): no `..`,
//!   no absolute path, no drive prefix, no link that leads outside.
//!
//! Refusals carry their reason code at the start of the message
//! (`no-active-project: …`), since the ABI's error codes are a closed set.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use deckpilot_core::{
    ActionParameters, ExecutionContext, MachineResourceBinding, ParameterValue, ResolveError,
};
use deckpilot_plugin_abi::privileged::{ResourceHandle, ResourceKind, ResourceResolve};
use deckpilot_plugin_abi::{CallErrorCode, CallResult};
use deckpilot_plugin_broker::PluginId;

use super::{Executor, PrivilegedRecord};

/// Most `resource.resolve` calls of one invocation (X2).
pub(crate) const MAX_RESOLVES: u32 = 20;

/// What the engine resolved for one invocation before the plugin runs:
/// bindings and why not, never anything the guest said.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct InvocationResources {
    /// The action's `resource` parameters that have a value.
    pub params: BTreeMap<String, ParamResource>,
    /// The run's project and its roles; `None`: no project is active.
    pub project: Option<ProjectResources>,
    /// Other projects of the profile that the action's own project
    /// parameters name.
    pub named: Vec<ProjectResources>,
    /// The Projects feature is off: every role is `no-active-project`.
    pub projects_off: bool,
}

/// One `resource` parameter's binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParamResource {
    /// Taken from a role of the run's project (not a fixed binding).
    pub via_role: bool,
    pub binding: Result<MachineResourceBinding, ResolveError>,
}

/// A project and its roles' bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectResources {
    pub id: String,
    pub roles: BTreeMap<String, Result<MachineResourceBinding, ResolveError>>,
}

impl InvocationResources {
    /// The resources of an invocation with `parameters` in `context`,
    /// within the plugin module's capabilities (machine resources and the
    /// profile; without them everything is `not-permitted`).
    pub fn of(parameters: &ActionParameters, context: &ExecutionContext<'_>) -> Self {
        let project = context.project().ok().flatten();
        let roles = |project: &deckpilot_core::Project| ProjectResources {
            id: project.id.to_string(),
            roles: project
                .resources
                .keys()
                .map(|role| {
                    let binding = context.resolve_project_resource(project, role).cloned();
                    (role.clone(), binding)
                })
                .collect(),
        };
        let mut out = Self {
            project: project.map(roles),
            ..Self::default()
        };
        for (key, value) in parameters.iter() {
            match value {
                ParameterValue::Resource(reference) => {
                    out.params.insert(
                        key.clone(),
                        ParamResource {
                            via_role: matches!(
                                reference,
                                deckpilot_core::ResourceRef::ProjectRole { .. }
                            ),
                            binding: context.resolve(reference, None).cloned(),
                        },
                    );
                }
                ParameterValue::Project(id) if project.is_none_or(|p| p.id != *id) => {
                    let named = context
                        .profile()
                        .ok()
                        .and_then(|profile| profile.projects.iter().find(|p| p.id == *id));
                    if let Some(named) = named
                        && out.named.iter().all(|p| p.id != named.id.to_string())
                    {
                        out.named.push(roles(named));
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// The same with the Projects feature off: no project, and the
    /// parameters taken from a project role are `no-active-project`.
    #[must_use]
    pub fn without_projects(mut self) -> Self {
        self.project = None;
        self.named.clear();
        self.projects_off = true;
        for param in self.params.values_mut() {
            if param.via_role {
                param.binding = Err(ResolveError::NoActiveProject);
            }
        }
        self
    }
}

/// A binding as a handle may carry it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Issued {
    pub kind: ResourceKind,
    /// The binding's key.
    pub label: String,
    /// The bound path (directory, file), application or Docker context.
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Source {
    Param(String),
    Role { project: String, role: String },
}

/// The handles of one invocation.
#[derive(Debug, Default)]
pub(super) struct HandleTable {
    /// The plugin the handles belong to; `None`: this invocation has none.
    plugin: Option<PluginId>,
    /// Its version, for the audit.
    version: String,
    /// The manifest's `kinds` of each `resource` parameter of the action.
    declared: BTreeMap<String, Vec<ResourceKind>>,
    resolved: InvocationResources,
    issued: BTreeMap<String, Issued>,
    sources: BTreeMap<Source, String>,
    resolves: u32,
}

impl HandleTable {
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The handle of `source`, issued once per invocation.
    fn issue(&mut self, source: Source, issued: Issued) -> Result<String, Refusal> {
        if let Some(handle) = self.sources.get(&source) {
            return Ok(handle.clone());
        }
        let handle = super::dialog_id().ok_or_else(|| Refusal {
            code: "unavailable",
            call: CallErrorCode::Unavailable,
            message: "the system's random source is not available".into(),
        })?;
        self.issued.insert(handle.clone(), issued);
        self.sources.insert(source, handle.clone());
        Ok(handle)
    }
}

/// A refusal: its reason code and its ABI code.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Refusal {
    code: &'static str,
    call: CallErrorCode,
    message: String,
}

impl Refusal {
    fn new(code: &'static str, call: CallErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            call,
            message: message.into(),
        }
    }

    fn not_permitted(message: impl Into<String>) -> Self {
        Self::new("not-permitted", CallErrorCode::NotPermitted, message)
    }

    fn result(&self) -> CallResult {
        CallResult::error(self.call, format!("{}: {}", self.code, self.message))
    }
}

fn resolve_refusal(error: &ResolveError) -> Refusal {
    let call = match error {
        ResolveError::NotPermitted | ResolveError::KindMismatch { .. } => {
            CallErrorCode::NotPermitted
        }
        ResolveError::NoActiveProject => CallErrorCode::Unavailable,
        ResolveError::RoleNotDefined { .. } | ResolveError::Unbound(_) => CallErrorCode::Failed,
    };
    Refusal::new(error.code(), call, error.to_string())
}

/// The plugin kind of a binding: folders, applications and Docker
/// environments as such, a binding of kind `other` whose value is an
/// absolute path as a file. Terminals, services and URLs are none.
fn kind_of(binding: &MachineResourceBinding) -> Option<ResourceKind> {
    use deckpilot_core::ResourceKind as Core;
    match binding.kind {
        Core::Directory => Some(ResourceKind::Directory),
        Core::Application => Some(ResourceKind::Application),
        Core::DockerEnvironment => Some(ResourceKind::DockerEnvironment),
        Core::Other if Path::new(binding.value.trim()).is_absolute() => Some(ResourceKind::File),
        Core::Terminal | Core::Service | Core::Url | Core::Other => None,
    }
}

fn kind_name(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Directory => "directory",
        ResourceKind::File => "file",
        ResourceKind::Application => "application",
        ResourceKind::DockerEnvironment => "dockerEnvironment",
    }
}

/// The declared kinds of each `resource` parameter of `action`, as the
/// ABI's.
pub(crate) fn declared(
    action: &deckpilot_plugin_manifest::Action,
) -> BTreeMap<String, Vec<ResourceKind>> {
    use deckpilot_plugin_manifest::{ParameterKind, ResourceKind as Declared};
    action
        .parameters
        .iter()
        .filter_map(|p| match &p.kind {
            ParameterKind::Resource { kinds } => Some((
                p.key.clone(),
                kinds
                    .iter()
                    .map(|kind| match kind {
                        Declared::Directory => ResourceKind::Directory,
                        Declared::File => ResourceKind::File,
                        Declared::Application => ResourceKind::Application,
                        Declared::DockerEnvironment => ResourceKind::DockerEnvironment,
                    })
                    .collect(),
            )),
            _ => None,
        })
        .collect()
}

/// `binding` as a handle of one of `kinds`: the kind must be accepted and
/// a folder or file must exist at an absolute path. The messages name the
/// binding's key, never its path.
fn checked(binding: &MachineResourceBinding, kinds: &[ResourceKind]) -> Result<Issued, Refusal> {
    let label = binding.key.to_string();
    let Some(kind) = kind_of(binding).filter(|kind| kinds.contains(kind)) else {
        return Err(Refusal::new(
            "resource-kind-mismatch",
            CallErrorCode::NotPermitted,
            format!(
                "`{label}` is not a {}",
                kinds
                    .iter()
                    .map(|k| kind_name(*k))
                    .collect::<Vec<_>>()
                    .join(" or ")
            ),
        ));
    };
    let value = binding.value.trim().to_owned();
    let path = Path::new(&value);
    let exists = match kind {
        ResourceKind::Directory => path.is_absolute() && path.is_dir(),
        ResourceKind::File => path.is_absolute() && path.is_file(),
        ResourceKind::Application | ResourceKind::DockerEnvironment => true,
    };
    if !exists {
        return Err(Refusal::new(
            deckpilot_core::paths::codes::NOT_FOUND,
            CallErrorCode::Failed,
            format!("the {} bound as `{label}` does not exist", kind_name(kind)),
        ));
    }
    Ok(Issued { kind, label, value })
}

impl Executor {
    /// The executor of an invocation whose engine resolved `resolved`, for
    /// `plugin` at `version` whose action declares the `resource`
    /// parameters `declared`, and the handles of the parameters that
    /// resolved to a binding of a declared kind, by parameter key: the
    /// values the guest receives for them.
    pub fn with_resources(
        self,
        plugin: &str,
        version: &str,
        declared: BTreeMap<String, Vec<ResourceKind>>,
        resolved: InvocationResources,
    ) -> (Self, BTreeMap<String, String>) {
        let mut handles = BTreeMap::new();
        {
            let mut table = self.handles();
            *table = HandleTable {
                plugin: PluginId::new(plugin).ok(),
                version: version.to_owned(),
                declared,
                resolved,
                ..HandleTable::default()
            };
            let bound: Vec<(String, Issued)> = table
                .resolved
                .params
                .iter()
                .filter_map(|(key, param)| {
                    let kinds = table.declared.get(key)?;
                    let binding = param.binding.as_ref().ok()?;
                    Some((key.clone(), checked(binding, kinds).ok()?))
                })
                .collect();
            for (key, issued) in bound {
                if let Ok(handle) = table.issue(Source::Param(key.clone()), issued) {
                    handles.insert(key, handle);
                }
            }
        }
        (self, handles)
    }

    pub(super) fn handles(&self) -> std::sync::MutexGuard<'_, HandleTable> {
        self.handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `resource.resolve`, which the broker allowed (granted `resources`;
    /// `paths` with `resources:paths`).
    pub(super) fn resolve_resource(
        &self,
        plugin: &PluginId,
        request: &ResourceResolve,
        paths: bool,
    ) -> CallResult {
        let started = std::time::Instant::now();
        let (subject, outcome) = {
            let mut table = self.handles();
            let subject = match (&request.param, &request.role) {
                (Some(param), _) => ("param", param.clone()),
                (None, role) => ("role", role.clone().unwrap_or_default()),
            };
            (subject, resolve_in(&mut table, plugin, request))
        };
        let mut details = vec![subject];
        let result = match &outcome {
            Ok((handle, issued)) => {
                details.push(("kind", kind_name(issued.kind).to_owned()));
                details.push(("label", issued.label.clone()));
                serde_json::to_value(ResourceHandle {
                    handle: handle.clone(),
                    kind: issued.kind,
                    label: issued.label.clone(),
                    display_path: (paths
                        && matches!(issued.kind, ResourceKind::Directory | ResourceKind::File))
                    .then(|| issued.value.clone()),
                })
                .map_or_else(
                    |e| CallResult::error(CallErrorCode::Failed, e.to_string()),
                    CallResult::Ok,
                )
            }
            Err(refusal) => refusal.result(),
        };
        self.audit_privileged(&PrivilegedRecord {
            plugin: plugin.to_string(),
            op: "resource.resolve",
            grant: if paths {
                "resources:paths"
            } else {
                "resources"
            },
            outcome: outcome.as_ref().err().map(|r| r.code),
            elapsed: started.elapsed(),
            details,
        });
        result
    }

    /// The binding of `handle`, which must be one this invocation issued
    /// to `plugin`.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "`tool.run` and `resource.open` (WP-A4, WP-A5)")
    )]
    pub(super) fn bound(&self, plugin: &PluginId, handle: &str) -> Result<Issued, CallResult> {
        let table = self.handles();
        if table.plugin.as_ref() != Some(plugin) {
            return Err(Refusal::not_permitted("the handle is not one of this plugin's").result());
        }
        table.issued.get(handle).cloned().ok_or_else(|| {
            Refusal::not_permitted("the handle was not issued to this invocation").result()
        })
    }

    /// The existing file or folder at `relative` inside the folder of
    /// `handle` (the folder itself without `relative`): by name and by
    /// where links lead, it stays inside the binding.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "`tool.run` and `resource.open` (WP-A4, WP-A5)")
    )]
    pub(super) fn bound_inside(
        &self,
        plugin: &PluginId,
        handle: &str,
        relative: Option<&str>,
        directory: bool,
    ) -> Result<PathBuf, CallResult> {
        let issued = self.bound(plugin, handle)?;
        if issued.kind != ResourceKind::Directory {
            return Err(Refusal::new(
                "resource-kind-mismatch",
                CallErrorCode::NotPermitted,
                format!("`{}` is not a directory", issued.label),
            )
            .result());
        }
        deckpilot_core::paths::existing_inside(&issued.value, relative, directory).map_err(
            |failure| {
                // The core's messages may name the path; these name the
                // binding only.
                let refusal = if failure.code.as_deref()
                    == Some(deckpilot_core::paths::codes::INVALID_PATH)
                {
                    Refusal::new(
                        deckpilot_core::paths::codes::INVALID_PATH,
                        CallErrorCode::NotPermitted,
                        format!("the path leads outside `{}`", issued.label),
                    )
                } else {
                    Refusal::new(
                        deckpilot_core::paths::codes::NOT_FOUND,
                        CallErrorCode::Failed,
                        format!("the path does not exist in `{}`", issued.label),
                    )
                };
                refusal.result()
            },
        )
    }
}

/// One resolve against the invocation's table.
fn resolve_in(
    table: &mut HandleTable,
    plugin: &PluginId,
    request: &ResourceResolve,
) -> Result<(String, Issued), Refusal> {
    if table.plugin.as_ref() != Some(plugin) {
        return Err(Refusal::not_permitted(
            "this invocation has no resources of this plugin",
        ));
    }
    table.resolves += 1;
    if table.resolves > MAX_RESOLVES {
        return Err(Refusal::new(
            "throttled",
            CallErrorCode::Throttled,
            format!("at most {MAX_RESOLVES} resources may be resolved per invocation"),
        ));
    }
    let (source, issued) = match (&request.param, &request.role) {
        (Some(param), None) => {
            if request.project_id.is_some() {
                return Err(Refusal::new(
                    "bad-request",
                    CallErrorCode::BadRequest,
                    "a parameter is resolved without a project",
                ));
            }
            let declared = table.declared.get(param).ok_or_else(|| {
                Refusal::not_permitted(format!("the action has no resource parameter `{param}`"))
            })?;
            let kinds: Vec<ResourceKind> = declared
                .iter()
                .copied()
                .filter(|kind| request.kinds.contains(kind))
                .collect();
            let binding = match table.resolved.params.get(param) {
                None => {
                    return Err(Refusal::new(
                        "resource-not-set",
                        CallErrorCode::Unavailable,
                        format!("the parameter `{param}` is not set"),
                    ));
                }
                Some(ParamResource { binding, .. }) => {
                    binding.as_ref().map_err(resolve_refusal)?.clone()
                }
            };
            (Source::Param(param.clone()), checked(&binding, &kinds)?)
        }
        (None, Some(role)) => {
            let resolved = &table.resolved;
            let no_project = || resolve_refusal(&ResolveError::NoActiveProject);
            if resolved.projects_off {
                return Err(no_project());
            }
            let project = match &request.project_id {
                None => resolved.project.as_ref().ok_or_else(no_project)?,
                Some(id) => resolved
                    .project
                    .iter()
                    .chain(&resolved.named)
                    .find(|p| p.id == *id)
                    .ok_or_else(|| {
                        Refusal::not_permitted(
                            "the project is neither the run's nor one the action's parameters name",
                        )
                    })?,
            };
            let binding = project
                .roles
                .get(role)
                .ok_or_else(|| {
                    Refusal::new(
                        "resource-role-missing",
                        CallErrorCode::Failed,
                        format!("the project does not define the resource role `{role}`"),
                    )
                })?
                .as_ref()
                .map_err(resolve_refusal)?
                .clone();
            let source = Source::Role {
                project: project.id.clone(),
                role: role.clone(),
            };
            (source, checked(&binding, &request.kinds)?)
        }
        _ => {
            return Err(Refusal::new(
                "bad-request",
                CallErrorCode::BadRequest,
                "exactly one of `param` and `role`",
            ));
        }
    };
    let handle = table.issue(source, issued.clone())?;
    Ok((handle, issued))
}

/// The handle table of a fresh executor.
pub(super) fn table() -> Mutex<HandleTable> {
    Mutex::new(HandleTable::default())
}

#[cfg(test)]
mod tests;
