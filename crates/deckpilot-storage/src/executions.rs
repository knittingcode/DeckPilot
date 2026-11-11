//! Execution history: the records of workflow runs, written by the agent.
//!
//! Records are runtime state, not configuration: they are never exported
//! and are limited in number ([`Retention`]). A record holds only what
//! [`deckpilot_core::history`] considers safe to store.

use rusqlite::{Connection, OptionalExtension, params};

use deckpilot_core::{ExecutionId, ExecutionRecord, ExecutionState, ProfileId, WorkflowId};

use crate::Result;

/// Schema migration 6.
pub(crate) const SCHEMA: &str = r"
    CREATE TABLE workflow_executions (
        id          TEXT PRIMARY KEY NOT NULL,
        profile_id  TEXT NOT NULL,
        workflow_id TEXT NOT NULL,
        state       TEXT NOT NULL,
        started_at  INTEGER NOT NULL,
        finished_at INTEGER,
        document    TEXT NOT NULL
    );
    CREATE INDEX workflow_executions_started ON workflow_executions (profile_id, started_at);
    CREATE INDEX workflow_executions_workflow ON workflow_executions (workflow_id, started_at);
";

/// How many records are kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    /// Per profile.
    pub per_profile: usize,
    /// Per workflow.
    pub per_workflow: usize,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            per_profile: 500,
            per_workflow: 50,
        }
    }
}

/// Which records to list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionQuery {
    pub profile: ProfileId,
    pub workflow: Option<WorkflowId>,
    pub limit: usize,
}

