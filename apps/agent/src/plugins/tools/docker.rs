//! `docker` for `tool.run` (X3b, X3c): what the built-in docker module
//! runs (`modules/docker/src/cli.rs`, `actions.rs`), and nothing else.
//!
//! | Subcommand | Allowed | Grant |
//! | --- | --- | --- |
//! | `context show`, `compose version --short` | – | read |
//! | `version` | `--format={{json .}}` | read |
//! | `ps` | `--no-trunc`, `--all`, `--format={{json .}}`, `--filter=name=<name>` | read |
//! | `compose ps` | `--format=json\|{{json .}}`, `--all` | read |
//! | `compose logs` | `--tail=1..1000`, `--since`, `--no-color`, `-- <service>…` | read |
//! | `logs` | `--tail`, `--since`, `-- <container>` | read |
//! | `compose up` | `--detach` (required), `--remove-orphans`, `-- <service>…` | modify |
//! | `compose down` | `--remove-orphans` (never `-v`, `--rmi`) | modify |
//! | `compose restart` | `-- <service>…` | modify |
//! | `start`, `stop`, `restart` | `-- <container>` | modify |
//!
//! The host adds `--context <name>` (a Docker environment handle's, else
//! the host-held setting, else none) and, for compose, `compose
//! --project-directory <dir> -f <file> --project-name=<name>`, all three
//! from the binding and never from the plugin:
//!
//! * the file is the bound file, or for a bound folder the first of
//!   `compose.yaml`, `compose.yml`, `docker-compose.yaml` and
//!   `docker-compose.yml` in it, whose real path (links followed) is
//!   inside the folder; no file there is a refusal. Compose never searches
//!   the folders above, and an explicit `-f` overrides a `COMPOSE_FILE` in
//!   the project's `.env`;
//! * the project name is the file's own top-level `name:` when it is a
//!   literal project name, else compose's default, the folder's name
//!   normalized (lower case, `[a-z0-9_-]` only). It overrides a
//!   `COMPOSE_PROJECT_NAME` in `.env`, so a run can never reach another
//!   project's containers.
//!
//! The project's `.env` inside the binding still sets the interpolated
//! variables and the other `COMPOSE_*` settings (profiles, for example);
//! that is the user's file, as for the built-in. The environment adds
//! `DOCKER_HOST`, `DOCKER_CONTEXT`, `DOCKER_CONFIG`, `DOCKER_CERT_PATH`,
//! `DOCKER_TLS_VERIFY` and `XDG_RUNTIME_DIR` from the agent's own, and
//! `SSH_AUTH_SOCK` only when the context in use is an `ssh://` one.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::{
    Checked, DOCKER_READ_TIMEOUT, Denial, MAX_LOG_OUTPUT, MAX_STDOUT, Opt, Plan, Slot, Spec,
    Target, ToolHost, codes,
};

const fn spec(
    subcommand: &'static str,
    options: &'static [Opt],
    slots: &'static [Slot],
    (min, max): (usize, usize),
) -> Spec {
    Spec {
        subcommand,
        options,
        required: &[],
        slots,
        min,
        max,
        separator: max > 0,
    }
}

/// Most services or containers of one run.
const MAX_NAMES: usize = 16;

pub(super) const SPECS: &[Spec] = &[
    spec("context show", &[], &[], (0, 0)),
    spec(
        "version",
        &[Opt::Value("--format", Slot::JsonTemplate)],
        &[],
        (0, 0),
    ),
    spec("compose version", &[Opt::Flag("--short")], &[], (0, 0)),
    spec(
        "ps",
        &[
            Opt::Flag("--no-trunc"),
            Opt::Flag("--all"),
            Opt::Value("--format", Slot::JsonTemplate),
            Opt::Value("--filter", Slot::NameFilter),
        ],
        &[],
        (0, 0),
    ),
    spec(
        "compose ps",
        &[
            Opt::Value("--format", Slot::ComposeFormat),
            Opt::Flag("--all"),
        ],
        &[],
        (0, 0),
    ),
    spec(
        "compose logs",
        &[
            Opt::Value("--tail", Slot::Tail),
            Opt::Value("--since", Slot::Since),
            Opt::Flag("--no-color"),
        ],
        &[Slot::Name],
        (0, MAX_NAMES),
    ),
    spec(
        "logs",
        &[
            Opt::Value("--tail", Slot::Tail),
            Opt::Value("--since", Slot::Since),
        ],
        &[Slot::Name],
        (1, 1),
    ),
    Spec {
        required: &["--detach"],
        ..spec(
            "compose up",
            &[Opt::Flag("--detach"), Opt::Flag("--remove-orphans")],
            &[Slot::Name],
            (0, MAX_NAMES),
        )
    },
    spec(
        "compose down",
        &[Opt::Flag("--remove-orphans")],
        &[],
        (0, 0),
    ),
    spec("compose restart", &[], &[Slot::Name], (0, MAX_NAMES)),
    spec("start", &[], &[Slot::Name], (1, 1)),
    spec("stop", &[], &[Slot::Name], (1, 1)),
    spec("restart", &[], &[Slot::Name], (1, 1)),
];

