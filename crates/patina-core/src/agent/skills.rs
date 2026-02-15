use std::path::{Path, PathBuf};

use regex::Regex;
use tracing::warn;

/// Metadata parsed from a skill's YAML frontmatter.
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub source: SkillSource,
    pub always: bool,
    pub available: bool,
    pub missing_requirements: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SkillSource {
    Workspace,
    Builtin,
}

/// Loads markdown-based skills from workspace and builtin directories.
pub struct SkillsLoader {
    workspace_skills: PathBuf,
    builtin_skills: Option<PathBuf>,
    frontmatter_re: Regex,
}

impl SkillsLoader {
    pub fn new(workspace: &Path, builtin: Option<&Path>) -> Self {
        Self {
            workspace_skills: workspace.join("skills"),
            builtin_skills: builtin.map(|p| p.to_path_buf()),
            frontmatter_re: Regex::new(r"(?s)^---\n(.*?)\n---").unwrap(),
        }
    }

    /// List all available skills with metadata.
    pub fn list_skills(&self) -> Vec<SkillInfo> {
        let mut skills = Vec::new();
        let mut seen_names = std::collections::HashSet::new();

        // Workspace skills (highest priority)
        if self.workspace_skills.exists() {
            self.scan_dir(&self.workspace_skills, SkillSource::Workspace, &mut skills);
            for s in &skills {
                seen_names.insert(s.name.clone());
            }
        }

        // Builtin skills (only if not overridden by workspace)
        if let Some(ref builtin) = self.builtin_skills {
            if builtin.exists() {
                let mut builtin_skills = Vec::new();
                self.scan_dir(builtin, SkillSource::Builtin, &mut builtin_skills);
                for s in builtin_skills {
                    if !seen_names.contains(&s.name) {
                        seen_names.insert(s.name.clone());
                        skills.push(s);
                    }
                }
            }
        }

        skills
    }

    /// Get names of skills marked as `always: true`.
    pub fn get_always_skills(&self) -> Vec<String> {
        self.list_skills()
            .into_iter()
            .filter(|s| s.always && s.available)
            .map(|s| s.name)
            .collect()
    }

    /// Load a skill's full content by name.
    pub fn load_skill(&self, name: &str) -> Option<String> {
        // Try workspace first
        let workspace_path = self.workspace_skills.join(name).join("SKILL.md");
        if workspace_path.exists() {
            return std::fs::read_to_string(&workspace_path).ok();
        }

        // Try builtin
        if let Some(ref builtin) = self.builtin_skills {
            let builtin_path = builtin.join(name).join("SKILL.md");
            if builtin_path.exists() {
                return std::fs::read_to_string(&builtin_path).ok();
            }
        }

        None
    }

    /// Load specific skills for context injection, stripping frontmatter.
    pub fn load_skills_for_context(&self, skill_names: &[String]) -> String {
        let mut parts = Vec::new();
        for name in skill_names {
            if let Some(content) = self.load_skill(name) {
                let body = self.strip_frontmatter(&content);
                parts.push(format!("### Skill: {name}\n\n{body}"));
            }
        }
        parts.join("\n\n---\n\n")
    }

    /// Build an XML summary of all skills for the system prompt.
    pub fn build_skills_summary(&self) -> String {
        let skills = self.list_skills();
        if skills.is_empty() {
            return String::new();
        }

        let mut lines = vec!["<skills>".to_string()];
        for s in &skills {
            if s.always {
                continue; // Always-loaded skills are shown in full, not in summary
            }
            let avail = if s.available { "true" } else { "false" };
            lines.push(format!("  <skill available=\"{avail}\">"));
            lines.push(format!("    <name>{}</name>", xml_escape(&s.name)));
            lines.push(format!(
                "    <description>{}</description>",
                xml_escape(&s.description)
            ));
            lines.push(format!("    <location>{}</location>", s.path.display()));
            if !s.missing_requirements.is_empty() {
                lines.push(format!(
                    "    <requires>{}</requires>",
                    xml_escape(&s.missing_requirements.join(", "))
                ));
            }
            lines.push("  </skill>".to_string());
        }
        lines.push("</skills>".to_string());
        lines.join("\n")
    }

