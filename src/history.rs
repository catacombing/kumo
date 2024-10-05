//! Browser history.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::RwLock;
use std::{fs, process};

use rusqlite::{params, Connection as SqliteConnection};
use smallvec::SmallVec;
use tracing::error;

use crate::engine::Engine;
use crate::window::WindowId;

/// Maximum scored history matches compared.
pub const MAX_MATCHES: usize = 25;

/// Browser history.
#[derive(Clone)]
pub struct History {
    entries: Rc<RwLock<HashMap<HistoryUri, HistoryEntry>>>,
    db: Option<Rc<HistoryDb>>,
    pid: u32,
}

impl History {
    pub fn new() -> Self {
        let pid = process::id();

        // Get storage path, ignoring persistence if it can't be retrieved.
        let data_dir = match dirs::data_dir() {
            Some(data_dir) => data_dir.join("kumo/default/history.sqlite"),
            None => return Self { pid, entries: Default::default(), db: Default::default() },
        };

        let (db, entries) = match HistoryDb::new(&data_dir) {
            Ok(db) => {
                let entries = match db.load() {
                    Ok(entries) => Rc::new(RwLock::new(entries)),
                    Err(err) => {
                        error!("Could not load history: {err}");
                        Default::default()
                    },
                };
                (Some(Rc::new(db)), entries)
            },
            Err(err) => {
                error!("Could not open history DB: {err}");
                (None, Default::default())
            },
        };

        Self { entries, pid, db }
    }

    /// Increment URI visit count for history.
    pub fn visit(&self, uri: String) {
        let mut history_uri = HistoryUri::new(&uri);
        history_uri.normalize();

        // Ignore invalid URIs.
        if history_uri.base.is_empty() {
            return;
        }

        // Update filesystem history.
        if let Some(db) = &self.db {
            let normalized_uri = history_uri.to_string(true);
            if let Err(err) = db.visit(&normalized_uri) {
                error!("Failed to write URI to history: {err}");
            }
        }

        // Update in-memory history.
        let mut entries = self.entries.write().unwrap();
        let history = entries.entry(history_uri).or_default();
        history.views += 1;
    }

    /// Set the title for a URI.
    pub fn set_title(&self, uri: &str, title: String) {
        let mut history_uri = HistoryUri::new(uri);
        history_uri.normalize();

        // Ignore invalid URIs.
        if history_uri.base.is_empty() {
            return;
        }

        // Update filesystem history.
        if let Some(db) = &self.db {
            let normalized_uri = history_uri.to_string(true);
            if let Err(err) = db.set_title(&normalized_uri, &title) {
                error!("Failed to write title to history: {err}");
            }
        }

        // Update in-memory history.
        let mut entries = self.entries.write().unwrap();
        if let Some(history) = entries.get_mut(&history_uri) {
            history.title = title;
        }
    }

    /// Get autocomplete suggestion for an input.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn autocomplete(&self, input: &str) -> Option<String> {
        // Question marks suggest query parameters or search engine query, neither has
        // sensible autocomplete suggestions.
        if input.contains('?') {
            return None;
        }

        // Ignore empty input and scheme-only.
        let input_uri = HistoryUri::new(input);
        if input_uri.base.is_empty() {
            return None;
        }

        // Find matching URI with most views.
        let entries = self.entries.read().unwrap();
        let (uri, _) = entries
            .iter()
            .filter(|(uri, _)| uri.autocomplete(&input_uri))
            .max_by_key(|(_, entry)| entry.views)?;

