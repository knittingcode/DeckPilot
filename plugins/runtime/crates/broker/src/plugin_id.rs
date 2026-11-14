//! Plugin identifiers (`publisher.name`, lowercase, dot-separated).

use std::fmt;

/// A validated plugin id: 2 to 4 dot-separated segments of `[a-z0-9-]`,
/// each starting with a letter and not ending with `-`; at most 64 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PluginId(String);

impl PluginId {
    pub fn new(id: &str) -> Result<Self, InvalidPluginId> {
        let segments: Vec<&str> = id.split('.').collect();
        let segment_ok = |s: &&str| {
            let b = s.as_bytes();
            !b.is_empty()
                && b[0].is_ascii_lowercase()
                && b[b.len() - 1] != b'-'
                && b.iter()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
        };
        if id.len() <= 64 && (2..=4).contains(&segments.len()) && segments.iter().all(segment_ok) {
            Ok(Self(id.to_owned()))
        } else {
            Err(InvalidPluginId)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidPluginId;

impl fmt::Display for InvalidPluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid plugin id")
    }
}

impl std::error::Error for InvalidPluginId {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        for ok in ["acme.weather", "a1.b-2", "knittingcode.jira.cloud"] {
            PluginId::new(ok).unwrap();
        }
        for bad in [
            "",
            "acme",
            ".acme.x",
            "acme..x",
            "Acme.x",
            "acme.x-",
            "1acme.x",
            "acme.x/y",
            "a.b.c.d.e",
            "acme.wéather",
            "acme.x ",
        ] {
            assert!(PluginId::new(bad).is_err(), "{bad:?}");
        }
        assert!(PluginId::new(&format!("a.{}", "b".repeat(63))).is_err());
    }
}
