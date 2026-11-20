//! The host side of `tool.run` (plugin API 1.9, X3 in
//! `docs/marketplace/module-migration.md` §7): which argument arrays of
//! `git` and `docker` a first-party plugin may have the agent run, and how
//! the agent runs them.
//!
//! * **Executables.** Only `git` and `docker`, the user's own: a path the
//!   host holds ([`ToolHost`]) or the first match on the agent's own
//!   `PATH`, resolved by the agent for each run. The file name, and the
//!   name of the file it leads to, must be `git` or `docker` (`.exe` on
//!   Windows). The plugin never names or sees it.
//! * **Arguments.** An array, never a shell. [`check`] re-validates every
//!   element at run time, in this order: the denylists of
//!   [`deckpilot_plugin_manifest::tools`] (every option token before `--`,
//!   in the `--x=v`, `--x v` and attached short `-xv` forms, and each letter
//!   of a combined short option), the subcommand's denylist, the host's
//!   built-in table (`git.rs`, `docker.rs`), the manifest crate's policy,
//!   and the plugin's own manifest declaration: an option must be in all
//!   three, so a manifest can only narrow. Every value and positional
//!   fills a typed slot whose validator refuses a leading `-`, `..`,
//!   paths, URLs and anything outside its alphabet. The host emits a
//!   canonical argv: options as `--name=value`, `--` before positionals
//!   where the tool reads them as names, its own fixed prefix first.
//! * **Working directory.** Only a handle this invocation issued (X1/X2):
//!   a directory for git; a directory or a compose file for docker (also
//!   a Docker environment binding whose value is such a path, as the
//!   built-in's compose role allowed); for docker's commands outside
//!   compose also a Docker environment binding, whose context is then
//!   used, or no handle at all ([`Target::Unbound`]): `ps`, `logs` of a
//!   container, `version`, `context show` and `compose version` read no
//!   folder, and run in an empty folder the agent made for them
//!   ([`neutral_directory`]).
//! * **Environment.** Cleared, then [`BASE_ENVIRONMENT`] from the agent's
//!   own environment, `LC_ALL=C`, and each tool's own allowlist (`git.rs`,
//!   `docker.rs`). Nothing from the plugin; no stdin.
//! * **Limits.** A time limit per class (the plugin may only shorten it,
//!   never past the invocation's deadline), output caps per stream
//!   ([`MAX_STDOUT`], [`MAX_LOG_OUTPUT`], [`MAX_STDERR`]), at most
//!   [`MAX_CONCURRENT`] runs per plugin and one modifying run at a time
//!   ([`RunSlot`]). The process runs in its own process group and is stopped
//!   with it on the deadline, when the invocation is aborted (a disable, an
//!   update, an uninstall, a cancelled run) and when the grant it used is
//!   revoked ([`execute`]).
//! * **Output.** The exit status and the capped output, with the userinfo
//!   of every URL removed ([`redact_userinfo`]).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use deckpilot_core::CancellationToken;
use deckpilot_core::process::{ProcessError, ProcessLimits, ProcessOutput};
use deckpilot_plugin_abi::privileged::{Tool, ToolRun};
use deckpilot_plugin_manifest::tools::{self as policy, ToolGrant};
use deckpilot_plugin_manifest::{ToolDeclaration, ToolExecutable};

mod docker;
mod git;
#[cfg(test)]
mod tests;

/// Variables every run keeps from the agent's environment (the core's
/// list without the locale, which is fixed to `C`, and without `ComSpec`,
/// the shell). Each tool adds its own.
const BASE_ENVIRONMENT: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "TEMP",
    "TMP",
    "TZ",
    // Windows
    "SystemRoot",
    "SystemDrive",
    "windir",
    "PATHEXT",
    "USERPROFILE",
    "USERNAME",
    "USERDOMAIN",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "CommonProgramFiles",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "OS",
];

/// Bytes of standard output a run keeps. With every byte a NUL (`git
/// status -z`), escaped as `\u0000`, the reply still fits the 1 MiB host
/// call reply.
pub(crate) const MAX_STDOUT: usize = 128 * 1024;
/// Bytes of output of `logs` and `compose logs` (sensitive; as the
/// built-in).
pub(crate) const MAX_LOG_OUTPUT: usize = 64 * 1024;
/// Bytes of standard error a run keeps.
pub(crate) const MAX_STDERR: usize = 16 * 1024;
/// Most runs of one plugin at the same time; more wait for a slot.
pub(crate) const MAX_CONCURRENT: u32 = 4;
/// Time between the polite and the forced stop.
const GRACE: Duration = Duration::from_secs(2);
/// How often a running process is checked against its invocation and
/// grant.
const WATCH: Duration = Duration::from_millis(20);

