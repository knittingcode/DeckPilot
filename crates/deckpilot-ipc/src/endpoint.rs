//! The agent's local socket.
//!
//! * **Unix**: a filesystem socket (never an abstract socket, which would
//!   ignore file permissions) inside a per-user directory with mode `0700`,
//!   which the OS checks on `connect`. Where supported (Linux), the socket
//!   file additionally gets mode `0600`. The directory lives in
//!   `$XDG_RUNTIME_DIR` when available, otherwise in the per-user temporary
//!   directory. When the socket path there would not fit in `sun_path`
//!   (104 bytes on macOS and the BSDs, 108 on Linux; a long `TMPDIR`), the
//!   directory is `/tmp/deckpilot-<uid>` instead. Both the agent and its
//!   clients verify that the directory is a real directory (not a symlink)
//!   owned by the current user and inaccessible to others, and refuse it
//!   otherwise, so a client never sends its token to a socket in a directory
//!   another user could have created.
//! * **Windows**: a named pipe whose security descriptor grants access only
//!   to the pipe's owner and the local system, with remote clients rejected.
//!
//! The name is derived from the data directory, so isolated development
//! instances (`DECKPILOT_DATA_DIR`) get their own endpoint.

use std::io;
use std::path::Path;

use interprocess::ConnectWaitMode;
use interprocess::local_socket::tokio::{Listener, Stream, prelude::*};
use interprocess::local_socket::{ConnectOptions, ListenerOptions, Name};

/// Attempts to create the Windows pipe while a previous instance closes:
/// up to 5 s. Under load (CI runners, a busy computer after an update) the
/// previous agent's instance can take more than 2 s to go away.
#[cfg(windows)]
const LISTEN_RETRIES: u32 = 100;
/// Between attempts to open a pipe whose instances are all busy.
#[cfg(windows)]
const CONNECT_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(20);
#[cfg(windows)]
const LISTEN_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(50);
/// How long a connect waits for a busy endpoint. On Windows a pipe whose
/// instances are all taken (or left behind by a previous agent) would
/// otherwise be waited for indefinitely.
const CONNECT_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    #[cfg(unix)]
    path: std::path::PathBuf,
    #[cfg(windows)]
    pipe: String,
}

/// FNV-1a: a small, stable hash for deriving endpoint names.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

impl Endpoint {
    pub fn for_data_dir(data_dir: &Path) -> Self {
        let id = format!("{:016x}", fnv1a(data_dir.as_os_str().as_encoded_bytes()));
        #[cfg(unix)]
        {
            Self {
                path: unix::socket_path(&format!("agent-{id}.sock")),
            }
        }
        #[cfg(windows)]
        {
            Self {
                pipe: format!("deckpilot-agent-{id}"),
            }
        }
    }

    /// Human-readable location for diagnostics.
    pub fn display(&self) -> String {
        #[cfg(unix)]
        {
            self.path.display().to_string()
        }
        #[cfg(windows)]
        {
            format!(r"\\.\pipe\{}", self.pipe)
        }
    }

