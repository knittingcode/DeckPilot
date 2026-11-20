//! `resource.open` (X4) and `url.openParam` (X5) of plugin API 1.9
//! (`docs/marketplace/module-migration.md` §7): a bound location in a
//! bound application through the core launcher, and the literal http or
//! https URL the user stored in the invocation's own parameter, both only
//! in runs the user started.
//!
//! * `resource.open` opens the folder or file of a handle this invocation
//!   issued to the plugin ([`Executor::bound`], [`Executor::bound_inside`]),
//!   with the application of another such handle of kind `application`
//!   (a binding the user chose) or with the system's default handler. The
//!   program and its arguments come from the core's plans
//!   ([`deckpilot_core::launch::editor_plan`], the arguments of the editor
//!   plan the request names); the plugin never names a program, an
//!   argument or a path. A handle of kind `url` (a project's URL role)
//!   opens in the default browser only, never with an application, under
//!   the same rules as `url.openParam`'s values. 5 per invocation (the
//!   broker: 1 per second).
//! * `url.openParam` opens line `index` of the action's own text or list
//!   parameter, as the engine resolved it ([`InvocationResources::with_urls`]),
//!   only when it is written literally as an `http` or `https` URL: no
//!   other scheme, no user name or password, at most 2,048 bytes, printable
//!   ASCII only (an internationalized host in its `xn--` form), no
//!   backslash, and a host exactly as the URL parser reads it (no
//!   percent-encoded or numeric-shorthand host). Private and local hosts
//!   open, because the user typed them. Only a value the user wrote as it
//!   is opens, in any run the user started, a workflow's included: the
//!   core's provenance ([`deckpilot_core::resolution::Provenance`]) says
//!   so, and a default, a template, a reference or a workflow node's
//!   mapped value is refused (X5). 10 per invocation (the broker: the
//!   rates of `url.open`).
//! * Both re-check that the user started the run, whatever the broker
//!   decided, and write one audit record per call
//!   ([`super::PrivilegedRecord`]): the labels of the handles, the editor
//!   plan and the outcome; for a URL its origin only, never a path or a
//!   query.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use deckpilot_core::Os;
use deckpilot_core::launch::{self, Editor, LaunchPlan, Launcher, OpenTarget, Position};
use deckpilot_plugin_abi::privileged::{EditorPlan, OpenUrlParam, ResourceKind, ResourceOpen};
use deckpilot_plugin_abi::{CallErrorCode, CallResult};
use deckpilot_plugin_broker::PluginId;
use serde_json::Value;

use super::resources::InvocationResources;
use super::{Executor, PrivilegedRecord};

/// Most `resource.open` calls of one invocation (X4).
pub(super) const MAX_RESOURCE_OPENS: u32 = 5;
/// Most `url.openParam` calls of one invocation (X5).
pub(super) const MAX_PARAM_URLS: u32 = 10;
/// The highest line or column `resource.open` accepts (X4).
const MAX_OPEN_POSITION: u32 = 1_000_000;

/// What the opening operations of one invocation count and launch with.
#[derive(Debug)]
pub(super) struct Opens {
    resources: AtomicU32,
    urls: AtomicU32,
    os: Os,
    /// `None`: the system's launcher for programs and the executor's
    /// [`super::Opener`] for pages; tests record plans instead.
    launcher: Option<Arc<dyn Launcher>>,
}

impl Default for Opens {
    fn default() -> Self {
        Self {
            resources: AtomicU32::new(0),
            urls: AtomicU32::new(0),
            os: Os::current(),
            launcher: test_launcher::from_env(),
        }
    }
}

/// Records launch plans for agent tests (feature `test-hooks`, never in a
/// release build): with `DECKPILOT_TEST_PLUGIN_LAUNCH` set, the agent
/// appends every plan `resource.open` and `url.openParam` would run to that
/// file, one JSON object (`program`, `args`) per line, and starts nothing.
mod test_launcher {
    use std::sync::Arc;

    use deckpilot_core::launch::Launcher;

    #[cfg(feature = "test-hooks")]
    pub const ENV: &str = "DECKPILOT_TEST_PLUGIN_LAUNCH";

    #[cfg(feature = "test-hooks")]
    #[derive(Debug)]
    struct FileLauncher(std::path::PathBuf);