    fn scan_dir(&self, dir: &Path, source: SkillSource, out: &mut Vec<SkillInfo>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let skill_file = path.join("SKILL.md");
            if !skill_file.exists() {
                continue;
            }

            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            let content = match std::fs::read_to_string(&skill_file) {
                Ok(c) => c,
                Err(e) => {
                    warn!("Failed to read skill {name}: {e}");
                    continue;
                }
            };

            let meta = self.parse_frontmatter(&content);
            let description = meta.get("description").cloned().unwrap_or_default();
            let always = meta.get("always").map(|v| v == "true").unwrap_or(false);

            let (available, missing) = self.check_requirements(&meta);

            out.push(SkillInfo {
                name,
                description,
                path: skill_file,
                source: source.clone(),
                always,
                available,
                missing_requirements: missing,
            });
        }
    }

    fn parse_frontmatter(&self, content: &str) -> std::collections::HashMap<String, String> {
        let mut meta = std::collections::HashMap::new();

        if let Some(caps) = self.frontmatter_re.captures(content) {
            let yaml_block = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            for line in yaml_block.lines() {
                if let Some((key, value)) = line.split_once(':') {
                    let key = key.trim().to_string();
                    let value = value
                        .trim()
                        .trim_matches('"')
                        .trim_matches('\'')
                        .to_string();
                    meta.insert(key, value);
                }
            }
        }

        meta
    }

    fn check_requirements(
        &self,
        meta: &std::collections::HashMap<String, String>,
    ) -> (bool, Vec<String>) {
        let mut missing = Vec::new();

        let metadata_str = match meta.get("metadata") {
            Some(s) => s,
            None => return (true, missing),
        };

        let skill_meta: serde_json::Value = match serde_json::from_str(metadata_str) {
            Ok(v) => v,
            Err(_) => return (true, missing),
        };

        // Support both "patina" and "nanobot" metadata keys for backward compatibility
        let requires = match skill_meta
            .get("patina")
            .and_then(|n| n.get("requires"))
            .or_else(|| skill_meta.get("nanobot").and_then(|n| n.get("requires")))
        {
            Some(r) => r,
            None => return (true, missing),
        };

        // Check binary requirements
        if let Some(bins) = requires.get("bins").and_then(|b| b.as_array()) {
            for bin in bins {
                if let Some(bin_name) = bin.as_str() {
                    if which::which(bin_name).is_err() {
                        missing.push(format!("CLI: {bin_name}"));
                    }
                }
            }
        }

        // Check environment variable requirements
        if let Some(envs) = requires.get("env").and_then(|e| e.as_array()) {
            for env in envs {
                if let Some(env_name) = env.as_str() {
                    if std::env::var(env_name).is_err() {
                        missing.push(format!("ENV: {env_name}"));
                    }
                }
            }
        }

        (missing.is_empty(), missing)
    }

    fn strip_frontmatter<'a>(&self, content: &'a str) -> &'a str {
        if let Some(m) = self.frontmatter_re.find(content) {
            content[m.end()..].trim_start()
        } else {
            content
        }
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl std::fmt::Display for SkillSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkillSource::Workspace => write!(f, "workspace"),
            SkillSource::Builtin => write!(f, "builtin"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(base: &Path, name: &str, content: &str) {
        let dir = base.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), content).unwrap();
    }

    #[test]
    fn workspace_overrides_builtin() {
        let workspace = tempfile::tempdir().unwrap();
        let builtin = tempfile::tempdir().unwrap();

        let ws_skills = workspace.path().join("skills");
        std::fs::create_dir_all(&ws_skills).unwrap();

        write_skill(
            &ws_skills,
            "demo",
            "---\nname: demo\ndescription: workspace\n---\nworkspace body",
        );
        write_skill(
            builtin.path(),
            "demo",
            "---\nname: demo\ndescription: builtin\n---\nbuiltin body",
        );

        let loader = SkillsLoader::new(workspace.path(), Some(builtin.path()));
        let skills = loader.list_skills();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "demo");
        assert_eq!(skills[0].source, SkillSource::Workspace);

        let loaded = loader.load_skill("demo").unwrap();
        assert!(loaded.contains("workspace body"));
    }

    #[test]
    fn marks_skill_unavailable_when_requirements_missing() {
        let workspace = tempfile::tempdir().unwrap();
        let ws_skills = workspace.path().join("skills");
        std::fs::create_dir_all(&ws_skills).unwrap();

        write_skill(
            &ws_skills,
            "needs-bin",
            "---\nname: needs-bin\ndescription: test\nmetadata: {\"nanobot\":{\"requires\":{\"bins\":[\"__missing_bin_for_test__\"]}}}\n---\nbody",
        );

        let loader = SkillsLoader::new(workspace.path(), None);
        let skills = loader.list_skills();
        assert_eq!(skills.len(), 1);
        assert!(!skills[0].available);
        assert!(skills[0]
            .missing_requirements
            .iter()
            .any(|r| r.contains("CLI: __missing_bin_for_test__")));
    }
}
