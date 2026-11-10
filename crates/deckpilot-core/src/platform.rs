//! Operating-system identification.
//!
//! Platform-specific behaviour (launching applications, opening terminals,
//! talking to HID devices) is kept out of the domain model. Code that needs it
//! receives the relevant information through explicit parameters such as
//! [`Os`] or a resolved [`MachineResourceBinding`](crate::MachineResourceBinding).

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Os {
    Linux,
    MacOs,
    Windows,
    Other,
}

impl Os {
    /// Operating system this binary was compiled for.
    pub const fn current() -> Self {
        if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Other
        }
    }
}

impl fmt::Display for Os {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Linux => "Linux",
            Self::MacOs => "macOS",
            Self::Windows => "Windows",
            Self::Other => "Other",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_os_is_supported_target() {
        assert_ne!(Os::current(), Os::Other);
    }
}
