//! Plugin identity: `<publisher>.<name>` (architecture section 7, rule 1).
//!
//! #191 does not use reverse-DNS names. A plugin id is the publisher id and
//! the plugin name joined by one dot, both lowercase words separated by
//! single hyphens, each hyphen followed by a letter (`acme.github-prs`).
//! That makes the lowerCamel form used in variable paths
//! (`plugins.acme.githubPrs`) injective.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Longest publisher id (`$defs/publisherId`).
pub const MAX_PUBLISHER_LEN: usize = 24;
/// Longest plugin name inside the id (`$defs/pluginName`).
pub const MAX_PLUGIN_NAME_LEN: usize = 32;

/// Publisher ids no plugin may use: DeckPilot's own names and every
/// built-in module id (architecture section 7, rule 1).
///
/// The built-in module ids are mirrored here rather than read from
/// `deckpilot-modules`, so this crate stays free of the app's crates. Keep
/// the list in step with `deckpilot_modules::builtin`.
pub const RESERVED_PUBLISHERS: &[&str] = &[
    "deckpilot",
    "knittingcode",
    "core",
    "system",
    "plugin",
    "plugins",
    "builtin",
    "dev",
    "local",
    "test",
    "user",
    // Built-in modules.
    "general",
    "projects",
    "git",
    "docker",
    "http",
    "notifications",
    "github",
    "gitlab",
    "jira",
    "workspace",
    "work-session",
];

/// Why a plugin or publisher id was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    #[error("`{0}` is not <publisher>.<name>: it needs exactly one dot")]
    Dots(String),
    #[error(
        "`{0}` is not a valid publisher id: use 2-24 lowercase letters and digits, starting with \
         a letter, with single hyphens each followed by a letter"
    )]
    Publisher(String),
    #[error(
        "`{0}` is not a valid plugin name: use 2-32 lowercase letters and digits, starting with a \
         letter, with single hyphens each followed by a letter"
    )]
    Name(String),
}

/// `[a-z][a-z0-9]*(-[a-z][a-z0-9]*)*` with the given length range.
pub(crate) fn is_kebab_word(text: &str, min: usize, max: usize) -> bool {
    if text.len() < min || text.len() > max {
        return false;
    }
    text.split('-').all(|part| {
        let mut chars = part.chars();
        chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    })
}

/// Whether `text` is a valid publisher id (`$defs/publisherId`).
pub fn is_publisher_id(text: &str) -> bool {
    is_kebab_word(text, 2, MAX_PUBLISHER_LEN)
}

/// `kebab-case` to `lowerCamel`: `github-prs` becomes `githubPrs`.
pub fn lower_camel(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (index, part) in text.split('-').enumerate() {
        let mut chars = part.chars();
        if index > 0
            && let Some(first) = chars.next()
        {
            out.push(first.to_ascii_uppercase());
        }
        out.extend(chars);
    }
    out
}

/// A plugin id, `<publisher>.<name>` (`$defs/pluginId`).
///
/// Parsing checks the grammar only. Whether the publisher is reserved or
/// matches `publisher.id` is a manifest rule, checked by
/// [`Manifest::validate`](crate::Manifest::validate).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PluginId {
    text: String,
    dot: usize,
}

impl PluginId {
    pub fn parse(text: &str) -> Result<Self, IdError> {
        let mut parts = text.split('.');
        let (Some(publisher), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
            return Err(IdError::Dots(text.to_owned()));
        };
        if !is_publisher_id(publisher) {
            return Err(IdError::Publisher(text.to_owned()));
        }
        if !is_kebab_word(name, 2, MAX_PLUGIN_NAME_LEN) {
            return Err(IdError::Name(text.to_owned()));
        }
        Ok(Self {
            text: text.to_owned(),
            dot: publisher.len(),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The publisher part, before the dot.
    pub fn publisher(&self) -> &str {
        &self.text[..self.dot]
    }

    /// The plugin name part, after the dot.
    pub fn name(&self) -> &str {
        &self.text[self.dot + 1..]
    }

    /// The variable namespace, `plugins.<publisherCamel>.<nameCamel>`.
    pub fn variable_namespace(&self) -> String {
        format!(
            "plugins.{}.{}",
            lower_camel(self.publisher()),
            lower_camel(self.name())
        )
    }
}

impl TryFrom<String> for PluginId {
    type Error = IdError;

    fn try_from(text: String) -> Result<Self, IdError> {
        Self::parse(&text)
    }
}

impl From<PluginId> for String {
    fn from(id: PluginId) -> Self {
        id.text
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// `$defs/localId` (action ids): `[a-z][a-z0-9]*(-[a-z0-9]+)*`.
pub(crate) fn is_local_id(text: &str, max: usize) -> bool {
    if text.is_empty() || text.len() > max {
        return false;
    }
    let mut parts = text.split('-');
    let first_ok = parts.next().is_some_and(|first| {
        let mut chars = first.chars();
        chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    });
    first_ok
        && parts.all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

/// `$defs/camelKey`: `[a-z][A-Za-z0-9]{0,31}`.
pub(crate) fn is_camel_key(text: &str) -> bool {
    let mut chars = text.chars();
    text.len() <= 32
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_alphanumeric())
}

/// One segment of a variable path: `[a-z][A-Za-z0-9_]{0,31}`.
pub(crate) fn is_variable_segment(text: &str) -> bool {
    let mut chars = text.chars();
    text.len() <= 32
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_ids() {
        let id = PluginId::parse("acme.github-prs").unwrap();
        assert_eq!(id.publisher(), "acme");
        assert_eq!(id.name(), "github-prs");
        assert_eq!(id.variable_namespace(), "plugins.acme.githubPrs");
        for bad in [
            "acme",
            "a.b.c",
            "Acme.prs",
            "acme.Prs",
            "acme.pr--s",
            "acme.pr-1",
            "acme.-prs",
            "acme.prs-",
            "a.prs",
            "acme.p",
            "1acme.prs",
            "acme..prs",
            "",
        ] {
            assert!(PluginId::parse(bad).is_err(), "{bad}");
        }
        assert!(PluginId::parse(&format!("{}.{}", "a".repeat(24), "b".repeat(32))).is_ok());
        assert!(PluginId::parse(&format!("{}.ab", "a".repeat(25))).is_err());
        assert!(PluginId::parse(&format!("ab.{}", "b".repeat(33))).is_err());
    }

    #[test]
    fn local_ids_and_keys() {
        assert!(is_local_id("pr-count", 48));
        assert!(is_local_id("v2-1", 48));
        assert!(!is_local_id("pr--count", 48));
        assert!(!is_local_id("pr-", 48));
        assert!(!is_local_id("Pr", 48));
        assert!(is_camel_key("pollMinutes"));
        assert!(!is_camel_key("PollMinutes"));
        assert!(!is_camel_key("poll_minutes"));
        assert_eq!(lower_camel("work-session"), "workSession");
    }
}