/// Reason codes of a refusal or a failure, at the start of the message
/// (`tool-option-denied: …`), as `resources.rs` does.
pub(crate) mod codes {
    /// A denylisted option, in any spelling.
    pub const OPTION_DENIED: &str = "tool-option-denied";
    /// A denylisted subcommand, or an option where the subcommand belongs.
    pub const SUBCOMMAND_DENIED: &str = "tool-subcommand-denied";
    /// Not in the host's table.
    pub const NOT_ALLOWED: &str = "tool-not-allowed";
    /// Not in the plugin's manifest declaration.
    pub const NOT_DECLARED: &str = "tool-not-declared";
    /// A value or positional its slot does not accept, a missing required
    /// option, too many or too few positionals.
    pub const INVALID_ARGUMENT: &str = "tool-invalid-argument";
    /// The working directory handle is of a kind the subcommand cannot use.
    pub const INVALID_CWD: &str = "tool-invalid-cwd";
    /// The executable setting is not `git` or `docker`.
    pub const INVALID_EXECUTABLE: &str = "tool-executable-invalid";
    /// Neither the setting nor `PATH` has the executable.
    pub const NOT_FOUND: &str = "tool-not-found";
    /// The broker's classification and the host's policy disagree.
    pub const GRANT_MISMATCH: &str = "tool-grant-mismatch";
    /// A modifying run outside a run the user (or their work session)
    /// started.
    pub const NOT_USER_INITIATED: &str = "not-user-initiated";
    /// `ls-remote` of a remote the repository does not configure.
    pub const UNKNOWN_REMOTE: &str = "tool-unknown-remote";
    /// The bound folder is not a git work tree (not a repository, or a
    /// bare one).
    pub const NOT_A_REPOSITORY: &str = "not-a-repository";
    /// The repository git found is not the binding (its work tree is a
    /// folder above it).
    pub const OUTSIDE_BINDING: &str = "tool-outside-binding";
    /// The plugin has no manifest here.
    pub const UNAVAILABLE: &str = "unavailable";
    pub const TIMED_OUT: &str = "timed-out";
    pub const CANCELLED: &str = "cancelled";
    pub const REVOKED: &str = "revoked";
    pub const SPAWN_FAILED: &str = "spawn-failed";
}

/// Why a run was refused or failed: a reason code of [`codes`] and a
/// message that never holds an argument value, a path or the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Denial {
    pub code: &'static str,
    pub message: String,
}

impl Denial {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// A typed slot: what one option value or positional may be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    /// A git ref or branch name (`main`, `origin/main`, `refs/heads/x`).
    RefName,
    /// `branch.<ref name>.remote` or `branch.<ref name>.merge`.
    BranchKey,
    /// A remote's name, never a URL or a path.
    RemoteName,
    /// A `git log --format` of known placeholders and printable text.
    LogFormat,
    /// `no`, `normal` or `all`.
    Untracked,
    /// `{{json .}}`.
    JsonTemplate,
    /// `json` or `{{json .}}`.
    ComposeFormat,
    /// `name=<container name>`.
    NameFilter,
    /// A line count from 1 to [`MAX_TAIL`].
    Tail,
    /// A docker `--since`: a duration (`10m`) or a timestamp.
    Since,
    /// A container, service or context name.
    Name,
}

/// Highest `--tail` (X3b).
pub(crate) const MAX_TAIL: u32 = 1000;

/// An allowed option: an exact token, or a name whose value fills a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Opt {
    Flag(&'static str),
    Value(&'static str, Slot),
}

impl Opt {
    pub fn name(self) -> &'static str {
        match self {
            Self::Flag(name) | Self::Value(name, _) => name,
        }
    }
}

/// One subcommand of the host's table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Spec {
    pub subcommand: &'static str,
    pub options: &'static [Opt],
    /// Options the run must pass (`config --get`, `compose up --detach`).
    pub required: &'static [&'static str],
    /// The positionals' slots in order; the last one repeats up to `max`.
    pub slots: &'static [Slot],
    pub min: usize,
    pub max: usize,
    /// The host puts `--` before the positionals.
    pub separator: bool,
}

/// The arguments of one run, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Checked {
    pub tool: Tool,
    pub spec: &'static Spec,
    /// The grant the manifest crate's policy says the subcommand needs.
    pub grant: ToolGrant,
    /// In the order given: the option's name and its value.
    pub options: Vec<(&'static str, Option<String>)>,
    pub positionals: Vec<String>,
}

impl Checked {
    /// The option names, for the audit (never values).
    pub fn option_names(&self) -> Vec<&'static str> {
        self.options.iter().map(|(name, _)| *name).collect()
    }

    /// What the working directory handle may be.
    pub fn accepts(&self, target: TargetKind) -> bool {
        match self.tool {
            Tool::Git => target == TargetKind::Directory,
            Tool::Docker if docker::is_project(self.spec.subcommand) => {
                matches!(target, TargetKind::Directory | TargetKind::File)
            }
            Tool::Docker => true,
        }
    }

    /// Whether the run may go without a handle: docker's commands that
    /// read no folder. Compose's project commands need their bound folder
    /// or file, and git always works in a repository.
    pub fn runs_unbound(&self) -> bool {
        self.tool == Tool::Docker && !docker::is_project(self.spec.subcommand)
    }
}

/// The kind of the working directory handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetKind {
    Directory,
    File,
    DockerEnvironment,
}

/// The working directory handle's binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// An existing folder inside the binding.
    Directory(PathBuf),
    /// A compose file.
    File(PathBuf),
    /// A Docker context name.
    DockerEnvironment(String),
    /// No handle: docker's commands that read no folder, in an empty
    /// folder of the agent's own, with the host-held context.
    Unbound,
}

