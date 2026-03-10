//! Host-based browser engine preference.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::RwLock;

use rusqlite::{Connection as SqliteConnection, Transaction};
use tracing::error;
use url::Url;

use crate::engine::EngineType;
use crate::storage::DbVersion;

/// Host-based browser engine preference.
#[derive(Clone)]
pub struct EnginePreference {
    preferences: Rc<RwLock<HashMap<String, EngineType>>>,
    #[cfg_attr(not(all(feature = "servo", feature = "webkit")), expect(unused))]
    db: Option<EnginePreferenceDb>,
}

impl EnginePreference {
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn new(connection: Option<Rc<SqliteConnection>>) -> Self {
        let (db, preferences) = match connection {
            Some(connection) => {
                let db = EnginePreferenceDb::new(connection);
                let preferences = db.preferences().unwrap_or_default();
                (Some(db), Rc::new(RwLock::new(preferences)))
            },
            None => (None, Default::default()),
        };

        Self { preferences, db }
    }

    /// Get engine preference for a host.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn get(&self, url: &str) -> Option<EngineType> {
        let url = Url::parse(url).ok()?;
        let host = url.host_str()?;

        self.preferences.read().unwrap().get(host).copied()
    }

    /// Add a new engine preference for a host.
    #[cfg(all(feature = "servo", feature = "webkit"))]
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn set(&self, url: &str, engine_type: EngineType) {
        // Ignore URLs without valid host.
        let url = Url::parse(url).ok();
        let host = match url.as_ref().and_then(|url| url.host_str()) {
            Some(host) => host,
            None => return,
        };

        // Update the database.
        if let Some(db) = &self.db {
            db.set(host, engine_type)
        };

        // Update the cache.
        let mut preferences = self.preferences.write().unwrap();
        preferences.insert(host.to_string(), engine_type);
    }
}

/// Host-based browser engine preference.
#[derive(Clone)]
struct EnginePreferenceDb {
    connection: Rc<SqliteConnection>,
}

impl EnginePreferenceDb {
    fn new(connection: Rc<SqliteConnection>) -> Self {
        Self { connection }
    }

    /// Get all current host engine preferences.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn preferences(&self) -> Option<HashMap<String, EngineType>> {
        let mut stmt = self
            .connection
            .prepare("SELECT host, engine_type FROM engine_preference")
            .inspect_err(|err| error!("Failed to prepare SQL query: {err}"))
            .ok()?;

        let preferences = stmt
            .query_map([], |row| {
                let host = row.get(0)?;
                let engine_type = row.get(1)?;
                Ok((host, engine_type))
            })
            .inspect_err(|err| error!("Failed to get engine preferences: {err}"))
            .ok()?
            .flatten()
            .collect();

        Some(preferences)
    }

    /// Update the preferred engine for a host.
    #[cfg(all(feature = "servo", feature = "webkit"))]
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn set(&self, host: &str, engine_type: EngineType) {
        let mut stmt = match self.connection.prepare_cached(
            "INSERT INTO engine_preference (host, engine_type) VALUES (?1, ?2) ON CONFLICT(host) \
             DO UPDATE SET engine_type = ?2",
        ) {
            Ok(stmt) => stmt,
            Err(err) => {
                error!("Failed to prepare SQL query: {err}");
                return;
            },
        };

        let _ = stmt
            .execute((host, engine_type))
            .inspect_err(|err| error!("Failed to set engine preference: {err}"));
    }
}

/// Run database migrations inside a transaction.
pub fn run_migrations(
    transaction: &Transaction<'_>,
    db_version: DbVersion,
) -> rusqlite::Result<()> {
    // Create table if it doesn't exist yet.
    if db_version < DbVersion::Three {
        transaction.execute(
            "CREATE TABLE IF NOT EXISTS engine_preference (
                host TEXT NOT NULL,
                engine_type TEXT NOT NULL,
                UNIQUE(host)
            )",
            [],
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use tempfile::NamedTempFile;

    use super::*;

    #[cfg(all(feature = "servo", feature = "webkit"))]
    #[test]
    fn cached_preference() {
        let engine_preference = EnginePreference::new(None);

        assert_eq!(engine_preference.get("https://example.org"), None);

        engine_preference.set("https://example.org", EngineType::Servo);
        assert_eq!(engine_preference.get("https://example.org"), Some(EngineType::Servo));

        engine_preference.set("https://example.org", EngineType::WebKit);
        assert_eq!(engine_preference.get("https://example.org"), Some(EngineType::WebKit));
    }

    #[test]
    fn sqlite_preference_load() {
        // Prepare test database.
        let db_file = NamedTempFile::new().unwrap();
        let mut connection = Connection::open(&db_file).unwrap();
        let transaction = connection.transaction().unwrap();
        run_migrations(&transaction, DbVersion::Zero).unwrap();
        transaction.commit().unwrap();

        // Manually insert initial engine preferences.
        connection
            .execute(
                "INSERT INTO engine_preference (host, engine_type) VALUES ('example.org', \
                 'servo'), ('catacombing.org', 'webkit')",
                [],
            )
            .unwrap();

        // Ensure preferences are loaded correctly.
        let engine_preference = EnginePreference::new(Some(Rc::new(connection)));
        assert_eq!(engine_preference.get("https://example.org"), Some(EngineType::Servo));
        assert_eq!(engine_preference.get("https://catacombing.org"), Some(EngineType::WebKit));
        assert_eq!(engine_preference.get("https://alacritty.org"), None);
    }

    #[cfg(all(feature = "servo", feature = "webkit"))]
    #[test]
    fn update_through_cache() {
        // Prepare test database.
        let db_file = NamedTempFile::new().unwrap();
        let mut connection = Connection::open(&db_file).unwrap();
        let transaction = connection.transaction().unwrap();
        run_migrations(&transaction, DbVersion::Zero).unwrap();
        transaction.commit().unwrap();

        // Engine type is initially undefined.
        let connection = Rc::new(connection);
        let engine_preference = EnginePreference::new(Some(connection.clone()));
        assert_eq!(engine_preference.get("https://example.org"), None);

        // Update the cached value.
        engine_preference.set("https://example.org", EngineType::Servo);
        assert_eq!(engine_preference.get("https://example.org"), Some(EngineType::Servo));

        // Data is persisted to the database.
        let engine_type: EngineType = connection
            .query_one(
                "SELECT engine_type FROM engine_preference WHERE host = 'example.org'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(engine_type, EngineType::Servo);
    }
}
