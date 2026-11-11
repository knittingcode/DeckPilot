//! Credentials stored on this computer (schema 7).
//!
//! Secrets live only in this encrypted database, in their own table: they
//! are not part of profiles, machine configuration or module settings, so
//! no export contains them. Listing returns names and descriptions only.

use rusqlite::{Connection, OptionalExtension, params};

use deckpilot_core::{CredentialId, CredentialInfo, MemoryCredentials, Secret};

use crate::Result;

pub(crate) const SCHEMA: &str = r"
    CREATE TABLE credentials (
        id          TEXT PRIMARY KEY NOT NULL,
        description TEXT NOT NULL DEFAULT '',
        secret      TEXT NOT NULL,
        updated_at  INTEGER NOT NULL
    );
";

/// Longest secret accepted, in bytes.
pub const MAX_SECRET_BYTES: usize = 16 * 1024;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

fn to_sql(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

pub(crate) fn list(connection: &Connection) -> Result<Vec<CredentialInfo>> {
    let mut statement =
        connection.prepare("SELECT id, description, updated_at FROM credentials ORDER BY id")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let mut infos = Vec::new();
    for row in rows {
        let (id, description, updated_at) = row?;
        // Rows that no longer validate are skipped rather than failing.
        if let Ok(id) = CredentialId::new(id) {
            infos.push(CredentialInfo {
                id,
                description,
                updated_at: u64::try_from(updated_at).unwrap_or_default(),
            });
        }
    }
    Ok(infos)
}

/// Stores or replaces a credential. An empty `secret` keeps the stored one
/// and only changes the description.
pub(crate) fn set(
    connection: &Connection,
    id: &CredentialId,
    description: &str,
    secret: Option<&Secret>,
) -> Result<()> {
    let description: String = description.chars().take(200).collect();
    match secret {
        Some(secret) => {
            connection.execute(
                "INSERT INTO credentials (id, description, secret, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (id) DO UPDATE SET
                     description = excluded.description,
                     secret = excluded.secret,
                     updated_at = excluded.updated_at",
                params![id.as_str(), description, secret.expose(), to_sql(now_ms())],
            )?;
        }
        None => {
            connection.execute(
                "UPDATE credentials SET description = ?2, updated_at = ?3 WHERE id = ?1",
                params![id.as_str(), description, to_sql(now_ms())],
            )?;
        }
    }
    Ok(())
}

pub(crate) fn delete(connection: &Connection, id: &CredentialId) -> Result<bool> {
    Ok(connection.execute(
        "DELETE FROM credentials WHERE id = ?1",
        params![id.as_str()],
    )? > 0)
}

pub(crate) fn exists(connection: &Connection, id: &CredentialId) -> Result<bool> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM credentials WHERE id = ?1",
            params![id.as_str()],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Every credential with its secret, for the engine.
pub(crate) fn load(connection: &Connection) -> Result<MemoryCredentials> {
    let mut statement = connection.prepare("SELECT id, secret FROM credentials")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut credentials = MemoryCredentials::new();
    for row in rows {
        let (id, secret) = row?;
        if let Ok(id) = CredentialId::new(id) {
            credentials.insert(id, Secret::new(secret));
        }
    }
    Ok(credentials)
}

#[cfg(test)]
mod tests {
    use deckpilot_core::CredentialSource;

    use crate::SqliteStore;

    use super::*;

    #[test]
    fn credentials_are_listed_without_secrets_and_replaced() {
        let store = SqliteStore::open_in_memory().unwrap();
        let id = CredentialId::new("github.token").unwrap();
        store
            .set_credential(&id, "CI token", Some(&Secret::new("ghp_one")))
            .unwrap();
        let listed = store.credentials().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].description, "CI token");
        assert!(!serde_json::to_string(&listed).unwrap().contains("ghp_one"));
        assert_eq!(
            store.load_credentials().unwrap().secret(&id),
            Some(Secret::new("ghp_one"))
        );

        // A new description alone keeps the secret; a new secret replaces it.
        store.set_credential(&id, "Renamed", None).unwrap();
        assert_eq!(
            store.load_credentials().unwrap().secret(&id),
            Some(Secret::new("ghp_one"))
        );
        store
            .set_credential(&id, "Renamed", Some(&Secret::new("ghp_two")))
            .unwrap();
        assert_eq!(
            store.load_credentials().unwrap().secret(&id),
            Some(Secret::new("ghp_two"))
        );
        assert!(
            store
                .set_credential(&id, "", Some(&Secret::new("")))
                .is_err()
        );
        let unknown = CredentialId::new("unknown").unwrap();
        assert!(store.set_credential(&unknown, "x", None).is_err());

        assert!(store.delete_credential(&id).unwrap());
        assert!(!store.delete_credential(&id).unwrap());
        assert_eq!(store.credentials().unwrap(), []);
    }
}