/// The empty folder docker's commands without a handle run in: made by
/// the agent in the system's temporary folder, readable by this user only,
/// named with 64 random bits, and made again if it is gone or anything
/// has appeared in it. Never a folder of the user's or the plugin's
/// choosing.
pub(crate) fn neutral_directory() -> Result<PathBuf, Denial> {
    static NEUTRAL: Mutex<Option<PathBuf>> = Mutex::new(None);
    let mut kept = NEUTRAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let usable = |dir: &Path| {
        std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir())
            && std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_none())
    };
    if let Some(dir) = kept.as_ref().filter(|dir| usable(dir)) {
        return Ok(dir.clone());
    }
    let unavailable = || {
        Denial::new(
            codes::SPAWN_FAILED,
            "the agent could not make an empty folder to run docker in",
        )
    };
    let mut bytes = [0_u8; 8];
    getrandom::fill(&mut bytes).map_err(|_| unavailable())?;
    let name = bytes.iter().fold(String::with_capacity(16), |mut out, b| {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
        out
    });
    let dir = std::env::temp_dir().join(format!("deckpilot-empty-{name}"));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    // `create`, never `create_all`: a folder that exists already is not
    // the agent's.
    builder.create(&dir).map_err(|_| unavailable())?;
    if let Some(old) = kept.replace(dir.clone()) {
        let _ = std::fs::remove_dir(old);
    }
    Ok(dir)
}

pub(crate) fn executable_of(tool: Tool) -> ToolExecutable {
    match tool {
        Tool::Git => ToolExecutable::Git,
        Tool::Docker => ToolExecutable::Docker,
    }
}

fn specs(tool: Tool) -> &'static [Spec] {
    match tool {
        Tool::Git => git::SPECS,
        Tool::Docker => docker::SPECS,
    }
}

/// Checks `run`'s arguments against the denylists, the host's table, the
/// manifest crate's policy and the plugin's declaration of the tool.
pub(crate) fn check(run: &ToolRun, declared: Option<&ToolDeclaration>) -> Result<Checked, Denial> {
    let executable = executable_of(run.tool);
    let name = run.tool.as_str();
    denied_options(executable, &run.args)?;
    let first = run.args.first().map(String::as_str).unwrap_or_default();
    if first.starts_with('-') || first.is_empty() {
        return Err(Denial::new(
            codes::SUBCOMMAND_DENIED,
            format!(
                "the arguments of {name} start with its subcommand; only the host sets options of {name} itself"
            ),
        ));
    }
    let (subcommand, words) = subcommand(run);
    for candidate in [first, subcommand.as_str()] {
        if let Some(reason) = policy::denied_subcommand(executable, candidate) {
            return Err(Denial::new(
                codes::SUBCOMMAND_DENIED,
                format!("`{name} {candidate}` is never allowed: it {reason}"),
            ));
        }
    }
    let Some(spec) = specs(run.tool).iter().find(|s| s.subcommand == subcommand) else {
        return Err(Denial::new(
            codes::NOT_ALLOWED,
            format!("`{name} {subcommand}` is not a subcommand DeckPilot runs for plugins"),
        ));
    };
    let Some(allowed) = policy::subcommand(executable, spec.subcommand) else {
        return Err(Denial::new(
            codes::NOT_ALLOWED,
            format!("`{name} {subcommand}` is not in the policy"),
        ));
    };
    let Some(declaration) =
        declared.and_then(|d| d.subcommands.iter().find(|s| s.name == spec.subcommand))
    else {
        return Err(Denial::new(
            codes::NOT_DECLARED,
            format!("the plugin's manifest does not declare `{name} {subcommand}`"),
        ));
    };
    let sub_granted = match allowed.grant {
        ToolGrant::Read => true,
        ToolGrant::Network => declared.is_some_and(ToolDeclaration::network),
        ToolGrant::Modify => declared.is_some_and(ToolDeclaration::modify),
    };
    if !sub_granted {
        return Err(Denial::new(
            codes::NOT_DECLARED,
            format!(
                "the plugin's manifest does not declare the sub-grant `{name} {subcommand}` needs"
            ),
        ));
    }
    let permitted = |option: &str| {
        allowed.options.contains(&option) && declaration.options().iter().any(|d| d == option)
    };
    let (options, positionals) = arguments(run.tool, spec, &run.args[words..], &permitted)?;
    for required in spec.required {
        if options.iter().all(|(n, _)| n != required) {
            return Err(Denial::new(
                codes::INVALID_ARGUMENT,
                format!("`{name} {subcommand}` needs `{required}`"),
            ));
        }
    }
    Ok(Checked {
        tool: run.tool,
        spec,
        grant: allowed.grant,
        options,
        positionals,
    })
}

/// The subcommand and how many arguments it takes: `docker compose …` and
/// `docker context …` two, everything else one.
fn subcommand(run: &ToolRun) -> (String, usize) {
    match run.subcommand() {
        Some(name) => {
            let words = name.split(' ').count();
            (name, words)
        }
        None => (String::new(), 1),
    }
}

/// The denylists, first, for every option token before `--`: as given,
/// as the name before `=`, a short option with its value attached, and
/// each letter of a combined short option (`-zc`).
fn denied_options(tool: ToolExecutable, args: &[String]) -> Result<(), Denial> {
    for token in args.iter().take_while(|a| a.as_str() != "--") {
        if !token.starts_with('-') {
            continue;
        }
        let mut forms = vec![token.clone()];
        if !token.starts_with("--") && token.len() > 2 {
            forms.extend(token[1..].chars().map(|c| format!("-{c}")));
        }
        for form in &forms {
            if let Some(reason) = policy::denied_option(tool, form) {
                let shown = form.split_once('=').map_or(form.as_str(), |(n, _)| n);
                return Err(Denial::new(
                    codes::OPTION_DENIED,
                    format!(
                        "`{shown}` is never allowed for {}: it {reason}",
                        tool.as_str()
                    ),
                ));
            }
        }
    }
    Ok(())
}

