//! Credentials stored on this computer and referred to by identifier.
//!
//! A workflow or action never contains a secret: it names a
//! [`CredentialId`] such as `github.token`. The secret is stored on each
//! computer (in the encrypted configuration database), is never exported,
//! and reaches a module only through
//! [`ExecutionContext::credential`](crate::ExecutionContext::credential),
//! which requires the module's
//! [`Capability::Credentials`](crate::Capability::Credentials).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::IdentifierError;

/// Name of a stored credential: lower case letters, digits, `.`, `_` and
/// `-`, starting with a letter, at most 64 characters.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CredentialId(String);

impl CredentialId {
    pub fn new(id: impl Into<String>) -> Result<Self, IdentifierError> {
        let id = id.into();
        let error = |reason| IdentifierError {
            kind: "credential id",
            value: id.clone(),
            reason,
        };
        if id.is_empty() || id.len() > 64 {
            return Err(error("must be between 1 and 64 characters"));
        }
        if !id.starts_with(|c: char| c.is_ascii_lowercase()) {
            return Err(error("must start with a lowercase ASCII letter"));
        }
        if !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
        {
            return Err(error(
                "only lowercase ASCII letters, digits, `.`, `_` and `-` are allowed",
            ));
        }
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CredentialId {
    type Error = IdentifierError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<CredentialId> for String {
    fn from(value: CredentialId) -> Self {
        value.0
    }
}

impl fmt::Display for CredentialId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A secret value. Its `Debug` output never shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The secret itself, for the moment it is used.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// Where the engine looks credentials up.
pub trait CredentialSource: Send + Sync + fmt::Debug {
    fn secret(&self, id: &CredentialId) -> Option<Secret>;
}

/// No credentials: every lookup fails.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoCredentials;

impl CredentialSource for NoCredentials {
    fn secret(&self, _: &CredentialId) -> Option<Secret> {
        None
    }
}

/// Credentials held in memory, e.g. loaded from the database when the
/// configuration is (re)loaded.
#[derive(Default, Clone)]
pub struct MemoryCredentials(BTreeMap<CredentialId, Secret>);

impl fmt::Debug for MemoryCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.0.keys()).finish()
    }
}

impl MemoryCredentials {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with(mut self, id: CredentialId, secret: Secret) -> Self {
        self.0.insert(id, secret);
        self
    }

    pub fn insert(&mut self, id: CredentialId, secret: Secret) {
        self.0.insert(id, secret);
    }
}

impl CredentialSource for MemoryCredentials {
    fn secret(&self, id: &CredentialId) -> Option<Secret> {
        self.0.get(id).cloned()
    }
}

/// A stored credential as listed: never with its secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialInfo {
    pub id: CredentialId,
    /// What it is for, as the user described it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Milliseconds since the Unix epoch.
    pub updated_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CredentialError {
    #[error("the module is not allowed to use stored credentials")]
    NotPermitted,
    #[error("no credential named `{0}` is stored on this computer")]
    Missing(CredentialId),
}

impl CredentialError {
    /// Stable error code for [`crate::ActionResult::code`].
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotPermitted => "not-permitted",
            Self::Missing(_) => "credential-missing",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_validated_and_secrets_never_printed() {
        assert!(CredentialId::new("github.token").is_ok());
        assert!(CredentialId::new("api_key-2").is_ok());
        for bad in ["", "Token", "1abc", "a b", "a/b", &"a".repeat(65)] {
            assert!(CredentialId::new(bad).is_err(), "{bad}");
        }
        let secret = Secret::new("hunter2");
        assert_eq!(format!("{secret:?}"), "Secret(***)");
        let store = MemoryCredentials::new().with(CredentialId::new("a").unwrap(), secret.clone());
        assert!(!format!("{store:?}").contains("hunter2"));
        assert_eq!(store.secret(&CredentialId::new("a").unwrap()), Some(secret));
        assert_eq!(NoCredentials.secret(&CredentialId::new("a").unwrap()), None);
    }
}