    #[cfg(feature = "test-hooks")]
    impl Launcher for FileLauncher {
        fn launch(
            &self,
            plan: &deckpilot_core::launch::LaunchPlan,
        ) -> Result<(), deckpilot_core::launch::LaunchError> {
            use std::io::Write as _;
            let line = serde_json::json!({"program": plan.program, "args": plan.args});
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.0)
                .and_then(|mut file| writeln!(file, "{line}"))
                .map_err(|source| deckpilot_core::launch::LaunchError::Spawn {
                    program: plan.program.clone(),
                    source,
                })
        }
    }

    /// The recording launcher, when an agent test asked for one.
    pub fn from_env() -> Option<Arc<dyn Launcher>> {
        #[cfg(feature = "test-hooks")]
        if let Some(path) = std::env::var_os(ENV).filter(|p| !p.is_empty()) {
            return Some(Arc::new(FileLauncher(path.into())));
        }
        None
    }
}

/// The real absolute path of `path`, without the `\\?\` prefix Windows
/// adds to a plain drive path (which editors do not all read).
fn canonical(path: &std::path::Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(path).ok()?;
    #[cfg(windows)]
    {
        let text = real.to_string_lossy();
        if let Some(plain) = text.strip_prefix(r"\\?\")
            && plain.as_bytes().get(1) == Some(&b':')
        {
            return Some(PathBuf::from(plain));
        }
    }
    Some(real)
}

/// A refusal: its reason code, its ABI code and a message that names
/// labels and origins only.
#[derive(Debug)]
struct Refused {
    code: &'static str,
    call: CallErrorCode,
    message: String,
}

impl Refused {
    fn new(code: &'static str, call: CallErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            call,
            message: message.into(),
        }
    }

    /// A refusal of the handle helpers (`code: message`), kept as it is.
    fn from_result(result: CallResult) -> Self {
        match result {
            CallResult::Error(error) => {
                let prefix = error.message.split(':').next().unwrap_or_default();
                let code = [
                    "not-permitted",
                    "resource-kind-mismatch",
                    "unavailable",
                    deckpilot_core::paths::codes::INVALID_PATH,
                    deckpilot_core::paths::codes::NOT_FOUND,
                ]
                .into_iter()
                .find(|code| *code == prefix)
                .unwrap_or("refused");
                Self {
                    code,
                    call: error.code,
                    message: error.message,
                }
            }
            CallResult::Ok(_) => Self::new("refused", CallErrorCode::Failed, "refused"),
        }
    }

    fn result(&self) -> CallResult {
        if self.message.starts_with(self.code) {
            CallResult::error(self.call, self.message.clone())
        } else {
            CallResult::error(self.call, format!("{}: {}", self.code, self.message))
        }
    }
}

fn not_user_initiated() -> Refused {
    Refused::new(
        "not-user-initiated",
        CallErrorCode::NotPermitted,
        deckpilot_plugin_broker::DenyReason::PrivilegedNotUserInitiated.to_string(),
    )
}

fn editor(plan: EditorPlan) -> Editor {
    match plan {
        EditorPlan::Vscode => Editor::VsCode,
        EditorPlan::Cursor => Editor::Cursor,
        EditorPlan::Jetbrains => Editor::JetBrains,
        EditorPlan::Zed => Editor::Zed,
        EditorPlan::Sublime => Editor::Sublime,
        EditorPlan::Other => Editor::Other,
    }
}

/// The value of a text or list parameter, for `url.openParam`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UrlParam {
    pub value: String,
    /// The user wrote it as it is: no default, template, reference or
    /// mapping produced it.
    pub literal: bool,
}

impl InvocationResources {
    /// The same with the values of the action's text and list parameters
    /// in `parameters` (of an action of `definition`), which
    /// `url.openParam` opens from, literal where `provenance` says so
    /// (without it, none is).
    #[must_use]
    pub fn with_urls(
        mut self,
        definition: &deckpilot_core::ActionType,
        parameters: &deckpilot_core::ActionParameters,
        provenance: Option<&deckpilot_core::resolution::Provenances>,
    ) -> Self {
        use deckpilot_core::{ParameterKind, ParameterValue};
        for spec in &definition.parameters {
            if !matches!(
                spec.kind,
                ParameterKind::Text { .. } | ParameterKind::Lines { .. }
            ) {
                continue;
            }
            if let Some(ParameterValue::Text(text)) = parameters.get(&spec.key) {
                self.urls.insert(
                    spec.key.clone(),
                    UrlParam {
                        value: text.clone(),
                        literal: provenance.is_some_and(|p| p.is_literal(&spec.key)),
                    },
                );
            }
        }
        self
    }
}