type Arguments = (Vec<(&'static str, Option<String>)>, Vec<String>);

/// The options and positionals after the subcommand, each in its slot.
fn arguments(
    tool: Tool,
    spec: &'static Spec,
    args: &[String],
    permitted: &dyn Fn(&str) -> bool,
) -> Result<Arguments, Denial> {
    let name = tool.as_str();
    let mut options: Vec<(&'static str, Option<String>)> = Vec::new();
    let mut positionals = Vec::new();
    let mut separated = false;
    let mut tokens = args.iter();
    while let Some(token) = tokens.next() {
        if !separated && token == "--" {
            separated = true;
            continue;
        }
        if separated || !token.starts_with('-') {
            positionals.push(token.clone());
            continue;
        }
        let (opt, value) = option(spec, token, &mut tokens).ok_or_else(|| {
            let shown = token.split_once('=').map_or(token.as_str(), |(n, _)| n);
            Denial::new(
                codes::NOT_ALLOWED,
                format!(
                    "`{shown}` is not an option DeckPilot allows for `{name} {}`",
                    spec.subcommand
                ),
            )
        })?;
        if !permitted(opt.name()) {
            return Err(Denial::new(
                codes::NOT_DECLARED,
                format!(
                    "the plugin's manifest does not declare `{}` for `{name} {}`",
                    opt.name(),
                    spec.subcommand
                ),
            ));
        }
        if options.iter().any(|(n, _)| *n == opt.name()) {
            return Err(Denial::new(
                codes::INVALID_ARGUMENT,
                format!("`{}` is given twice", opt.name()),
            ));
        }
        if let (Opt::Value(option, slot), Some(value)) = (opt, &value) {
            if let Some(reason) =
                policy::denied_option(executable_of(tool), &format!("{option}={value}"))
            {
                return Err(Denial::new(
                    codes::OPTION_DENIED,
                    format!("the value of `{option}` is never allowed: it {reason}"),
                ));
            }
            if !valid(slot, value) {
                return Err(Denial::new(
                    codes::INVALID_ARGUMENT,
                    format!("the value of `{option}` is not a valid {}", slot_name(slot)),
                ));
            }
        }
        options.push((opt.name(), value));
    }
    positional_slots(tool, spec, &positionals)?;
    Ok((options, positionals))
}

/// The option `token` is (with its value, from `=` or the next token).
fn option(
    spec: &Spec,
    token: &str,
    rest: &mut std::slice::Iter<'_, String>,
) -> Option<(Opt, Option<String>)> {
    for opt in spec.options {
        match *opt {
            Opt::Flag(flag) if flag == token => return Some((*opt, None)),
            Opt::Value(name, _) if name == token => {
                // A missing value is an empty one, which no slot accepts.
                return Some((*opt, Some(rest.next().cloned().unwrap_or_default())));
            }
            Opt::Value(name, _) => {
                if let Some(value) = token.strip_prefix(name).and_then(|v| v.strip_prefix('=')) {
                    return Some((*opt, Some(value.to_owned())));
                }
            }
            Opt::Flag(_) => {}
        }
    }
    None
}

fn positional_slots(tool: Tool, spec: &Spec, positionals: &[String]) -> Result<(), Denial> {
    let what = format!("`{} {}`", tool.as_str(), spec.subcommand);
    if positionals.len() < spec.min || positionals.len() > spec.max {
        return Err(Denial::new(
            codes::INVALID_ARGUMENT,
            if spec.max == 0 {
                format!("{what} takes no positional arguments")
            } else {
                format!(
                    "{what} takes {} to {} positional arguments",
                    spec.min, spec.max
                )
            },
        ));
    }
    for (index, value) in positionals.iter().enumerate() {
        let Some(slot) = spec.slots.get(index).or_else(|| spec.slots.last()).copied() else {
            return Err(Denial::new(
                codes::INVALID_ARGUMENT,
                format!("{what} takes no positional arguments"),
            ));
        };
        if !valid(slot, value) {
            return Err(Denial::new(
                codes::INVALID_ARGUMENT,
                format!(
                    "positional argument {} of {what} is not a valid {}",
                    index + 1,
                    slot_name(slot)
                ),
            ));
        }
    }
    Ok(())
}

fn slot_name(slot: Slot) -> &'static str {
    match slot {
        Slot::RefName => "ref name",
        Slot::BranchKey => "`branch.<name>.remote` or `branch.<name>.merge` key",
        Slot::RemoteName => "remote name",
        Slot::LogFormat => "log format",
        Slot::Untracked => "`no`, `normal` or `all`",
        Slot::JsonTemplate => "`{{json .}}`",
        Slot::ComposeFormat => "`json` or `{{json .}}`",
        Slot::NameFilter => "`name=<name>` filter",
        Slot::Tail => "line count from 1 to 1000",
        Slot::Since => "`--since` value",
        Slot::Name => "container, service or context name",
    }
}

/// Whether `value` fills `slot`. No slot accepts a leading `-`, an empty
/// value or a control character.
pub(crate) fn valid(slot: Slot, value: &str) -> bool {
    if value.is_empty() || value.starts_with('-') || value.chars().any(char::is_control) {
        return false;
    }
    match slot {
        Slot::RefName => git::valid_ref(value),
        Slot::BranchKey => git::valid_branch_key(value),
        Slot::RemoteName => git::valid_remote(value),
        Slot::LogFormat => git::valid_format(value),
        Slot::Untracked => matches!(value, "no" | "normal" | "all"),
        Slot::JsonTemplate => value == "{{json .}}",
        Slot::ComposeFormat => matches!(value, "json" | "{{json .}}"),
        Slot::NameFilter => value.strip_prefix("name=").is_some_and(docker::valid_name),
        Slot::Tail => value
            .parse::<u32>()
            .is_ok_and(|n| (1..=MAX_TAIL).contains(&n) && n.to_string() == value),
        Slot::Since => docker::valid_since(value),
        Slot::Name => docker::valid_name(value),
    }
}

/// Where the agent's environment comes from: the agent's process, or (in
/// tests) a fixed map.
#[derive(Debug, Clone)]
enum Environment {
    Agent,
    #[cfg(test)]
    Fixed(std::sync::Arc<BTreeMap<String, OsString>>),
}

impl Environment {
    fn var(&self, name: &str) -> Option<OsString> {
        match self {
            Self::Agent => std::env::var_os(name),
            #[cfg(test)]
            Self::Fixed(map) => map.get(name).cloned(),
        }
    }
}

/// What the host holds for `tool.run`, never the plugin: the executables,
/// the Docker context and the time limits the user set (`gitExecutable`,
/// `dockerExecutable`, `context`, `networkTimeoutSeconds`,
/// `timeoutSeconds` of the built-ins), and the agent's environment.
#[derive(Debug, Clone)]
pub(crate) struct ToolHost {
    git: Option<PathBuf>,
    docker: Option<PathBuf>,
    docker_context: Option<String>,
    network_timeout: Duration,
    modify_timeout: Duration,
    environment: Environment,
}

/// git's local reads.
pub(crate) const GIT_READ_TIMEOUT: Duration = Duration::from_secs(10);
/// docker's reads.
pub(crate) const DOCKER_READ_TIMEOUT: Duration = Duration::from_secs(30);
/// `networkTimeoutSeconds`: default and bounds.
const NETWORK_TIMEOUT: (u64, u64, u64) = (20, 5, 120);
/// `timeoutSeconds`: default and bounds.
const MODIFY_TIMEOUT: (u64, u64, u64) = (300, 10, 1800);

impl Default for ToolHost {
    fn default() -> Self {
        Self::from_agent()
    }
}

impl ToolHost {
    /// No executable or context set: `git` and `docker` from the agent's
    /// `PATH`, the default time limits, the agent's environment.
    pub fn from_agent() -> Self {
        let settings = SETTINGS
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        Self::unconfigured().with_settings(&settings)
    }

    /// As [`from_agent`](Self::from_agent), without the user's settings:
    /// a test never reads the process-wide settings that another test's
    /// agent may have configured meanwhile ([`configure`]).
    #[cfg(test)]
    pub fn for_test() -> Self {
        Self::unconfigured()
    }

    fn unconfigured() -> Self {
        Self {
            git: None,
            docker: None,
            docker_context: None,
            network_timeout: Duration::from_secs(NETWORK_TIMEOUT.0),
            modify_timeout: Duration::from_secs(MODIFY_TIMEOUT.0),
            environment: Environment::Agent,
        }
    }

    /// The user's settings (`ToolSettings`).
    pub fn with_settings(mut self, settings: &ToolSettings) -> Self {
        if let Some(path) = &settings.git {
            self = self.with_executable(Tool::Git, path);
        }
        if let Some(path) = &settings.docker {
            self = self.with_executable(Tool::Docker, path);
        }
        if let Some(context) = &settings.docker_context {
            self = self.with_docker_context(context);
        }
        self.with_timeouts(
            settings.network_seconds.unwrap_or(NETWORK_TIMEOUT.0),
            settings.modify_seconds.unwrap_or(MODIFY_TIMEOUT.0),
        )
    }

    /// The user's executable setting for `tool`.
    pub fn with_executable(mut self, tool: Tool, path: impl Into<PathBuf>) -> Self {
        match tool {
            Tool::Git => self.git = Some(path.into()),
            Tool::Docker => self.docker = Some(path.into()),
        }
        self
    }

    /// The user's Docker context setting.
    pub fn with_docker_context(mut self, context: &str) -> Self {
        self.docker_context = Some(context.to_owned());
        self
    }

    /// `networkTimeoutSeconds` and `timeoutSeconds`, within their bounds.
    pub fn with_timeouts(mut self, network_seconds: u64, modify_seconds: u64) -> Self {
        let (_, low, high) = NETWORK_TIMEOUT;
        self.network_timeout = Duration::from_secs(network_seconds.clamp(low, high));
        let (_, low, high) = MODIFY_TIMEOUT;
        self.modify_timeout = Duration::from_secs(modify_seconds.clamp(low, high));
        self
    }

    /// The agent's environment as `variables` (tests).
    #[cfg(test)]
    pub fn with_environment(mut self, variables: &[(&str, &str)]) -> Self {
        self.environment = Environment::Fixed(std::sync::Arc::new(
            variables
                .iter()
                .map(|(k, v)| ((*k).to_owned(), OsString::from(v)))
                .collect(),
        ));
        self
    }

    fn var(&self, name: &str) -> Option<OsString> {
        self.environment.var(name).filter(|v| !v.is_empty())
    }

    /// The executable of `tool`: the setting, or the first `git`/`docker`
    /// on the agent's `PATH`. Its name, and that of the file it leads to,
    /// must be the tool's.
    pub fn executable(&self, tool: Tool) -> Result<PathBuf, Denial> {
        let file_name = executable_name(tool);
        let configured = match tool {
            Tool::Git => self.git.as_ref(),
            Tool::Docker => self.docker.as_ref(),
        };
        let path = match configured {
            Some(path) => path.clone(),
            None => self
                .search_path(tool)
                .into_iter()
                .map(|dir| dir.join(file_name))
                .find(|candidate| candidate.is_file())
                .ok_or_else(|| {
                    Denial::new(
                        codes::NOT_FOUND,
                        format!("{} was not found; install it or set its location in DeckPilot's settings", tool.as_str()),
                    )
                })?,
        };
        let invalid = |why: &str| {
            Denial::new(
                codes::INVALID_EXECUTABLE,
                format!("the {} executable setting {why}", tool.as_str()),
            )
        };
        if !path.is_absolute() {
            return Err(invalid("is not an absolute path"));
        }
        let named = |p: &Path| {
            p.file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case(file_name))
        };
        if !named(&path) {
            return Err(invalid(&format!(
                "does not name a file called `{file_name}`"
            )));
        }
        let real = std::fs::canonicalize(&path).map_err(|_| {
            Denial::new(
                codes::NOT_FOUND,
                format!("{} was not found at its set location", tool.as_str()),
            )
        })?;
        if !real.is_file() || !named(&real) || !runnable(&real) {
            return Err(invalid(&format!(
                "does not lead to a program called `{file_name}`"
            )));
        }
        Ok(path)
    }

    /// The time limit of a run of `checked`; the plugin's own may only
    /// shorten it.
    pub fn timeout(&self, checked: &Checked, requested_ms: Option<u32>) -> Duration {
        let limit = match (checked.tool, checked.grant) {
            (_, ToolGrant::Network) => self.network_timeout,
            (_, ToolGrant::Modify) => self.modify_timeout,
            (Tool::Git, ToolGrant::Read) => GIT_READ_TIMEOUT,
            (Tool::Docker, ToolGrant::Read) => DOCKER_READ_TIMEOUT,
        };
        requested_ms.map_or(limit, |ms| limit.min(Duration::from_millis(u64::from(ms))))
    }

    /// The folders `tool` is looked for in: the absolute entries of the
    /// agent's `PATH`, then (macOS, where an agent started from the
    /// desktop has the system's short `PATH`) where Homebrew and Docker
    /// Desktop install it.
    fn search_path(&self, tool: Tool) -> Vec<PathBuf> {
        let mut dirs = self.path_entries();
        if cfg!(target_os = "macos") {
            let extra: &[&str] = match tool {
                Tool::Git => &["/opt/homebrew/bin", "/usr/local/bin"],
                Tool::Docker => &[
                    "/opt/homebrew/bin",
                    "/usr/local/bin",
                    "/Applications/Docker.app/Contents/Resources/bin",
                ],
            };
            for dir in extra.iter().map(PathBuf::from) {
                if !dirs.contains(&dir) {
                    dirs.push(dir);
                }
            }
        }
        dirs
    }

    /// The absolute, non-empty entries of the agent's `PATH`: a relative
    /// one (`.`, an empty entry) would find programs in the repository.
    fn path_entries(&self) -> Vec<PathBuf> {
        self.var("PATH")
            .map(|paths| {
                std::env::split_paths(&paths)
                    .filter(|dir| dir.is_absolute())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The base environment from the agent's own, `LC_ALL=C`, and
    /// `extra` from the agent's own. `PATH` keeps only absolute entries,
    /// and gets the folder of `program` (where docker's credential helpers
    /// and compose plugin live) when it is not on it.
    fn environment(&self, extra: &[&str], program: &Path) -> Vec<(String, OsString)> {
        let mut env: Vec<(String, OsString)> = BASE_ENVIRONMENT
            .iter()
            .chain(extra)
            .filter(|name| **name != "PATH")
            .filter_map(|name| self.var(name).map(|v| ((*name).to_owned(), v)))
            .collect();
        let mut path = self.path_entries();
        if let Some(dir) = program.parent().filter(|d| d.is_absolute())
            && !path.iter().any(|p| p == dir)
        {
            path.push(dir.to_path_buf());
        }
        if let Ok(joined) = std::env::join_paths(path)
            && !joined.is_empty()
        {
            env.insert(0, ("PATH".into(), joined));
        }
        env.push(("LC_ALL".into(), "C".into()));
        env
    }
}

fn executable_name(tool: Tool) -> &'static str {
    match (tool, cfg!(windows)) {
        (Tool::Git, false) => "git",
        (Tool::Git, true) => "git.exe",
        (Tool::Docker, false) => "docker",
        (Tool::Docker, true) => "docker.exe",
    }
}

#[cfg(unix)]
fn runnable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn runnable(_path: &Path) -> bool {
    true
}

/// Exactly what the agent runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Plan {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// The whole environment (it is cleared first).
    pub env: Vec<(String, OsString)>,
    pub cwd: PathBuf,
    pub timeout: Duration,
    pub max_stdout: usize,
    /// `ls-remote`: the remote, which `git remote` must list first.
    pub remote: Option<String>,
}

/// The plan of `checked` in `target`.
pub(crate) fn plan(
    host: &ToolHost,
    checked: &Checked,
    target: &Target,
    requested_ms: Option<u32>,
) -> Result<Plan, Denial> {
    let program = host.executable(checked.tool)?;
    let timeout = host.timeout(checked, requested_ms);
    let mut plan = match checked.tool {
        Tool::Git => git::plan(host, checked, target, program)?,
        Tool::Docker => docker::plan(host, checked, target, program)?,
    };
    plan.timeout = timeout;
    Ok(plan)
}

/// The `git remote` run that must list the remote of an `ls-remote` plan.
pub(crate) fn remote_listing(host: &ToolHost, ls_remote: &Plan) -> Plan {
    git::remote_listing(host, ls_remote)
}

/// The `git rev-parse --show-toplevel` run that must name the binding (or
/// a folder inside it) before any git run in it.
pub(crate) fn toplevel_listing(host: &ToolHost, plan: &Plan) -> Plan {
    git::toplevel(host, plan)
}

/// The user's settings of the tools: what the built-in `git` and `docker`
/// modules keep (`gitExecutable`, `networkTimeoutSeconds`,
/// `dockerExecutable`, `context`, `timeoutSeconds`), read by the host and
/// never by a plugin. Absent: the default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ToolSettings {
    pub git: Option<PathBuf>,
    pub docker: Option<PathBuf>,
    pub docker_context: Option<String>,
    pub network_seconds: Option<u64>,
    pub modify_seconds: Option<u64>,
}

/// The settings every new [`ToolHost::from_agent`] starts from.
static SETTINGS: std::sync::RwLock<ToolSettings> = std::sync::RwLock::new(ToolSettings {
    git: None,
    docker: None,
    docker_context: None,
    network_seconds: None,
    modify_seconds: None,
});

impl ToolSettings {
    /// The settings in the configuration of the built-in `git` and
    /// `docker` modules. An executable setting that is just the tool's
    /// name (the default) means the agent's `PATH`.
    pub fn from_modules(
        git: Option<&deckpilot_core::ActionParameters>,
        docker: Option<&deckpilot_core::ActionParameters>,
    ) -> Self {
        let text = |config: Option<&deckpilot_core::ActionParameters>, key: &str| {
            config
                .and_then(|c| c.text(key))
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        };
        let executable = |value: Option<String>, name: &str| {
            value
                .filter(|v| {
                    !v.eq_ignore_ascii_case(name) && !v.eq_ignore_ascii_case(&format!("{name}.exe"))
                })
                .map(PathBuf::from)
        };
        let seconds = |config: Option<&deckpilot_core::ActionParameters>, key: &str| {
            config.and_then(|c| c.integer(key)).map(i64::unsigned_abs)
        };
        Self {
            git: executable(text(git, "gitExecutable"), "git"),
            docker: executable(text(docker, "dockerExecutable"), "docker"),
            docker_context: text(docker, "context"),
            network_seconds: seconds(git, "networkTimeoutSeconds"),
            modify_seconds: seconds(docker, "timeoutSeconds"),
        }
    }
}

/// The user's settings changed (the agent loaded its modules): runs from
/// now on use them.
pub(crate) fn configure(settings: ToolSettings) {
    *SETTINGS
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = settings;
}

/// Why a running process was stopped by its watcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stop {
    /// The invocation was aborted, or its host exited.
    Cancelled,
    /// The grant the run used was revoked.
    Revoked,
}