/// Whether `subcommand` works on the bound compose project.
pub(super) fn is_project(subcommand: &str) -> bool {
    subcommand.starts_with("compose ") && subcommand != "compose version"
}

/// Variables docker keeps from the agent's environment besides the base:
/// which engine and context, its configuration and TLS material, the
/// rootless engine's socket directory.
const ENVIRONMENT: &[&str] = &[
    "DOCKER_HOST",
    "DOCKER_CONTEXT",
    "DOCKER_CONFIG",
    "DOCKER_CERT_PATH",
    "DOCKER_TLS_VERIFY",
    "XDG_RUNTIME_DIR",
];

pub(super) fn plan(
    host: &ToolHost,
    checked: &Checked,
    target: &Target,
    program: PathBuf,
) -> Result<Plan, Denial> {
    let subcommand = checked.spec.subcommand;
    let context = match target {
        Target::DockerEnvironment(name) => Some(name.clone()),
        _ => host.docker_context.clone(),
    };
    if context.as_deref().is_some_and(|c| !valid_name(c)) {
        return Err(Denial::new(
            codes::INVALID_CWD,
            "the Docker context is not a valid context name",
        ));
    }
    let mut args: Vec<OsString> = Vec::new();
    if let Some(context) = &context {
        args.extend(["--context".into(), context.into()]);
    }
    let words: Vec<&str> = subcommand.split(' ').collect();
    let cwd = if is_project(subcommand) {
        let project = compose_project(target)?;
        args.extend([
            "compose".into(),
            "--project-directory".into(),
            project.dir.as_os_str().to_owned(),
            "-f".into(),
            project.file.into_os_string(),
            format!("--project-name={}", project.name).into(),
        ]);
        args.extend(words[1..].iter().map(OsString::from));
        project.dir
    } else {
        args.extend(words.iter().map(OsString::from));
        match target {
            Target::Directory(dir) => dir.clone(),
            Target::File(file) => file
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| Denial::new(codes::INVALID_CWD, "the file has no folder"))?,
            Target::DockerEnvironment(_) => std::env::temp_dir(),
        }
    };
    for (name, value) in &checked.options {
        match value {
            Some(value) => args.push(format!("{name}={value}").into()),
            None => args.push((*name).into()),
        }
    }
    if checked.spec.separator && !checked.positionals.is_empty() {
        args.push("--".into());
    }
    args.extend(checked.positionals.iter().map(OsString::from));
    let mut env = host.environment(ENVIRONMENT, &program);
    if uses_ssh(host, context.as_deref())
        && let Some(sock) = host.var("SSH_AUTH_SOCK")
    {
        env.push(("SSH_AUTH_SOCK".into(), sock));
    }
    Ok(Plan {
        program,
        args,
        env,
        cwd,
        timeout: DOCKER_READ_TIMEOUT,
        max_stdout: if words.last() == Some(&"logs") {
            MAX_LOG_OUTPUT
        } else {
            MAX_STDOUT
        },
        remote: None,
    })
}

/// The compose project of a binding: its folder, its file and its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ComposeProject {
    pub dir: PathBuf,
    pub file: PathBuf,
    pub name: String,
}

/// The compose files compose looks for in a folder, in its order.
const COMPOSE_FILES: &[&str] = &[
    "compose.yaml",
    "compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
];