fn to_sql(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

pub(crate) fn save(connection: &Connection, record: &ExecutionRecord) -> Result<()> {
    let summary = &record.summary;
    connection.execute(
        "INSERT INTO workflow_executions
             (id, profile_id, workflow_id, state, started_at, finished_at, document)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (id) DO UPDATE SET
             state = excluded.state, finished_at = excluded.finished_at,
             document = excluded.document",
        params![
            summary.id.to_string(),
            summary.profile.to_string(),
            summary.workflow.to_string(),
            summary.state.as_str(),
            to_sql(summary.started_at),
            summary.finished_at.map(to_sql),
            serde_json::to_string(record)?,
        ],
    )?;
    Ok(())
}

pub(crate) fn get(connection: &Connection, id: ExecutionId) -> Result<Option<ExecutionRecord>> {
    let document: Option<String> = connection
        .query_row(
            "SELECT document FROM workflow_executions WHERE id = ?1",
            [id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    document
        .map(|d| serde_json::from_str(&d).map_err(Into::into))
        .transpose()
}

/// Newest first. Records that cannot be read are left out.
pub(crate) fn list(connection: &Connection, query: ExecutionQuery) -> Result<Vec<ExecutionRecord>> {
    let limit = i64::try_from(query.limit).unwrap_or(i64::MAX);
    let mut statement = connection.prepare(
        "SELECT document FROM workflow_executions
         WHERE profile_id = ?1 AND (?2 IS NULL OR workflow_id = ?2)
         ORDER BY started_at DESC, id LIMIT ?3",
    )?;
    let rows = statement.query_map(
        params![
            query.profile.to_string(),
            query.workflow.map(|w| w.to_string()),
            limit
        ],
        |row| row.get::<_, String>(0),
    )?;
    let mut records = Vec::new();
    for document in rows {
        if let Ok(record) = serde_json::from_str(&document?) {
            records.push(record);
        }
    }
    Ok(records)
}

/// Marks runs that are recorded as running, other than `except`, as
/// interrupted: the agent stopped while they ran. Returns how many.
pub(crate) fn interrupt_stale(
    connection: &Connection,
    except: &[ExecutionId],
    now_ms: u64,
) -> Result<usize> {
    let mut statement =
        connection.prepare("SELECT id, document FROM workflow_executions WHERE state = ?1")?;
    let stale: Vec<(String, String)> = statement
        .query_map([ExecutionState::Running.as_str()], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut count = 0;
    for (id, document) in stale {
        if except.iter().any(|e| e.to_string() == id) {
            continue;
        }
        match serde_json::from_str::<ExecutionRecord>(&document) {
            Ok(mut record) => {
                record.end(
                    ExecutionState::Interrupted,
                    "the agent stopped before the workflow finished",
                    now_ms,
                );
                save(connection, &record)?;
            }
            Err(_) => {
                connection.execute("DELETE FROM workflow_executions WHERE id = ?1", [&id])?;
            }
        }
        count += 1;
    }
    Ok(count)
}

/// Removes the oldest finished records beyond `retention`.
pub(crate) fn prune(connection: &Connection, retention: Retention) -> Result<()> {
    let per_workflow = i64::try_from(retention.per_workflow).unwrap_or(i64::MAX);
    let per_profile = i64::try_from(retention.per_profile).unwrap_or(i64::MAX);
    connection.execute(
        "DELETE FROM workflow_executions WHERE state != ?1 AND id IN (
             SELECT id FROM (
                 SELECT id, ROW_NUMBER() OVER (
                     PARTITION BY workflow_id ORDER BY started_at DESC, id) AS n
                 FROM workflow_executions)
             WHERE n > ?2)",
        params![ExecutionState::Running.as_str(), per_workflow],
    )?;
    connection.execute(
        "DELETE FROM workflow_executions WHERE state != ?1 AND id IN (
             SELECT id FROM (
                 SELECT id, ROW_NUMBER() OVER (
                     PARTITION BY profile_id ORDER BY started_at DESC, id) AS n
                 FROM workflow_executions)
             WHERE n > ?2)",
        params![ExecutionState::Running.as_str(), per_profile],
    )?;
    Ok(())
}

/// Deletes the finished records of `profile` (and `workflow`, if given).
pub(crate) fn clear(
    connection: &Connection,
    profile: ProfileId,
    workflow: Option<WorkflowId>,
) -> Result<usize> {
    Ok(connection.execute(
        "DELETE FROM workflow_executions
         WHERE profile_id = ?1 AND (?2 IS NULL OR workflow_id = ?2) AND state != ?3",
        params![
            profile.to_string(),
            workflow.map(|w| w.to_string()),
            ExecutionState::Running.as_str()
        ],
    )?)
}

#[cfg(test)]
mod tests {
    use deckpilot_core::{ExecutionTrigger, Profile, Workflow};

    use super::*;
    use crate::SqliteStore;

    fn record(profile: &Profile, workflow: &Workflow, started_at: u64) -> ExecutionRecord {
        ExecutionRecord::start(
            ExecutionId::new(),
            workflow,
            profile,
            ExecutionTrigger::App,
            started_at,
        )
    }

    #[test]
    fn records_are_saved_listed_and_updated() {
        let store = SqliteStore::open_in_memory().unwrap();
        let profile = Profile::new("Work");
        let workflow = Workflow::new("Build");
        let mut first = record(&profile, &workflow, 1_000);
        let second = record(&profile, &workflow, 2_000);
        store.save_execution(&first).unwrap();
        store.save_execution(&second).unwrap();
        first.end(ExecutionState::Failed, "boom", 1_500);
        store.save_execution(&first).unwrap();

        let query = ExecutionQuery {
            profile: profile.id,
            workflow: None,
            limit: 10,
        };
        let listed = store.executions(query).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].summary.id, second.summary.id);
        assert_eq!(listed[1].summary.state, ExecutionState::Failed);
        assert_eq!(store.execution(first.summary.id).unwrap(), Some(first));

        // Other profiles see nothing.
        let other = ExecutionQuery {
            profile: Profile::new("Other").id,
            ..query
        };
        assert_eq!(store.executions(other).unwrap(), Vec::new());
    }

    #[test]
    fn stale_runs_are_interrupted() {
        let store = SqliteStore::open_in_memory().unwrap();
        let profile = Profile::new("Work");
        let workflow = Workflow::new("Build");
        let stale = record(&profile, &workflow, 1_000);
        let live = record(&profile, &workflow, 1_100);
        store.save_execution(&stale).unwrap();
        store.save_execution(&live).unwrap();
        let count = store
            .interrupt_stale_executions(&[live.summary.id], 5_000)
            .unwrap();
        assert_eq!(count, 1);
        let stale = store.execution(stale.summary.id).unwrap().unwrap();
        assert_eq!(stale.summary.state, ExecutionState::Interrupted);
        assert_eq!(stale.summary.finished_at, Some(5_000));
        let live = store.execution(live.summary.id).unwrap().unwrap();
        assert_eq!(live.summary.state, ExecutionState::Running);
    }

    #[test]
    fn retention_keeps_the_newest_finished_runs() {
        let store = SqliteStore::open_in_memory().unwrap();
        let profile = Profile::new("Work");
        let build = Workflow::new("Build");
        let deploy = Workflow::new("Deploy");
        for n in 0..6 {
            let mut done = record(&profile, &build, 1_000 + n);
            done.end(ExecutionState::Succeeded, "", 2_000 + n);
            store.save_execution(&done).unwrap();
        }
        let running = record(&profile, &build, 500);
        store.save_execution(&running).unwrap();
        let mut other = record(&profile, &deploy, 100);
        other.end(ExecutionState::Succeeded, "", 200);
        store.save_execution(&other).unwrap();

        store
            .prune_executions(Retention {
                per_profile: 4,
                per_workflow: 3,
            })
            .unwrap();
        let all = store
            .executions(ExecutionQuery {
                profile: profile.id,
                workflow: None,
                limit: 100,
            })
            .unwrap();
        let started: Vec<u64> = all.iter().map(|r| r.summary.started_at).collect();
        // Running records are never pruned.
        assert!(started.contains(&500));
        assert_eq!(
            started.iter().filter(|&&s| s >= 1_000).count(),
            3,
            "{started:?}"
        );
        assert_eq!(
            store.clear_executions(profile.id, Some(build.id)).unwrap(),
            3
        );
        assert!(store.execution(running.summary.id).unwrap().is_some());
    }

    #[test]
    fn deleting_a_profile_deletes_its_history() {
        let store = SqliteStore::open_in_memory().unwrap();
        let profile = Profile::new("Work");
        crate::ConfigStore::save_profile(&store, &profile).unwrap();
        let run = record(&profile, &Workflow::new("Build"), 1);
        store.save_execution(&run).unwrap();
        crate::ConfigStore::delete_profile(&store, profile.id).unwrap();
        assert!(store.execution(run.summary.id).unwrap().is_none());
    }
}