    fn name(&self) -> io::Result<Name<'_>> {
        #[cfg(unix)]
        {
            use interprocess::local_socket::GenericFilePath;
            self.path.as_path().to_fs_name::<GenericFilePath>()
        }
        #[cfg(windows)]
        {
            use interprocess::local_socket::GenericNamespaced;
            self.pipe.as_str().to_ns_name::<GenericNamespaced>()
        }
    }

    /// Creates the listening socket. Must only be called while holding the
    /// agent's instance lock, because a stale socket file is replaced.
    pub fn listen(&self) -> io::Result<Listener> {
        #[cfg(unix)]
        {
            use interprocess::os::unix::local_socket::ListenerOptionsExt;
            unix::ensure_private_dir(self.path.parent().unwrap_or(Path::new("/")))?;
            let options = || {
                Ok::<_, io::Error>(
                    ListenerOptions::new()
                        .name(self.name()?)
                        .try_overwrite(true),
                )
            };
            // Mode 0600 on the socket where the OS supports setting it before
            // `bind` (Linux); elsewhere the 0700 directory, which is checked
            // on `connect`, restricts access.
            match options()?.mode(0o600).create_tokio() {
                Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                    options()?.create_tokio()
                }
                result => result,
            }
        }
        #[cfg(windows)]
        {
            use interprocess::os::windows::local_socket::ListenerOptionsExt;
            // The pipe is created as the first instance of its name, which
            // fails while an instance of a previous agent still exists. When
            // a listener is dropped during `accept`, the pending connect is
            // cancelled asynchronously and its instance closes a moment
            // later, so an agent restarted right away retries briefly. A name
            // that stays taken still fails.
            let mut attempt = 0;
            loop {
                let result = ListenerOptions::new()
                    .name(self.name()?)
                    .security_descriptor(windows::owner_only()?)
                    .create_tokio();
                match result {
                    Err(error)
                        if error.kind() == io::ErrorKind::PermissionDenied
                            && attempt < LISTEN_RETRIES =>
                    {
                        attempt += 1;
                        std::thread::sleep(LISTEN_RETRY_DELAY);
                    }
                    other => return other,
                }
            }
        }
    }

    /// Connects, waiting at most [`CONNECT_WAIT`] for a busy endpoint. A
    /// timeout is reported as [`io::ErrorKind::TimedOut`]; no agent
    /// listening as [`io::ErrorKind::NotFound`] (or `ConnectionRefused`).
    pub async fn connect(&self) -> io::Result<Stream> {
        #[cfg(unix)]
        {
            // The token is sent right after connecting, so the socket must
            // be in a directory only this user controls. A missing directory
            // means no agent is running (`NotFound`).
            unix::verify_private_dir(self.path.parent().unwrap_or(Path::new("/")))?;
            let options = ConnectOptions::new()
                .name(self.name()?)
                .wait_mode(ConnectWaitMode::Timeout(CONNECT_WAIT));
            match tokio::time::timeout(CONNECT_WAIT * 2, options.connect_tokio()).await {
                Ok(Ok(stream)) => Ok(stream),
                Ok(Err(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                    Err(io::Error::new(io::ErrorKind::TimedOut, error))
                }
                Ok(Err(error)) => Err(error),
                Err(_) => Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the agent endpoint did not accept the connection",
                )),
            }
        }
        #[cfg(windows)]
        {
            self.connect_pipe().await
        }
    }

    /// Windows: tries to open the pipe without waiting, and retries on the
    /// runtime's timer while all its instances are busy.
    ///
    /// The crate's own wait (`ConnectWaitMode::Timeout`) runs
    /// `WaitNamedPipeW` on Tokio's blocking pool. A runtime that shuts down
    /// waits for its blocking tasks without a limit, and a wait on the pipe
    /// of an agent that is going away kept the runtime (the app's, a test's)
    /// from shutting down. With a zero wait the crate only calls
    /// `CreateFileW`, which does not block: it opens an instance, or fails
    /// at once with "not found" (no agent) or "busy".
    #[cfg(windows)]
    async fn connect_pipe(&self) -> io::Result<Stream> {
        let deadline = tokio::time::Instant::now() + CONNECT_WAIT;
        loop {
            let options = ConnectOptions::new()
                .name(self.name()?)
                .wait_mode(ConnectWaitMode::Timeout(std::time::Duration::ZERO));
            match options.connect_tokio().await {
                Ok(stream) => return Ok(stream),
                // All instances busy (the crate reports it as a timeout).
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) =>
                {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "the agent endpoint did not accept the connection",
                        ));
                    }
                    tokio::time::sleep(CONNECT_RETRY_DELAY).await;
                }
                Err(error) => return Err(error),
            }
        }
    }
}