        Some(uri.to_string(!input_uri.scheme.is_empty()))
    }

    /// Get history matches for the input in ascending relevance.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn matches(&self, input: &str) -> SmallVec<[HistoryMatch; MAX_MATCHES]> {
        // Empty input always results in no matches.
        if input.is_empty() {
            return SmallVec::new();
        }

        // Perform case-sensitive search if any uppercase characters are in the query.
        let is_case_sensitive = input.chars().any(|c| c.is_uppercase());

        // Get up to `MAX_MATCHES` matching URIs.
        let entries = self.entries.read().unwrap();
        let mut matches: SmallVec<_> = entries
            .iter()
            .filter_map(|(uri, entry)| {
                let uri_str = uri.to_string(true);

                let mut match_uri = Cow::Borrowed(&uri_str);
                let mut title = Cow::Borrowed(&entry.title);

                // Convert to lowercase for case-insensitive search.
                if !is_case_sensitive {
                    match_uri = Cow::Owned(match_uri.to_lowercase());
                    title = Cow::Owned(title.to_lowercase());
                }

                // Score match by number of occurences, preferring a match at the start.
                let mut score = match_uri.matches(input).count();
                score += title.matches(input).count();
                if uri.base.starts_with(input) || title.starts_with(input) {
                    score += 1_000;
                }

                // Ignore URIs without any match.
                (score != 0).then(|| HistoryMatch {
                    score,
                    title: entry.title.clone(),
                    uri: uri_str,
                })
            })
            .take(MAX_MATCHES)
            .collect();

        // Sort matches based on their score.
        matches.sort_by_key(|m: &HistoryMatch| m.score);

        matches
    }

    /// Update the browser session for a window.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn set_session(&self, window_id: WindowId, session: Vec<SessionRecord>) {
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
    pub fn orphan_sessions(&self) -> Vec<SessionEntry> {
        let db = match &self.db {
            Some(db) => db,
            None => return Vec::new(),
        };

        // Get all sessions from the DB.
        let mut sessions = db
            .sessions()
            .inspect_err(|err| error!("Failed sessions read: {err}"))
            .unwrap_or_default();

        // Remove sessions of currently active Kumo processes.
        let mut known_pids = HashMap::new();
        sessions.retain(|session| {
            // Short-circuit if we've already probed the PID owning this session.
            if let Some(running) = known_pids.get(&session.pid) {
                return !running;
            }

            // Probe PID to check if the session is currently active.
            let running = unsafe { libc::kill(session.pid as i32, 0) } >= 0;
            known_pids.insert(session.pid, running);

            !running
        });

        sessions
    }

    /// Delete sessions for orphan PIDs.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn delete_orphan_sessions(&self, pids: impl IntoIterator<Item = u32>) {
        let db = match &self.db {
            Some(db) => db,
            None => return,
        };

        if let Err(err) = db.delete_sessions(pids) {
            error!("Failed delete orphan sessions: {err}");
        }
    }
}

/// DB for persisting history data.
struct HistoryDb {
    connection: SqliteConnection,
}

impl HistoryDb {
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn new(path: &Path) -> rusqlite::Result<Self> {
        // Ensure necessary directories exist.
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }

        let connection = SqliteConnection::open(path)?;

