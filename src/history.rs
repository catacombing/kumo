//! Browser history.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::sync::RwLock;

use rusqlite::Connection as SqliteConnection;
use smallvec::SmallVec;

/// Maximum scored history matches compared.
pub const MAX_MATCHES: usize = 25;

/// Browser history.
#[derive(Clone)]
pub struct History {
    entries: Rc<RwLock<HashMap<HistoryUri, HistoryEntry>>>,
    db: Option<Rc<HistoryDb>>,
}

impl History {
    pub fn new() -> Self {
        // Get storage path, ignoring persistence if it can't be retrieved.
        let data_dir = match dirs::data_dir() {
            Some(data_dir) => data_dir.join("kumo/default/history.sqlite"),
            None => return Self { entries: Default::default(), db: Default::default() },
        };

        let (db, entries) = match HistoryDb::new(&data_dir) {
            Ok(db) => {
                let entries = match db.load() {
                    Ok(entries) => Rc::new(RwLock::new(entries)),
                    Err(err) => {
                        eprintln!("Could not load history: {err}");
                        Default::default()
                    },
                };
                (Some(Rc::new(db)), entries)
            },
            Err(err) => {
                eprintln!("Could not open history DB: {err}");
                (None, Default::default())
            },
        };

        Self { entries, db }
    }

    /// Increment URI visit count for history.
    pub fn visit(&self, uri: String) {
        let history_uri = HistoryUri::new(&uri);

        // Ignore invalid URIs.
        if history_uri.base.is_empty() {
            return;
        }

        // Update filesystem history.
        if let Some(db) = &self.db {
            if let Err(err) = db.visit(&uri) {
                eprintln!("Failed to write URI to history: {err}");
            }
        }

        // Update in-memory history.
        let mut entries = self.entries.write().unwrap();
        let history = entries.entry(history_uri).or_default();
        history.views += 1;
    }

    /// Set the title for a URI.
    pub fn set_title(&self, uri: &str, title: String) {
        let history_uri = HistoryUri::new(uri);

        // Ignore invalid URIs.
        if history_uri.base.is_empty() {
            return;
        }

        // Update filesystem history.
        if let Some(db) = &self.db {
            if let Err(err) = db.set_title(uri, &title) {
                eprintln!("Failed to write title to history: {err}");
            }
        }

        // Update in-memory history.
        let mut entries = self.entries.write().unwrap();
        if let Some(history) = entries.get_mut(&history_uri) {
            history.title = title;
        }
    }

    /// Get autocomplete suggestion for an input.
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
    pub fn matches(&self, input: &str) -> SmallVec<[HistoryMatch; MAX_MATCHES]> {
        // Empty input always results in no matches.
        if input.is_empty() {
            return SmallVec::new();
        }

        // Get up to `MAX_MATCHES` matching URIs.
        let entries = self.entries.read().unwrap();
        let mut matches: SmallVec<_> = entries
            .iter()
            .filter_map(|(uri, entry)| {
                let uri_str = uri.to_string(true);

                // Score match by number of occurences, preferring a match at the start.
                let mut score = uri_str.matches(input).count();
                score += entry.title.matches(input).count();
                if uri.base.starts_with(input) || entry.title.starts_with(input) {
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
}

/// DB for persisting history data.
struct HistoryDb {
    connection: SqliteConnection,
}

impl HistoryDb {
    fn new(path: &Path) -> rusqlite::Result<Self> {
        // Ensure necessary directories exist.
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }

        let connection = SqliteConnection::open(path)?;

        // Setup table if it doesn't exist yet.
        connection.execute(
            "CREATE TABLE IF NOT EXISTS history (
                uri TEXT NOT NULL PRIMARY KEY,
                title TEXT DEFAULT '',
                views INTEGER NOT NULL DEFAULT 1
            )",
            [],
        )?;

        Ok(Self { connection })
    }

    /// Load history from file.
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
    fn visit(&self, uri: &str) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT INTO history (uri) VALUES (?1)
                ON CONFLICT (uri) DO UPDATE SET views=views+1",
            [uri],
        )?;

        Ok(())
    }

    /// Update title for a URI.
    fn set_title(&self, uri: &str, title: &str) -> rusqlite::Result<()> {
        self.connection.execute("UPDATE history SET title=?1 WHERE uri=?2", [title, uri])?;

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
        uri = uri.trim_end_matches('/');

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_uri_parsing() {
        let expected =
            HistoryUri { scheme: "".into(), base: "example.org".into(), path: Vec::new() };
        let uri = HistoryUri::new("example.org");
        assert_eq!(uri, expected);

        let expected =
            HistoryUri { scheme: "".into(), base: "example.org".into(), path: vec!["path".into()] };
        let uri = HistoryUri::new("example.org/path");
        assert_eq!(uri, expected);

        let expected = HistoryUri { scheme: "https".into(), base: "".into(), path: Vec::new() };
        let uri = HistoryUri::new("https:");
        assert_eq!(uri, expected);

        let expected = HistoryUri { scheme: "https".into(), base: "".into(), path: Vec::new() };
        let uri = HistoryUri::new("https:/");
        assert_eq!(uri, expected);

        let expected = HistoryUri { scheme: "https".into(), base: "".into(), path: Vec::new() };
        let uri = HistoryUri::new("https://");
        assert_eq!(uri, expected);

        let expected =
            HistoryUri { scheme: "https".into(), base: "example.org".into(), path: Vec::new() };
        let uri = HistoryUri::new("https://example.org");
        assert_eq!(uri, expected);

        let expected =
            HistoryUri { scheme: "https".into(), base: "example.org".into(), path: Vec::new() };
        let uri = HistoryUri::new("https://example.org/");
        assert_eq!(uri, expected);

        let expected = HistoryUri {
            scheme: "https".into(),
            base: "example.org".into(),
            path: vec!["path".into()],
        };
        let uri = HistoryUri::new("https://example.org/path");
        assert_eq!(uri, expected);

        let expected = HistoryUri {
            scheme: "https".into(),
            base: "example.org".into(),
            path: vec!["path".into()],
        };
        let uri = HistoryUri::new("https://example.org/path/");
        assert_eq!(uri, expected);

        let expected = HistoryUri {
            scheme: "https".into(),
            base: "example.org".into(),
            path: vec!["path".into(), "segments".into()],
        };
        let uri = HistoryUri::new("https://example.org/path/segments");
        assert_eq!(uri, expected);

        let expected = HistoryUri {
            scheme: "https".into(),
            base: "example.org".into(),
            path: vec!["path".into(), "segments".into()],
        };
        let uri = HistoryUri::new("https://example.org/path/segments?query=a");
        assert_eq!(uri, expected);

        let expected = HistoryUri {
            scheme: "https".into(),
            base: "example.org".into(),
            path: vec!["path".into(), "segments".into()],
        };
        let uri = HistoryUri::new("https://example.org/path/segments?query=a&other=b");
        assert_eq!(uri, expected);
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
    }
}