/// The project of `target`: a bound file as it is (the handle resolved
/// its real path), or the first compose file inside a bound folder whose
/// real path stays inside it.
pub(super) fn compose_project(target: &Target) -> Result<ComposeProject, Denial> {
    let (dir, file) = match target {
        Target::File(file) => {
            let dir = file
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| Denial::new(codes::INVALID_CWD, "the compose file has no folder"))?;
            (dir, file.clone())
        }
        Target::Directory(dir) => {
            let dir = std::fs::canonicalize(dir).map_err(|_| {
                Denial::new(codes::INVALID_CWD, "the compose folder does not exist")
            })?;
            let mut found = None;
            for name in COMPOSE_FILES {
                let candidate = dir.join(name);
                if std::fs::symlink_metadata(&candidate).is_err() {
                    continue;
                }
                let real = std::fs::canonicalize(&candidate)
                    .ok()
                    .filter(|real| real.starts_with(&dir) && real.is_file())
                    .ok_or_else(|| {
                        Denial::new(
                            codes::INVALID_CWD,
                            format!("`{name}` in the compose folder leads outside it"),
                        )
                    })?;
                found = Some(real);
                break;
            }
            let file = found.ok_or_else(|| {
                Denial::new(
                    codes::INVALID_CWD,
                    "the compose folder has no compose.yaml, compose.yml, docker-compose.yaml or \
                     docker-compose.yml",
                )
            })?;
            (dir, file)
        }
        Target::DockerEnvironment(_) => {
            return Err(Denial::new(
                codes::INVALID_CWD,
                "a compose command runs only in a compose file or folder handle",
            ));
        }
    };
    let name = declared_name(&file)
        .or_else(|| {
            dir.file_name()
                .map(|n| normalized_project(&n.to_string_lossy()))
                .filter(|n| valid_project(n))
        })
        .ok_or_else(|| {
            Denial::new(
                codes::INVALID_CWD,
                "no compose project name follows from the bound folder's name",
            )
        })?;
    Ok(ComposeProject { dir, file, name })
}

/// The top-level `name:` of a compose file, when it is a literal project
/// name (not interpolated).
fn declared_name(file: &Path) -> Option<String> {
    let meta = std::fs::metadata(file).ok()?;
    if meta.len() > 1024 * 1024 {
        return None;
    }
    let text = std::fs::read_to_string(file).ok()?;
    text.lines().find_map(|line| {
        let value = line.strip_prefix("name:")?;
        let value = value.split(" #").next().unwrap_or_default().trim();
        let value = value.trim_matches(|c| c == '"' || c == '\'');
        valid_project(value).then(|| value.to_owned())
    })
}

/// Compose's default project name of a folder: lower case, only
/// `[a-z0-9_-]`, no leading `-` or `_`.
pub(super) fn normalized_project(folder: &str) -> String {
    folder
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_'))
        .collect::<String>()
        .trim_start_matches(['-', '_'])
        .to_owned()
}

/// Whether the engine docker talks to is reached over SSH: `DOCKER_HOST`
/// for the default context, the context's endpoint otherwise.
fn uses_ssh(host: &ToolHost, context: Option<&str>) -> bool {
    let ssh_host = || {
        host.var("DOCKER_HOST")
            .is_some_and(|h| h.to_string_lossy().starts_with("ssh://"))
    };
    let config = host.var("DOCKER_CONFIG").map(PathBuf::from).or_else(|| {
        host.var("HOME")
            .or_else(|| host.var("USERPROFILE"))
            .map(|home| PathBuf::from(home).join(".docker"))
    });
    let name = match context {
        Some(name) => Some(name.to_owned()),
        None if host.var("DOCKER_HOST").is_some() => return ssh_host(),
        None => host
            .var("DOCKER_CONTEXT")
            .map(|c| c.to_string_lossy().into_owned())
            .or_else(|| {
                let text = small_file(&config.as_ref()?.join("config.json"))?;
                let value: serde_json::Value = serde_json::from_str(&text).ok()?;
                value["currentContext"].as_str().map(str::to_owned)
            }),
    };
    match name.filter(|n| n != "default") {
        None => ssh_host(),
        Some(name) => {
            use sha2::Digest as _;
            let digest = sha2::Sha256::digest(name.as_bytes());
            let id = digest.iter().fold(String::new(), |mut id, b| {
                use std::fmt::Write as _;
                let _ = write!(id, "{b:02x}");
                id
            });
            config
                .and_then(|dir| small_file(&dir.join("contexts/meta").join(id).join("meta.json")))
                .is_some_and(|meta| meta.contains("\"ssh://"))
        }
    }
}

/// A configuration file of at most 64 KiB.
fn small_file(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    (meta.is_file() && meta.len() <= 64 * 1024)
        .then(|| std::fs::read_to_string(path).ok())
        .flatten()
}

/// A name Docker accepts for a context, container or service, which can
/// never be taken for an option (`modules/docker/src/cli.rs`).
pub(super) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '+' | '/' | ':'))
}

/// A compose project name: lower case letters, digits, `-` and `_`.
pub(super) fn valid_project(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// A `--since` value: a duration such as `10m` or `2h`, or a timestamp.
pub(super) fn valid_since(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 40
        && value.starts_with(|c: char| c.is_ascii_digit())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '.' | '+'))
}
