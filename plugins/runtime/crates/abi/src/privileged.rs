//! The capability operations of plugin API 1.9's privileged kinds
//! (`docs/marketplace/module-migration.md` §7, X1 to X9): resource handles
//! (`resource.resolve`, `resource.open`), brokered `git` and `docker`
//! (`tool.run`), the URL of the invocation's own parameter
//! (`url.openParam`), the project context (`project.current`,
//! `project.list`, `project.select`, `project.cycle`) and the profile read
//! (`profile.read`).
//!
//! Only first-party plugins shipped inside the app may be granted the
//! first four (the publisher gate, X12); `profile.read` is the existing
//! `profile` kind of every publisher. The broker decides each call against
//! the current grants and the run's origin; the agent carries out what it
//! allows. A guest never names a path, an executable, an environment, a URL
//! or another plugin: only a parameter key, a role, an opaque handle or a
//! project id.
//!
//! Added compatibly within ABI 1: a guest built before them never sends
//! them, and a host or agent that predates them refuses the unknown op.

use serde::{Deserialize, Serialize};

use crate::capability::RequestError;

/// Characters of a resource handle: 128 random bits as lowercase hex.
pub const HANDLE_LEN: usize = 32;
/// Longest parameter key (the manifest's camel keys) and role name.
pub const MAX_PARAM_KEY: usize = 32;
/// Longest project id.
pub const MAX_PROJECT_ID: usize = 64;
/// Most resource kinds one call may accept.
pub const MAX_RESOURCE_KINDS: usize = 4;
/// Most arguments of one `tool.run`.
pub const MAX_TOOL_ARGS: usize = 32;
/// Longest argument of `tool.run`, in bytes.
pub const MAX_TOOL_ARG: usize = 1024;
/// Longest time limit of `tool.run` (the docker `timeoutSeconds` maximum).
pub const MAX_TOOL_TIMEOUT_MS: u32 = 1_800_000;
/// Highest line or column of `resource.open`.
pub const MAX_POSITION: u32 = 1_000_000;
/// Highest `index` of `url.openParam` (a `lines` parameter of at most 10).
pub const MAX_URL_PARAM_INDEX: u32 = 9;

/// The kinds of machine resource (the manifest's `resource.kinds`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResourceKind {
    Directory,
    File,
    Application,
    DockerEnvironment,
}

/// `resource.resolve`: one handle for an action parameter or a role of
/// the run's project (X2). Exactly one of `param` and `role`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceResolve {
    /// A `resource` parameter of the invoked action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
    /// A role of the project (`repository`, `compose`…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// A project of the active profile; the run's project when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub kinds: Vec<ResourceKind>,
}

/// The `ok` value of `resource.resolve`. Unknown fields are **ignored**
/// (host to guest only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceHandle {
    /// Opaque, valid for this invocation of this plugin only.
    pub handle: String,
    pub kind: ResourceKind,
    /// The binding's name, which the user chose.
    pub label: String,
    /// The bound path, only with the `resources:paths` grant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_path: Option<String>,
}

/// The editor argument plans `resource.open` can use (X4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EditorPlan {
    Vscode,
    Cursor,
    Jetbrains,
    Zed,
    Sublime,
    Other,
}

/// A line and optional column, both from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
}

/// `resource.open`: a bound location, optionally in a bound application
/// (X4). The answer is `null`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceOpen {
    pub handle: String,
    /// A handle of kind `application`; the system's default handler when
    /// absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub with: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor: Option<EditorPlan>,
}

/// The executables of `tool.run`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Git,
    Docker,
}

impl Tool {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::Docker => "docker",
        }
    }
}

/// `tool.run`: the user's own `git` or `docker` with an argument array,
/// in the directory of a resource handle (X3). The host checks the array
/// against its policy, prepends its fixed prefix, sets the environment and
/// the executable; the guest never does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolRun {
    pub tool: Tool,
    /// The subcommand first, then its options and positionals. Never a
    /// shell string.
    pub args: Vec<String>,
    /// A handle of kind `directory` (or `file` for a compose file).
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u32>,
}

