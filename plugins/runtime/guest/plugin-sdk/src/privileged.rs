//! Plugin API 1.9's operations (`docs/marketplace/module-migration.md` §7):
//! resource handles, brokered `git` and `docker`, the URL of the
//! invocation's own parameter, the project context and the profile read.
//!
//! All but [`Context::profile`] need permissions only DeckPilot's own
//! plugins shipped inside the app can declare (`tools`, `resources`,
//! `openUrl.fromParams`, `projectContext`); for every other plugin the
//! calls are denied. Each request is checked against the ABI's bounds here
//! first ([`CapabilityErrorKind::BadRequest`] without a host call). An
//! agent that does not carry an operation out yet answers
//! [`CapabilityErrorKind::Unsupported`].

use deckpilot_plugin_abi::CapabilityRequest;
use deckpilot_plugin_abi::capability::NoArgs;
use deckpilot_plugin_abi::privileged::{
    CycleDirection, OpenUrlParam, ProfileInfo, ProjectCycle, ProjectInfo, ProjectList,
    ProjectSelect, ResourceHandle, ResourceKind, ResourceOpen, ResourceResolve, Tool, ToolOutput,
    ToolRun,
};
use serde::de::DeserializeOwned;

use super::{CapabilityError, CapabilityErrorKind, Context, malformed};

fn bad_request(op: &'static str, problem: &impl std::fmt::Display) -> CapabilityError {
    CapabilityError {
        op,
        kind: CapabilityErrorKind::BadRequest,
        detail: format!("the request's {problem}"),
        target: None,
        network: None,
    }
}

