//! The host's policy for the `tools` permission (plugin API 1.9, X3 in
//! `docs/marketplace/module-migration.md` §7): the subcommands of `git` and
//! `docker` a first-party plugin may declare, the options of each, which
//! sub-grant each needs, and the denylists checked first for a precise
//! message.
//!
//! The allowlist is the boundary. A manifest may declare only a subset of
//! it; the agent (WP-A4) checks every argument array against the same
//! table before it runs anything, with the typed value validators an
//! option or positional needs. The denylist is a second line of defence and
//! gives the reason; on its own it would be incomplete by nature.

use crate::model::ToolExecutable;

/// Which grant a subcommand needs besides the tool's own read grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ToolGrant {
    /// `tools:<tool>`.
    Read,
    /// `tools:git:network`: contacts remotes with the user's SSH identity.
    Network,
    /// `tools:docker:modify`: starts, stops or restarts containers.
    Modify,
}

/// One allowed subcommand and its options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolPolicy {
    /// One word, or two separated by one space (`compose up`).
    pub subcommand: &'static str,
    /// Exact tokens (`-z`, `--porcelain=v2`) or names of options whose
    /// value the host checks (`--format`, `--tail`).
    pub options: &'static [&'static str],
    pub grant: ToolGrant,
}

const fn policy(
    subcommand: &'static str,
    options: &'static [&'static str],
    grant: ToolGrant,
) -> ToolPolicy {
    ToolPolicy {
        subcommand,
        options,
        grant,
    }
}

/// What the built-in git module runs today (`modules/git/src/repo.rs`), X3a.
pub const GIT_POLICY: &[ToolPolicy] = &[
    policy(
        "status",
        &["--porcelain=v2", "--branch", "-z"],
        ToolGrant::Read,
    ),
    policy("log", &["-1", "--format"], ToolGrant::Read),
    policy("config", &["--get"], ToolGrant::Read),
    policy("remote", &[], ToolGrant::Read),
    policy("rev-parse", &["--verify", "-q"], ToolGrant::Read),
    policy("ls-remote", &[], ToolGrant::Network),
];

/// What the built-in docker module runs today (`modules/docker/src/cli.rs`,
/// `actions.rs`), X3b and X3c. `compose` subcommands always get the bound
/// compose file and project from the host, never from the plugin.
pub const DOCKER_POLICY: &[ToolPolicy] = &[
    policy("context show", &[], ToolGrant::Read),
    policy("version", &["--format"], ToolGrant::Read),
    policy("compose version", &["--short"], ToolGrant::Read),
    policy(
        "ps",
        &["--no-trunc", "--format", "--all", "--filter"],
        ToolGrant::Read,
    ),
    policy("compose ps", &["--format"], ToolGrant::Read),
    policy("compose logs", &["--tail", "--since"], ToolGrant::Read),
    policy("logs", &["--tail", "--since"], ToolGrant::Read),
    policy("compose up", &["--detach"], ToolGrant::Modify),
    policy("compose down", &["--remove-orphans"], ToolGrant::Modify),
    policy("compose restart", &[], ToolGrant::Modify),
    policy("start", &[], ToolGrant::Modify),
    policy("stop", &[], ToolGrant::Modify),
    policy("restart", &[], ToolGrant::Modify),
];

/// git options that run programs, change where git looks, or write files
/// (X3a "Denied always"). A `core.*` or `alias.*` key is refused with them.
pub const GIT_DENIED_OPTIONS: &[&str] = &[
    "-c",
    "--config-env",
    "-C",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--exec-path",
    "--upload-pack",
    "-u",
    "--receive-pack",
    "--output",
    "-o",
    "--ext-diff",
    "--textconv",
    "--exec",
    "--super-prefix",
];

/// docker subcommands that create, change or enter containers, print
/// secrets or reach other daemons (X3b, X3c "Denied always").
pub const DOCKER_DENIED_SUBCOMMANDS: &[&str] = &[
    "run",
    "exec",
    "create",
    "cp",
    "attach",
    "build",
    "push",
    "pull",
    "login",
    "logout",
    "inspect",
    "kill",
    "system",
    "volume",
    "network",
    "plugin",
    "swarm",
    "secret",
    "image",
    "compose config",
    "compose run",
    "compose exec",
    "compose cp",
    "compose build",
    "compose push",
    "compose pull",
    "compose kill",
];

/// docker options that mount, escalate, pick another daemon or file, or
/// destroy data (X3b, X3c).
pub const DOCKER_DENIED_OPTIONS: &[&str] = &[
    "-v",
    "--volume",
    "--volumes",
    "--mount",
    "--privileged",
    "--cap-add",
    "--device",
    "--pid",
    "--network",
    "--security-opt",
    "-H",
    "--host",
    "--context",
    "--config",
    "--tls",
    "--tlsverify",
    "--tlscacert",
    "--tlscert",
    "--tlskey",
    "-e",
    "--env",
    "--env-file",
    "--project-directory",
    "-f",
    "--file",
    "--rmi",
    "--build",
    "--force-recreate",
    "--renew-anon-volumes",
];

