use std::path::{Path, PathBuf};

use anyhow::Result;
use async_trait::async_trait;

use super::Tool;

/// Resolve a path, expanding ~ and enforcing optional directory restriction.
fn resolve_path(path: &str, allowed_dir: Option<&Path>) -> std::result::Result<PathBuf, String> {
    let expanded = if path.starts_with("~/") || path == "~" {
        dirs::home_dir()
            .map(|h| h.join(path.strip_prefix("~/").unwrap_or("")))
            .unwrap_or_else(|| PathBuf::from(path))
    } else {
        PathBuf::from(path)
    };

    let resolved = expanded
        .canonicalize()
        .unwrap_or_else(|_| std::path::absolute(&expanded).unwrap_or(expanded));

    if let Some(allowed) = allowed_dir {
        let allowed_resolved = allowed
            .canonicalize()
            .unwrap_or_else(|_| allowed.to_path_buf());
        if !resolved.starts_with(&allowed_resolved) {
            return Err(format!(
                "Path {path} is outside allowed directory {}",
                allowed.display()
            ));
        }
    }

    Ok(resolved)
}

/// Resolve a path for write operations — parent must exist or be creatable.
fn resolve_path_for_write(
    path: &str,
    allowed_dir: Option<&Path>,
) -> std::result::Result<PathBuf, String> {
    let expanded = if path.starts_with("~/") || path == "~" {
        dirs::home_dir()
            .map(|h| h.join(path.strip_prefix("~/").unwrap_or("")))
            .unwrap_or_else(|| PathBuf::from(path))
    } else {
        PathBuf::from(path)
    };

    let resolved = std::path::absolute(&expanded).unwrap_or(expanded);

    if let Some(allowed) = allowed_dir {
        let allowed_resolved = allowed
            .canonicalize()
            .unwrap_or_else(|_| allowed.to_path_buf());
        if !resolved.starts_with(&allowed_resolved) {
            return Err(format!(
                "Path {path} is outside allowed directory {}",
                allowed.display()
            ));
        }
    }

    Ok(resolved)
}

// ---------------------------------------------------------------------------
// ReadFileTool
// ---------------------------------------------------------------------------

pub struct ReadFileTool {
    allowed_dir: Option<PathBuf>,
}