/// `text` as a URL to open, when it is written literally as an `http` or
/// `https` address (module docs). The messages never repeat the value.
fn literal_url(text: &str) -> Result<url::Url, Refused> {
    let invalid = |why: &str| {
        Refused::new(
            deckpilot_core::paths::codes::INVALID_URL,
            CallErrorCode::NotPermitted,
            format!("the parameter's value {why}"),
        )
    };
    let text = text.trim_matches([' ', '\t']);
    if text.is_empty() {
        return Err(invalid("is empty"));
    }
    if text.len() > deckpilot_plugin_abi::limits::MAX_OPEN_URL_LEN {
        return Err(invalid("is longer than 2048 bytes"));
    }
    if !text.chars().all(|c| c.is_ascii_graphic()) {
        return Err(invalid(
            "contains spaces, control or non-ASCII characters \
             (an internationalized host is written in its xn-- form)",
        ));
    }
    if text.contains('\\') {
        return Err(invalid("contains a backslash"));
    }
    let lower = text.to_ascii_lowercase();
    let Some(rest) = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
    else {
        return Err(invalid("is not an http or https address"));
    };
    let url = url::Url::parse(text).map_err(|_| invalid("is not a valid URL"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(invalid("is not an http or https address"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("carries a user name or password"));
    }
    // The host exactly as written: no userinfo (`@`), no percent-encoded,
    // numeric-shorthand or otherwise rewritten host.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let written = if authority.starts_with('[') {
        authority
            .find(']')
            .map_or(authority, |end| &authority[..=end])
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    match url.host_str() {
        Some(host) if !host.is_empty() && host == written => {}
        _ => return Err(invalid("does not name its host literally")),
    }
    if url.as_str().len() > deckpilot_plugin_abi::limits::MAX_OPEN_URL_LEN {
        return Err(invalid("is longer than 2048 bytes"));
    }
    Ok(url)
}

impl Executor {
    /// Plans are run by `launcher` on `os` instead of the system's
    /// launcher and opener.
    #[cfg(test)]
    pub fn with_launcher(mut self, os: Os, launcher: Arc<dyn Launcher>) -> Self {
        self.opens.os = os;
        self.opens.launcher = Some(launcher);
        self
    }

    fn launch(&self, plan: &LaunchPlan) -> Result<(), Refused> {
        let result = match &self.opens.launcher {
            Some(launcher) => launcher.launch(plan),
            None => launch::SystemLauncher.launch(plan),
        };
        result.map_err(|e| Refused::new(e.code(), CallErrorCode::Failed, e.to_string()))
    }

    /// `resource.open`, which the broker allowed (granted, a run the user
    /// started, within its rate).
    pub(super) fn open_resource(&self, plugin: &PluginId, request: &ResourceOpen) -> CallResult {
        let started = Instant::now();
        let mut details = Vec::new();
        let outcome = self.try_open_resource(plugin, request, &mut details);
        self.audit_privileged(&PrivilegedRecord {
            plugin: plugin.to_string(),
            op: "resource.open",
            grant: "resources:open",
            outcome: outcome.as_ref().err().map(|r| r.code),
            elapsed: started.elapsed(),
            details,
        });
        match outcome {
            Ok(()) => CallResult::Ok(Value::Null),
            Err(refused) => refused.result(),
        }
    }

    fn try_open_resource(
        &self,
        plugin: &PluginId,
        request: &ResourceOpen,
        details: &mut Vec<(&'static str, String)>,
    ) -> Result<(), Refused> {
        if !self.run.is_user_initiated() {
            return Err(not_user_initiated());
        }
        let target = self
            .bound(plugin, &request.handle)
            .map_err(Refused::from_result)?;
        details.push(("label", target.label.clone()));
        if target.kind == ResourceKind::Url {
            return self.open_bound_url(request, &target.value, details);
        }
        if request.path.is_some() && target.kind != ResourceKind::Directory {
            return Err(Refused::new(
                deckpilot_core::paths::codes::INVALID_PATH,
                CallErrorCode::BadRequest,
                format!("a path needs a folder, and `{}` is not one", target.label),
            ));
        }
        let path = self.location(plugin, request, &target, details)?;
        // Bound paths are absolute, so an argument made of one never reads
        // as an option.
        if !path.is_absolute() || path.to_string_lossy().starts_with('-') {
            return Err(Refused::new(
                deckpilot_core::paths::codes::INVALID_PATH,
                CallErrorCode::NotPermitted,
                format!("`{}` is not bound to an absolute path", target.label),
            ));
        }
        if request.position.is_some() && request.path.is_some() && path.is_dir() {
            return Err(Refused::new(
                launch::codes::INVALID_POSITION,
                CallErrorCode::BadRequest,
                "a line needs a file, not a folder",
            ));
        }
        let position = match request.position {
            None => None,
            Some(p) => {
                let in_range = |n: u32| (1..=MAX_OPEN_POSITION).contains(&n);
                if !in_range(p.line) || p.column.is_some_and(|c| !in_range(c)) {
                    return Err(Refused::new(
                        launch::codes::INVALID_POSITION,
                        CallErrorCode::BadRequest,
                        format!("a line and column are from 1 to {MAX_OPEN_POSITION}"),
                    ));
                }
                Some(Position {
                    line: p.line,
                    column: p.column,
                })
            }
        };
        let plan = match &request.with {
            None => {
                details.push(("with", "system".into()));
                launch::plan(self.opens.os, &OpenTarget::Path(path))
                    .map_err(|e| Refused::new(e.code(), CallErrorCode::Failed, e.to_string()))?
            }
            Some(with) => {
                let application = self.bound(plugin, with).map_err(Refused::from_result)?;
                details.push(("with", application.label.clone()));
                if application.kind != ResourceKind::Application {
                    return Err(Refused::new(
                        "resource-kind-mismatch",
                        CallErrorCode::NotPermitted,
                        format!("`{}` is not an application", application.label),
                    ));
                }
                let program = application.value.trim();
                if program.is_empty() || program.starts_with('-') {
                    return Err(Refused::new(
                        "invalid-application",
                        CallErrorCode::NotPermitted,
                        format!("`{}` does not name an application", application.label),
                    ));
                }
                let editor = editor(request.editor.unwrap_or(EditorPlan::Other));
                details.push(("editor", editor.name().into()));
                let (plan, located) =
                    launch::editor_plan(self.opens.os, editor, program, &path, position).map_err(
                        |e| Refused::new(e.code(), CallErrorCode::Failed, e.to_string()),
                    )?;
                details.push(("located", located.to_string()));
                plan
            }
        };
        self.count_resource_open()?;
        self.launch(&plan)
    }

    /// The folder or file `request` opens: the bound one, or with `path`
    /// a file or folder inside a bound folder, by its real path.
    fn location(
        &self,
        plugin: &PluginId,
        request: &ResourceOpen,
        target: &super::resources::Issued,
        details: &mut Vec<(&'static str, String)>,
    ) -> Result<PathBuf, Refused> {
        Ok(match target.kind {
            ResourceKind::Directory => match request.path.as_deref() {
                None => self
                    .bound_inside(plugin, &request.handle, None, true)
                    .map_err(Refused::from_result)?,
                // A file or folder inside the binding, by name and by where
                // links lead; opened by its real path.
                Some(relative) => {
                    details.push(("path", "relative".into()));
                    let inside = self
                        .bound_inside(plugin, &request.handle, Some(relative), false)
                        .map_err(Refused::from_result)?;
                    canonical(&inside).ok_or_else(|| {
                        Refused::new(
                            deckpilot_core::paths::codes::NOT_FOUND,
                            CallErrorCode::Failed,
                            format!("the path does not exist in `{}`", target.label),
                        )
                    })?
                }
            },
            ResourceKind::File => {
                deckpilot_core::paths::existing(PathBuf::from(&target.value), false).map_err(
                    |_| {
                        Refused::new(
                            deckpilot_core::paths::codes::NOT_FOUND,
                            CallErrorCode::Failed,
                            format!("the file bound as `{}` does not exist", target.label),
                        )
                    },
                )?
            }
            ResourceKind::Application | ResourceKind::DockerEnvironment | ResourceKind::Url => {
                return Err(Refused::new(
                    "resource-kind-mismatch",
                    CallErrorCode::NotPermitted,
                    format!("`{}` is not a folder or a file", target.label),
                ));
            }
        })
    }

    fn count_resource_open(&self) -> Result<(), Refused> {
        if self.opens.resources.fetch_add(1, Ordering::SeqCst) >= MAX_RESOURCE_OPENS {
            return Err(Refused::new(
                "throttled",
                CallErrorCode::Throttled,
                format!("at most {MAX_RESOURCE_OPENS} resources are opened in one action"),
            ));
        }
        Ok(())
    }

    /// A bound web address (kind `url`): in the default browser only, as
    /// a literal http or https URL.
    fn open_bound_url(
        &self,
        request: &ResourceOpen,
        value: &str,
        details: &mut Vec<(&'static str, String)>,
    ) -> Result<(), Refused> {
        if request.with.is_some() || request.position.is_some() || request.editor.is_some() {
            return Err(Refused::new(
                "bad-request",
                CallErrorCode::BadRequest,
                "a web address opens in the default browser, without an application, \
                 a position or an editor",
            ));
        }
        let url = literal_url(value).map_err(|refused| {
            Refused::new(
                refused.code,
                refused.call,
                refused
                    .message
                    .replacen("the parameter's value", "the bound address", 1),
            )
        })?;
        details.push(("with", "browser".into()));
        details.push(("origin", url.origin().ascii_serialization()));
        self.count_resource_open()?;
        self.open_page(&url)
    }

    /// Opens `url` in the default browser.
    fn open_page(&self, url: &url::Url) -> Result<(), Refused> {
        match &self.opens.launcher {
            Some(_) => {
                let plan = launch::url_plan(self.opens.os, url.as_str(), None)
                    .map_err(|e| Refused::new(e.code(), CallErrorCode::Failed, e.to_string()))?;
                self.launch(&plan)
            }
            None => self.opener.open(url).map_err(|problem| {
                Refused::new(
                    "open-failed",
                    CallErrorCode::Failed,
                    format!("the browser could not be started: {problem}"),
                )
            }),
        }
    }

    /// `url.openParam`, which the broker allowed.
    pub(super) fn open_url_param(&self, plugin: &PluginId, request: &OpenUrlParam) -> CallResult {
        let started = Instant::now();
        let mut details = vec![("param", request.param.clone())];
        let outcome = self.try_open_url_param(plugin, request, &mut details);
        self.audit_privileged(&PrivilegedRecord {
            plugin: plugin.to_string(),
            op: "url.openParam",
            grant: "openUrl:from-params",
            outcome: outcome.as_ref().err().map(|r| r.code),
            elapsed: started.elapsed(),
            details,
        });
        match outcome {
            Ok(()) => CallResult::Ok(Value::Null),
            Err(refused) => refused.result(),
        }
    }

    fn try_open_url_param(
        &self,
        plugin: &PluginId,
        request: &OpenUrlParam,
        details: &mut Vec<(&'static str, String)>,
    ) -> Result<(), Refused> {
        if !self.run.is_user_initiated() {
            return Err(not_user_initiated());
        }
        let param = self
            .handles()
            .text_param(plugin, &request.param)
            .ok_or_else(|| {
                Refused::new(
                    "url-param-missing",
                    CallErrorCode::NotPermitted,
                    format!(
                        "the action has no text or list parameter `{}` with a value",
                        request.param
                    ),
                )
            })?;
        if !param.literal {
            return Err(Refused::new(
                "url-param-not-literal",
                CallErrorCode::NotPermitted,
                format!(
                    "the parameter `{}` was not written as it is (a default, a template, \
                     a variable or a workflow mapping produced it); only a URL the user \
                     typed is opened",
                    request.param
                ),
            ));
        }
        let value = param.value;
        let index = request.index.unwrap_or(0);
        details.push(("index", index.to_string()));
        let line = usize::try_from(index)
            .ok()
            .and_then(|i| deckpilot_core::lines_of(&value).nth(i))
            .ok_or_else(|| {
                Refused::new(
                    "url-param-index",
                    CallErrorCode::BadRequest,
                    format!("the parameter `{}` has no line {index}", request.param),
                )
            })?;
        let url = literal_url(line)?;
        details.push(("origin", url.origin().ascii_serialization()));
        if self.opens.urls.fetch_add(1, Ordering::SeqCst) >= MAX_PARAM_URLS {
            return Err(Refused::new(
                "throttled",
                CallErrorCode::Throttled,
                format!("at most {MAX_PARAM_URLS} URLs are opened in one action"),
            ));
        }
        self.open_page(&url)
    }
}

#[cfg(test)]
mod tests;
