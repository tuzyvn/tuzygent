use std::path::{Path, PathBuf};
use tracing::{info, warn};

use super::engines::EnginesConfig;

pub fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

impl EnginesConfig {
    /// Load hierarchical configuration:
    /// 1. Sane built-in defaults
    /// 2. Global OS config: ~/.tuzygent/engines.toml
    /// 3. Workspace local config: <workspace>/tuzygent.toml or .tuzygent.toml
    pub fn load_hierarchical(workspace_path: Option<&Path>) -> Self {
        let mut config = Self::default();

        // 1. Check Global OS config
        if let Some(home) = dirs_home() {
            let global_path = home.join(".tuzygent").join("engines.toml");
            if global_path.exists() {
                info!("Loading global OS engine config from {:?}", global_path);
                if let Ok(content) = std::fs::read_to_string(&global_path) {
                    if config.merge_toml(&content).is_ok() {
                    } else {
                        warn!("Failed to parse global engines.toml at {:?}", global_path);
                    }
                }
            }
        }

        // 2. Check Workspace local config
        if let Some(ws) = workspace_path {
            let local_candidates = [
                ws.join("tuzygent.toml"),
                ws.join(".tuzygent.toml"),
                ws.join(".tuzygent").join("engines.toml"),
            ];

            for path in &local_candidates {
                if path.exists() {
                    info!("Loading local workspace engine config from {:?}", path);
                    if let Ok(content) = std::fs::read_to_string(path) {
                        if config.merge_toml(&content).is_ok() {
                            break;
                        } else {
                            warn!("Failed to parse local config at {:?}", path);
                        }
                    }
                }
            }
        }

        config
    }

    /// Overlay only explicitly present keys. Empty arrays intentionally clear policy.
    pub fn merge_toml(&mut self, content: &str) -> Result<(), String> {
        fn overlay(base: &mut toml::Value, patch: toml::Value) {
            match (base, patch) {
                (toml::Value::Table(base), toml::Value::Table(patch)) => {
                    for (key, value) in patch {
                        if let Some(current) = base.get_mut(&key) {
                            overlay(current, value);
                        } else {
                            base.insert(key, value);
                        }
                    }
                }
                (base, value) => *base = value,
            }
        }
        let patch: toml::Value = toml::from_str(content).map_err(|error| error.to_string())?;
        let mut merged = toml::Value::try_from(&*self).map_err(|error| error.to_string())?;
        overlay(&mut merged, patch);
        let parsed = merged
            .try_into()
            .map_err(|error: toml::de::Error| error.to_string())?;
        *self = parsed;
        Ok(())
    }
}

pub fn load_hierarchical(workspace_path: Option<&Path>) -> EnginesConfig {
    EnginesConfig::load_hierarchical(workspace_path)
}

pub fn merge_toml(config: &mut EnginesConfig, content: &str) -> Result<(), String> {
    config.merge_toml(content)
}
