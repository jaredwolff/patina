use std::path::PathBuf;

/// Loads markdown-based skills from workspace and builtin directories.
pub struct SkillsLoader {
    workspace_skills: PathBuf,
    builtin_skills: PathBuf,
}

impl SkillsLoader {
    pub fn new(workspace: &std::path::Path, builtin: &std::path::Path) -> Self {
        Self {
            workspace_skills: workspace.join("skills"),
            builtin_skills: builtin.to_path_buf(),
        }
    }

    /// List all available skills with metadata.
    pub fn list_skills(&self) -> Vec<SkillInfo> {
        // TODO: scan directories, parse YAML frontmatter
        let _ = (&self.workspace_skills, &self.builtin_skills);
        Vec::new()
    }

    /// Load a skill's full content by name.
    pub fn load_skill(&self, _name: &str) -> Option<String> {
        // TODO: read SKILL.md from workspace or builtin
        None
    }
}

pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub available: bool,
}