impl ReadFileTool {
    pub fn new(allowed_dir: Option<PathBuf>) -> Self {
        Self { allowed_dir }
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a file at the given path."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file path to read"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<String> {
        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: path"))?;

        match resolve_path(path, self.allowed_dir.as_deref()) {
            Ok(file_path) => {
                if !file_path.exists() {
                    return Ok(format!("Error: File not found: {path}"));
                }
                if !file_path.is_file() {
                    return Ok(format!("Error: Not a file: {path}"));
                }
                match std::fs::read_to_string(&file_path) {
                    Ok(content) => {
                        const MAX_LEN: usize = 50_000;
                        if content.len() > MAX_LEN {
                            let mut end = MAX_LEN;
                            while end > 0 && !content.is_char_boundary(end) {
                                end -= 1;
                            }
                            Ok(format!(
                                "{}\n... (truncated, {} more chars)",
                                &content[..end],
                                content.len() - end
                            ))
                        } else {
                            Ok(content)
                        }
                    }
                    Err(e) => Ok(format!("Error reading file: {e}")),
                }
            }
            Err(e) => Ok(format!("Error: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// WriteFileTool
// ---------------------------------------------------------------------------

pub struct WriteFileTool {
    allowed_dir: Option<PathBuf>,
}

impl WriteFileTool {
    pub fn new(allowed_dir: Option<PathBuf>) -> Self {
        Self { allowed_dir }
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file at the given path. Creates parent directories if needed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file path to write to"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write"
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<String> {
        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: path"))?;
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: content"))?;

        match resolve_path_for_write(path, self.allowed_dir.as_deref()) {
            Ok(file_path) => {
                if let Some(parent) = file_path.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        return Ok(format!("Error creating directories: {e}"));
                    }
                }
                match std::fs::write(&file_path, content) {
                    Ok(()) => Ok(format!(
                        "Successfully wrote {} bytes to {path}",
                        content.len()
                    )),
                    Err(e) => Ok(format!("Error writing file: {e}")),
                }
            }
            Err(e) => Ok(format!("Error: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// EditFileTool
// ---------------------------------------------------------------------------

pub struct EditFileTool {
    allowed_dir: Option<PathBuf>,
}

impl EditFileTool {
    pub fn new(allowed_dir: Option<PathBuf>) -> Self {
        Self { allowed_dir }
    }
}

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Edit a file by replacing old_text with new_text. The old_text must exist exactly in the file."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file path to edit"
                },
                "old_text": {
                    "type": "string",
                    "description": "The exact text to find and replace"
                },
                "new_text": {
                    "type": "string",
                    "description": "The text to replace with"
                }
            },
            "required": ["path", "old_text", "new_text"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<String> {
        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: path"))?;
        let old_text = params
            .get("old_text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: old_text"))?;
        let new_text = params
            .get("new_text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: new_text"))?;

        match resolve_path(path, self.allowed_dir.as_deref()) {
            Ok(file_path) => {
                if !file_path.exists() {
                    return Ok(format!("Error: File not found: {path}"));
                }

                let content = match std::fs::read_to_string(&file_path) {
                    Ok(c) => c,
                    Err(e) => return Ok(format!("Error reading file: {e}")),
                };

                if !content.contains(old_text) {
                    return Ok(
                        "Error: old_text not found in file. Make sure it matches exactly."
                            .to_string(),
                    );
                }

                let count = content.matches(old_text).count();
                if count > 1 {
                    return Ok(format!(
                        "Warning: old_text appears {count} times. Please provide more context to make it unique."
                    ));
                }

                let new_content = content.replacen(old_text, new_text, 1);
                match std::fs::write(&file_path, new_content) {
                    Ok(()) => Ok(format!("Successfully edited {path}")),
                    Err(e) => Ok(format!("Error writing file: {e}")),
                }
            }
            Err(e) => Ok(format!("Error: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// ListDirTool
// ---------------------------------------------------------------------------

pub struct ListDirTool {
    allowed_dir: Option<PathBuf>,
}

impl ListDirTool {
    pub fn new(allowed_dir: Option<PathBuf>) -> Self {
        Self { allowed_dir }
    }
}

#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &str {
        "list_dir"
    }

    fn description(&self) -> &str {
        "List the contents of a directory."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The directory path to list"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<String> {
        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: path"))?;

        match resolve_path(path, self.allowed_dir.as_deref()) {
            Ok(dir_path) => {
                if !dir_path.exists() {
                    return Ok(format!("Error: Directory not found: {path}"));
                }
                if !dir_path.is_dir() {
                    return Ok(format!("Error: Not a directory: {path}"));
                }

                let mut entries: Vec<String> = Vec::new();
                match std::fs::read_dir(&dir_path) {
                    Ok(read_dir) => {
                        let mut items: Vec<_> = read_dir.flatten().collect();
                        items.sort_by_key(|e| e.file_name());

                        for item in items {
                            let name = item.file_name().to_string_lossy().to_string();
                            let prefix = if item.path().is_dir() {
                                "[dir]  "
                            } else {
                                "[file] "
                            };
                            entries.push(format!("{prefix}{name}"));
                        }
                    }
                    Err(e) => return Ok(format!("Error listing directory: {e}")),
                }

                if entries.is_empty() {
                    Ok(format!("Directory {path} is empty"))
                } else {
                    Ok(entries.join("\n"))
                }
            }
            Err(e) => Ok(format!("Error: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_read_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "hello world").unwrap();

        let tool = ReadFileTool::new(None);
        let result = tool
            .execute(serde_json::json!({"path": file.to_str().unwrap()}))
            .await
            .unwrap();
        assert_eq!(result, "hello world");
    }

    #[tokio::test]
    async fn test_read_file_not_found() {
        let tool = ReadFileTool::new(None);
        let result = tool
            .execute(serde_json::json!({"path": "/tmp/nonexistent_patina_test_file.txt"}))
            .await
            .unwrap();
        assert!(result.contains("not found"));
    }

    #[tokio::test]
    async fn test_read_file_is_directory() {
        let dir = tempfile::tempdir().unwrap();
        let tool = ReadFileTool::new(None);
        let result = tool
            .execute(serde_json::json!({"path": dir.path().to_str().unwrap()}))
            .await
            .unwrap();
        assert!(result.contains("Not a file"));
    }

    #[tokio::test]
    async fn test_read_file_outside_allowed_dir() {
        let dir = tempfile::tempdir().unwrap();
        let tool = ReadFileTool::new(Some(dir.path().to_path_buf()));
        let result = tool
            .execute(serde_json::json!({"path": "/etc/hostname"}))
            .await
            .unwrap();
        assert!(result.contains("outside allowed directory"));
    }

    #[tokio::test]
    async fn test_write_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("output.txt");

        let tool = WriteFileTool::new(None);
        let result = tool
            .execute(serde_json::json!({
                "path": file.to_str().unwrap(),
                "content": "written content"
            }))
            .await
            .unwrap();

        assert!(result.contains("Successfully wrote"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "written content");
    }

    #[tokio::test]
    async fn test_write_file_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a/b/c/deep.txt");

        let tool = WriteFileTool::new(None);
        tool.execute(serde_json::json!({
            "path": file.to_str().unwrap(),
            "content": "deep"
        }))
        .await
        .unwrap();

        assert_eq!(std::fs::read_to_string(&file).unwrap(), "deep");
    }

    #[tokio::test]
    async fn test_write_file_outside_allowed_dir() {
        let dir = tempfile::tempdir().unwrap();
        let tool = WriteFileTool::new(Some(dir.path().to_path_buf()));
        let result = tool
            .execute(serde_json::json!({
                "path": "/tmp/patina_escape_test.txt",
                "content": "nope"
            }))
            .await
            .unwrap();
        assert!(result.contains("outside allowed directory"));
    }

    #[tokio::test]
    async fn test_edit_file_replace() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("edit.txt");
        std::fs::write(&file, "hello world").unwrap();

        let tool = EditFileTool::new(None);
        let result = tool
            .execute(serde_json::json!({
                "path": file.to_str().unwrap(),
                "old_text": "world",
                "new_text": "rust"
            }))
            .await
            .unwrap();

        assert!(result.contains("Successfully edited"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "hello rust");
    }

    #[tokio::test]
    async fn test_edit_file_not_found_text() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("edit.txt");
        std::fs::write(&file, "hello world").unwrap();

        let tool = EditFileTool::new(None);
        let result = tool
            .execute(serde_json::json!({
                "path": file.to_str().unwrap(),
                "old_text": "nonexistent",
                "new_text": "replacement"
            }))
            .await
            .unwrap();

        assert!(result.contains("old_text not found"));
    }

    #[tokio::test]
    async fn test_edit_file_duplicate_text() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("edit.txt");
        std::fs::write(&file, "foo bar foo baz").unwrap();

        let tool = EditFileTool::new(None);
        let result = tool
            .execute(serde_json::json!({
                "path": file.to_str().unwrap(),
                "old_text": "foo",
                "new_text": "qux"
            }))
            .await
            .unwrap();

        assert!(result.contains("appears 2 times"));
    }

    #[tokio::test]
    async fn test_list_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file_a.txt"), "").unwrap();
        std::fs::write(dir.path().join("file_b.txt"), "").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();

        let tool = ListDirTool::new(None);
        let result = tool
            .execute(serde_json::json!({"path": dir.path().to_str().unwrap()}))
            .await
            .unwrap();

        assert!(result.contains("[file] file_a.txt"));
        assert!(result.contains("[file] file_b.txt"));
        assert!(result.contains("[dir]  subdir"));
    }

    #[tokio::test]
    async fn test_list_dir_empty() {
        let dir = tempfile::tempdir().unwrap();
        let tool = ListDirTool::new(None);
        let result = tool
            .execute(serde_json::json!({"path": dir.path().to_str().unwrap()}))
            .await
            .unwrap();
        assert!(result.contains("is empty"));
    }

    #[tokio::test]
    async fn test_list_dir_not_found() {
        let tool = ListDirTool::new(None);
        let result = tool
            .execute(serde_json::json!({"path": "/tmp/nonexistent_patina_dir_test"}))
            .await
            .unwrap();
        assert!(result.contains("not found"));
    }
}