/// A finished run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Finished {
    pub output: ProcessOutput,
    pub elapsed: Duration,
}

/// Runs `plan` without a shell, stdin closed, in its own process group,
/// until it exits, its time limit or `deadline` passes, or `stop` says
/// why it must stop (checked every [`WATCH`]).
pub(crate) fn execute(
    plan: &Plan,
    deadline: Option<Instant>,
    stop: &(dyn Fn() -> Option<Stop> + Sync),
) -> Result<Finished, Denial> {
    let started = Instant::now();
    let mut command = Command::new(&plan.program);
    command.args(&plan.args).current_dir(&plan.cwd).env_clear();
    for (name, value) in &plan.env {
        command.env(name, value);
    }
    let own = started + plan.timeout;
    let until = deadline.map_or(own, |d| d.min(own));
    let root = CancellationToken::new();
    let token = root.child_with_deadline(until);
    let done = AtomicBool::new(false);
    let why = AtomicU8::new(0);
    let limits = ProcessLimits {
        grace: GRACE,
        max_output: plan.max_stdout,
    };
    let result = std::thread::scope(|scope| {
        scope.spawn(|| {
            while !done.load(Ordering::SeqCst) {
                if let Some(reason) = stop() {
                    why.store(
                        match reason {
                            Stop::Cancelled => 1,
                            Stop::Revoked => 2,
                        },
                        Ordering::SeqCst,
                    );
                    root.cancel();
                    return;
                }
                std::thread::sleep(WATCH);
            }
        });
        let result = deckpilot_core::process::run(command, Some(&token), limits);
        done.store(true, Ordering::SeqCst);
        result
    });
    let elapsed = started.elapsed();
    match result {
        Ok(mut output) => {
            cap(&mut output.stderr, &mut output.stderr_truncated, MAX_STDERR);
            Ok(Finished { output, elapsed })
        }
        Err(ProcessError::TimedOut) => Err(Denial::new(
            codes::TIMED_OUT,
            format!(
                "the program did not finish within {} ms and was stopped",
                until.saturating_duration_since(started).as_millis()
            ),
        )),
        Err(ProcessError::Cancelled) if why.load(Ordering::SeqCst) == 2 => Err(Denial::new(
            codes::REVOKED,
            "the grant was revoked; the program was stopped",
        )),
        Err(ProcessError::Cancelled) => Err(Denial::new(
            codes::CANCELLED,
            "the invocation was stopped; so was the program",
        )),
        Err(error @ ProcessError::Spawn(_)) if error.code() == "program-not-found" => {
            Err(Denial::new(
                codes::NOT_FOUND,
                "the program could not be started: not found",
            ))
        }
        Err(error) => Err(Denial::new(
            codes::SPAWN_FAILED,
            format!("the program could not be run ({})", error.code()),
        )),
    }
}

