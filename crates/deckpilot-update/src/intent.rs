//! The update intent: a record, written before the agent is stopped for an
//! update, that lets the next start of the application tell whether the
//! update finished, and what to do if it did not
//! (docs/distribution/README.md, section 7, steps 4 and 11).
//!
//! The file is `update-intent.json` in the data directory, never in an
//! installation directory: a package manager, a store or the updater may
//! replace those, and the record must survive the replacement. It holds no
//! secrets: versions, the channel, the database schema and a time.
//!
//! [`assess`] is the decision taken at start-up, as a pure function of the
//! intent and what the application observes. It never proposes an
//! automatic downgrade: rolling back is the user's decision, and safe only
//! while the database has not been migrated ([`Rollback`]).

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::manifest::parse_version;

/// The file's name in the data directory.
pub const FILE_NAME: &str = "update-intent.json";
/// The format written by this build; a file of another format is ignored
/// (and replaced by the next update).
pub const FORMAT: u32 = 1;

/// What an update was about to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateIntent {
    pub format: u32,
    /// The version that was installed when the update started.
    pub from_version: String,
    /// The version being installed.
    pub to_version: String,
    /// The installation's distribution channel (`github`).
    pub distribution_channel: String,
    /// The schema version of the configuration database when the update
    /// started; `None` when it could not be read (the configuration was
    /// locked).
    pub schema_version: Option<u32>,
    /// Seconds since the Unix epoch.
    pub created_at: u64,
    /// Whether an agent was running and was stopped for the update.
    pub agent_was_running: bool,
}

/// Why the intent could not be read.
#[derive(Debug, thiserror::Error)]
pub enum IntentError {
    #[error("cannot read the update intent: {0}")]
    Io(#[from] io::Error),
    #[error("the update intent is not valid: {0}")]
    Invalid(String),
}

/// Where the intent of the data directory `data_dir` is kept.
pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

impl UpdateIntent {
    /// Writes the intent atomically: to a temporary file in the same
    /// directory, flushed to disk, then renamed over any previous one, so
    /// an interruption never leaves a partial record.
    pub fn write(&self, data_dir: &Path) -> io::Result<()> {
        use std::io::Write as _;
        std::fs::create_dir_all(data_dir)?;
        let target = path(data_dir);
        let partial = data_dir.join(format!("{FILE_NAME}.partial"));
        let json = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        {
            let mut file = std::fs::File::create(&partial)?;
            file.write_all(&json)?;
            file.sync_all()?;
        }
        std::fs::rename(&partial, &target)
    }

