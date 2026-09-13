use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::{walk_up_for, Config};

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
                root: path.canonicalize().with_context(|| {
                    format!("cannot resolve project path {}", path.display())
                })?,
            });
        }

        let cwd = std::env::current_dir()?;
        if let Some(root) = walk_up_for(&cwd, CAM_DIR) {
            return Ok(Self { root });
        }
        if let Some(root) = walk_up_for(&cwd, ".git") {
            return Ok(Self { root });
        }

        let cfg = Config::load()?;
        if let Some(stored) = cfg.current_project {
            let root = PathBuf::from(stored);
            if root.exists() {
                return Ok(Self { root });
            }
        }

        bail!("no project found; run `cam init` in a project directory")
    }

    pub fn cam_dir(&self) -> PathBuf {
        self.root.join(CAM_DIR)
    }

    pub fn db_path(&self) -> PathBuf {
        self.cam_dir().join(DB_NAME)
    }

    pub fn init(path: Option<&Path>) -> Result<Self> {
        let root = match path {
            Some(p) => p
                .canonicalize()
                .with_context(|| format!("cannot resolve {}", p.display()))?,
            None => std::env::current_dir()?,
        };
        let project = Self { root };
        fs::create_dir_all(project.cam_dir())?;
        let mut cfg = Config::load()?;
        cfg.current_project = Some(project.root.to_string_lossy().into_owned());
        cfg.save()?;
        Ok(project)
    }

    pub fn ensure_initialized(&self) -> Result<()> {
        fs::create_dir_all(self.cam_dir())?;
        Ok(())
    }
}
