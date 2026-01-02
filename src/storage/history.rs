//! Browser history DB storage.

use std::borrow::Cow;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::RwLock;

use rusqlite::types::Type;
use rusqlite::{Connection as SqliteConnection, Transaction};
use smallvec::SmallVec;
use tracing::error;

use crate::Error;
use crate::storage::DbVersion;

/// Maximum scored history matches compared.
pub const MAX_MATCHES: usize = 25;

/// Browser history.
#[derive(Clone)]
pub struct History {
    entries: Rc<RwLock<HashMap<HistoryUri, HistoryEntry>>>,
    db: Option<HistoryDb>,
}

impl History {
    pub fn new(db: Option<Rc<SqliteConnection>>) -> rusqlite::Result<Self> {
        let (db, entries) = match db {
            Some(db) => {
                let db = HistoryDb::new(db);
                let entries = db.load()?;
                (Some(db), Rc::new(RwLock::new(entries)))
            },
            None => (None, Default::default()),
        };

        Ok(Self { entries, db })
    }

    /// Increment URI visit count for history.
    pub fn visit(&self, uri: &str) {
        let mut history_uri = match HistoryUri::new(uri, false) {
            Some(history_uri) => history_uri,
            // Ignore invalid URIs.
            None => return,
        };
        history_uri.normalize();

        // Update filesystem history.
        if let Some(db) = &self.db {
            let normalized_uri = history_uri.to_string(true);
            if let Err(err) = db.visit(&normalized_uri) {
                error!("Failed to write URI {normalized_uri:?} to history: {err}");
            }
        }

        // Update in-memory history.
        let mut entries = self.entries.write().unwrap();
        let history = entries.entry(history_uri).or_default();
        history.views += 1;
    }

    /// Set the title for a URI.
    pub fn set_title(&self, uri: &str, title: String) {
        let mut history_uri = match HistoryUri::new(uri, false) {
            Some(history_uri) => history_uri,
            // Ignore invalid URIs.
            None => return,
        };
        history_uri.normalize();

        // Update filesystem history.
        if let Some(db) = &self.db {
            let normalized_uri = history_uri.to_string(true);
            if let Err(err) = db.set_title(&normalized_uri, &title) {
                error!("Failed to write title for {normalized_uri:?} to history: {err}");
            }
        }

        // Update in-memory history.
        let mut entries = self.entries.write().unwrap();
        if let Some(history) = entries.get_mut(&history_uri) {
            history.title = title;
        }
    }

    /// Delete an entry from the history ( ͡° ͜ʖ ͡°).
    pub fn delete(&self, uri: &str) {
        let mut history_uri = match HistoryUri::new(uri, false) {
            Some(history_uri) => history_uri,
            // Ignore invalid URIs.
            None => return,
        };
        history_uri.normalize();

        // Update filesystem history.
        if let Some(db) = &self.db {
            let normalized_uri = history_uri.to_string(true);
            if let Err(err) = db.delete(&normalized_uri) {
                error!("Failed to delete {normalized_uri:?} from history: {err}");
            }
        }

        // Update in-memory history.
        let mut entries = self.entries.write().unwrap();
        entries.retain(|key, _| key != &history_uri);
    }