/// The allowlist of `tool`.
pub fn policy_of(tool: ToolExecutable) -> &'static [ToolPolicy] {
    match tool {
        ToolExecutable::Git => GIT_POLICY,
        ToolExecutable::Docker => DOCKER_POLICY,
    }
}

/// The allowed subcommand `name` of `tool`.
pub fn subcommand(tool: ToolExecutable, name: &str) -> Option<&'static ToolPolicy> {
    policy_of(tool).iter().find(|p| p.subcommand == name)
}

/// Why `name` is never allowed for `tool`, if it is on the denylist.
pub fn denied_subcommand(tool: ToolExecutable, name: &str) -> Option<&'static str> {
    match tool {
        ToolExecutable::Docker if DOCKER_DENIED_SUBCOMMANDS.contains(&name) => {
            Some("creates, changes or enters containers, prints secrets or reaches another daemon")
        }
        ToolExecutable::Git if name.starts_with('-') => {
            Some("is an option of git itself, which only the host sets")
        }
        ToolExecutable::Docker if name.starts_with('-') => {
            Some("is an option of docker itself, which only the host sets")
        }
        _ => None,
    }
}

/// Why `option` is never allowed for `tool`, if it is on the denylist (also
/// as `--option=value`, or a short option with its value attached).
pub fn denied_option(tool: ToolExecutable, option: &str) -> Option<&'static str> {
    let name = option.split_once('=').map_or(option, |(name, _)| name);
    let listed = |list: &[&str]| {
        list.iter().any(|denied| {
            name == *denied
                // `-cfoo=bar`, `-ofile`: a short option with its value.
                || (denied.len() == 2 && !denied.starts_with("--") && name.starts_with(denied))
        })
    };
    match tool {
        ToolExecutable::Git if listed(GIT_DENIED_OPTIONS) => {
            Some("runs programs, changes where git looks or writes files")
        }
        ToolExecutable::Git if option.contains("core.") || option.contains("alias.") => {
            Some("names a `core.*` or `alias.*` key")
        }
        ToolExecutable::Docker if listed(DOCKER_DENIED_OPTIONS) || name.starts_with("--tls") => {
            Some("mounts, escalates, picks another daemon or file, or destroys data")
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_allowlist_and_the_denylist_never_overlap() {
        for tool in ToolExecutable::ALL {
            for policy in policy_of(tool) {
                assert_eq!(
                    denied_subcommand(tool, policy.subcommand),
                    None,
                    "{policy:?}"
                );
                for option in policy.options {
                    assert_eq!(denied_option(tool, option), None, "{tool:?} {option}");
                }
                let words: Vec<&str> = policy.subcommand.split(' ').collect();
                assert!(
                    (1..=2).contains(&words.len()) && words.iter().all(|w| !w.is_empty()),
                    "{policy:?}"
                );
            }
        }
    }

    #[test]
    fn denied_options_are_found_in_every_spelling() {
        for option in [
            "-c",
            "-cfoo=bar",
            "--config-env=x",
            "--upload-pack=evil",
            "-ofile",
        ] {
            assert!(
                denied_option(ToolExecutable::Git, option).is_some(),
                "{option}"
            );
        }
        for option in [
            "--tlsverify",
            "--tls-anything",
            "-H",
            "--host=tcp://x",
            "-v",
        ] {
            assert!(
                denied_option(ToolExecutable::Docker, option).is_some(),
                "{option}"
            );
        }
        assert!(denied_option(ToolExecutable::Git, "--format=core.x").is_some());
        assert!(denied_option(ToolExecutable::Git, "--format").is_none());
    }

    #[test]
    fn only_ls_remote_needs_the_network_and_only_state_changes_modify() {
        let network: Vec<_> = GIT_POLICY
            .iter()
            .filter(|p| p.grant == ToolGrant::Network)
            .map(|p| p.subcommand)
            .collect();
        assert_eq!(network, ["ls-remote"]);
        assert!(GIT_POLICY.iter().all(|p| p.grant != ToolGrant::Modify));
        assert!(DOCKER_POLICY.iter().all(|p| p.grant != ToolGrant::Network));
        for name in [
            "compose up",
            "compose down",
            "compose restart",
            "start",
            "stop",
            "restart",
        ] {
            assert_eq!(
                subcommand(ToolExecutable::Docker, name).map(|p| p.grant),
                Some(ToolGrant::Modify),
                "{name}"
            );
        }
    }
}
