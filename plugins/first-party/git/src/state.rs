//! Key state of `status` and `branch` keys: the branch as the key's
//! second line and, for `status`, whether the work tree is clean (green),
//! has changes (amber) or conflicts (red), as the built-in shows it
//! (`modules/git/src/state.rs`).
//!
//! **What changed.** The built-in read the repository for a shown key
//! every 30 s. The agent allows no `tool.run` in a plugin's state call
//! (only storage, settings, time and log), and a state call gets no
//! resource handle. So a key shows what the last run of its action found
//! (a press, a workflow node, `runAction`): each run of `status` or
//! `branch` stores the state in the plugin's storage, under the action and
//! the one parameter a state call also sees, `projectId`; the state call
//! only reads it back, every 5 s. A key that has not run yet shows its
//! own design. Keys whose `repository` parameter names different folders
//! but that share `projectId` share one stored state (the last run wins):
//! the state call cannot tell them apart.

use deckpilot_plugin_sdk::prelude::*;
use serde::{Deserialize, Serialize};

use crate::porcelain::Status;

/// What a run stored for a key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Stored {
    State(KeyState),
    Failed { code: String, message: String },
}

/// The storage key of `kind` (`status`, `branch`) for keys with `project`.
pub fn storage_key(kind: &str, project: Option<&str>) -> String {
    format!("keyState.{kind}.{}", project.unwrap_or_default())
}

/// The key state for `status`. With `changes`: an indicator, green when
/// clean, amber with uncommitted changes, red with conflicts.
pub fn state_of(status: &Status, changes: bool) -> KeyState {
    let branch = &status.branch;
    let detail = match (&branch.name, &branch.head) {
        (Some(name), _) => name.clone(),
        (None, Some(head)) => format!("@{}", &head[..head.len().min(7)]),
        (None, None) => "no branch".to_owned(),
    };
    let state = KeyState::new().detail(detail);
    if !changes {
        return state;
    }
    let indicator = if status.counts().conflicts > 0 {
        Indicator::Error
    } else if status.clean() {
        Indicator::Ok
    } else {
        Indicator::Warning
    };
    state.indicator(indicator)
}

fn stored(read: Result<&Status, &PluginError>, changes: bool) -> Stored {
    match read {
        Ok(status) => Stored::State(state_of(status, changes)),
        Err(e) => Stored::Failed {
            code: e.code().to_owned(),
            message: e.message().chars().take(200).collect(),
        },
    }
}

fn store(ctx: &mut Context<'_>, key: String, value: &Stored) {
    if let Err(e) = ctx.storage_set(key, value) {
        ctx.log(
            LogLevel::Warn,
            format!("the key state was not kept ({:?})", e.kind()),
        );
    }
}

/// After a `status` run: the status key's state, and the branch key's
/// (a status run reads the branch too).
pub fn remember_status(
    ctx: &mut Context<'_>,
    project: Option<&str>,
    read: Result<&Status, &PluginError>,
) {
    store(ctx, storage_key("status", project), &stored(read, true));
    store(ctx, storage_key("branch", project), &stored(read, false));
}

/// After a `branch` run: the branch key's state.
pub fn remember_branch(
    ctx: &mut Context<'_>,
    project: Option<&str>,
    read: Result<&Status, &PluginError>,
) {
    store(ctx, storage_key("branch", project), &stored(read, false));
}

fn recall(ctx: &mut Context<'_>, kind: &str, args: &Args<'_>) -> Result<KeyState, PluginError> {
    let project: Option<String> = args.get(crate::PROJECT)?;
    match ctx.storage_get_as::<Stored>(storage_key(kind, project.as_deref()))? {
        None => Ok(KeyState::new()),
        Some(Stored::State(state)) => Ok(state),
        Some(Stored::Failed { code, message }) => Err(PluginError::new(code, message)),
    }
}

/// The state handler of `status` keys.
pub fn status(ctx: &mut Context<'_>, args: &Args<'_>) -> Result<KeyState, PluginError> {
    recall(ctx, "status", args)
}

/// The state handler of `branch` keys.
pub fn branch(ctx: &mut Context<'_>, args: &Args<'_>) -> Result<KeyState, PluginError> {
    recall(ctx, "branch", args)
}
