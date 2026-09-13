use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const DEFAULT_STALE_DAYS: u32 = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_stale_days")]
    pub stale_days: u32,
    #[serde(default)]
    pub current_project: Option<String>,
}

fn default_stale_days() -> u32 {
    DEFAULT_STALE_DAYS
}

impl Default for Config {
    fn default() -> Self {
        Self {
            stale_days: DEFAULT_STALE_DAYS,
            current_project: None,
        }
    }
}

impl Config {
    pub fn global_dir() -> Result<PathBuf> {
        let home = dirs::home_dir().context("cannot resolve home directory")?;
        Ok(home.join(".cam"))
    }

    pub fn path() -> Result<PathBuf> {
        Ok(Self::global_dir()?.join("config.toml"))
    }

    pub fn models_dir() -> Result<PathBuf> {
        Ok(Self::global_dir()?.join("models"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(&path)
            .with_context(|| format!("read config {}", path.display()))?;
        toml::from_str(&text).context("parse ~/.cam/config.toml")
    }

    pub fn save(&self) -> Result<()> {
        let dir = Self::global_dir()?;
        fs::create_dir_all(&dir)?;
        let path = dir.join("config.toml");
        fs::write(&path, toml::to_string_pretty(self)?)
            .with_context(|| format!("write config {}", path.display()))
    }

    pub fn is_stale(&self, updated_at_unix: i64) -> bool {
        let now = chrono::Utc::now().timestamp();
        now - updated_at_unix > i64::from(self.stale_days) * 86_400
    }

    pub fn age_days(updated_at_unix: i64) -> i64 {
        let now = chrono::Utc::now().timestamp();
        ((now - updated_at_unix).max(0)) / 86_400
    }
}

pub fn walk_up_for(start: &Path, marker: &str) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        if current.join(marker).exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}