    /// The intent in `data_dir`, if there is one.
    pub fn read(data_dir: &Path) -> Result<Option<Self>, IntentError> {
        let bytes = match std::fs::read(path(data_dir)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let intent: Self =
            serde_json::from_slice(&bytes).map_err(|e| IntentError::Invalid(e.to_string()))?;
        if intent.format != FORMAT {
            return Err(IntentError::Invalid(format!(
                "format {} is not {FORMAT}",
                intent.format
            )));
        }
        Ok(Some(intent))
    }

    /// Removes the intent of `data_dir`; no intent is fine.
    pub fn remove(data_dir: &Path) -> io::Result<()> {
        match std::fs::remove_file(path(data_dir)) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
}

/// What the application sees when it starts and finds an intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation<'a> {
    /// This application's version: the installed one.
    pub app_version: &'a str,
    /// The running agent's version; `None` when no agent runs.
    pub agent_version: Option<&'a str>,
    /// The schema version this build's storage migrates the database to.
    pub supported_schema: u32,
    /// The database's schema version now; `None` when it cannot be read
    /// (the configuration is locked).
    pub current_schema: Option<u32>,
}

/// Whether going back to the previous version is safe for the data.
///
/// Migrations are forward only and an older build refuses a newer schema
/// before writing anything (`deckpilot_storage`), so the database decides:
/// as long as it has its old schema, reinstalling the previous version is
/// safe; after a migration it needs the pre-migration backup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rollback {
    /// The new version uses the same schema: reinstalling the previous one
    /// is safe.
    Safe,
    /// The new version migrates the database, but has not yet: reinstalling
    /// the previous one is safe now, and stops being safe once the new
    /// agent has opened the configuration.
    SafeUntilMigrated,
    /// The database may already have been migrated (its schema cannot be
    /// read now): reinstalling the previous version is safe only if the new
    /// one never opened the configuration.
    SafeOnlyIfNotMigrated { backup_schema: u32 },
    /// The database was migrated: the previous version refuses it. Going
    /// back means restoring the pre-migration backup
    /// (`backups/deckpilot-schema{backup_schema}-*.db`), losing the changes
    /// made since, and reinstalling the previous version.
    NeedsRestore { backup_schema: u32 },
    /// The schema at the start of the update is not known.
    Unknown,
}

/// What the application does about an intent it finds at start-up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recovery {
    /// The new version runs with its own agent: the update is complete.
    /// The intent is removed.
    Completed,
    /// The previous version is still installed: the update failed or was
    /// interrupted before the installation changed anything. The previous
    /// agent must run again (the application starts it; `agent_running`
    /// says whether it does); the intent is removed, and the update can be
    /// installed again.
    NotInstalled { agent_running: bool },
    /// The new version is installed but its agent does not run. The intent
    /// is kept, so the next start checks again.
    AgentNotRunning { rollback: Rollback },
    /// Neither the previous nor the new version is installed (DeckPilot was
    /// installed by other means since). The intent is removed.
    Superseded,
}

impl Recovery {
    /// Whether the intent stays for the next start.
    pub fn keeps_intent(&self) -> bool {
        matches!(self, Self::AgentNotRunning { .. })
    }
}

