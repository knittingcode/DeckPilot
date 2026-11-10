//! Security settings chosen by the user. They live in the encrypted
//! database, so they can only be read and changed while it is unlocked.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{ConfigStore, Result};

const AUTO_LOCK: &str = "security.autoLock";
const REAUTHENTICATION: &str = "security.reauthentication";

/// When the agent locks the configuration by itself.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AutoLock {
    #[default]
    Never,
    /// When the computer goes to sleep.
    Sleep,
    /// After 15 minutes without activity.
    Idle15Minutes,
    /// After one hour without activity.
    Idle1Hour,
}

impl AutoLock {
    pub fn idle_after(self) -> Option<Duration> {
        match self {
            Self::Idle15Minutes => Some(Duration::from_secs(15 * 60)),
            Self::Idle1Hour => Some(Duration::from_secs(60 * 60)),
            Self::Never | Self::Sleep => None,
        }
    }
}

/// When the user has to authenticate again although the configuration is
/// open. Sensitive operations always require it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reauthentication {
    #[default]
    SensitiveOnly,
    /// Also once after each update.
    AfterUpdate,
    /// Also every time the configuration app starts.
    EveryLaunch,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityPolicy {
    pub auto_lock: AutoLock,
    pub reauthentication: Reauthentication,
}

/// Unknown values (from a newer version) fall back to the default.
fn read<T: Default + for<'de> Deserialize<'de>>(value: Option<String>) -> T {
    value
        .and_then(|value| serde_json::from_value(serde_json::Value::String(value)).ok())
        .unwrap_or_default()
}

fn name<T: Serialize>(value: T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => name,
        _ => String::new(),
    }
}

pub fn security_policy(store: &impl ConfigStore) -> Result<SecurityPolicy> {
    Ok(SecurityPolicy {
        auto_lock: read(store.setting(AUTO_LOCK)?),
        reauthentication: read(store.setting(REAUTHENTICATION)?),
    })
}

pub fn set_security_policy(store: &impl ConfigStore, policy: &SecurityPolicy) -> Result<()> {
    store.set_setting(AUTO_LOCK, &name(policy.auto_lock))?;
    store.set_setting(REAUTHENTICATION, &name(policy.reauthentication))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SqliteStore;

    #[test]
    fn policies_round_trip_and_default_safely() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert_eq!(security_policy(&store).unwrap(), SecurityPolicy::default());
        let policy = SecurityPolicy {
            auto_lock: AutoLock::Idle15Minutes,
            reauthentication: Reauthentication::EveryLaunch,
        };
        set_security_policy(&store, &policy).unwrap();
        assert_eq!(security_policy(&store).unwrap(), policy);
        store.set_setting(AUTO_LOCK, "someFutureValue").unwrap();
        assert_eq!(security_policy(&store).unwrap().auto_lock, AutoLock::Never);
    }
}