/// Cuts `text` to at most `max` bytes, on a character boundary.
fn cap(text: &mut String, truncated: &mut bool, max: usize) {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        *truncated = true;
    }
}

/// Query parameters whose value is a secret.
const SECRET_PARAMS: &[&str] = &[
    "access_token",
    "private_token",
    "oauth_token",
    "token",
    "password",
];

/// Removes the credentials of every URL in `text`: the userinfo
/// (`https://user:token@host/x` becomes `https://host/x`, a password with
/// `/` in it too) and the values of [`SECRET_PARAMS`] in a query
/// (`?private_token=…` becomes `?private_token=[redacted]`).
pub(crate) fn redact_userinfo(text: &str) -> String {
    redact_params(&redact_authority(text))
}

fn redact_authority(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find("://") {
        let (head, tail) = rest.split_at(index + 3);
        out.push_str(head);
        // The URL runs to white space or a quote.
        let url_end = tail
            .find(|c: char| {
                c.is_whitespace() || c.is_control() || matches!(c, '\'' | '"' | '<' | '>' | '`')
            })
            .unwrap_or(tail.len());
        let url = &tail[..url_end];
        let before_query = url.find(['?', '#']).unwrap_or(url.len());
        // The last `@` before the host whose userinfo is `user:pass` (the
        // password may hold `/` and `@`) or `user`; a path with `@` in it
        // (`host/x@y`) is not userinfo.
        let skip = url[..before_query]
            .match_indices('@')
            .map(|(at, _)| at)
            .rev()
            .find(|&at| {
                let userinfo = &url[..at];
                match (userinfo.find('/'), userinfo.find(':')) {
                    (None, _) => true,
                    (Some(slash), Some(colon)) => colon < slash,
                    (Some(_), None) => false,
                }
            })
            .map(|at| at + 1);
        out.push_str(&url[skip.unwrap_or(0)..]);
        rest = &tail[url_end..];
    }
    out.push_str(rest);
    out
}

