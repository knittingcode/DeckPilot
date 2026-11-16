//! Whether a running agent belongs to this application version.
//!
//! The application and the agent ship together and must run the same
//! version, from the same distribution channel. After an update the
//! application detects an older agent that is still running and restarts
//! it; an agent of another channel (rule R6 of docs/distribution/README.md)
//! is replaced by the agent in the app's own bundle the same way.

use deckpilot_distribution::{Distribution, DistributionChannel};
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
    /// The agent speaks a protocol this application cannot work with at all
    /// (from DeckPilot 0.3 or earlier). It can only be stopped and replaced
    /// by the agent of this application.
    UpdateRequired {
        agent: String,
        app: String,
    },
    /// The agent was delivered by another distribution channel than the
    /// application (for example an APT agent next to a GitHub app, rule
    /// R6). `agent` and `app` are the channels' names. The application
    /// does not work with it; restarting starts the agent in the
    /// application's own bundle.
    OtherDistribution {
        agent: String,
        app: String,
    },
}

/// The channel of agents that report no distribution record: every build
/// before the record existed (DeckPilot 0.6.0-alpha.3 and earlier) came
/// from GitHub Releases.
pub const CHANNEL_OF_AGENTS_WITHOUT_RECORD: DistributionChannel = DistributionChannel::Github;

/// [`check`], and first whether the agent's channel (`agent_channel`, the
/// name it reports; `None` from agents without a record) is the
/// application's own (rule R6). A channel this build does not know is
/// another channel.
pub fn check_agent(
    app: &Distribution,
    agent_version: &str,
    agent_channel: Option<&str>,
) -> AgentCompatibility {
    let app_channel = app.distribution_channel();
    let agent_channel = agent_channel.unwrap_or(CHANNEL_OF_AGENTS_WITHOUT_RECORD.name());
    if agent_channel != app_channel.name() {
        return AgentCompatibility::OtherDistribution {
            agent: agent_channel.to_owned(),
            app: app_channel.name().to_owned(),
        };
    }
    check(app.application_version(), agent_version)
}

/// A message for the user about an agent of another channel.
pub fn other_distribution_message(agent: &str, app: &str) -> String {
    format!(
        "the running agent was installed from {agent}, this app from {app}; DeckPilot works only \
         with the agent of its own installation. Restart the agent to replace it with this \
         app's agent"
    )
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
    use deckpilot_distribution::{Architecture, Platform};

    fn build(channel: DistributionChannel, version: &'static str) -> Distribution {
        Distribution::new(
            channel,
            None,
            version,
            Platform::Linux,
            Architecture::X86_64,
        )
        .unwrap()
    }

    #[test]
    fn an_agent_of_another_channel_is_refused() {
        let github = build(DistributionChannel::Github, "0.6.0");
        let apt = build(DistributionChannel::Apt, "0.6.0");
        assert_eq!(
            check_agent(&github, "0.6.0", Some("github")),
            AgentCompatibility::Compatible
        );
        assert_eq!(
            check_agent(&github, "0.6.0", Some("apt")),
            AgentCompatibility::OtherDistribution {
                agent: "apt".into(),
                app: "github".into()
            }
        );
        // The channel is checked before the version: restarting fixes both.
        assert!(matches!(
            check_agent(&apt, "0.5.0", Some("github")),
            AgentCompatibility::OtherDistribution { .. }
        ));
        // A channel this build does not know is another channel.
        assert!(matches!(
            check_agent(&github, "0.6.0", Some("snap")),
            AgentCompatibility::OtherDistribution { .. }
        ));
        // Same channel: the version rules as before.
        assert!(matches!(
            check_agent(&github, "0.5.0", Some("github")),
            AgentCompatibility::AgentOutdated { .. }
        ));
    }

    #[test]
    fn agents_without_a_record_count_as_github_builds() {
        let github = build(DistributionChannel::Github, "0.6.0");
        // An older GitHub agent is outdated, as before the record existed.
        assert!(matches!(
            check_agent(&github, "0.6.0-alpha.3", None),
            AgentCompatibility::AgentOutdated { .. }
        ));
        assert_eq!(
            check_agent(&github, "0.6.0", None),
            AgentCompatibility::Compatible
        );
        // A store build never works with such an agent.
        let apt = build(DistributionChannel::Apt, "0.6.0");
        assert_eq!(
            check_agent(&apt, "0.6.0", None),
            AgentCompatibility::OtherDistribution {
                agent: "github".into(),
                app: "apt".into()
            }
        );
    }

    #[test]
    fn the_variant_is_serialized_like_the_others() {
        let json = serde_json::to_value(AgentCompatibility::OtherDistribution {
            agent: "apt".into(),
            app: "github".into(),
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "type": "otherDistribution", "agent": "apt", "app": "github" })
        );
        let message = other_distribution_message("apt", "github");
        assert!(message.contains("apt") && message.contains("Restart the agent"));
    }

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