        // Setup history table if it doesn't exist yet.
        connection.execute(
            "CREATE TABLE IF NOT EXISTS history (
                uri TEXT NOT NULL PRIMARY KEY,
                title TEXT DEFAULT '',
                views INTEGER NOT NULL DEFAULT 1
            )",
            [],
        )?;

        // Setup session table if it doesn't exist yet.
        connection.execute(
            "CREATE TABLE IF NOT EXISTS session (
                pid INTEGER NOT NULL,
                window_id INTEGER NOT NULL,
                data BLOB NOT NULL,
                uri TEXT NOT NULL
            )",
            [],
        )?;

        Ok(Self { connection })
    }

    /// Load history from file.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn load(&self) -> rusqlite::Result<HashMap<HistoryUri, HistoryEntry>> {
        let mut statement = self.connection.prepare("SELECT uri, title, views FROM history")?;
        let history = statement
            .query_map([], |row| {
                let uri: String = row.get(0)?;
                let title: String = row.get(1)?;
                let views: i32 = row.get(2)?;
                Ok((HistoryUri::new(&uri), HistoryEntry { title, views: views as u32 }))
            })?
            .flatten()
            .collect();
        Ok(history)
    }

    /// Increment visits for a page.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn visit(&self, uri: &str) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT INTO history (uri) VALUES (?1)
                ON CONFLICT (uri) DO UPDATE SET views=views+1",
            [uri],
        )?;

        Ok(())
    }

    /// Update title for a URI.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn set_title(&self, uri: &str, title: &str) -> rusqlite::Result<()> {
        self.connection.execute("UPDATE history SET title=?1 WHERE uri=?2", [title, uri])?;

        Ok(())
    }

    /// Update the active browser session for a window.
    fn set_session(
        &self,
        pid: u32,
        window_id: WindowId,
        session: Vec<SessionRecord>,
    ) -> rusqlite::Result<()> {
        let tx = self.connection.unchecked_transaction()?;

        // Delete old session.
        let window_id = window_id.as_raw();
        tx.execute("DELETE FROM session WHERE pid = ?1 AND window_id = ?2", params![
            pid, window_id
        ])?;

        // Save current session.
        if !session.is_empty() {
            let mut stmt = tx.prepare(
                "INSERT INTO session (pid, window_id, data, uri) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for entry in session {
                stmt.execute(params![pid, window_id, entry.data, entry.uri])?;
            }
        }

        tx.commit()?;

        Ok(())
    }

    /// Get all browser sessions,
    fn sessions(&self) -> rusqlite::Result<Vec<SessionEntry>> {
        let mut statement =
            self.connection.prepare("SELECT pid, window_id, data, uri FROM session")?;
        let sessions = statement
            .query_map([], |row| {
                let pid: u32 = row.get(0)?;
                let window_id: usize = row.get(1)?;
                let session_data: Vec<u8> = row.get(2)?;
                let uri: String = row.get(3)?;
                Ok(SessionEntry { pid, window_id, session_data, uri })
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

/// Match for a history query.
pub struct HistoryMatch {
    pub uri: String,
    pub title: String,
    score: usize,
}

/// Single entry in the browser history.
#[derive(Default)]
pub struct HistoryEntry {
    pub title: String,
    pub views: u32,
}

/// URI split into scheme, base, and path.
#[derive(Hash, Eq, PartialEq, Default, Debug)]
struct HistoryUri {
    /// Scheme without trailing colons or slashes.
    scheme: String,
    base: String,
    /// Path segments without query parameters.
    path: Vec<String>,
}

impl HistoryUri {
    fn new(mut uri: &str) -> Self {
        // Remove query parameters.
        if let Some(index) = uri.rfind('?') {
            uri = &uri[..index];
        }

        // Extract scheme.
        let (scheme, mut uri) = uri.split_once(':').unwrap_or(("", uri));
        uri = uri.trim_start_matches('/');

        // Extract base.
        let mut split = uri.split('/');
        let base = split.next().unwrap().into();

        // Collect path segments.
        let path = split.map(String::from).collect();

        Self { base, path, scheme: scheme.into() }
    }

    /// Get autocomplete suggestion for this URI.
    fn autocomplete(&self, input_uri: &HistoryUri) -> bool {
        // Ignore exact matches, since there's nothing to complete.
        if self.base == input_uri.base && self.path == input_uri.path {
            return false;
        }

        // Ensure scheme matches if present.
        if !input_uri.scheme.is_empty() && self.scheme != input_uri.scheme {
            return false;
        }

        // Check if input is submatch of base without any path segments.
        if self.base != input_uri.base {
            return input_uri.path.is_empty() && self.base.starts_with(&input_uri.base);
        }

        // Abort if input is longer than URI.
        let input_path_len = input_uri.path.len();
        if self.path.len() < input_path_len {
            return false;
        }

        // Check for difference in path segments.
        for (i, (segment, input_segment)) in self.path.iter().zip(&input_uri.path).enumerate() {
            if !segment.starts_with(input_segment)
                || (input_segment != segment && i + 1 != input_path_len)
            {
                return false;
            }
        }

        true
    }

    /// Trim redundant path segments.
    ///
    /// This removes path segments like double slashes or trailing slashes to
    /// avoid bloating the history with multiple URIs per target resource.
    fn normalize(&mut self) {
        self.path.retain(|p| !p.is_empty())
    }

    /// Convert the URI back to its string representation.
    fn to_string(&self, include_scheme: bool) -> String {
        // Calculate the maximum possible length for allocation purposes.
        let path_len: usize = self.path.iter().map(|path| path.len() + "/".len()).sum();
        let max_len = self.scheme.len() + "://".len() + self.base.len() + "/".len() + path_len;
        let mut uri = String::with_capacity(max_len);

        if include_scheme {
            uri.push_str(&self.scheme);
            uri.push_str("://");
        }

        uri.push_str(&self.base);

        // Add trailing slash if it's only the base.
        if self.path.is_empty() {
            uri.push('/');
        }

        for segment in &self.path {
            uri.push('/');
            uri.push_str(segment);
        }

        uri
    }
}

impl From<&str> for HistoryUri {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

/// Browser history session entry.
#[derive(Debug)]
pub struct SessionEntry {
    pub pid: u32,
    pub window_id: usize,
    pub session_data: Vec<u8>,
    pub uri: String,
}

/// Object for writing sessions to the DB.
pub struct SessionRecord {
    pub data: Vec<u8>,
    pub uri: String,
}

impl From<&Box<dyn Engine>> for SessionRecord {
    fn from(engine: &Box<dyn Engine>) -> Self {
        Self { data: engine.session(), uri: engine.uri() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_uri_parsing() {
        let build_uri = |scheme: &str, base: &str, path: &[&str]| HistoryUri {
            scheme: scheme.into(),
            base: base.into(),
            path: path.iter().map(|s| String::from(*s)).collect(),
        };

        let uri = HistoryUri::new("example.org");
        let expected = build_uri("", "example.org", &[]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("example.org/path");
        let expected = build_uri("", "example.org", &["path"]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https:");
        let expected = build_uri("https", "", &[]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https:/");
        let expected = build_uri("https", "", &[]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https://");
        let expected = build_uri("https", "", &[]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https://example.org");
        let expected = build_uri("https", "example.org", &[]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https://example.org/");
        let expected = build_uri("https", "example.org", &[""]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https://example.org/path");
        let expected = build_uri("https", "example.org", &["path"]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https://example.org/path/");
        let expected = build_uri("https", "example.org", &["path", ""]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https://example.org/path/segments");
        let expected = build_uri("https", "example.org", &["path", "segments"]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https://example.org/path/segments?query=a");
        let expected = build_uri("https", "example.org", &["path", "segments"]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https://example.org/path/segments?query=a&other=b");
        let expected = build_uri("https", "example.org", &["path", "segments"]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https://example.org//");
        let expected = build_uri("https", "example.org", &["", ""]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https://example.org/path//segment");
        let expected = build_uri("https", "example.org", &["path", "", "segment"]);
        assert_eq!(uri, expected);
    }

    #[test]
    fn normalize_normalized_uri() {
        let mut uri = HistoryUri::new("https://example.org");
        uri.normalize();
        assert_eq!(uri, uri);

        let mut uri = HistoryUri::new("https://example.org/test/ing");
        uri.normalize();
        assert_eq!(uri, uri);
    }

    #[test]
    fn normalize_trailing_slash() {
        let mut uri = HistoryUri::new("https://example.org/");
        uri.normalize();
        assert_eq!(uri, HistoryUri::new("https://example.org"));

        let mut uri = HistoryUri::new("https://example.org/test/");
        uri.normalize();
        assert_eq!(uri, HistoryUri::new("https://example.org/test"));
    }

    #[test]
    fn normalize_multi_slash() {
        let mut uri = HistoryUri::new("https://example.org//");
        uri.normalize();
        assert_eq!(uri, HistoryUri::new("https://example.org"));

        let mut uri = HistoryUri::new("https://example.org/test//");
        uri.normalize();
        assert_eq!(uri, HistoryUri::new("https://example.org/test"));

        let mut uri = HistoryUri::new("https://example.org/test///ing");
        uri.normalize();
        assert_eq!(uri, HistoryUri::new("https://example.org/test/ing"));
    }

    #[test]
    fn history_uri_autocomplete() {
        let uri = HistoryUri::new("https://example.org/path/segments/xxx");
        assert!(!uri.autocomplete(&"https://example.org/path/segments/xxx/longer".into()));
        assert!(!uri.autocomplete(&"https://example.org/path/segments/xxxlonger".into()));
        assert!(!uri.autocomplete(&"https://example.org/path/segments/xxx/".into()));
        assert!(!uri.autocomplete(&"https://example.org/path/segments/xxx".into()));
        assert!(!uri.autocomplete(&"https://example.org/path/seg/xxx".into()));
        assert!(uri.autocomplete(&"https://example.org/path/segments".into()));
        assert!(uri.autocomplete(&"https://example.org/".into()));
        assert!(uri.autocomplete(&"https://example.org".into()));
        assert!(!uri.autocomplete(&"http://example.org".into()));
        assert!(uri.autocomplete(&"example.org".into()));
        assert!(uri.autocomplete(&"example".into()));
        assert!(!uri.autocomplete(&"org".into()));
        assert!(uri.autocomplete(&"example.org/path/segments".into()));
        assert!(!uri.autocomplete(&"other.org/p".into()));
        assert!(!uri.autocomplete(&"example.org/path/segmen/".into()));
        assert!(!uri.autocomplete(&"example.org/path//segment".into()));
        assert!(!uri.autocomplete(&"example.org//".into()));
    }
}
