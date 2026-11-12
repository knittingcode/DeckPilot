//! Opaque, globally unique identifiers for configuration entities.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! entity_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Generates a new random identifier.
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self)
            }
        }
    };
}

entity_id!(
    /// Identifies a [`Profile`](crate::Profile).
    ProfileId
);
entity_id!(
    /// Identifies a [`Project`](crate::Project) within a profile.
    ProjectId
);
entity_id!(
    /// Identifies an [`ActionInstance`](crate::ActionInstance).
    ActionInstanceId
);
entity_id!(
    /// Identifies a [`Workflow`](crate::Workflow).
    WorkflowId
);
entity_id!(
    /// Identifies a [`DeviceLayout`](crate::DeviceLayout).
    LayoutId
);
entity_id!(
    /// Identifies a page (or folder) within a layout.
    PageId
);
entity_id!(
    /// Identifies one run of a workflow.
    ExecutionId
);
entity_id!(
    /// Identifies an [`Icon`](crate::Icon) within a profile.
    IconId
);
entity_id!(
    /// Identifies one work session (see [`crate::work_session`]).
    WorkSessionId
);
entity_id!(
    /// Identifies one run of a work session's end verification; an
    /// approval to finish names the verification that gave it.
    VerificationId
);

/// Validates a dotted, lowercase identifier such as `deckpilot.core.log`.
pub(crate) fn validate_dotted(
    kind: &'static str,
    value: &str,
) -> Result<(), crate::IdentifierError> {
    let error = |reason| crate::IdentifierError {
        kind,
        value: value.to_owned(),
        reason,
    };
    if value.is_empty() || value.len() > 128 {
        return Err(error("must be between 1 and 128 characters"));
    }
    if value.split('.').any(str::is_empty) {
        return Err(error("segments separated by `.` must not be empty"));
    }
    let valid = value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'));
    if !valid {
        return Err(error(
            "only lowercase ASCII letters, digits, `.`, `_` and `-` are allowed",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_through_strings() {
        let id = ProfileId::new();
        let parsed: ProfileId = id.to_string().parse().unwrap();
        assert_eq!(id, parsed);
        assert_ne!(ProfileId::new(), ProfileId::new());
    }

    #[test]
    fn dotted_identifiers() {
        assert!(validate_dotted("test", "deckpilot.core.log").is_ok());
        assert!(validate_dotted("test", "shop.repo-dir_2").is_ok());
        assert!(validate_dotted("test", "").is_err());
        assert!(validate_dotted("test", "a..b").is_err());
        assert!(validate_dotted("test", ".a").is_err());
        assert!(validate_dotted("test", "Upper.case").is_err());
        assert!(validate_dotted("test", "with space").is_err());
        assert!(validate_dotted("test", "C:\\path").is_err());
    }
}