fn same_version(a: &str, b: &str) -> bool {
    match (parse_version(a), parse_version(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a.trim() == b.trim(),
    }
}

/// The decision at start-up for `intent`, given `observation`.
pub fn assess(intent: &UpdateIntent, observation: &Observation<'_>) -> Recovery {
    if same_version(observation.app_version, &intent.to_version) {
        if observation
            .agent_version
            .is_some_and(|agent| same_version(agent, &intent.to_version))
        {
            return Recovery::Completed;
        }
        return Recovery::AgentNotRunning {
            rollback: rollback(intent, observation),
        };
    }
    if same_version(observation.app_version, &intent.from_version) {
        return Recovery::NotInstalled {
            agent_running: observation.agent_version.is_some(),
        };
    }
    Recovery::Superseded
}

fn rollback(intent: &UpdateIntent, observation: &Observation<'_>) -> Rollback {
    let Some(before) = intent.schema_version else {
        return Rollback::Unknown;
    };
    if observation.supported_schema <= before {
        return Rollback::Safe;
    }
    match observation.current_schema {
        Some(now) if now <= before => Rollback::SafeUntilMigrated,
        Some(_) => Rollback::NeedsRestore {
            backup_schema: before,
        },
        None => Rollback::SafeOnlyIfNotMigrated {
            backup_schema: before,
        },
    }
}

/// What to tell the user about going back to `from`.
pub fn rollback_advice(rollback: &Rollback, from: &str) -> String {
    match rollback {
        Rollback::Safe => format!(
            "reinstalling version {from} is safe: both versions use the same configuration \
             database"
        ),
        Rollback::SafeUntilMigrated => format!(
            "reinstalling version {from} is safe as long as the new agent has not opened the \
             configuration; once it has upgraded the database, version {from} cannot read it"
        ),
        Rollback::SafeOnlyIfNotMigrated { backup_schema } => format!(
            "reinstalling version {from} is safe only if the new agent never opened the \
             configuration. If it did, version {from} refuses the upgraded database: restore \
             the backup backups/deckpilot-schema{backup_schema}-*.db from the data directory \
             first (changes made since are lost; see docs/updates.md)"
        ),
        Rollback::NeedsRestore { backup_schema } => format!(
            "the configuration database was upgraded, so version {from} cannot read it: going \
             back needs the backup backups/deckpilot-schema{backup_schema}-*.db from the data \
             directory restored first, and changes made since are lost (see docs/updates.md). \
             Staying on this version and waiting for a fix is always possible"
        ),
        Rollback::Unknown => format!(
            "whether reinstalling version {from} is safe depends on whether the configuration \
             database was upgraded (see docs/updates.md)"
        ),
    }
}

/// The message for the user and the update log.
pub fn describe(intent: &UpdateIntent, recovery: &Recovery) -> String {
    let (from, to) = (&intent.from_version, &intent.to_version);
    match recovery {
        Recovery::Completed => format!("update from {from} to {to} completed"),
        Recovery::NotInstalled { agent_running } => format!(
            "the update from {from} to {to} did not complete; version {from} is still \
             installed{}. Install the update again",
            if *agent_running {
                " and its agent runs again"
            } else {
                ", but its agent is not running: start it from Settings or restart DeckPilot"
            }
        ),
        Recovery::AgentNotRunning { rollback } => format!(
            "version {to} was installed, but its agent is not running. Start it from \
             Settings, or restart DeckPilot; if it does not start, its log is in the data \
             directory's logs folder. If you need to go back: {}",
            rollback_advice(rollback, from)
        ),
        Recovery::Superseded => format!(
            "an update from {from} to {to} was interrupted, and DeckPilot has been installed \
             again since; nothing to recover"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(schema: Option<u32>) -> UpdateIntent {
        UpdateIntent {
            format: FORMAT,
            from_version: "0.6.0".into(),
            to_version: "0.7.0".into(),
            distribution_channel: "github".into(),
            schema_version: schema,
            created_at: 1_700_000_000,
            agent_was_running: true,
        }
    }

    fn observe<'a>(app: &'a str, agent: Option<&'a str>) -> Observation<'a> {
        Observation {
            app_version: app,
            agent_version: agent,
            supported_schema: 12,
            current_schema: None,
        }
    }

    fn dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "deckpilot-intent-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_intent_is_written_atomically_read_back_and_removed() {
        let dir = dir("roundtrip");
        assert!(UpdateIntent::read(&dir).unwrap().is_none());
        let written = intent(Some(11));
        written.write(&dir).unwrap();
        assert_eq!(UpdateIntent::read(&dir).unwrap(), Some(written.clone()));
        // No temporary file is left behind.
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, [FILE_NAME]);
        // The documented names, so other tools and later versions read it.
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path(&dir)).unwrap()).unwrap();
        for key in [
            "format",
            "fromVersion",
            "toVersion",
            "distributionChannel",
            "schemaVersion",
            "createdAt",
            "agentWasRunning",
        ] {
            assert!(json.get(key).is_some(), "{key} missing: {json}");
        }
        // A second update replaces it.
        let mut next = written;
        next.to_version = "0.8.0".into();
        next.write(&dir).unwrap();
        assert_eq!(UpdateIntent::read(&dir).unwrap(), Some(next));
        UpdateIntent::remove(&dir).unwrap();
        assert!(UpdateIntent::read(&dir).unwrap().is_none());
        // Removing nothing is fine.
        UpdateIntent::remove(&dir).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_damaged_or_foreign_intent_is_reported() {
        let dir = dir("damaged");
        std::fs::write(path(&dir), b"{ not json").unwrap();
        assert!(matches!(
            UpdateIntent::read(&dir),
            Err(IntentError::Invalid(_))
        ));
        let mut future = serde_json::to_value(intent(None)).unwrap();
        future["format"] = 2.into();
        std::fs::write(path(&dir), future.to_string()).unwrap();
        assert!(matches!(
            UpdateIntent::read(&dir),
            Err(IntentError::Invalid(_))
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_new_version_with_its_agent_completes_the_update() {
        let recovery = assess(&intent(Some(11)), &observe("0.7.0", Some("0.7.0")));
        assert_eq!(recovery, Recovery::Completed);
        assert!(!recovery.keeps_intent());
        assert_eq!(
            describe(&intent(None), &recovery),
            "update from 0.6.0 to 0.7.0 completed"
        );
    }

    #[test]
    fn a_failed_installation_leaves_the_previous_version_to_restart() {
        // The previous app runs again: whatever the agent does, the update
        // did not install.
        for agent in [None, Some("0.6.0")] {
            let recovery = assess(&intent(Some(11)), &observe("0.6.0", agent));
            assert_eq!(
                recovery,
                Recovery::NotInstalled {
                    agent_running: agent.is_some()
                }
            );
            assert!(!recovery.keeps_intent());
        }
        let message = describe(
            &intent(None),
            &Recovery::NotInstalled {
                agent_running: true,
            },
        );
        assert!(
            message.contains("version 0.6.0 is still installed and its agent runs again"),
            "{message}"
        );
        let message = describe(
            &intent(None),
            &Recovery::NotInstalled {
                agent_running: false,
            },
        );
        assert!(message.contains("agent is not running"), "{message}");
    }

    #[test]
    fn a_new_version_whose_agent_does_not_run_is_reported_and_kept() {
        // No agent, or still the old one.
        for agent in [None, Some("0.6.0")] {
            let recovery = assess(&intent(Some(12)), &observe("0.7.0", agent));
            assert_eq!(
                recovery,
                Recovery::AgentNotRunning {
                    rollback: Rollback::Safe
                }
            );
            assert!(recovery.keeps_intent());
        }
    }

    #[test]
    fn rollback_is_called_safe_only_before_a_migration() {
        let rollback_for = |schema: Option<u32>, current: Option<u32>| {
            let mut observation = observe("0.7.0", None);
            observation.current_schema = current;
            match assess(&intent(schema), &observation) {
                Recovery::AgentNotRunning { rollback } => rollback,
                other => panic!("{other:?}"),
            }
        };
        // Same schema in both versions.
        assert_eq!(rollback_for(Some(12), None), Rollback::Safe);
        // The new version migrates 11 -> 12.
        assert_eq!(
            rollback_for(Some(11), Some(11)),
            Rollback::SafeUntilMigrated
        );
        assert_eq!(
            rollback_for(Some(11), Some(12)),
            Rollback::NeedsRestore { backup_schema: 11 }
        );
        assert_eq!(
            rollback_for(Some(11), None),
            Rollback::SafeOnlyIfNotMigrated { backup_schema: 11 }
        );
        assert_eq!(rollback_for(None, Some(12)), Rollback::Unknown);
        let advice = rollback_advice(&Rollback::NeedsRestore { backup_schema: 11 }, "0.6.0");
        assert!(
            advice.contains("backups/deckpilot-schema11-*.db") && advice.contains("lost"),
            "{advice}"
        );
        // Never "safe" without a qualification once a migration is possible.
        for rollback in [
            Rollback::SafeUntilMigrated,
            Rollback::SafeOnlyIfNotMigrated { backup_schema: 11 },
            Rollback::NeedsRestore { backup_schema: 11 },
            Rollback::Unknown,
        ] {
            assert!(
                !rollback_advice(&rollback, "0.6.0")
                    .starts_with("reinstalling version 0.6.0 is safe:"),
                "{rollback:?}"
            );
        }
    }

    #[test]
    fn an_installation_by_other_means_supersedes_the_intent() {
        let recovery = assess(&intent(None), &observe("0.8.0", Some("0.8.0")));
        assert_eq!(recovery, Recovery::Superseded);
        assert!(!recovery.keeps_intent());
    }

    #[test]
    fn versions_compare_as_versions() {
        assert!(same_version("v0.7.0", "0.7.0"));
        assert!(!same_version("0.7.0-alpha.1", "0.7.0"));
    }
}
