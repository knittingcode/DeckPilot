//! Shared-secret token proving that a client runs as the agent's user.

use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::Path;

const TOKEN_BYTES: usize = 32;

/// Generates a new random token and writes it to `path`, replacing any
/// previous token. On Unix the file is created with mode `0600`.
pub fn create(path: &Path) -> io::Result<String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
    let token = bytes
        .iter()
        .fold(String::with_capacity(TOKEN_BYTES * 2), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        });

    // Written next to the target and renamed over it, so readers never see
    // a partial token. Windows refuses to replace a file that is still
    // being closed or deleted (e.g. by the previous agent, or a scanner),
    // so the rename is retried briefly.
    let temporary = path.with_extension(format!("{}.tmp", &token[..16]));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    {
        let mut file = options.open(&temporary)?;
        file.write_all(token.as_bytes())?;
        file.sync_all()?;
    }
    let mut attempt = 0;
    loop {
        match std::fs::rename(&temporary, path) {
            Ok(()) => break,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied && attempt < 20 => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(error) => {
                let _ = std::fs::remove_file(&temporary);
                return Err(error);
            }
        }
    }
    Ok(token)
}

pub fn read(path: &Path) -> io::Result<String> {
    Ok(std::fs::read_to_string(path)?.trim().to_owned())
}

/// Compares tokens in constant time with respect to their content.
pub fn matches(expected: &str, provided: &str) -> bool {
    let (a, b) = (expected.as_bytes(), provided.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_private_and_comparable() {
        let dir = std::env::temp_dir().join(format!("deckpilot-token-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agent.token");
        let first = create(&path).unwrap();
        let second = create(&path).unwrap();
        assert_eq!(first.len(), 64);
        assert_ne!(first, second);
        assert_eq!(read(&path).unwrap(), second);
        assert!(matches(&second, &second));
        assert!(!matches(&second, &first));
        assert!(!matches(&second, ""));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