#[cfg(unix)]
mod unix {
    use std::io;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};

    /// Size of `sockaddr_un::sun_path`, including the terminating NUL.
    pub(super) const SUN_PATH_CAPACITY: usize =
        if cfg!(any(target_os = "linux", target_os = "android")) {
            108
        } else {
            104
        };

    /// Base of the socket directory when the preferred one gives a path that
    /// does not fit in `sun_path`. Short, present on every Unix, and sticky,
    /// so other users cannot remove or rename our directory in it.
    pub(super) const FALLBACK_BASE: &str = "/tmp";

    pub(super) fn uid() -> u32 {
        rustix::process::geteuid().as_raw()
    }

    /// The socket path for `file_name` in this process's environment.
    pub(super) fn socket_path(file_name: &str) -> PathBuf {
        let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute() && dir.is_dir());
        resolve_socket_path(runtime_dir, std::env::temp_dir(), uid(), file_name)
    }

    /// `<base>/deckpilot-<uid>/<file_name>`, where `base` is the runtime
    /// directory if given, otherwise the temporary directory, or
    /// [`FALLBACK_BASE`] when that path would not fit in `sun_path` (or is
    /// not absolute). Depends only on its arguments, so an agent and a
    /// client with the same environment agree.
    pub(super) fn resolve_socket_path(
        runtime_dir: Option<PathBuf>,
        temp_dir: PathBuf,
        uid: u32,
        file_name: &str,
    ) -> PathBuf {
        let dir_name = format!("deckpilot-{uid}");
        let preferred = runtime_dir
            .unwrap_or(temp_dir)
            .join(&dir_name)
            .join(file_name);
        if preferred.is_absolute() && fits_sun_path(&preferred) {
            preferred
        } else {
            Path::new(FALLBACK_BASE).join(dir_name).join(file_name)
        }
    }

    pub(super) fn fits_sun_path(path: &Path) -> bool {
        path.as_os_str().len() < SUN_PATH_CAPACITY
    }

    /// Creates the socket directory with mode 0700 if needed, then checks it
    /// with [`verify_private_dir`].
    pub(super) fn ensure_private_dir(dir: &Path) -> io::Result<()> {
        crate::paths::create_private_dir(dir)?;
        verify_private_dir(dir)
    }

    /// Refuses a socket directory that is a symlink, is owned by another
    /// user, or is accessible to group or others. It is not repaired: a
    /// directory others could write to may already hold their socket.
    pub(super) fn verify_private_dir(dir: &Path) -> io::Result<()> {
        let metadata = std::fs::symlink_metadata(dir)?;
        if !metadata.is_dir() || metadata.uid() != uid() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "{} is not a directory owned by the current user",
                    dir.display()
                ),
            ));
        }
        let mode = metadata.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "{} is accessible to other users (mode {mode:o}); remove it or \
                     restrict it to mode 700",
                    dir.display()
                ),
            ));
        }
        Ok(())
    }
}

#[cfg(windows)]
mod windows {
    use std::io;

    use interprocess::os::windows::security_descriptor::SecurityDescriptor;
    use widestring::U16CString;

    /// Protected DACL: full access for the owner (`OW`) and the local system
    /// (`SY`); everyone else, including other users, is denied.
    const SDDL: &str = "D:P(A;;GA;;;OW)(A;;GA;;;SY)";

