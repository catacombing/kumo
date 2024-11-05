//! Bulk data storage.

use std::fs;
use std::rc::Rc;

use rusqlite::Connection;
use tracing::error;

use crate::storage::cookie_whitelist::CookieWhitelist;
use crate::storage::history::History;
use crate::storage::session::Session;
use crate::Error;

pub mod cookie_whitelist;
pub mod history;
pub mod session;

/// Persistent data storage.
pub struct Storage {
    pub cookie_whitelist: CookieWhitelist,
    pub history: History,
    pub session: Session,
}

impl Storage {
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn new() -> Result<Self, Error> {
        let connection = Self::open_db();
        let cookie_whitelist = CookieWhitelist::new(connection.clone())?;
        let history = History::new(connection.clone())?;
        let session = Session::new(connection.clone())?;

        Ok(Self { cookie_whitelist, history, session })
    }

    /// Attempt to create or access the SQLite database.
    fn open_db() -> Option<Rc<Connection>> {
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
            Ok(connection) => Some(Rc::new(connection)),
            Err(err) => {
                error!("Failed to open database at {db_path:?}: {err}");
                None
            },
        }
    }
}