    /// Bulk delete history entries.
    pub fn bulk_delete(&self, filter: Option<&str>) {
        // Update filesystem history.
        if let Some(db) = &self.db
            && let Err(err) = db.bulk_delete(filter)
        {
            error!("Failed to delete items matching {filter:?} from history: {err}");
        }

        // Update in-memory history.
        let mut entries = self.entries.write().unwrap();
        match filter {
            Some(filter) => {
                entries.retain(|uri, entry| {
                    !uri.to_string(true).contains(filter) && !entry.title.contains(filter)
                });
            },
            None => entries.clear(),
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
        let input_uri = HistoryUri::new(input, true)?;

        // Find matching URI with most views.
        let entries = self.entries.read().unwrap();
        let (uri, _) = entries
            .iter()
            .filter(|(uri, _)| uri.autocomplete(&input_uri))
            .max_by_key(|(_, entry)| entry.views)?;
        let mut uri = Cow::Borrowed(uri);

        // Strip `www` from suggestion if input did not contain it.
        if !input_uri.base.starts_with("www.") && !"www".starts_with(&input_uri.base) {
            let mut new_uri = HistoryUri::clone(&uri);
            if let Some(base) = uri.base.strip_prefix("www.") {
                new_uri.base = base.into();
            }
            uri = Cow::Owned(new_uri);
        }

        let mut uri_str = uri.to_string(!input_uri.scheme.is_empty());

        // Add trailing slash for base-only URIs, to help future autocompletions.
        if uri.path.is_empty() {
            uri_str.push('/');
        }

        Some(uri_str)
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

    /// Get history entries sorted by their last access timestamp.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn entries(&self) -> rusqlite::Result<Vec<(HistoryUri, HistoryEntry)>> {
        match &self.db {
            Some(db) => db.load(),
            None => Ok(Vec::new()),
        }
    }
}

/// DB for persisting history data.
#[derive(Clone)]
struct HistoryDb {
    connection: Rc<SqliteConnection>,
}

impl HistoryDb {
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn new(connection: Rc<SqliteConnection>) -> Self {
        Self { connection }
    }

    /// Load history from file.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn load<T>(&self) -> rusqlite::Result<T>
    where
        T: FromIterator<(HistoryUri, HistoryEntry)>,
    {
        let mut statement = self.connection.prepare(
            "SELECT uri, title, views, last_access FROM history ORDER BY last_access DESC",
        )?;
        let history = statement
            .query_map([], |row| {
                let uri: String = row.get(0)?;
                let title: String = row.get(1)?;
                let views: i32 = row.get(2)?;
                let last_access: i64 = row.get(3)?;
                let history_uri = HistoryUri::new(&uri, false).ok_or_else(|| {
                    let err = Box::new(Error::InvalidDatabaseType);
                    rusqlite::Error::FromSqlConversionFailure(0, Type::Text, err)
                })?;
                Ok((history_uri, HistoryEntry { title, views: views as u32, last_access }))
            })?
            .flatten()
            .collect();

        Ok(history)
    }

    /// Increment visits for a page.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn visit(&self, uri: &str) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT INTO history (uri, last_access) VALUES (?1, unixepoch())
                ON CONFLICT (uri) DO UPDATE SET views=views+1, last_access=unixepoch()",
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

    /// Delete a URI from the history.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn delete(&self, uri: &str) -> rusqlite::Result<()> {
        self.connection.execute("DELETE FROM history WHERE uri=?1", [uri])?;

        Ok(())
    }

