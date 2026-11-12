//! `deckpilot-dev validation`: helpers for the reproducible update test
//! (`docs/release-validation.md`). They act on an isolated data directory,
//! never on `.dev` or the real configuration, and do what a user would do
//! in the app: set up a configuration, unlock it, look at the agent.

use std::path::{Path, PathBuf};

use deckpilot_ipc::{AgentClient, AgentPaths, ClientKind, KeyMaterial, Request, Response};
use deckpilot_storage::{ConfigStore as _, DatabaseKey, Encryption, SqliteStore};
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
    seed::seed(&paths, seed::MissingDatabase::CreateWithDevPassword).map_err(|e| e.to_string())?;
    let key = seed::dev_key(&paths)
        .ok_or("the configuration does not open with the development password")?;
    let store = SqliteStore::open(dir.join("deckpilot.db"), &Encryption::Key(key))
        .map_err(|e| e.to_string())?;
    update_automatically(&store)?;
    println!(
        "seeded {} (password protected, updates: install when idle)",
        dir.display()
    );
    Ok(())
}

/// The key of the running, unlocked agent (as the configuration app gets it).
fn agent_key(dir: &Path) -> Result<DatabaseKey, String> {
    match runtime().block_on(request(dir, Request::GetDatabaseKey))? {
        Response::DatabaseKey { key } => {
            DatabaseKey::from_hex(key.expose()).map_err(|e| e.to_string())
        }
        other => Err(format!("unexpected {other:?}")),
    }
}

/// Automatic updates ("install when idle"), so that an update test build
/// updates without any clicks.
fn update_automatically(store: &SqliteStore) -> Result<(), String> {
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
        .map_err(|e| e.to_string())
}

/// `populate <dir>`: fills the configuration the running agent created
/// (with a system-managed key) with the demonstration profiles and
/// automatic updates, using the key from the agent.
fn populate(dir: &Path) -> Result<(), String> {
    let key = agent_key(dir)?;
    seed::seed_with_key(&DevPaths::isolated(dir), key.clone()).map_err(|e| e.to_string())?;
    let store = SqliteStore::open(dir.join("deckpilot.db"), &Encryption::Key(key))
        .map_err(|e| e.to_string())?;
    update_automatically(&store)?;
    runtime().block_on(request(dir, Request::ReloadConfiguration))?;
    println!("populated {} through the agent", dir.display());
    Ok(())
}

/// `inventory <dir>`: what the configuration contains, read with the key
/// from the agent.
fn inventory(dir: &Path) -> Result<(), String> {
    let store = SqliteStore::open(dir.join("deckpilot.db"), &Encryption::Key(agent_key(dir)?))
        .map_err(|e| e.to_string())?;
    let (mut projects, mut actions, mut workflows, mut layouts) = (0, 0, 0, 0);
    let summaries = store.list_profiles().map_err(|e| e.to_string())?;
    for summary in &summaries {
        if let Some(profile) = store.load_profile(summary.id).map_err(|e| e.to_string())? {
            projects += profile.projects.len();
            actions += profile.actions.len();
            workflows += profile.workflows.len();
            layouts += profile.layouts.len();
        }
    }
    let protection = deckpilot_storage::vault::Vault::new(dir.join("deckpilot.db"))
        .protection()
        .map_err(|e| e.to_string())?
        .map(|p| format!("{:?}", p.mode()));
    println!(
        "profiles={} projects={projects} actions={actions} workflows={workflows} layouts={layouts} protection={}",
        summaries.len(),
        protection.unwrap_or_else(|| "-".into())
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
        "version={} pid={} security={:?} protection={:?} waiting_for_key_store={} profiles={} active_profile={} active_project={}",
        status.version,
        status.pid,
        status.security,
        status.protection,
        status.waiting_for_key_store,
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
        "populate" => populate(&dir),
        "inventory" => inventory(&dir),
        "unlock" => unlock(&dir),
        "status" => status(&dir),
        other => Err(format!("unknown validation command `{other}`")),
    }
}