impl Context<'_> {
    fn checked<T: DeserializeOwned>(
        &mut self,
        op: &'static str,
        request: &CapabilityRequest,
        expected: &str,
    ) -> Result<T, CapabilityError> {
        if let Err(problem) = request.validate() {
            return Err(bad_request(op, &problem));
        }
        let value = self.call(op, request)?;
        serde_json::from_value(value).map_err(|_| malformed(op, expected))
    }

    /// A handle for the `resource` parameter `param` of the invoked action
    /// (op `resource.resolve`, plugin API 1.9): one of `kinds`, labelled
    /// with the binding's name. Needs `resources`; the bound path only
    /// with `resources.paths`. The handle is valid in this invocation only.
    ///
    /// ```
    /// use deckpilot_plugin_sdk::testing::MockHost;
    /// use deckpilot_plugin_sdk::ResourceKind;
    ///
    /// let mut host = MockHost::allow_all();
    /// let mut ctx = host.context();
    /// let repo = ctx.resource_param("repository", &[ResourceKind::Directory]).unwrap();
    /// assert_eq!(repo.kind, ResourceKind::Directory);
    /// assert!(repo.display_path.is_none());
    /// ```
    pub fn resource_param(
        &mut self,
        param: &str,
        kinds: &[ResourceKind],
    ) -> Result<ResourceHandle, CapabilityError> {
        self.resolve_resource(ResourceResolve {
            param: Some(param.to_owned()),
            role: None,
            project_id: None,
            kinds: kinds.to_vec(),
        })
    }

    /// A handle for the role `role` (`repository`, `compose`…) of the
    /// run's project (op `resource.resolve`). Without an active project the
    /// agent answers `no-active-project`.
    pub fn resource_role(
        &mut self,
        role: &str,
        kinds: &[ResourceKind],
    ) -> Result<ResourceHandle, CapabilityError> {
        self.resolve_resource(ResourceResolve {
            param: None,
            role: Some(role.to_owned()),
            project_id: None,
            kinds: kinds.to_vec(),
        })
    }

    /// `resource.resolve` with every field (a role of another project of
    /// the active profile that the action's own project parameter names).
    pub fn resolve_resource(
        &mut self,
        request: ResourceResolve,
    ) -> Result<ResourceHandle, CapabilityError> {
        self.checked(
            "resource.resolve",
            &CapabilityRequest::ResourceResolve(request),
            "a resource handle",
        )
    }

    /// Opens a bound location, optionally in a bound application, at a
    /// line and column (op `resource.open`, plugin API 1.9). Needs
    /// `resources.open` and a run the user started; 1 per second.
    pub fn open_resource(&mut self, request: ResourceOpen) -> Result<(), CapabilityError> {
        let request = CapabilityRequest::ResourceOpen(request);
        if let Err(problem) = request.validate() {
            return Err(bad_request("resource.open", &problem));
        }
        self.call("resource.open", &request).map(drop)
    }

    /// Runs the user's own `git` or `docker` (op `tool.run`, plugin API
    /// 1.9) with `args` (the subcommand first; an array, never a shell
    /// string) in the directory of `cwd`, a handle of this invocation.
    /// Needs `tools` for the tool, and `network` (`ls-remote`) or `modify`
    /// (docker's start, stop and restart, only in runs the user started)
    /// where the subcommand needs them. The agent checks the arguments
    /// against its policy, adds its fixed options and environment, and
    /// limits the time and the output.
    ///
    /// ```
    /// use deckpilot_plugin_sdk::testing::MockHost;
    /// use deckpilot_plugin_sdk::{CallResult, CapabilityRequest, Tool};
    ///
    /// let mut host = MockHost::new(|req| match req {
    ///     CapabilityRequest::ToolRun(run) if run.args[0] == "status" => {
    ///         CallResult::Ok(serde_json::json!({"exit": 0, "stdout": "# branch.head main"}))
    ///     }
    ///     _ => CallResult::Ok(serde_json::json!({"handle": "0".repeat(32),
    ///                                            "kind": "directory", "label": "Shop"})),
    /// });
    /// let mut ctx = host.context();
    /// let repo = ctx.resource_role("repository", &[deckpilot_plugin_sdk::ResourceKind::Directory]).unwrap();
    /// let out = ctx.run_tool(Tool::Git, &["status", "--porcelain=v2", "--branch", "-z"], &repo.handle, None).unwrap();
    /// assert_eq!(out.exit, Some(0));
    /// ```
    pub fn run_tool(
        &mut self,
        tool: Tool,
        args: &[&str],
        cwd: &str,
        timeout_ms: Option<u32>,
    ) -> Result<ToolOutput, CapabilityError> {
        self.checked(
            "tool.run",
            &CapabilityRequest::ToolRun(ToolRun {
                tool,
                args: args.iter().map(|a| (*a).to_owned()).collect(),
                cwd: cwd.to_owned(),
                timeout_ms,
            }),
            "a tool's output",
        )
    }

    /// Opens the http or https URL the user stored in the invoked action's
    /// parameter `param` (line `index` of a `lines` parameter; op
    /// `url.openParam`, plugin API 1.9). The plugin never names the URL.
    /// Needs `openUrl.fromParams` and a run the user started; the rates of
    /// [`Context::open_url`], 10 per invocation.
    pub fn open_url_param(
        &mut self,
        param: &str,
        index: Option<u32>,
    ) -> Result<(), CapabilityError> {
        let request = CapabilityRequest::OpenUrlParam(OpenUrlParam {
            param: param.to_owned(),
            index,
        });
        if let Err(problem) = request.validate() {
            return Err(bad_request("url.openParam", &problem));
        }
        self.call("url.openParam", &request).map(drop)
    }

    /// The active project (op `project.current`, plugin API 1.9; `None`
    /// without one). Needs `projectContext`.
    pub fn current_project(&mut self) -> Result<Option<ProjectInfo>, CapabilityError> {
        self.checked(
            "project.current",
            &CapabilityRequest::ProjectCurrent(NoArgs {}),
            "a project",
        )
    }

    /// The active profile's projects: ids, keys and names only (op
    /// `project.list`). Needs `projectContext`.
    pub fn projects(&mut self) -> Result<Vec<ProjectInfo>, CapabilityError> {
        self.checked::<ProjectList>(
            "project.list",
            &CapabilityRequest::ProjectList(NoArgs {}),
            "a project list",
        )
        .map(|list| list.projects)
    }

    /// Makes project `id` of the active profile the active one (op
    /// `project.select`). Needs `projectContext.select` and a run the user
    /// started; 2 per second.
    pub fn select_project(&mut self, id: &str) -> Result<(), CapabilityError> {
        let request = CapabilityRequest::ProjectSelect(ProjectSelect { id: id.to_owned() });
        if let Err(problem) = request.validate() {
            return Err(bad_request("project.select", &problem));
        }
        self.call("project.select", &request).map(drop)
    }

    /// The next or previous project (op `project.cycle`), wrapping at the
    /// ends unless `wrap` is `Some(false)`. Like [`Context::select_project`].
    pub fn cycle_project(
        &mut self,
        direction: CycleDirection,
        wrap: Option<bool>,
    ) -> Result<(), CapabilityError> {
        self.call(
            "project.cycle",
            &CapabilityRequest::ProjectCycle(ProjectCycle { direction, wrap }),
        )
        .map(drop)
    }

    /// The profile's name and the active project, as far as the `profile`
    /// permission's `read` scopes allow (op `profile.read`, plugin API 1.9).
    pub fn profile(&mut self) -> Result<ProfileInfo, CapabilityError> {
        self.checked(
            "profile.read",
            &CapabilityRequest::ProfileRead(NoArgs {}),
            "a profile",
        )
    }
}

#[cfg(test)]
mod tests {
    use deckpilot_plugin_abi::privileged::Tool;
    use deckpilot_plugin_abi::{CallErrorCode, CallResult, CapabilityRequest};

    use crate::CapabilityErrorKind;
    use crate::testing::MockHost;

    #[test]
    fn bad_requests_never_reach_the_host() {
        let mut host = MockHost::allow_all();
        let mut ctx = host.context();
        let error = ctx.run_tool(Tool::Git, &[], "nope", None).unwrap_err();
        assert_eq!(error.kind(), CapabilityErrorKind::BadRequest);
        let error = ctx.select_project("a/b").unwrap_err();
        assert_eq!(error.kind(), CapabilityErrorKind::BadRequest);
        let error = ctx.open_url_param("urls", Some(10)).unwrap_err();
        assert_eq!(error.kind(), CapabilityErrorKind::BadRequest);
        assert_eq!(ctx.calls(), 0);
    }

    #[test]
    fn an_agent_without_the_operation_says_so() {
        let mut host = MockHost::new(|req| match req {
            CapabilityRequest::ProfileRead(_) => {
                CallResult::Ok(serde_json::json!({"profile": "Work"}))
            }
            _ => CallResult::error(CallErrorCode::UnsupportedOp, "not implemented"),
        });
        let mut ctx = host.context();
        assert_eq!(ctx.profile().unwrap().profile.as_deref(), Some("Work"));
        assert_eq!(
            ctx.current_project().unwrap_err().kind(),
            CapabilityErrorKind::Unsupported
        );
    }
}
