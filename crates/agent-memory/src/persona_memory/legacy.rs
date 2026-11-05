use std::path::Path;

use anyhow::{bail, ensure, Result};

use super::service::MemoryService;

impl MemoryService {
    /// Explicit operator assignment; imported text remains a candidate, never trusted instructions.
    pub fn adopt_legacy(&self, id: &str) -> Result<String> {
        let legacy = Self {
            persona: "__legacy__".into(),
            ..self.clone()
        };
        let r = legacy.show(id)?;
        ensure!(r.kind == "legacy", "legacy record required");
        self.insert(
            "legacy",
            &r.content,
            "",
            id,
            false,
            Some(&format!("adopt:{}:{}:{}", self.project, self.persona, id)),
        )
    }

    /// Import quarantines old text. It cannot become prompt context by accident.
    pub fn import_legacy(&self, path: &Path) -> Result<String> {
        let content = std::fs::read_to_string(path)?;
        if content.trim().is_empty() {
            bail!("empty legacy file")
        }
        let backup = path.with_extension("persona-memory-backup");
        if !backup.exists() {
            std::fs::copy(path, &backup)?;
        }
        let legacy = Self {
            persona: "__legacy__".into(),
            ..self.clone()
        };
        let key = format!(
            "{}:{}:{}",
            self.project,
            path.canonicalize()?.display(),
            content
        );
        legacy.insert(
            "legacy",
            &content,
            "",
            &path.display().to_string(),
            false,
            Some(&key),
        )
    }
}
