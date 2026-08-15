use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub aliases: HashMap<String, u64>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = Self::path();
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config at {}", path.display()))?;

        toml::from_str(&content)
            .with_context(|| format!("Failed to parse config at {}", path.display()))
    }

    pub fn resolve_alias(&self, name: &str) -> Option<String> {
        self.aliases.get(name).map(|id| id.to_string())
    }

    fn path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".rbxswitch.toml")
    }
}
