//! `deckpilot-dev validation`: helpers for the reproducible update test
//! (`docs/release-validation.md`). They act on an isolated data directory,
//! never on `.dev` or the real configuration, and do what a user would do
//! in the app: set up a configuration, unlock it, look at the agent.

use std::path::{Path, PathBuf};

use deckpilot_ipc::{AgentClient, AgentPaths, ClientKind, KeyMaterial, Request, Response};
use deckpilot_storage::{ConfigStore as _, Encryption, SqliteStore};
use deckpilot_update::policy::{PREFERENCES_SETTING, UpdateMode, UpdatePreferences};

use crate::seed;
use crate::session::DevPaths;

fn data_dir(args: &[String]) -> Result<PathBuf, String> {
    let dir = PathBuf::from(args.first().ok_or("expected a data directory")?);
    let production = AgentPaths::resolve_default().map(|p| p.data_dir).ok();
    if production.is_some_and(|p| p == dir) {
        return Err("refusing to use the real DeckPilot data directory".into());
    }
    Ok(dir)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

async fn request(dir: &Path, request: Request) -> Result<Response, String> {
    let mut client = AgentClient::connect(&AgentPaths::in_dir(dir), ClientKind::Other)
        .await
        .map_err(|e| e.to_string())?;
    client.request(request).await.map_err(|e| e.to_string())
}

/// `seed <dir>`: a password-protected configuration (development password)
/// with the demonstration profiles and automatic updates ("install when
/// idle"), so that an update test build updates without any clicks.
fn seed(dir: &Path) -> Result<(), String> {
    let paths = DevPaths::isolated(dir);
    seed::seed(&paths, true).map_err(|e| e.to_string())?;
    let key = seed::dev_key(&paths)
        .ok_or("the configuration does not open with the development password")?;
    let store = SqliteStore::open(dir.join("deckpilot.db"), &Encryption::Key(key))
        .map_err(|e| e.to_string())?;
    let preferences = UpdatePreferences {
        mode: UpdateMode::InstallWhenIdle,
        automatic_checks: true,
        ..UpdatePreferences::default()
    };
    store
        .set_setting(
            PREFERENCES_SETTING,
            &serde_json::to_string(&preferences).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    println!(
        "seeded {} (password protected, updates: install when idle)",
        dir.display()
    );
    Ok(())
}

/// `unlock <dir>`: unlocks the agent with the development password, as a
/// user would by typing it.
fn unlock(dir: &Path) -> Result<(), String> {
    let key = seed::dev_key(&DevPaths::isolated(dir))
        .ok_or("the configuration does not open with the development password")?;
    runtime().block_on(request(
        dir,
        Request::Unlock {
            key: KeyMaterial::new(key.to_hex()),
        },
    ))?;
    println!("agent unlocked");
    Ok(())
}

/// `status <dir>`: one line with what the update test compares.
fn status(dir: &Path) -> Result<(), String> {
    let Response::Status { status } = runtime().block_on(request(dir, Request::GetStatus))? else {
        return Err("unexpected response".into());
    };
    println!(
        "version={} pid={} security={:?} profiles={} active_profile={} active_project={}",
        status.version,
        status.pid,
        status.security,
        status.profiles.len(),
        status.active_profile.map_or_else(|| "-".into(), |p| p.name),
        status
            .active_project
            .map_or_else(|| "-".into(), |p| p.key.to_string()),
    );
    Ok(())
}

pub fn command(args: &[String]) -> Result<(), String> {
    let (name, rest) = args
        .split_first()
        .ok_or("expected seed, unlock or status")?;
    let dir = data_dir(rest)?;
    match name.as_str() {
        "seed" => seed(&dir),
        "unlock" => unlock(&dir),
        "status" => status(&dir),
        other => Err(format!("unknown validation command `{other}`")),
    }
}
