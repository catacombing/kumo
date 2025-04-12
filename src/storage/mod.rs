//! Bulk data storage.

use std::rc::Rc;
use std::{fs, mem};

use rusqlite::{Connection, OptionalExtension, Transaction};
use tracing::error;

use crate::Error;
use crate::storage::cookie_whitelist::CookieWhitelist;
use crate::storage::groups::Groups;
use crate::storage::history::History;
use crate::storage::session::Session;

pub mod cookie_whitelist;
pub mod groups;
pub mod history;
pub mod session;

/// Persistent data storage.
pub struct Storage {
    pub cookie_whitelist: CookieWhitelist,
    pub history: History,
    pub session: Session,
    pub groups: Groups,
}

impl Storage {
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn new() -> Result<Self, Error> {
        // Connect to database and get the current migration state.
        let (mut connection, version) = match Self::open_db() {
            Some(connection) => {
                // Create version table if it does not exist.
                let version = Self::version(&connection)?;
                if version == DbVersion::Zero {
                    Self::create_version(&connection)?;
                }

                (Some(connection), version)
            },
            None => (None, DbVersion::Zero),
        };

        // Migrate database to the latest version.
        if let Some(connection) = &mut connection {
            let transaction = connection.transaction()?;

            // Run table migrations.
            cookie_whitelist::run_migrations(&transaction, version)?;
            history::run_migrations(&transaction, version)?;
            session::run_migrations(&transaction, version)?;
            groups::run_migrations(&transaction, version)?;

            // Update the version itself.
            Self::update_version(&transaction, version)?;

            transaction.commit()?;
        }

        let connection = connection.map(Rc::new);
        let cookie_whitelist = CookieWhitelist::new(connection.clone());
        let history = History::new(connection.clone())?;
        let session = Session::new(connection.clone());
        let groups = Groups::new(connection.clone());

        Ok(Self { cookie_whitelist, history, session, groups })
    }

    /// Attempt to create or access the SQLite database.
    fn open_db() -> Option<Connection> {
        let db_path = match dirs::data_dir() {
            Some(data_dir) => data_dir.join("kumo/default/storage.sqlite"),
            None => return None,
        };

        // Ensure necessary directories exist.
        if let Some(dir) = db_path.parent() {
            let _ = fs::create_dir_all(dir);
        }

        // Attempt to create or access the database.
        match Connection::open(&db_path) {
            Ok(connection) => Some(connection),
            Err(err) => {
                error!("Failed to open database at {db_path:?}: {err}");
                None
            },
        }
    }

    /// Get database version.
    fn version(connection: &Connection) -> Result<DbVersion, Error> {
        let load_version = || {
            let mut statement = connection.prepare("SELECT version FROM db_version").ok()?;
            statement.query_row([], |row| row.get(0)).optional().ok()?
        };

        let version = load_version().unwrap_or_default();
        let latest_version = DbVersion::Last as u8 - 1;

        // Report an error if the DB version is newer than expected.
        if version <= latest_version {
            Ok(unsafe { mem::transmute::<u8, DbVersion>(version) })
        } else {
            Err(Error::UnknownDbVersion(version, latest_version))
        }
    }

    /// Create database migration version table.
    fn create_version(connection: &Connection) -> Result<(), Error> {
        connection.execute(
            "CREATE TABLE IF NOT EXISTS db_version (
                version INTEGER NOT NULL PRIMARY KEY,
                timestamp INTEGER NOT NULL
            )",
            [],
        )?;
        connection.execute(
            "INSERT INTO db_version (version, timestamp) VALUES (0, unixepoch()) ON CONFLICT \
             (version) DO UPDATE SET version=0, timestamp=unixepoch()",
            [],
        )?;
        Ok(())
    }

    /// Update migration version to the latest version.
    fn update_version(transaction: &Transaction, version: DbVersion) -> Result<(), Error> {
        // Nothing to do if we're already on the latest version.
        let latest_version = DbVersion::Last as u8 - 1;
        if version as u8 >= latest_version {
            return Ok(());
        }

        transaction
            .execute("UPDATE db_version SET version=?1, timestamp=unixepoch()", [latest_version])?;

        Ok(())
    }
}

/// Database migration version.
///
/// This version is used to detect the current database state and automatically
/// migrate tables when required.
#[repr(u8)]
#[derive(PartialEq, Eq, Copy, Clone, Default, Debug)]
pub enum DbVersion {
    /// Any version before versioning was introduced.
    #[default]
    Zero,
    /// First versioned database.
    _One,

    /// SAFETY: Must be last field, with no gap in variant values before it.
    ///
    /// The value automatically assigned is used to ensure transmutes are safe.
    Last,
}
