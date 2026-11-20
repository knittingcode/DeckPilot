//! `git` for `tool.run` (X3a): what the built-in git module runs
//! (`modules/git/src/repo.rs`, `actions.rs`), and nothing else.
//!
//! | Subcommand | Allowed |
//! | --- | --- |
//! | `status` | `--porcelain=v2`, `--branch`, `-z`, `--untracked-files=no\|normal\|all`; no pathspec |
//! | `log` | `-1` (required), `--no-show-signature`, `--format=format:…` or `tformat:…` of known placeholders (never a `pretty.*` name); no revision or path |
//! | `config` | `--get` (required) and one key, `branch.<ref>.remote` or `branch.<ref>.merge` |
//! | `remote` | bare (names only; never `-v` or `get-url`) |
//! | `rev-parse` | `--verify` (required), `-q`, one ref name |
//! | `ls-remote` | `-- <remote> [<ref>]`, the remote a name `git remote` lists (`tools:git:network`) |
//!
//! The fixed prefix is `-C <handle directory> --no-pager
//! --no-optional-locks` and, as configuration only the host sets (and only
//! to harden): `core.fsmonitor=false`, `log.showSignature=false` (no `gpg`
//! for a hostile repository's signatures), `protocol.allow=never` with
//! `https`, `ssh` and `git` allowed again (no `ext::`, `file://` or remote
//! helper transport such as `foo::bar`), `credential.interactive=never`
//! and an empty `core.askPass` (no prompt and no credential window, in any
//! run).
//!
//! The environment adds `GIT_TERMINAL_PROMPT=0`, `GIT_OPTIONAL_LOCKS=0`,
//! `GIT_PROTOCOL_FROM_USER=0`, `GCM_INTERACTIVE=never`, an empty
//! `GIT_ASKPASS`, `GIT_CEILING_DIRECTORIES=<the binding's parent>` (git
//! never looks for a repository above the binding), `GIT_EXEC_PATH` and
//! `XDG_CONFIG_HOME` from the agent's own, and `SSH_AUTH_SOCK` only for
//! `ls-remote`. Before every run the host asks `rev-parse --show-toplevel
//! --absolute-git-dir` and refuses unless the work tree and the git
//! directory (real paths) are the binding or inside it; a genuine linked
//! worktree's git directory is accepted, a submodule's is not. The user's global and system configuration stay in effect
//! (credential helpers, `safe.directory`); environment overrides of it
//! (`GIT_CONFIG_*`, `GIT_SSH_COMMAND`, `GIT_DIR`, `SSH_ASKPASS`…) never
//! reach git.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use deckpilot_plugin_manifest::tools::ToolGrant;

use super::{
    Checked, Denial, GIT_READ_TIMEOUT, MAX_STDOUT, Opt, Plan, Slot, Spec, Target, ToolHost, codes,
};

pub(super) const SPECS: &[Spec] = &[
    Spec {
        subcommand: "status",
        options: &[
            Opt::Flag("--porcelain=v2"),
            Opt::Flag("--branch"),
            Opt::Flag("-z"),
            Opt::Value("--untracked-files", Slot::Untracked),
        ],
        required: &[],
        slots: &[],
        min: 0,
        max: 0,
        separator: false,
    },
    Spec {
        subcommand: "log",
        options: &[
            Opt::Flag("-1"),
            Opt::Flag("--no-show-signature"),
            Opt::Value("--format", Slot::LogFormat),
        ],
        required: &["-1"],
        slots: &[],
        min: 0,
        max: 0,
        separator: false,
    },
    Spec {
        subcommand: "config",
        options: &[Opt::Flag("--get")],
        required: &["--get"],
        slots: &[Slot::BranchKey],
        min: 1,
        max: 1,
        separator: false,
    },
    Spec {
        subcommand: "remote",
        options: &[],
        required: &[],
        slots: &[],
        min: 0,
        max: 0,
        separator: false,
    },
    Spec {
        subcommand: "rev-parse",
        options: &[Opt::Flag("--verify"), Opt::Flag("-q")],
        required: &["--verify"],
        slots: &[Slot::RefName],
        min: 1,
        max: 1,
        // `rev-parse --verify` takes no `--`; the ref slot refuses a
        // leading `-`.
        separator: false,
    },
    Spec {
        subcommand: "ls-remote",
        options: &[],
        required: &[],
        slots: &[Slot::RemoteName, Slot::RefName],
        min: 1,
        max: 2,
        separator: true,
    },
];

/// Variables git keeps from the agent's environment besides the base.
const ENVIRONMENT: &[&str] = &["GIT_EXEC_PATH", "XDG_CONFIG_HOME"];
/// … and for `ls-remote` (`tools:git:network`): the SSH agent.
const NETWORK_ENVIRONMENT: &[&str] = &["GIT_EXEC_PATH", "XDG_CONFIG_HOME", "SSH_AUTH_SOCK"];

/// The configuration the host sets on every run, only to harden.
pub(super) const HARDENING: &[&str] = &[
    "core.fsmonitor=false",
    "log.showSignature=false",
    "protocol.allow=never",
    "protocol.https.allow=always",
    "protocol.ssh.allow=always",
    "protocol.git.allow=always",
    "protocol.ext.allow=never",
    "protocol.file.allow=never",
    "credential.interactive=never",
    "core.askPass=",
];

/// The fixed prefix before the subcommand.
fn prefix(dir: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "-C".into(),
        dir.as_os_str().to_owned(),
        "--no-pager".into(),
        "--no-optional-locks".into(),
    ];
    for setting in HARDENING {
        args.push("-c".into());
        args.push((*setting).into());
    }
    args
}

