use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::config::walk_up_for;

pub const CAM_DIR: &str = ".cam";
pub const DB_NAME: &str = "cam.db";

#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
}

impl Project {
    pub fn resolve(explicit: Option<&Path>) -> Result<Self> {
        if let Some(path) = explicit {
            return Ok(Self {
                root: path
                    .canonicalize()
                    .with_context(|| format!("cannot resolve project path {}", path.display()))?,
            });
        }

        let cwd = std::env::current_dir()?;
        let root = walk_up_for(&cwd, CAM_DIR)
            .or_else(|| walk_up_for(&cwd, ".git"))
            .unwrap_or(cwd);
        Ok(Self { root })
    }

    pub fn cam_dir(&self) -> PathBuf {
        self.root.join(CAM_DIR)
    }

    pub fn db_path(&self) -> PathBuf {
        self.cam_dir().join(DB_NAME)
    }

    pub fn init(path: Option<&Path>) -> Result<Self> {
        let project = match path {
            Some(p) => Self::resolve(Some(p))?,
            None => Self {
                root: std::env::current_dir()?,
            },
        };
        project.ensure_initialized()?;
        Ok(project)
    }

    pub fn ensure_initialized(&self) -> Result<()> {
        self.connect().map(|_| ())
    }

    pub(crate) fn connect(&self) -> Result<Connection> {
        crate::db::open_db(&self.db_path())
    }
}