fn redact_params(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'scan: while !rest.is_empty() {
        for (index, c) in rest.char_indices() {
            if !matches!(c, '?' | '&' | ';') {
                continue;
            }
            let after = &rest[index + 1..];
            let Some(name) = SECRET_PARAMS.iter().find(|name| {
                after.as_bytes().get(name.len()) == Some(&b'=')
                    && after
                        .get(..name.len())
                        .is_some_and(|n| n.eq_ignore_ascii_case(name))
            }) else {
                continue;
            };
            let value_start = index + 1 + name.len() + 1;
            let value_end = rest[value_start..]
                .find(|c: char| {
                    c.is_whitespace()
                        || c.is_control()
                        || matches!(c, '&' | ';' | '#' | '\'' | '"' | '<' | '>' | '`')
                })
                .map_or(rest.len(), |end| value_start + end);
            out.push_str(&rest[..value_start]);
            out.push_str("[redacted]");
            rest = &rest[value_end..];
            continue 'scan;
        }
        out.push_str(rest);
        break;
    }
    out
}

/// One of a plugin's running `tool.run` processes: at most
/// [`MAX_CONCURRENT`] per plugin, of which at most one modifies. Released
/// on drop.
#[derive(Debug)]
pub(crate) struct RunSlot {
    plugin: String,
    modify: bool,
}

