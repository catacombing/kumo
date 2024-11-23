//! Tab groups DB storage.

use std::collections::HashSet;
use std::rc::Rc;

use rusqlite::{params, Connection as SqliteConnection, OptionalExtension};
use tracing::error;
use uuid::Uuid;

use crate::engine::Group;

/// Tab groups.
#[derive(Clone)]
pub struct Groups {
    db: Option<GroupsDb>,
}

impl Groups {
    pub fn new(db: Option<Rc<SqliteConnection>>) -> rusqlite::Result<Self> {
        let db = match db {
            Some(db) => Some(GroupsDb::new(db)?),
            None => None,
        };

        Ok(Self { db })
    }

    /// Get a group's config from its ID.
    pub fn group_by_id(&self, uuid: Uuid) -> Option<Group> {
        let db = match &self.db {
            Some(db) => db,
            None => return None,
        };

        match db.group_by_id(uuid) {
            Ok(group) => group,
            Err(err) => {
                error!("Failed load all groups: {err}");
                None
            },
        }
    }

    /// Update the known tab groups.
    pub fn persist<'a>(&self, groups: impl IntoIterator<Item = &'a Group>) {
        let db = match &self.db {
            Some(db) => db,
            None => return,
        };

        // Store all groups.
        if let Err(err) = db.persist(groups) {
            error!("Failed to save groups: {err}");
        }

        // Nuke groups which aren't used by any session.
        self.delete_orphans();
    }

    /// Delete groups not used by any session.
    pub fn delete_orphans(&self) {
        let db = match &self.db {
            Some(db) => db,
            None => return,
        };

        if let Err(err) = db.delete_orphans() {
            error!("Failed to delete unused groups: {err}");
        }
    }

    /// Get all known group IDs.
    pub fn all_group_ids(&self) -> HashSet<Uuid> {
        let db = match &self.db {
            Some(db) => db,
            None => return HashSet::new(),
        };

        match db.all_group_ids() {
            Ok(groups) => groups,
            Err(err) => {
                error!("Failed load all groups: {err}");
                HashSet::new()
            },
        }
    }
}

/// DB for persisting group data.
#[derive(Clone)]
struct GroupsDb {
    connection: Rc<SqliteConnection>,
}

impl GroupsDb {
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn new(connection: Rc<SqliteConnection>) -> rusqlite::Result<Self> {
        // Setup table if it doesn't exist yet.
        connection.execute(
            "CREATE TABLE IF NOT EXISTS tab_group (
                id BLOB NOT NULL PRIMARY KEY,
                label TEXT NOT NULL
            )",
            [],
        )?;

        Ok(Self { connection })
    }

    /// Get a group's details from its ID.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn group_by_id(&self, uuid: Uuid) -> rusqlite::Result<Option<Group>> {
        let mut statement =
            self.connection.prepare("SELECT id, label FROM tab_group WHERE id = ?1")?;
        let group = statement
            .query_row([uuid], |row| Ok(Group::with_uuid(uuid, row.get(1)?, false)))
            .optional()?;
        Ok(group)
    }

    /// Insert or update a list of groups.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn persist<'a>(&self, groups: impl IntoIterator<Item = &'a Group>) -> rusqlite::Result<()> {
        let mut stmt = self.connection.prepare(
            "INSERT INTO tab_group (id, label) VALUES (?1, ?2) ON CONFLICT (id) DO UPDATE SET \
             label = excluded.label",
        )?;
        for group in groups {
            stmt.execute(params![group.id().uuid(), group.label])?;
        }
        Ok(())
    }

    /// Delete unused groups.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn delete_orphans(&self) -> rusqlite::Result<()> {
        self.connection
            .execute("DELETE FROM tab_group WHERE id NOT IN (SELECT group_id FROM session)", [])?;
        Ok(())
    }

    /// Get group IDs for all PIDs and windows.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn all_group_ids(&self) -> rusqlite::Result<HashSet<Uuid>> {
        let mut statement = self.connection.prepare("SELECT id FROM tab_group")?;
        let groups = statement.query_map([], |row| row.get(0))?.flatten().collect();
        Ok(groups)
    }
}
