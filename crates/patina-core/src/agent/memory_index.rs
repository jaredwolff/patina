use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Result;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use tracing::{debug, info, warn};

/// A chunk of text from an indexed file.
struct Chunk {
    text: String,
    start_line: usize,
    end_line: usize,
}

/// A search result from the FTS5 index.
pub struct SearchResult {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
    pub score: f64,
}

/// Full-text search index over workspace markdown files using SQLite FTS5.
///
/// The index is a regenerable cache — flat files remain the source of truth.
/// Database is stored at `~/.patina/memory.sqlite` (safe to delete).
pub struct MemoryIndex {
    conn: Mutex<Connection>,
    workspace: PathBuf,
}

impl MemoryIndex {
    /// Open (or create) the memory index database.
    pub fn new(workspace: &Path, db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        // Create schema
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS files (
                path TEXT PRIMARY KEY,
                hash TEXT NOT NULL,
                mtime INTEGER NOT NULL,
                size INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS chunks (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                hash TEXT NOT NULL,
                text TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_chunks_path ON chunks(path);

            CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
                text,
                content=chunks,
                content_rowid=rowid
            );

            -- Triggers to keep FTS in sync with chunks table
            CREATE TRIGGER IF NOT EXISTS chunks_ai AFTER INSERT ON chunks BEGIN
                INSERT INTO chunks_fts(rowid, text)
                VALUES (new.rowid, new.text);
            END;

            CREATE TRIGGER IF NOT EXISTS chunks_ad AFTER DELETE ON chunks BEGIN
                INSERT INTO chunks_fts(chunks_fts, rowid, text)
                VALUES ('delete', old.rowid, old.text);
            END;

            CREATE TRIGGER IF NOT EXISTS chunks_au AFTER UPDATE ON chunks BEGIN
                INSERT INTO chunks_fts(chunks_fts, rowid, text)
                VALUES ('delete', old.rowid, old.text);
                INSERT INTO chunks_fts(rowid, text)
                VALUES (new.rowid, new.text);
            END;",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
            workspace: workspace.to_path_buf(),
        })
    }

    fn lock_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {e}"))
    }

    /// Reindex all markdown files in the workspace.
    /// Skips unchanged files (by SHA-256 hash). Removes stale entries.
    pub fn reindex(&self) -> Result<()> {
        let pattern = self.workspace.join("**/*.md").to_string_lossy().to_string();

        let paths: Vec<PathBuf> = glob::glob(&pattern)
            .map_err(|e| anyhow::anyhow!("Invalid glob pattern: {e}"))?
            .filter_map(|entry| entry.ok())
            .filter(|p| p.is_file())
            .collect();

        let conn = self.lock_conn()?;

        let mut indexed_paths: Vec<String> = Vec::new();
        let mut changed = 0usize;
        let mut skipped = 0usize;

        for path in &paths {
            let rel_path = path
                .strip_prefix(&self.workspace)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();

            indexed_paths.push(rel_path.clone());

            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(e) => {
                    warn!("Failed to read {}: {e}", path.display());
                    continue;
                }
            };

            let hash = hex_sha256(&content);
            let meta = std::fs::metadata(path).ok();
            let mtime = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let size = meta.map(|m| m.len() as i64).unwrap_or(0);

            // Check if file is unchanged
            let existing_hash: Option<String> = conn
                .query_row(
                    "SELECT hash FROM files WHERE path = ?1",
                    [&rel_path],
                    |row| row.get(0),
                )
                .ok();

            if existing_hash.as_deref() == Some(&hash) {
                skipped += 1;
                continue;
            }

            // File is new or changed — reindex it
            Self::index_file(&conn, &rel_path, &content, &hash, mtime, size)?;
            changed += 1;
        }

        // Remove stale entries (files that no longer exist)
        let mut stmt = conn.prepare("SELECT path FROM files")?;
        let db_paths: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        for db_path in &db_paths {
            if !indexed_paths.contains(db_path) {
                Self::remove_file(&conn, db_path)?;
                debug!("Removed stale index entry: {db_path}");
            }
        }

        info!(
            "Memory reindex: {changed} changed, {skipped} unchanged, {} total files",
            indexed_paths.len()
        );

        Ok(())
    }

    /// Index a single file: chunk its content and insert into the database.
    fn index_file(
        conn: &Connection,
        rel_path: &str,
        content: &str,
        hash: &str,
        mtime: i64,
        size: i64,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();

        // Remove old chunks for this file
        conn.execute("DELETE FROM chunks WHERE path = ?1", [rel_path])?;

        // Update file record
        conn.execute(
            "INSERT OR REPLACE INTO files (path, hash, mtime, size) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![rel_path, hash, mtime, size],
        )?;

        // Chunk and insert
        let chunks = chunk_text(content);
        let mut stmt = conn.prepare(
            "INSERT INTO chunks (id, path, start_line, end_line, hash, text, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;

        for chunk in &chunks {
            let id = uuid::Uuid::new_v4().to_string();
            let chunk_hash = hex_sha256(&chunk.text);
            stmt.execute(rusqlite::params![
                id,
                rel_path,
                chunk.start_line as i64,
                chunk.end_line as i64,
                chunk_hash,
                chunk.text,
                now,
            ])?;
        }

        debug!(
            "Indexed {rel_path}: {} chunks ({} chars)",
            chunks.len(),
            content.len()
        );

        Ok(())
    }

    /// Remove all index entries for a file.
    fn remove_file(conn: &Connection, rel_path: &str) -> Result<()> {
        conn.execute("DELETE FROM chunks WHERE path = ?1", [rel_path])?;
        conn.execute("DELETE FROM files WHERE path = ?1", [rel_path])?;
        Ok(())
    }

    /// Search the index using FTS5 full-text search with BM25 ranking.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }

        let fts_query = build_fts_query(query);
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.lock_conn()?;

        let mut stmt = conn.prepare(
            "SELECT c.path, c.start_line, c.end_line, c.text, rank
             FROM chunks_fts
             JOIN chunks c ON chunks_fts.rowid = c.rowid
             WHERE chunks_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;

        let results = stmt
            .query_map(rusqlite::params![fts_query, limit as i64], |row| {
                Ok(SearchResult {
                    path: row.get(0)?,
                    start_line: row.get::<_, i64>(1)? as usize,
                    end_line: row.get::<_, i64>(2)? as usize,
                    content: row.get(3)?,
                    // FTS5 rank is negative (lower = better), negate for display
                    score: -row.get::<_, f64>(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }

    /// Get chunk count (for testing).
    #[cfg(test)]
    fn chunk_count(&self) -> i64 {
        let conn = self.lock_conn().unwrap();
        conn.query_row("SELECT count(*) FROM chunks", [], |row| row.get(0))
            .unwrap()
    }
}

/// Build an FTS5 query from a user search string.
/// Tokenizes on whitespace, quotes each token, joins with space (implicit AND).
fn build_fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|token| {
            let escaped = token.replace('"', "\"\"");
            format!("\"{escaped}\"")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Split text into overlapping chunks on line boundaries.
/// Target: ~1600 chars per chunk, ~320 chars overlap.
fn chunk_text(text: &str) -> Vec<Chunk> {
    const TARGET_SIZE: usize = 1600;
    const OVERLAP: usize = 320;

    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut start_line = 0usize;

    while start_line < lines.len() {
        let mut char_count = 0usize;
        let mut end_line = start_line;

        // Accumulate lines until we reach the target size
        while end_line < lines.len() {
            let line_len = lines[end_line].len() + 1; // +1 for newline
            if char_count + line_len > TARGET_SIZE && end_line > start_line {
                break;
            }
            char_count += line_len;
            end_line += 1;
        }

        let chunk_text = lines[start_line..end_line].join("\n");
        chunks.push(Chunk {
            text: chunk_text,
            start_line: start_line + 1, // 1-indexed for display
            end_line,                   // 1-indexed (exclusive becomes last line)
        });

        if end_line >= lines.len() {
            break;
        }

        // Find overlap start: walk backwards from end_line until we have ~OVERLAP chars
        let mut overlap_chars = 0usize;
        let mut next_start = end_line;
        while next_start > start_line + 1 {
            next_start -= 1;
            overlap_chars += lines[next_start].len() + 1;
            if overlap_chars >= OVERLAP {
                break;
            }
        }

        // Ensure forward progress
        if next_start <= start_line {
            next_start = end_line;
        }

        start_line = next_start;
    }

    chunks
}

/// Compute the hex-encoded SHA-256 of a string.
fn hex_sha256(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_workspace(dir: &Path) {
        let memory_dir = dir.join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();
        std::fs::write(
            memory_dir.join("MEMORY.md"),
            "# Memory\n\nUser prefers Rust over Python.\nWorkspace is at /home/user/projects.\n",
        )
        .unwrap();
        std::fs::write(
            memory_dir.join("HISTORY.md"),
            "[2025-01-15 10:00] Discussed project architecture and chose SQLite for storage.\n\n\
             [2025-01-16 14:30] Implemented the agent loop with tool calling support.\n\n\
             [2025-01-17 09:00] Added Telegram channel integration with voice transcription.\n",
        )
        .unwrap();
    }

    #[test]
    fn test_chunk_text_small() {
        let text = "line 1\nline 2\nline 3";
        let chunks = chunk_text(text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 3);
        assert_eq!(chunks[0].text, text);
    }

    #[test]
    fn test_chunk_text_large() {
        // Create text larger than TARGET_SIZE
        let lines: Vec<String> = (0..100)
            .map(|i| {
                format!(
                    "This is line number {i} with some padding text to make it longer for testing purposes."
                )
            })
            .collect();
        let text = lines.join("\n");
        let chunks = chunk_text(&text);

        assert!(chunks.len() > 1, "Should produce multiple chunks");

        // Verify all content is covered
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks.last().unwrap().end_line, 100);

        // Verify overlap exists (second chunk starts before first chunk ends)
        if chunks.len() >= 2 {
            assert!(
                chunks[1].start_line < chunks[0].end_line + 1,
                "Chunks should overlap"
            );
        }
    }

    #[test]
    fn test_chunk_text_empty() {
        let chunks = chunk_text("");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_build_fts_query() {
        assert_eq!(build_fts_query("hello world"), "\"hello\" \"world\"");
        assert_eq!(build_fts_query("single"), "\"single\"");
        assert_eq!(build_fts_query(""), "");
        assert_eq!(build_fts_query("   "), "");
    }

    #[test]
    fn test_build_fts_query_quotes() {
        assert_eq!(build_fts_query(r#"say "hello""#), r#""say" """hello""""#);
    }

    #[test]
    fn test_hex_sha256() {
        let hash = hex_sha256("hello");
        assert_eq!(hash.len(), 64);
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn test_new_creates_schema() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.sqlite");
        let index = MemoryIndex::new(dir.path(), &db_path).unwrap();

        let conn = index.lock_conn().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('files', 'chunks')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_reindex_and_search() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace(dir.path());

        let db_path = dir.path().join("memory.sqlite");
        let index = MemoryIndex::new(dir.path(), &db_path).unwrap();
        index.reindex().unwrap();

        // Search for known content
        let results = index.search("SQLite storage", 5).unwrap();
        assert!(!results.is_empty(), "Should find SQLite reference");
        assert!(results[0].path.contains("HISTORY.md"));

        // Search for content in MEMORY.md
        let results = index.search("Rust Python", 5).unwrap();
        assert!(!results.is_empty(), "Should find Rust/Python reference");
    }

    #[test]
    fn test_reindex_skips_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace(dir.path());

        let db_path = dir.path().join("memory.sqlite");
        let index = MemoryIndex::new(dir.path(), &db_path).unwrap();

        index.reindex().unwrap();
        let count1 = index.chunk_count();

        // Second reindex without changes — should produce same count
        index.reindex().unwrap();
        let count2 = index.chunk_count();
        assert_eq!(count1, count2);
    }

    #[test]
    fn test_reindex_removes_deleted_files() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace(dir.path());

        let db_path = dir.path().join("memory.sqlite");
        let index = MemoryIndex::new(dir.path(), &db_path).unwrap();
        index.reindex().unwrap();

        // Delete HISTORY.md
        std::fs::remove_file(dir.path().join("memory/HISTORY.md")).unwrap();
        index.reindex().unwrap();

        // Should no longer find history content
        let results = index.search("SQLite storage", 5).unwrap();
        assert!(results.is_empty(), "Deleted file content should be gone");
    }

    #[test]
    fn test_search_empty_query() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace(dir.path());

        let db_path = dir.path().join("memory.sqlite");
        let index = MemoryIndex::new(dir.path(), &db_path).unwrap();
        index.reindex().unwrap();

        let results = index.search("", 5).unwrap();
        assert!(results.is_empty());

        let results = index.search("   ", 5).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_no_results() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace(dir.path());

        let db_path = dir.path().join("memory.sqlite");
        let index = MemoryIndex::new(dir.path(), &db_path).unwrap();
        index.reindex().unwrap();

        let results = index.search("xyznonexistent", 5).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_reindex_detects_changes() {
        let dir = tempfile::tempdir().unwrap();
        setup_workspace(dir.path());

        let db_path = dir.path().join("memory.sqlite");
        let index = MemoryIndex::new(dir.path(), &db_path).unwrap();
        index.reindex().unwrap();

        // Modify a file
        std::fs::write(
            dir.path().join("memory/MEMORY.md"),
            "# Memory\n\nUser switched to using Go instead of Rust.\n",
        )
        .unwrap();

        index.reindex().unwrap();

        // Should find new content
        let results = index.search("Go", 5).unwrap();
        assert!(!results.is_empty(), "Should find updated content");
    }
}
