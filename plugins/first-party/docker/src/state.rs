//! Key state of Docker keys, as the built-in shows it
//! (`modules/docker/src/state.rs`): whether Docker is available, how many
//! containers run, and whether a key's Compose project is up (with a
//! toggle on up and down keys). Logs keys have none; see below for the
//! Compose changes.
//!
//! **What changed.** The built-in read Docker for a shown key every 30 s.
//! The agent allows no `tool.run` in a plugin's state call and gives it no
//! resource handle, so a key shows what the last run found: each run
//! stores the state in the plugin's storage, under what a state call also
//! sees (the action's `context`, the `name` filter, `projectId` and
//! `projectName`), and the state call reads it back every 5 s. A Compose
//! change reads `compose ps` afterwards, so the project's `compose-status`
//! keys follow it. The agent allows a key state only for read-only
//! actions: `compose-up`, `compose-down` and `compose-restart` keys show
//! no state any more (the built-in's toggle on up and down keys is gone).
//! A key that has not run yet shows its own design. Keys that differ only in
//! their resource parameters (`compose`, `environment`) share one stored
//! state: the state call cannot tell them apart.

use deckpilot_plugin_sdk::ParamValue;
use deckpilot_plugin_sdk::prelude::*;
use serde::{Deserialize, Serialize};

use crate::cli::Container;
use crate::optional;

/// What a run stored for a key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Stored {
    Available(bool),
    Running(u32),
    Compose { running: u32, total: u32 },
    Failed { code: String, message: String },
}

/// FNV-1a: a short, stable storage key for parameter texts.
fn digest(parts: &[Option<String>]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for byte in part.as_deref().unwrap_or_default().bytes().chain([0xff]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

pub fn status_key(args: &Args<'_>) -> String {
    format!(
        "keyState.status.{}",
        digest(&[optional(args, crate::CONTEXT)])
    )
}

pub fn containers_key(args: &Args<'_>) -> String {
    format!(
        "keyState.containers.{}",
        digest(&[optional(args, "name"), optional(args, crate::CONTEXT)])
    )
}

pub fn compose_key(args: &Args<'_>) -> String {
    format!(
        "keyState.compose.{}",
        digest(&[
            optional(args, crate::PROJECT),
            optional(args, "projectName"),
            optional(args, crate::CONTEXT),
        ])
    )
}

fn small(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// From a `status` result.
pub fn availability(output: &Output) -> Stored {
    Stored::Available(matches!(
        output.data().get("available"),
        Some(ParamValue::Bool(true))
    ))
}

/// From a `containers` result.
pub fn running(output: &Output) -> Stored {
    match output.data().get("running") {
        Some(ParamValue::Integer(n)) => Stored::Running(u32::try_from(*n).unwrap_or(0)),
        _ => Stored::Running(0),
    }
}

/// From a project's containers.
pub fn compose_counts(containers: &[Container]) -> Stored {
    Stored::Compose {
        running: small(containers.iter().filter(|c| c.running()).count()),
        total: small(containers.len()),
    }
}

/// Stores what a run found for its keys (a failure too: the key shows it).
pub fn remember(ctx: &mut Context<'_>, key: &str, found: Result<Stored, &PluginError>) {
    let value = found.unwrap_or_else(|e| Stored::Failed {
        code: e.code().to_owned(),
        message: e.message().chars().take(200).collect(),
    });
    if let Err(e) = ctx.storage_set(key, &value) {
        ctx.log(
            LogLevel::Warn,
            format!("the key state was not kept ({:?})", e.kind()),
        );
    }
}

/// `docker.status`: green when the engine answers, red otherwise.
pub fn availability_state(available: bool) -> KeyState {
    if available {
        KeyState::new().indicator(Indicator::Ok)
    } else {
        KeyState::new()
            .detail("Unavailable")
            .indicator(Indicator::Error)
    }
}

/// `docker.containers`: how many run; green when any does.
pub fn containers_state(running: u32) -> KeyState {
    KeyState::new()
        .detail(format!("{running} running"))
        .indicator(if running > 0 {
            Indicator::Ok
        } else {
            Indicator::Unknown
        })
}

/// A Compose project: `running` of `total` containers up — green when all
/// are, amber when some are, grey when none are. With `toggle` (up and
/// down keys) also whether the project is up.
pub fn compose_state(running: u32, total: u32, toggle: bool) -> KeyState {
    let indicator = if running == 0 {
        Indicator::Unknown
    } else if running >= total {
        Indicator::Ok
    } else {
        Indicator::Warning
    };
    let state = KeyState::new()
        .detail(format!("{running}/{total} up"))
        .indicator(indicator);
    if toggle {
        state.toggle(running > 0 && running >= total)
    } else {
        state
    }
}

fn recall(ctx: &mut Context<'_>, key: &str) -> Result<KeyState, PluginError> {
    Ok(match ctx.storage_get_as::<Stored>(key)? {
        None => KeyState::new(),
        Some(Stored::Available(available)) => availability_state(available),
        Some(Stored::Running(running)) => containers_state(running),
        Some(Stored::Compose { running, total }) => compose_state(running, total, false),
        Some(Stored::Failed { code, message }) => return Err(PluginError::new(code, message)),
    })
}

pub fn status(ctx: &mut Context<'_>, args: &Args<'_>) -> Result<KeyState, PluginError> {
    recall(ctx, &status_key(args))
}

pub fn containers(ctx: &mut Context<'_>, args: &Args<'_>) -> Result<KeyState, PluginError> {
    recall(ctx, &containers_key(args))
}

pub fn compose_status(ctx: &mut Context<'_>, args: &Args<'_>) -> Result<KeyState, PluginError> {
    recall(ctx, &compose_key(args))
}
