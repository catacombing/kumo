//! Browser session DB storage.

use std::collections::HashMap;
use std::rc::Rc;
use std::{fs, process};

use rusqlite::{Connection as SqliteConnection, Transaction, params};
use tracing::error;
use uuid::Uuid;

use crate::engine::{Engine, Group};
use crate::storage::DbVersion;
use crate::window::WindowId;

/// Browser session storage.
#[derive(Clone)]
pub struct Session {
    db: Option<SessionDb>,
    pid: u32,
}

impl Session {
    pub fn new(db: Option<Rc<SqliteConnection>>) -> Self {
        let db = db.map(SessionDb::new);
        Self { db, pid: process::id() }
    }

    /// Update the browser session for a window.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn persist<'a, S>(&self, window_id: WindowId, session: S)
    where
        S: IntoIterator<Item = SessionRecord<'a>>,
    {
        let db = match &self.db {
            Some(db) => db,
            None => return,
        };

        // Write sessions to database.
        if let Err(err) = db.set_session(self.pid, window_id, session) {
            error!("Failed session update: {err}");
        }
    }

    /// Get all sessions not owned by any active process.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn orphans(&self) -> Vec<SessionEntry> {
        let db = match &self.db {
            Some(db) => db,
            None => return Default::default(),
        };

        // Get all sessions from the DB.
        let mut sessions = db
            .sessions()
            .inspect_err(|err| error!("Failed sessions read: {err}"))
            .unwrap_or_default();

        // Remove sessions of currently active Kumo processes.
        let mut known_pids = HashMap::new();
        sessions.retain(|session| {
            // Accept entries if we coincidentally got the same PID again.
            if session.pid == self.pid {
                return true;
            }

            // Short-circuit if we've already probed the PID owning this session.
            if let Some(running) = known_pids.get(&session.pid) {
                return !running;
            }

            // Check if there's a running Kumo process with the session's PID.
            let running = match fs::read_to_string(format!("/proc/{}/cmdline", session.pid)) {
                Ok(cmdline) => cmdline.contains("kumo"),
                Err(_) => false,
            };
            known_pids.insert(session.pid, running);

            !running
        });

        sessions
    }

    /// Delete sessions for orphan PIDs.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn delete_orphans(&self, pids: impl IntoIterator<Item = u32>) {
        let db = match &self.db {
            Some(db) => db,
            None => return,
        };

        if let Err(err) = db.delete_sessions(pids) {
            error!("Failed delete orphan sessions: {err}");
        }
    }
}

/// DB for persisting session data.
#[derive(Clone)]
struct SessionDb {
    connection: Rc<SqliteConnection>,
}

impl SessionDb {
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn new(connection: Rc<SqliteConnection>) -> Self {
        Self { connection }
    }

    /// Update the active browser session for a window.
    fn set_session<'a, S>(&self, pid: u32, window_id: WindowId, session: S) -> rusqlite::Result<()>
    where
        S: IntoIterator<Item = SessionRecord<'a>>,
    {
        let tx = self.connection.unchecked_transaction()?;

        // Delete old session.
        let window_id = window_id.as_raw();
        tx.execute("DELETE FROM session WHERE pid = ?1 AND window_id = ?2", params![
            pid, window_id
        ])?;

        // Save current session.
        let mut session = session.into_iter().peekable();
        if session.peek().is_some() {
            let mut stmt = tx.prepare(
                "INSERT INTO session (pid, window_id, group_id, data, uri, focused) VALUES (?1, \
                 ?2, ?3, ?4, ?5, ?6)",
            )?;
            for entry in session {
                let group_id = entry.group.id().uuid();
                stmt.execute(params![
                    pid,
                    window_id,
                    group_id,
                    entry.data,
                    entry.uri,
                    entry.focused
                ])?;
            }
        }

        tx.commit()?;

        Ok(())
    }

    /// Get all browser sessions,
    fn sessions(&self) -> rusqlite::Result<Vec<SessionEntry>> {
        let mut statement = self
            .connection
            .prepare("SELECT pid, window_id, group_id, data, uri, focused FROM session")?;

        let sessions = statement
            .query_map([], |row| {
                let pid: u32 = row.get(0)?;
                let window_id: usize = row.get(1)?;
                let group_id: Uuid = row.get(2)?;
                let session_data: Vec<u8> = row.get(3)?;
                let uri: String = row.get(4)?;
                let focused: bool = row.get(5)?;

                Ok(SessionEntry { pid, window_id, group_id, session_data, uri, focused })
            })?
            .flatten()
            .collect();

        Ok(sessions)
    }

    /// Delete browser sessions.
    fn delete_sessions(&self, pids: impl IntoIterator<Item = u32>) -> rusqlite::Result<()> {
        let mut stmt = self.connection.prepare("DELETE FROM session WHERE pid = ?1")?;
        for pid in pids {
            stmt.execute([pid])?;
        }
        Ok(())
    }
}

/// Database browser history session entry.
#[derive(Debug)]
pub struct SessionEntry {
    pub pid: u32,
    pub window_id: usize,
    pub group_id: Uuid,
    pub session_data: Vec<u8>,
    pub uri: String,
    pub focused: bool,
}

/// Object for writing sessions to the DB.
pub struct SessionRecord<'a> {
    pub group: &'a Group,
    pub data: Vec<u8>,
    pub uri: String,
    pub focused: bool,
}

impl<'a> SessionRecord<'a> {
    #[allow(clippy::borrowed_box)]
    pub fn new(engine: &Box<dyn Engine>, group: &'a Group, focused: bool) -> Option<Self> {
        // Never persist ephemeral tab groups.
        if group.ephemeral {
            return None;
        }

        Some(Self { group, data: engine.session(), uri: engine.uri().into(), focused })
    }
}

/// Run database migrations inside a transaction.
pub fn run_migrations(
    transaction: &Transaction<'_>,
    db_version: DbVersion,
) -> rusqlite::Result<()> {
    // Create table if it doesn't exist yet.
    if db_version == DbVersion::Zero {
        transaction.execute(
            "CREATE TABLE IF NOT EXISTS session (
                pid INTEGER NOT NULL,
                window_id INTEGER NOT NULL,
                group_id BLOB NOT NULL,
                data BLOB NOT NULL,
                uri TEXT NOT NULL,
                focused INTEGER NOT NULL
            )",
            [],
        )?;
    }

    Ok(())
}
