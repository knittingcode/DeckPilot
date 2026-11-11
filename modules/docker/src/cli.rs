//! The Docker command lines the actions run, how Docker's errors are
//! classified, and parsing its JSON output. Pure, so it is tested on every
//! platform without Docker.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::codes;

/// Where Compose finds the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeTarget {
    /// `--project-directory`.
    pub directory: PathBuf,
    /// `-f`, when the resource names a file rather than a folder.
    pub file: Option<PathBuf>,
    /// `-p`, when set.
    pub project: Option<String>,
}

/// The arguments for `docker --context <context> …`. The context is always
/// named, so it is visible which Docker a command talks to.
pub fn base(context: &str) -> Vec<String> {
    vec!["--context".into(), context.into()]
}

/// `docker --context <c> compose --project-directory <d> [-f <f>] [-p <p>]
/// <command> <args>`.
pub fn compose(context: &str, target: &ComposeTarget, command: &[&str]) -> Vec<String> {
    let mut args = base(context);
    args.push("compose".into());
    args.push("--project-directory".into());
    args.push(target.directory.display().to_string());
    if let Some(file) = &target.file {
        args.push("-f".into());
        args.push(file.display().to_string());
    }
    if let Some(project) = &target.project {
        args.push("-p".into());
        args.push(project.clone());
    }
    args.extend(command.iter().map(|a| (*a).to_owned()));
    args
}

/// The compose command of a state-changing action, without destructive
/// flags: `down` never removes volumes or images.
pub fn compose_change(kind: Change, services: &[String], remove_orphans: bool) -> Vec<String> {
    let mut args: Vec<String> = match kind {
        Change::Up => vec!["up".into(), "--detach".into()],
        Change::Down => vec!["down".into()],
        Change::Restart => vec!["restart".into()],
    };
    if remove_orphans && kind != Change::Restart {
        args.push("--remove-orphans".into());
    }
    if kind != Change::Down && !services.is_empty() {
        args.push("--".into());
        args.extend(services.iter().cloned());
    }
    args
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Up,
    Down,
    Restart,
}

/// A name Docker accepts for a context, container, service or project,
/// which can never be taken for an option.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '+' | '/' | ':'))
}

/// A Compose project name: lower case letters, digits, `-` and `_`.
pub fn valid_project(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// A `--since` value: a duration such as `10m` or `2h`, or a timestamp.
pub fn valid_since(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 40
        && value.starts_with(|c: char| c.is_ascii_digit())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '.' | '+'))
}

/// Where a compose resource points: a folder with a compose file, or the
/// file itself.
pub fn compose_target(value: &Path, project: Option<String>) -> Option<ComposeTarget> {
    if value.is_dir() {
        Some(ComposeTarget {
            directory: value.to_path_buf(),
            file: None,
            project,
        })
    } else if value.is_file() {
        Some(ComposeTarget {
            directory: value.parent()?.to_path_buf(),
            file: Some(value.to_path_buf()),
            project,
        })
    } else {
        None
    }
}