/// Running processes per plugin: all of them and the modifying ones.
static RUNNING: Mutex<BTreeMap<String, (u32, u32)>> = Mutex::new(BTreeMap::new());
static RELEASED: Condvar = Condvar::new();

impl RunSlot {
    /// Waits for a slot of `plugin` (a modifying one when `modify`) while
    /// `give_up` says no; `None` when it said yes first.
    pub fn acquire(plugin: &str, modify: bool, give_up: &dyn Fn() -> bool) -> Option<Self> {
        let mut running = RUNNING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            let (all, modifying) = running.get(plugin).copied().unwrap_or_default();
            if all < MAX_CONCURRENT && (!modify || modifying == 0) {
                running.insert(plugin.to_owned(), (all + 1, modifying + u32::from(modify)));
                return Some(Self {
                    plugin: plugin.to_owned(),
                    modify,
                });
            }
            if give_up() {
                return None;
            }
            running = RELEASED
                .wait_timeout(running, WATCH)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }
}

impl Drop for RunSlot {
    fn drop(&mut self) {
        let mut running = RUNNING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((all, modifying)) = running.get_mut(&self.plugin) {
            *all = all.saturating_sub(1);
            *modifying = modifying.saturating_sub(u32::from(self.modify));
            if *all == 0 {
                running.remove(&self.plugin);
            }
        }
        RELEASED.notify_all();
    }
}