/// The sub-grant a `tool.run` needs besides `tools:<tool>`: the coarse
/// classification the broker checks before the host's full policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolAccess {
    /// `tools:<tool>`.
    Read,
    /// `tools:git:network` (`ls-remote`).
    Network,
    /// `tools:docker:modify` (start, stop, restart, compose up, down,
    /// restart).
    Modify,
}

impl ToolRun {
    /// The subcommand: the first argument, and for `docker compose` the
    /// second too (`compose up`). `None` when the array starts with an
    /// option, which only the host sets.
    pub fn subcommand(&self) -> Option<String> {
        let first = self.args.first()?;
        if first.starts_with('-') {
            return None;
        }
        match (self.tool, first.as_str(), self.args.get(1)) {
            (Tool::Docker, "compose" | "context", Some(second)) if !second.starts_with('-') => {
                Some(format!("{first} {second}"))
            }
            _ => Some(first.clone()),
        }
    }

    /// Which grant the run needs: `ls-remote` the network one, docker's
    /// state changes the modify one, everything else the read one. `None`
    /// when there is no subcommand.
    pub fn access(&self) -> Option<ToolAccess> {
        let subcommand = self.subcommand()?;
        Some(match (self.tool, subcommand.as_str()) {
            (Tool::Git, "ls-remote") => ToolAccess::Network,
            (
                Tool::Docker,
                "start" | "stop" | "restart" | "compose up" | "compose down" | "compose restart",
            ) => ToolAccess::Modify,
            _ => ToolAccess::Read,
        })
    }
}

/// The `ok` value of `tool.run`. Unknown fields are **ignored**.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutput {
    /// The exit status; `None` when the process was stopped.
    #[serde(default)]
    pub exit: Option<i32>,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    /// Output over the cap was cut.
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub duration_ms: u64,
}

/// `url.openParam`: opens the literal http or https URL the user stored in
/// the invocation's own parameter `param` (line `index` of a `lines`
/// parameter). The plugin never passes a URL (X5). The answer is `null`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenUrlParam {
    pub param: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
}

/// `project.select`: makes a project of the active profile the active one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSelect {
    pub id: String,
}

/// Which way `project.cycle` goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CycleDirection {
    Next,
    Previous,
}

/// `project.cycle`: the next or previous project of the active profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectCycle {
    pub direction: CycleDirection,
    /// Past the last project, start again at the first (the default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap: Option<bool>,
}

/// A project as plugins see it: no bindings, no paths. Unknown fields are
/// **ignored**.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub id: String,
    pub key: String,
    pub name: String,
}

/// The `ok` value of `project.list`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProjectList {
    #[serde(default)]
    pub projects: Vec<ProjectInfo>,
}

/// The `ok` value of `profile.read` (X9): the profile's name with the
/// `profile` scope, the active project with the `project` scope (`None`
/// without one). Never layouts, other profiles, bindings or paths.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProfileInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectInfo>,
}

/// Whether `handle` has the shape of a handle (it says nothing about
/// whether it is live: only the agent's invocation table does).
pub fn is_handle(handle: &str) -> bool {
    handle.len() == HANDLE_LEN
        && handle
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A parameter key or role: a lowercase letter, then letters and digits.
pub fn is_param_key(key: &str) -> bool {
    let mut chars = key.chars();
    key.len() <= MAX_PARAM_KEY
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_alphanumeric())
}

