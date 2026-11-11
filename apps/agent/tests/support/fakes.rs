//! Fake programs: small scripts that stand in for `git`, `docker` and the
//! like, configured through module settings (an executable path), so no
//! real tool runs. Each records its arguments in `<name>.log`.

use std::path::{Path, PathBuf};

/// Writes an executable `name` into `dir`: a POSIX shell script on Unix, a
/// batch file `name.cmd` on Windows. Returns its path. Scripts should use
/// shell built-ins only: modules run programs with a minimal environment.
pub fn program(dir: &Path, name: &str, unix: &str, windows: &str) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = windows;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{unix}")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
    #[cfg(windows)]
    {
        let _ = unix;
        let path = dir.join(format!("{name}.cmd"));
        std::fs::write(
            &path,
            format!("@echo off\r\n{}", windows.replace('\n', "\r\n")),
        )
        .unwrap();
        path
    }
}

/// The argument lists a fake written with [`log_arguments`] was called
/// with, oldest first.
pub fn calls(log: &Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .split("@@end@@\n")
        .filter(|call| !call.is_empty())
        .map(|call| call.lines().map(str::to_owned).collect())
        .collect()
}

/// Shell lines that append the arguments to `log` (see [`calls`]).
pub fn log_arguments(log: &Path) -> String {
    format!(
        "printf '%s\\n' \"$@\" >> '{log}'\necho '@@end@@' >> '{log}'\n",
        log = log.display()
    )
}

/// A fake `git` (Unix): `status --porcelain=v2 -z` reports branch `branch`
/// with one modified file, `src/app.rs`.
pub fn git(dir: &Path, branch: &str) -> PathBuf {
    let log = dir.join("git.log");
    let script = format!(
        r#"{log}case " $* " in
  *" status "*)
    printf '%s\000' '# branch.oid 1234567890abcdef1234567890abcdef12345678' '# branch.head {branch}' \
      '# branch.upstream origin/{branch}' '# branch.ab +1 -0' \
      '1 .M N... 100644 100644 100644 aaaa bbbb src/app.rs' ;;
  *) echo "fake git: unsupported $*" >&2; exit 1 ;;
esac
"#,
        log = log_arguments(&log)
    );
    program(
        dir,
        "git",
        &script,
        "echo fake git is Unix only 1>&2\r\nexit /b 1",
    )
}

/// A fake `docker` CLI that answers like a running engine with Compose:
/// version, contexts, `compose ps` with one service, `up` and `down`.
pub fn docker(dir: &Path) -> PathBuf {
    let log = dir.join("docker.log");
    let unix = format!(
        r#"{log}[ "$1" = "--context" ] && shift 2
case "$1" in
  context) echo default ;;
  version) echo '{{"Client":{{"Version":"29.1.0","Os":"linux"}},"Server":{{"Version":"29.1.0","Os":"linux"}}}}' ;;
  compose)
    case " $* " in
      *" version --short "*) echo 2.30.0 ;;
      *" ps "*) echo '{{"Name":"shop-web-1","Service":"web","State":"running","Status":"Up"}}' ;;
      *" up "*) echo 'Container shop-web-1 Started' >&2 ;;
      *" down "*) echo 'Container shop-web-1 Removed' >&2 ;;
    esac ;;
esac
"#,
        log = log_arguments(&log)
    );
    let windows = format!(
        r#"echo %* >> "{log}"
echo @@end@@ >> "{log}"
if "%1"=="--context" shift & shift
if "%1"=="context" (echo default& exit /b 0)
if "%1"=="version" (echo {{"Client":{{"Version":"29.1.0","Os":"windows"}},"Server":{{"Version":"29.1.0","Os":"linux"}}}}& exit /b 0)
exit /b 0
"#,
        log = log.display()
    );
    program(dir, "docker", &unix, &windows)
}
