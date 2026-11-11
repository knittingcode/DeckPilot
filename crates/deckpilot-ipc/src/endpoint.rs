//! The agent's local socket.
//!
//! * **Unix**: a filesystem socket (never an abstract socket, which would
//!   ignore file permissions) inside a per-user directory with mode `0700`,
//!   which the OS checks on `connect`. Where supported (Linux), the socket
//!   file additionally gets mode `0600`. The directory lives in
//!   `$XDG_RUNTIME_DIR` when available, otherwise in the per-user temporary
//!   directory. The directory's owner and permissions are verified.
//! * **Windows**: a named pipe whose security descriptor grants access only
//!   to the pipe's owner and the local system, with remote clients rejected.
//!
//! The name is derived from the data directory, so isolated development
//! instances (`DECKPILOT_DATA_DIR`) get their own endpoint.

use std::io;
use std::path::Path;

use interprocess::local_socket::tokio::{Listener, Stream, prelude::*};
use interprocess::local_socket::{ListenerOptions, Name};

/// Attempts to create the Windows pipe while a previous instance closes.
#[cfg(windows)]
const LISTEN_RETRIES: u32 = 40;
#[cfg(windows)]
const LISTEN_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(50);

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
                path: unix::runtime_dir().join(format!("agent-{id}.sock")),
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

    pub async fn connect(&self) -> io::Result<Stream> {
        Stream::connect(self.name()?).await
    }
}

#[cfg(unix)]
mod unix {
    use std::io;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};

    fn uid() -> u32 {
        rustix::process::geteuid().as_raw()
    }

    pub(super) fn runtime_dir() -> PathBuf {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute() && dir.is_dir())
            .unwrap_or_else(std::env::temp_dir);
        base.join(format!("deckpilot-{}", uid()))
    }

    /// Creates the socket directory with mode 0700 and refuses to use it if
    /// another user owns it or it is accessible to others.
    pub(super) fn ensure_private_dir(dir: &Path) -> io::Result<()> {
        crate::paths::create_private_dir(dir)?;
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
        if metadata.permissions().mode() & 0o077 != 0 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
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
        // sun_path is 104 bytes on macOS and 108 on Linux.
        let endpoint = Endpoint::for_data_dir(Path::new("/a/very/long/data/directory/path"));
        assert!(endpoint.display().len() < 104, "{}", endpoint.display());
    }
}
