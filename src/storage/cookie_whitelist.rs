//! Allowed cookie hosts.

use std::rc::Rc;

use rusqlite::{Connection as SqliteConnection, Transaction};
use tracing::error;

use crate::storage::DbVersion;
use crate::Error;

/// Cookie persistance exceptions.
#[derive(Clone)]
pub struct CookieWhitelist {
    connection: Option<Rc<SqliteConnection>>,
}

impl CookieWhitelist {
    pub fn new(connection: Option<Rc<SqliteConnection>>) -> Self {
        Self { connection }
    }

    /// Get all allowed hosts.
    pub fn hosts(&self) -> Vec<String> {
        let connection = match &self.connection {
            Some(connection) => connection,
            None => return Vec::new(),
        };

        log_errors(|| {
            let mut statement = connection.prepare("SELECT host FROM cookie_exceptions")?;
            let hosts = statement.query_map([], |row| row.get::<_, String>(0))?.flatten().collect();
            Ok(hosts)
        })
        .unwrap_or_default()
    }

    /// Check whether a host's cookies will be persisted.
    pub fn contains(&self, host: &str) -> bool {
        let connection = match &self.connection {
            Some(connection) => connection,
            None => return false,
        };

        log_errors(|| {
            let mut statement =
                connection.prepare("SELECT 1 FROM cookie_exceptions WHERE host = ?1 LIMIT 1")?;
            let exists = statement.exists([host])?;
            Ok(exists)
        })
        .unwrap_or_default()
    }

    /// Add a host to the whitelist.
    pub fn add(&self, host: &str) {
        let connection = match &self.connection {
            Some(connection) => connection,
            None => return,
        };

        let _ = log_errors(|| {
            connection.execute(
                "INSERT INTO cookie_exceptions (host) VALUES (?1) ON CONFLICT(host) DO NOTHING",
                [host],
            )?;
            Ok(())
        });
    }

    /// Remove a host from the whitelist.
    pub fn remove(&self, host: &str) {
        let connection = match &self.connection {
            Some(connection) => connection,
            None => return,
        };

        let _ = log_errors(|| {
            connection.execute("DELETE FROM cookie_exceptions WHERE host = ?1", [host])?;
            Ok(())
        });
    }
}

/// Log errors returned by a function.
fn log_errors<F, T>(f: F) -> Result<T, Error>
where
    F: Fn() -> Result<T, Error>,
{
    let result = f();
    if let Err(err) = &result {
        error!("cookie whiteliste database error: {err}");
    }
    result
}

/// Run database migrations inside a transaction.
pub fn run_migrations(
    transaction: &Transaction<'_>,
    db_version: DbVersion,
) -> rusqlite::Result<()> {
    // Create table if it doesn't exist yet.
    if db_version == DbVersion::Zero {
        transaction.execute(
            "CREATE TABLE IF NOT EXISTS cookie_exceptions (
                host TEXT NOT NULL,
                UNIQUE(host)
            )",
            [],
        )?;
    }

    Ok(())
}
