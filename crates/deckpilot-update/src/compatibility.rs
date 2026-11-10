//! Whether a running agent belongs to this application version.
//!
//! The application and the agent ship together and must run the same
//! version. After an update the application detects an older agent that is
//! still running and restarts it.

use serde::{Deserialize, Serialize};

use crate::manifest::parse_version;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AgentCompatibility {
    Compatible,
    /// The agent is older than the application; restart it.
    AgentOutdated {
        agent: String,
        app: String,
    },
    /// The agent is newer, e.g. after a partial downgrade; restart it.
    AgentNewer {
        agent: String,
        app: String,
    },
    /// A version could not be parsed.
    Unknown {
        agent: String,
        app: String,
    },
}

pub fn check(app_version: &str, agent_version: &str) -> AgentCompatibility {
    let pair = || (agent_version.to_owned(), app_version.to_owned());
    match (parse_version(app_version), parse_version(agent_version)) {
        (Ok(app), Ok(agent)) if app == agent => AgentCompatibility::Compatible,
        (Ok(app), Ok(agent)) if agent < app => {
            let (agent, app) = pair();
            AgentCompatibility::AgentOutdated { agent, app }
        }
        (Ok(_), Ok(_)) => {
            let (agent, app) = pair();
            AgentCompatibility::AgentNewer { agent, app }
        }
        _ => {
            let (agent, app) = pair();
            AgentCompatibility::Unknown { agent, app }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_matching_versions() {
        assert_eq!(check("0.3.0", "0.3.0"), AgentCompatibility::Compatible);
        assert!(matches!(
            check("0.3.0", "0.2.0"),
            AgentCompatibility::AgentOutdated { .. }
        ));
        assert!(matches!(
            check("0.3.0", "0.3.1"),
            AgentCompatibility::AgentNewer { .. }
        ));
        assert!(matches!(
            check("0.3.0", "dev"),
            AgentCompatibility::Unknown { .. }
        ));
    }
}