    /// Bulk delete URIs from history.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn bulk_delete(&self, filter: Option<&str>) -> rusqlite::Result<()> {
        match filter {
            Some(filter) => {
                let filter = format!("%{filter}%");
                self.connection
                    .execute("DELETE FROM history WHERE uri like ?1 OR title like ?1", [filter])?;
            },
            None => {
                self.connection.execute("DELETE FROM history", [])?;
            },
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
#[derive(Clone, Default, Debug)]
pub struct HistoryEntry {
    pub title: String,
    pub views: u32,
    pub last_access: i64,
}

/// URI split into scheme, base, and path.
#[derive(Clone, Hash, Eq, PartialEq, Default, Debug)]
pub struct HistoryUri {
    /// Scheme without trailing colons or slashes.
    scheme: String,
    base: String,
    /// Path segments without query parameters.
    path: Vec<String>,
}

impl HistoryUri {
    fn new(mut uri: &str, autocomplete: bool) -> Option<Self> {
        // For autocomplete without a scheme, we just split base and path.
        if autocomplete && !uri.is_empty() && !uri.contains(':') {
            let mut split = uri.split('/');
            let base = split.next().unwrap().to_string();
            let path = split.map(String::from).collect();
            return Some(HistoryUri { scheme: String::new(), base, path });
        }

        // Handle URIs shorter than the smallest scheme.
        if uri.len() < 5 {
            return None;
        }

        match &uri[..4] {
            "http" | "abou" => {
                // Remove query parameters.
                if let Some(index) = uri.rfind('?') {
                    uri = &uri[..index];
                }

                // Extract scheme.
                let (scheme, mut uri) = uri.split_once(':').unwrap_or(("", uri));
                uri = uri.trim_start_matches('/');

                // Extract base.
                let mut split = uri.split('/');
                let base = split.next().unwrap().to_string();
                if !autocomplete && base.is_empty() {
                    return None;
                }

                // Collect path segments.
                let path = split.map(String::from).collect();

                Some(Self { base, path, scheme: scheme.into() })
            },
            "file" => {
                let path = uri.strip_prefix("file://").or_else(|| uri.strip_prefix("file:"))?;

                let mut split = path.split('/');
                let base = split.next().unwrap().to_string();
                let path = split.map(String::from).collect();

                Some(HistoryUri { scheme: "file".into(), base, path })
            },
            // Data URIs should never be stored in history.
            "data" => None,
            _ => None,
        }
    }

    /// Convert the URI back to its string representation.
    pub fn to_string(&self, include_scheme: bool) -> String {
        // Calculate the maximum possible length for allocation purposes.
        let path_len: usize = self.path.iter().map(|path| path.len() + "/".len()).sum();
        let max_len = self.scheme.len() + "://".len() + self.base.len() + "/".len() + path_len;
        let mut uri = String::with_capacity(max_len);

        if include_scheme {
            uri.push_str(&self.scheme);
            uri.push_str("://");
        }

        uri.push_str(&self.base);

        for segment in &self.path {
            uri.push('/');
            uri.push_str(segment);
        }

        uri
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

        // Ignore `www` subdomain since it is frequently optional.
        let base = if !input_uri.base.starts_with("www.") && !"www".starts_with(&input_uri.base) {
            self.base.strip_prefix("www.").unwrap_or(&self.base)
        } else {
            &self.base
        };

        // Check if input is submatch of base without any path segments.
        if base != input_uri.base {
            return input_uri.path.is_empty() && base.starts_with(&input_uri.base);
        }

        // Abort if input is longer than URI.
        let input_path_len = input_uri.path.len();
        if self.path.len() < input_path_len {
            return false;
        }

        // Check for difference in path segments.
        let ignore_case = input_uri.is_lowercase();
        for (i, (segment, input_segment)) in self.path.iter().zip(&input_uri.path).enumerate() {
            // Allow case mismatch with fully lowercased input URI.
            let segment = if ignore_case {
                Cow::Owned(segment.to_lowercase())
            } else {
                Cow::Borrowed(segment)
            };

            if !segment.starts_with(input_segment)
                || (*input_segment != *segment && i + 1 != input_path_len)
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

    /// Check if all characters are lowercase.
    ///
    /// Ambiguous characters that are neither upper- nor lowercase are
    /// considered lowercase.
    fn is_lowercase(&self) -> bool {
        self.scheme.chars().all(|c| !c.is_uppercase())
            && self.base.chars().all(|c| !c.is_uppercase())
            && self.path.iter().flat_map(|p| p.chars()).all(|c| !c.is_uppercase())
    }
}

/// Run database migrations inside a transaction.
pub fn run_migrations(
    transaction: &Transaction<'_>,
    db_version: DbVersion,
) -> rusqlite::Result<()> {
    match db_version {
        // Create table if it doesn't exist yet.
        DbVersion::Zero => {
            let _ = transaction.execute(
                "CREATE TABLE IF NOT EXISTS history (
                    uri TEXT NOT NULL PRIMARY KEY,
                    title TEXT DEFAULT '',
                    views INTEGER NOT NULL DEFAULT 1,
                    last_access INTEGER NOT NULL
                )",
                [],
            )?;
        },
        // Delete all file/data URIs, since they were persisted incorrectly.
        DbVersion::One => {
            let _ = transaction.execute("DELETE FROM history WHERE uri LIKE 'file:%'", [])?;
            let _ = transaction.execute("DELETE FROM history WHERE uri LIKE 'data:%'", [])?;
        },
        _ => (),
    }

    Ok(())
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

        let uri = HistoryUri::new("example.org", false);
        assert_eq!(uri, None);

        let uri = HistoryUri::new("example.org", true).unwrap();
        let expected = build_uri("", "example.org", &[]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("example.org/path", true).unwrap();
        let expected = build_uri("", "example.org", &["path"]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https:", false);
        assert_eq!(uri, None);

        let uri = HistoryUri::new("https://", false);
        assert_eq!(uri, None);

        let uri = HistoryUri::new("https:", true).unwrap();
        let expected = build_uri("https", "", &[]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https:/", true).unwrap();
        let expected = build_uri("https", "", &[]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https://", true).unwrap();
        let expected = build_uri("https", "", &[]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https://example.org", false).unwrap();
        let expected = build_uri("https", "example.org", &[]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https://example.org/", false).unwrap();
        let expected = build_uri("https", "example.org", &[""]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https://example.org/path", false).unwrap();
        let expected = build_uri("https", "example.org", &["path"]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https://example.org/path/", false).unwrap();
        let expected = build_uri("https", "example.org", &["path", ""]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https://example.org/path/segments", false).unwrap();
        let expected = build_uri("https", "example.org", &["path", "segments"]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https://example.org/path/segments?query=a", false).unwrap();
        let expected = build_uri("https", "example.org", &["path", "segments"]);
        assert_eq!(uri, expected);

        let uri =
            HistoryUri::new("https://example.org/path/segments?query=a&other=b", false).unwrap();
        let expected = build_uri("https", "example.org", &["path", "segments"]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https://example.org//", false).unwrap();
        let expected = build_uri("https", "example.org", &["", ""]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("https://example.org/path//segment", false).unwrap();
        let expected = build_uri("https", "example.org", &["path", "", "segment"]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("about:blank", false).unwrap();
        let expected = build_uri("about", "blank", &[]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("file:relative/path", false).unwrap();
        let expected = build_uri("file", "relative", &["path"]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("file://relative/path", false).unwrap();
        let expected = build_uri("file", "relative", &["path"]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("file:/absolute/path", false).unwrap();
        let expected = build_uri("file", "", &["absolute", "path"]);
        assert_eq!(uri, expected);

        let uri = HistoryUri::new("file:///absolute/path", false).unwrap();
        let expected = build_uri("file", "", &["absolute", "path"]);
        assert_eq!(uri, expected);
    }

    #[test]
    fn normalize_normalized_uri() {
        let mut uri = HistoryUri::new("https://example.org", false).unwrap();
        uri.normalize();
        assert_eq!(uri, uri);

        let mut uri = HistoryUri::new("https://example.org/test/ing", false).unwrap();
        uri.normalize();
        assert_eq!(uri, uri);
    }

    #[test]
    fn normalize_trailing_slash() {
        let mut uri = HistoryUri::new("https://example.org/", false).unwrap();
        uri.normalize();
        assert_eq!(uri, HistoryUri::new("https://example.org", false).unwrap());

        let mut uri = HistoryUri::new("https://example.org/test/", false).unwrap();
        uri.normalize();
        assert_eq!(uri, HistoryUri::new("https://example.org/test", false).unwrap());
    }

    #[test]
    fn normalize_multi_slash() {
        let mut uri = HistoryUri::new("https://example.org//", false).unwrap();
        uri.normalize();
        assert_eq!(uri, HistoryUri::new("https://example.org", false).unwrap());

        let mut uri = HistoryUri::new("https://example.org/test//", false).unwrap();
        uri.normalize();
        assert_eq!(uri, HistoryUri::new("https://example.org/test", false).unwrap());

        let mut uri = HistoryUri::new("https://example.org/test///ing", false).unwrap();
        uri.normalize();
        assert_eq!(uri, HistoryUri::new("https://example.org/test/ing", false).unwrap());
    }

    #[test]
    fn file_uris() {
        let mut uri = HistoryUri::new("file:///home/user", false).unwrap();
        uri.normalize();
        assert_eq!(uri.to_string(true), "file:///home/user");
        assert!(uri.base.is_empty());

        let mut uri = HistoryUri::new("file:///home/user/", false).unwrap();
        uri.normalize();
        assert_eq!(uri.to_string(true), "file:///home/user");
        assert!(uri.base.is_empty());

        let mut uri = HistoryUri::new("file:/home/user/", false).unwrap();
        uri.normalize();
        assert_eq!(uri.to_string(true), "file:///home/user");
        assert!(uri.base.is_empty());

        let mut uri = HistoryUri::new("file://relative/path", false).unwrap();
        uri.normalize();
        assert_eq!(uri.to_string(true), "file://relative/path");
        assert_eq!(uri.base, "relative");

        let mut uri = HistoryUri::new("file:relative/path/", false).unwrap();
        uri.normalize();
        assert_eq!(uri.to_string(true), "file://relative/path");
        assert_eq!(uri.base, "relative");

        let mut uri = HistoryUri::new("file:///some/wei?rd/path", false).unwrap();
        uri.normalize();
        assert_eq!(uri.to_string(true), "file:///some/wei?rd/path");
        assert!(uri.base.is_empty());
    }

    #[test]
    fn data_uris() {
        let uri = HistoryUri::new("data:image/jpeg;base64,/9j/4AAQSkZJRgABAgAAZABkAAD", false);
        assert_eq!(uri, None);
    }

    #[test]
    fn history_uri_autocomplete() {
        let uri = HistoryUri::new("https://example.org/path/segments/xxx", false).unwrap();
        assert!(!autocomplete(&uri, "https://example.org/path/segments/xxx/longer"));
        assert!(!autocomplete(&uri, "https://example.org/path/segments/xxxlonger"));
        assert!(!autocomplete(&uri, "https://example.org/path/segments/xxx/"));
        assert!(!autocomplete(&uri, "https://example.org/path/segments/xxx"));
        assert!(!autocomplete(&uri, "https://example.org/path/seg/xxx"));
        assert!(autocomplete(&uri, "https://example.org/path/segments"));
        assert!(autocomplete(&uri, "https://example.org/"));
        assert!(autocomplete(&uri, "https://example.org"));
        assert!(!autocomplete(&uri, "http://example.org"));
        assert!(autocomplete(&uri, "example.org"));
        assert!(autocomplete(&uri, "example"));
        assert!(!autocomplete(&uri, "org"));
        assert!(autocomplete(&uri, "example.org/path/segments"));
        assert!(!autocomplete(&uri, "other.org/p"));
        assert!(!autocomplete(&uri, "example.org/path/segmen/"));
        assert!(!autocomplete(&uri, "example.org/path//segment"));
        assert!(!autocomplete(&uri, "example.org//"));
    }

    #[test]
    fn subdomain_autocomplete() {
        let uri = HistoryUri::new("https://www.example.org/one/two/three", false).unwrap();
        assert!(!autocomplete(&uri, "ww."));
        assert!(autocomplete(&uri, "ww"));

        let uri = HistoryUri::new("https://catacomb.example.org/one/two/three", false).unwrap();
        assert!(!autocomplete(&uri, "ca."));
        assert!(autocomplete(&uri, "ca"));
    }

    #[test]
    fn www_subdomain_autocomplete() {
        let uri = HistoryUri::new("https://www.example.org/one/two/three", false).unwrap();
        assert!(autocomplete(&uri, "https://www.example.org/"));
        assert!(autocomplete(&uri, "https://example.org/"));
        assert!(autocomplete(&uri, "example.org/on"));
        assert!(autocomplete(&uri, "example.org/"));
        assert!(autocomplete(&uri, "example.org"));
    }

    #[test]
    fn ignore_case_autocomplete() {
        let uri = HistoryUri::new("https://example.org/One/Two/Three", false).unwrap();
        assert!(autocomplete(&uri, "example.org/one/two/thre"));
        assert!(autocomplete(&uri, "example.org/one/two"));
        assert!(autocomplete(&uri, "example.org/one/"));

        assert!(!autocomplete(&uri, "example.org/One/tw"));
    }

    #[test]
    fn no_empty_autocomplete() {
        let uri = HistoryUri::new("", true);
        assert_eq!(uri, None);
    }

    #[test]
    fn autocomplete_scheme_without_slashes() {
        let uri = HistoryUri::new("about://blank", false).unwrap();
        assert!(autocomplete(&uri, "about:blan"));

        let uri = HistoryUri::new("https://example.org", false).unwrap();
        assert!(autocomplete(&uri, "https:example"));
    }

    fn autocomplete(uri: &HistoryUri, input: &str) -> bool {
        let input_uri = HistoryUri::new(input, true).unwrap();
        uri.autocomplete(&input_uri)
    }
}