fn is_project_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_PROJECT_ID
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl ResourceResolve {
    pub(crate) fn validate(&self) -> Result<(), RequestError> {
        match (&self.param, &self.role) {
            (Some(key), None) | (None, Some(key)) if is_param_key(key) => {}
            (Some(_), None) | (None, Some(_)) => return Err(RequestError::Invalid("param")),
            _ => return Err(RequestError::Invalid("param or role")),
        }
        if self
            .project_id
            .as_deref()
            .is_some_and(|id| !is_project_id(id))
        {
            return Err(RequestError::Invalid("projectId"));
        }
        if self.kinds.is_empty() || self.kinds.len() > MAX_RESOURCE_KINDS {
            return Err(RequestError::TooLarge("kinds"));
        }
        let mut seen = self.kinds.clone();
        seen.sort_unstable();
        seen.dedup();
        if seen.len() != self.kinds.len() {
            return Err(RequestError::Invalid("kinds"));
        }
        Ok(())
    }
}

impl ResourceOpen {
    pub(crate) fn validate(&self) -> Result<(), RequestError> {
        if !is_handle(&self.handle) || self.with.as_deref().is_some_and(|h| !is_handle(h)) {
            return Err(RequestError::Invalid("handle"));
        }
        if let Some(position) = self.position {
            let in_range = |n: u32| (1..=MAX_POSITION).contains(&n);
            if !in_range(position.line) || position.column.is_some_and(|c| !in_range(c)) {
                return Err(RequestError::Invalid("position"));
            }
        }
        Ok(())
    }
}

impl ToolRun {
    pub(crate) fn validate(&self) -> Result<(), RequestError> {
        if self.args.is_empty() || self.args.len() > MAX_TOOL_ARGS {
            return Err(RequestError::TooLarge("args"));
        }
        if self
            .args
            .iter()
            .any(|a| a.is_empty() || a.len() > MAX_TOOL_ARG || a.chars().any(char::is_control))
        {
            return Err(RequestError::Invalid("args"));
        }
        if !is_handle(&self.cwd) {
            return Err(RequestError::Invalid("cwd"));
        }
        if self
            .timeout_ms
            .is_some_and(|t| t == 0 || t > MAX_TOOL_TIMEOUT_MS)
        {
            return Err(RequestError::TooLarge("timeoutMs"));
        }
        Ok(())
    }
}

impl OpenUrlParam {
    pub(crate) fn validate(&self) -> Result<(), RequestError> {
        if !is_param_key(&self.param) {
            return Err(RequestError::Invalid("param"));
        }
        if self.index.is_some_and(|i| i > MAX_URL_PARAM_INDEX) {
            return Err(RequestError::TooLarge("index"));
        }
        Ok(())
    }
}