/// A stable code for a failed Docker command, from its error output.
pub fn classify(stderr: &str) -> &'static str {
    let lower = stderr.to_lowercase();
    if lower.contains("permission denied") {
        codes::PERMISSION_DENIED
    } else if lower.contains("cannot connect to the docker daemon")
        || lower.contains("is the docker daemon running")
        || lower.contains("error during connect")
        || lower.contains("docker desktop is not running")
    {
        codes::DAEMON_UNAVAILABLE
    } else if lower.contains("context")
        && (lower.contains("not found") || lower.contains("does not exist"))
    {
        codes::CONTEXT_NOT_FOUND
    } else if lower.contains("'compose' is not a docker command")
        || lower.contains("unknown command") && lower.contains("compose")
    {
        codes::COMPOSE_UNAVAILABLE
    } else if lower.contains("no configuration file provided") {
        codes::COMPOSE_FILE_NOT_FOUND
    } else if lower.contains("no such container") || lower.contains("no such service") {
        codes::NOT_FOUND
    } else {
        codes::DOCKER_FAILED
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Versions {
    #[serde(rename = "Client")]
    pub client: Component,
    #[serde(rename = "Server", default)]
    pub server: Option<Component>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Component {
    #[serde(rename = "Version", default)]
    pub version: String,
    #[serde(rename = "Os", default)]
    pub os: String,
}

/// `docker version --format '{{json .}}'`. Docker prints the client part
/// even when the daemon cannot be reached.
pub fn parse_versions(stdout: &str) -> Option<Versions> {
    serde_json::from_str(stdout.trim()).ok()
}

/// A container as `docker ps` lists it; only what is safe to show.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Container {
    #[serde(rename = "Names", alias = "Name", default)]
    pub name: String,
    #[serde(rename = "Service", default)]
    pub service: String,
    #[serde(rename = "Image", default)]
    pub image: String,
    #[serde(rename = "State", default)]
    pub state: String,
    #[serde(rename = "Health", alias = "HealthStatus", default)]
    pub health: String,
    #[serde(rename = "Status", default)]
    pub status: String,
}

impl Container {
    pub fn running(&self) -> bool {
        self.state.eq_ignore_ascii_case("running")
    }

    /// `name  state  image  status`, tab separated.
    pub fn line(&self) -> String {
        let name = if self.service.is_empty() {
            self.name.clone()
        } else {
            format!("{} ({})", self.service, self.name)
        };
        format!("{name}\t{}\t{}\t{}", self.state, self.image, self.status)
    }
}

/// Containers from JSON lines (`docker ps`, newer Compose) or a JSON array
/// (older Compose v2).
pub fn parse_containers(stdout: &str) -> Result<Vec<Container>, serde_json::Error> {
    let text = stdout.trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    if text.starts_with('[') {
        return serde_json::from_str(text);
    }
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> ComposeTarget {
        ComposeTarget {
            directory: "/src/shop".into(),
            file: Some("/src/shop/compose.dev.yaml".into()),
            project: Some("shop".into()),
        }
    }

    #[test]
    fn commands_always_name_the_context() {
        assert_eq!(base("remote"), ["--context", "remote"]);
        let args = compose("default", &target(), &["ps", "--all"]);
        assert_eq!(
            args,
            [
                "--context",
                "default",
                "compose",
                "--project-directory",
                "/src/shop",
                "-f",
                "/src/shop/compose.dev.yaml",
                "-p",
                "shop",
                "ps",
                "--all"
            ]
        );
    }

    #[test]
    fn state_changes_never_remove_volumes_or_images() {
        let services = vec!["web".to_owned(), "db".to_owned()];
        for change in [Change::Up, Change::Down, Change::Restart] {
            for orphans in [false, true] {
                let args = compose_change(change, &services, orphans);
                for forbidden in ["-v", "--volumes", "--rmi", "--force", "-f", "--prune"] {
                    assert!(!args.iter().any(|a| a == forbidden), "{change:?}: {args:?}");
                }
            }
        }
        assert_eq!(
            compose_change(Change::Up, &services, false),
            ["up", "--detach", "--", "web", "db"]
        );
        assert_eq!(
            compose_change(Change::Down, &services, true),
            ["down", "--remove-orphans"]
        );
        assert_eq!(compose_change(Change::Restart, &[], true), ["restart"]);
    }

    #[test]
    fn names_cannot_be_options() {
        assert!(valid_name("desktop-linux") && valid_name("shop-web-1") && valid_name("tcp+ssh"));
        for bad in ["", "-H", "--privileged", "a b", "a;b", "$(x)"] {
            assert!(!valid_name(bad), "{bad}");
        }
        assert!(valid_project("shop_dev-1"));
        assert!(!valid_project("Shop") && !valid_project("-p") && !valid_project("a.b"));
        assert!(valid_since("10m") && valid_since("2026-01-02T10:00:00"));
        assert!(!valid_since("-1") && !valid_since("--follow"));
    }

    #[test]
    fn errors_are_classified() {
        for (stderr, code) in [
            (
                "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?",
                codes::DAEMON_UNAVAILABLE,
            ),
            (
                "permission denied while trying to connect to the Docker daemon socket at unix:///var/run/docker.sock",
                codes::PERMISSION_DENIED,
            ),
            (
                "context \"remote\" does not exist",
                codes::CONTEXT_NOT_FOUND,
            ),
            (
                "docker: 'compose' is not a docker command.",
                codes::COMPOSE_UNAVAILABLE,
            ),
            (
                "no configuration file provided: not found",
                codes::COMPOSE_FILE_NOT_FOUND,
            ),
            ("Error: No such container: web", codes::NOT_FOUND),
            ("something else", codes::DOCKER_FAILED),
        ] {
            assert_eq!(classify(stderr), code, "{stderr}");
        }
    }

    #[test]
    fn parses_versions_and_containers() {
        let versions = parse_versions(
            r#"{"Client":{"Version":"29.1.0","Os":"darwin","Context":"desktop-linux"},"Server":{"Version":"29.1.0","Os":"linux"}}"#,
        )
        .unwrap();
        assert_eq!(versions.client.version, "29.1.0");
        assert_eq!(versions.server.unwrap().os, "linux");
        let client_only =
            parse_versions(r#"{"Client":{"Version":"29.1.0"},"Server":null}"#).unwrap();
        assert!(client_only.server.is_none());

        let lines = concat!(
            r#"{"Names":"shop-web-1","Image":"nginx","State":"running","Status":"Up 2 hours","Labels":"secret=1"}"#,
            "\n",
            r#"{"Names":"shop-db-1","Image":"postgres","State":"exited","Status":"Exited (0)"}"#,
            "\n"
        );
        let containers = parse_containers(lines).unwrap();
        assert_eq!(containers.len(), 2);
        assert!(containers[0].running() && !containers[1].running());
        assert_eq!(
            containers[0].line(),
            "shop-web-1\trunning\tnginx\tUp 2 hours"
        );
        let array = parse_containers(
            r#"[{"Name":"shop-web-1","Service":"web","State":"running","Health":"healthy"}]"#,
        )
        .unwrap();
        assert_eq!(array[0].service, "web");
        assert!(array[0].line().starts_with("web (shop-web-1)"));
        assert_eq!(parse_containers("  ").unwrap(), []);
        assert!(parse_containers("not json").is_err());
    }
}