    pub(super) fn owner_only() -> io::Result<SecurityDescriptor> {
        let sddl = U16CString::from_str(SDDL)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        SecurityDescriptor::deserialize(&sddl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_differ_per_data_directory() {
        let a = Endpoint::for_data_dir(Path::new("/data/a"));
        let b = Endpoint::for_data_dir(Path::new("/data/b"));
        assert_ne!(a, b);
        assert_eq!(a, Endpoint::for_data_dir(Path::new("/data/a")));
    }

    #[cfg(unix)]
    #[test]
    fn unix_socket_paths_fit_sun_path() {
        let endpoint = Endpoint::for_data_dir(Path::new("/a/very/long/data/directory/path"));
        assert!(
            endpoint.display().len() < unix::SUN_PATH_CAPACITY,
            "{}",
            endpoint.display()
        );
    }

    #[cfg(unix)]
    mod unix_paths {
        use std::os::unix::fs::PermissionsExt;
        use std::path::{Path, PathBuf};

        use super::super::unix::*;

        const NAME: &str = "agent-0123456789abcdef.sock";

        fn long_dir() -> PathBuf {
            PathBuf::from(format!("/private/tmp/{}", "x".repeat(150)))
        }

        fn scratch(tag: &str) -> PathBuf {
            let dir = std::env::temp_dir().join(format!(
                "deckpilot-ipc-endpoint-{tag}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        #[test]
        fn short_temporary_directory_is_used() {
            let path = resolve_socket_path(None, PathBuf::from("/var/tmp/short"), 501, NAME);
            assert_eq!(path, Path::new("/var/tmp/short/deckpilot-501").join(NAME));
        }

        #[test]
        fn runtime_directory_takes_precedence() {
            let path = resolve_socket_path(
                Some(PathBuf::from("/run/user/501")),
                PathBuf::from("/tmp"),
                501,
                NAME,
            );
            assert_eq!(path, Path::new("/run/user/501/deckpilot-501").join(NAME));
        }

        #[test]
        fn long_temporary_directory_falls_back_to_a_short_path() {
            let path = resolve_socket_path(None, long_dir(), 501, NAME);
            assert_eq!(path, Path::new("/tmp/deckpilot-501").join(NAME));
            assert!(fits_sun_path(&path));
        }

        #[test]
        fn long_runtime_directory_falls_back_to_a_short_path() {
            let path = resolve_socket_path(Some(long_dir()), PathBuf::from("/tmp"), 501, NAME);
            assert_eq!(path, Path::new("/tmp/deckpilot-501").join(NAME));
        }

        #[test]
        fn relative_temporary_directory_falls_back() {
            let path = resolve_socket_path(None, PathBuf::from("tmp"), 501, NAME);
            assert_eq!(path, Path::new("/tmp/deckpilot-501").join(NAME));
        }

        #[test]
        fn path_at_the_limit_falls_back() {
            // A path of exactly SUN_PATH_CAPACITY bytes leaves no room for
            // the terminating NUL; one byte shorter fits.
            let suffix = format!("/deckpilot-501/{NAME}").len();
            let base = format!("/{}", "b".repeat(SUN_PATH_CAPACITY - suffix - 1));
            let path = resolve_socket_path(None, PathBuf::from(&base), 501, NAME);
            assert_eq!(path, Path::new("/tmp/deckpilot-501").join(NAME));
            let base = format!("/{}", "b".repeat(SUN_PATH_CAPACITY - suffix - 2));
            let path = resolve_socket_path(None, PathBuf::from(&base), 501, NAME);
            assert!(path.starts_with(&base));
            assert_eq!(path.as_os_str().len(), SUN_PATH_CAPACITY - 1);
        }

        #[test]
        fn private_directory_is_created_and_accepted() {
            let root = scratch("create");
            let dir = root.join("deckpilot-test");
            ensure_private_dir(&dir).unwrap();
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
            ensure_private_dir(&dir).unwrap();
            std::fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn directory_accessible_to_others_is_refused() {
            let root = scratch("mode");
            for mode in [0o755, 0o770, 0o707, 0o777] {
                let dir = root.join(format!("dir-{mode:o}"));
                std::fs::create_dir(&dir).unwrap();
                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode)).unwrap();
                let error = ensure_private_dir(&dir).unwrap_err();
                assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
                // Refused, not repaired.
                let after = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
                assert_eq!(after, mode);
                assert!(verify_private_dir(&dir).is_err());
            }
            std::fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn symlinked_directory_is_refused() {
            let root = scratch("symlink");
            let target = root.join("target");
            ensure_private_dir(&target).unwrap();
            let link = root.join("link");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            let error = ensure_private_dir(&link).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
            assert!(verify_private_dir(&link).is_err());
            std::fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn directory_of_another_user_is_refused() {
            if uid() == 0 {
                return; // Root owns `/`.
            }
            let error = verify_private_dir(Path::new("/")).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        }

        #[test]
        fn missing_directory_is_not_found() {
            let root = scratch("missing");
            let error = verify_private_dir(&root.join("absent")).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