impl ProjectSelect {
    pub(crate) fn validate(&self) -> Result<(), RequestError> {
        if is_project_id(&self.id) {
            Ok(())
        } else {
            Err(RequestError::Invalid("id"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CapabilityRequest;

    const HANDLE: &str = "0123456789abcdef0123456789abcdef";

    fn parse(json: &str) -> Result<CapabilityRequest, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn every_op_round_trips() {
        let cases = [
            (
                "resource.resolve",
                r#"{"op":"resource.resolve","args":{"role":"repository","kinds":["directory"]}}"#.to_owned(),
            ),
            (
                "resource.resolve",
                r#"{"op":"resource.resolve","args":{"param":"compose","projectId":"3f1c-aa","kinds":["file","directory"]}}"#.to_owned(),
            ),
            (
                "resource.open",
                format!(
                    r#"{{"op":"resource.open","args":{{"handle":"{HANDLE}","with":"{HANDLE}","position":{{"line":12,"column":4}},"editor":"vscode"}}}}"#
                ),
            ),
            (
                "tool.run",
                format!(
                    r#"{{"op":"tool.run","args":{{"tool":"git","args":["status","--porcelain=v2","--branch","-z"],"cwd":"{HANDLE}","timeoutMs":10000}}}}"#
                ),
            ),
            (
                "url.openParam",
                r#"{"op":"url.openParam","args":{"param":"urls","index":3}}"#.to_owned(),
            ),
            ("project.current", r#"{"op":"project.current","args":{}}"#.to_owned()),
            ("project.list", r#"{"op":"project.list","args":{}}"#.to_owned()),
            (
                "project.select",
                r#"{"op":"project.select","args":{"id":"7d9e2b1c-0000-4000-8000-000000000001"}}"#.to_owned(),
            ),
            (
                "project.cycle",
                r#"{"op":"project.cycle","args":{"direction":"previous","wrap":false}}"#.to_owned(),
            ),
            ("profile.read", r#"{"op":"profile.read","args":{}}"#.to_owned()),
        ];
        for (op, json) in cases {
            let request = parse(&json).unwrap_or_else(|e| panic!("{op}: {e}"));
            assert_eq!(request.op(), op);
            request.validate().unwrap_or_else(|e| panic!("{op}: {e}"));
            let back = serde_json::to_string(&request).unwrap();
            assert_eq!(parse(&back).unwrap(), request, "{op}");
            let original: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&back).unwrap(),
                original,
                "{op}"
            );
        }
    }

    #[test]
    fn no_request_names_a_path_an_executable_a_url_or_a_plugin() {
        for json in [
            r#"{"op":"resource.resolve","args":{"role":"repository","kinds":["directory"],"path":"/etc"}}"#.to_owned(),
            format!(r#"{{"op":"tool.run","args":{{"tool":"git","args":["status"],"cwd":"{HANDLE}","env":{{"GIT_SSH":"x"}}}}}}"#),
            format!(r#"{{"op":"tool.run","args":{{"tool":"git","args":["status"],"cwd":"{HANDLE}","executable":"/bin/sh"}}}}"#),
            format!(r#"{{"op":"tool.run","args":{{"tool":"bash","args":["-c","x"],"cwd":"{HANDLE}"}}}}"#),
            format!(r#"{{"op":"tool.run","args":{{"tool":"git","args":"status -z","cwd":"{HANDLE}"}}}}"#),
            r#"{"op":"url.openParam","args":{"param":"urls","url":"https://evil.example"}}"#.to_owned(),
            r#"{"op":"project.select","args":{"id":"a","plugin":"other.plugin"}}"#.to_owned(),
            r#"{"op":"project.current","args":{"profile":"other"}}"#.to_owned(),
            r#"{"op":"process.exec","args":{"argv":["git"]}}"#.to_owned(),
        ] {
            assert!(parse(&json).is_err(), "{json}");
        }
    }

    #[test]
    fn requests_are_bounded() {
        let run = |args: Vec<String>, cwd: &str, timeout_ms: Option<u32>| {
            CapabilityRequest::ToolRun(ToolRun {
                tool: Tool::Git,
                args,
                cwd: cwd.into(),
                timeout_ms,
            })
            .validate()
        };
        let status = || vec!["status".to_owned()];
        assert!(run(status(), HANDLE, Some(10_000)).is_ok());
        assert!(run(vec![], HANDLE, None).is_err());
        assert!(run(vec!["x".into(); MAX_TOOL_ARGS + 1], HANDLE, None).is_err());
        assert!(run(vec!["a\nb".into()], HANDLE, None).is_err());
        assert!(run(vec![String::new()], HANDLE, None).is_err());
        assert!(run(vec!["x".repeat(MAX_TOOL_ARG + 1)], HANDLE, None).is_err());
        for cwd in [
            "/home/me/repo",
            "",
            "0123456789ABCDEF0123456789ABCDEF",
            "../x",
        ] {
            assert!(run(status(), cwd, None).is_err(), "{cwd}");
        }
        assert!(run(status(), HANDLE, Some(0)).is_err());
        assert!(run(status(), HANDLE, Some(MAX_TOOL_TIMEOUT_MS + 1)).is_err());

        let resolve = |param: Option<&str>, role: Option<&str>, kinds: Vec<ResourceKind>| {
            CapabilityRequest::ResourceResolve(ResourceResolve {
                param: param.map(str::to_owned),
                role: role.map(str::to_owned),
                project_id: None,
                kinds,
            })
            .validate()
        };
        let dir = || vec![ResourceKind::Directory];
        assert!(resolve(Some("repo"), None, dir()).is_ok());
        assert!(resolve(None, None, dir()).is_err());
        assert!(resolve(Some("repo"), Some("repository"), dir()).is_err());
        assert!(resolve(Some("../etc"), None, dir()).is_err());
        assert!(resolve(Some("repo"), None, vec![]).is_err());
        assert!(
            resolve(
                Some("repo"),
                None,
                vec![ResourceKind::File, ResourceKind::File]
            )
            .is_err()
        );

        let open = |position: Option<Position>| {
            CapabilityRequest::ResourceOpen(ResourceOpen {
                handle: HANDLE.into(),
                with: None,
                position,
                editor: None,
            })
            .validate()
        };
        assert!(
            open(Some(Position {
                line: 1,
                column: Some(MAX_POSITION)
            }))
            .is_ok()
        );
        assert!(
            open(Some(Position {
                line: 0,
                column: None
            }))
            .is_err()
        );
        assert!(
            open(Some(Position {
                line: 1,
                column: Some(MAX_POSITION + 1)
            }))
            .is_err()
        );

        let param = |index| {
            CapabilityRequest::OpenUrlParam(OpenUrlParam {
                param: "urls".into(),
                index,
            })
            .validate()
        };
        assert!(param(Some(MAX_URL_PARAM_INDEX)).is_ok());
        assert!(param(Some(MAX_URL_PARAM_INDEX + 1)).is_err());
        let select =
            |id: &str| CapabilityRequest::ProjectSelect(ProjectSelect { id: id.into() }).validate();
        assert!(select("").is_err());
        assert!(select("a/b").is_err());
        assert!(select(&"a".repeat(MAX_PROJECT_ID + 1)).is_err());
    }

    #[test]
    fn the_broker_knows_which_sub_grant_a_run_needs() {
        let run = |tool, args: &[&str]| ToolRun {
            tool,
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            cwd: HANDLE.into(),
            timeout_ms: None,
        };
        assert_eq!(
            run(Tool::Git, &["status", "-z"]).access(),
            Some(ToolAccess::Read)
        );
        assert_eq!(
            run(Tool::Git, &["ls-remote", "--", "origin"]).access(),
            Some(ToolAccess::Network)
        );
        assert_eq!(
            run(Tool::Git, &["-c", "core.sshCommand=x", "status"]).access(),
            None
        );
        assert_eq!(
            run(Tool::Docker, &["ps", "--all"]).access(),
            Some(ToolAccess::Read)
        );
        for modify in [
            &["compose", "up", "--detach"][..],
            &["compose", "down"],
            &["stop", "--", "web"],
        ] {
            assert_eq!(
                run(Tool::Docker, modify).access(),
                Some(ToolAccess::Modify),
                "{modify:?}"
            );
        }
        assert_eq!(
            run(Tool::Docker, &["compose", "logs"])
                .subcommand()
                .as_deref(),
            Some("compose logs")
        );
        assert_eq!(run(Tool::Docker, &["--host", "x", "ps"]).access(), None);
    }

    #[test]
    fn replies_tolerate_additions() {
        let handle: ResourceHandle = serde_json::from_str(&format!(
            r#"{{"handle":"{HANDLE}","kind":"directory","label":"Shop","later":1}}"#
        ))
        .unwrap();
        assert_eq!(handle.display_path, None);
        let output: ToolOutput =
            serde_json::from_str(r#"{"exit":0,"stdout":"ok","later":true}"#).unwrap();
        assert_eq!(output.exit, Some(0));
        assert!(!output.truncated);
        let profile: ProfileInfo = serde_json::from_str(r#"{"profile":"Work"}"#).unwrap();
        assert_eq!(profile.project, None);
        let list: ProjectList = serde_json::from_str(
            r#"{"projects":[{"id":"a","key":"SHOP","name":"Shop","bindings":[]}]}"#,
        )
        .unwrap();
        assert_eq!(list.projects[0].key, "SHOP");
    }
}