fn environment(
    host: &ToolHost,
    network: bool,
    dir: &Path,
    program: &Path,
) -> Vec<(String, OsString)> {
    let mut env = host.environment(
        if network {
            NETWORK_ENVIRONMENT
        } else {
            ENVIRONMENT
        },
        program,
    );
    for (name, value) in [
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_OPTIONAL_LOCKS", "0"),
        ("GIT_PROTOCOL_FROM_USER", "0"),
        ("GCM_INTERACTIVE", "never"),
        ("GIT_ASKPASS", ""),
    ] {
        env.push((name.into(), value.into()));
    }
    // Repository discovery stops at the binding: never a repository of a
    // folder above it (a dotfiles repository in the home folder).
    if let Some(parent) = dir.parent() {
        env.push((
            "GIT_CEILING_DIRECTORIES".into(),
            parent.as_os_str().to_owned(),
        ));
    }
    env
}

pub(super) fn plan(
    host: &ToolHost,
    checked: &Checked,
    target: &Target,
    program: PathBuf,
) -> Result<Plan, Denial> {
    let Target::Directory(dir) = target else {
        return Err(Denial::new(
            codes::INVALID_CWD,
            "git runs only in a directory handle",
        ));
    };
    let mut args = prefix(dir);
    args.push(checked.spec.subcommand.into());
    for (name, value) in &checked.options {
        args.push(match value {
            Some(value) => format!("{name}={value}").into(),
            None => (*name).into(),
        });
    }
    if checked.spec.separator && !checked.positionals.is_empty() {
        args.push("--".into());
    }
    args.extend(checked.positionals.iter().map(OsString::from));
    let network = checked.grant == ToolGrant::Network;
    let env = environment(host, network, dir, &program);
    Ok(Plan {
        program,
        args,
        env,
        cwd: dir.clone(),
        timeout: GIT_READ_TIMEOUT,
        max_stdout: MAX_STDOUT,
        remote: network.then(|| checked.positionals[0].clone()),
    })
}

/// `git <prefix> <args>` in the directory of `of`, with the local
/// environment.
fn local(host: &ToolHost, of: &Plan, extra: &[&str]) -> Plan {
    let mut args = prefix(&of.cwd);
    args.extend(extra.iter().map(OsString::from));
    Plan {
        program: of.program.clone(),
        args,
        env: environment(host, false, &of.cwd, &of.program),
        cwd: of.cwd.clone(),
        timeout: GIT_READ_TIMEOUT,
        max_stdout: MAX_STDOUT,
        remote: None,
    }
}

/// `git <prefix> remote`: the remotes `ls-remote` may contact.
pub(super) fn remote_listing(host: &ToolHost, ls_remote: &Plan) -> Plan {
    local(host, ls_remote, &["remote"])
}

/// `git <prefix> rev-parse --show-toplevel --absolute-git-dir`: the work
/// tree and the git directory, which must be
/// the binding or inside it.
pub(super) fn toplevel(host: &ToolHost, plan: &Plan) -> Plan {
    local(
        host,
        plan,
        &["rev-parse", "--show-toplevel", "--absolute-git-dir"],
    )
}

/// A ref or branch name by `git check-ref-format`'s rules, a little
/// stricter: ASCII only, at most 255 bytes, no leading `-` or `/`.
pub(super) fn valid_ref(name: &str) -> bool {
    name.len() <= 255
        && !name.starts_with(['-', '/'])
        && !name.ends_with(['/', '.'])
        && !name.contains("..")
        && !name.contains("//")
        && !name.contains("@{")
        && name != "@"
        && name
            .chars()
            .all(|c| c.is_ascii_graphic() && !matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\'))
        && name.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && !part.to_ascii_lowercase().ends_with(".lock")
        })
}

/// `branch.<ref>.remote` or `branch.<ref>.merge`: the only keys
/// `config --get` reads, so `credential.*`, `http.*.extraHeader`, `url.*`
/// and `remote.*.url` cannot.
pub(super) fn valid_branch_key(key: &str) -> bool {
    key.strip_prefix("branch.")
        .and_then(|rest| {
            rest.strip_suffix(".remote")
                .or_else(|| rest.strip_suffix(".merge"))
        })
        .is_some_and(valid_ref)
}

/// A remote's name, never a URL (`ext::`, `file://`, `host:path`) or a
/// path.
pub(super) fn valid_remote(name: &str) -> bool {
    name.len() <= 100
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// `git log` placeholders a format may use (`%x00` separators included).
const PLACEHOLDERS: &[&str] = &[
    "x00", "x09", "x0a", "x1e", "x1f", "an", "ae", "aI", "ai", "at", "ad", "ar", "cn", "ce", "cI",
    "ci", "ct", "cd", "cr", "H", "h", "T", "t", "P", "p", "s", "b", "B", "D", "d", "n", "%",
];

/// A `--format` of `format:` or `tformat:` and known placeholders and
/// printable ASCII text. No `%(…)` expansions, no `pretty.*` names.
pub(super) fn valid_format(format: &str) -> bool {
    // Without the prefix a word (`short2`) names a `pretty.*` format of
    // the repository's configuration, which could expand to anything.
    let Some(body) = format
        .strip_prefix("format:")
        .or_else(|| format.strip_prefix("tformat:"))
    else {
        return false;
    };
    if format.len() > 256 || !body.chars().all(|c| c == ' ' || c.is_ascii_graphic()) {
        return false;
    }
    let mut rest = body;
    while let Some(at) = rest.find('%') {
        let after = &rest[at + 1..];
        let Some(placeholder) = PLACEHOLDERS.iter().find(|p| after.starts_with(**p)) else {
            return false;
        };
        rest = &after[placeholder.len()..];
    }
    true
}
