use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::memory::{Embedder, HashEmbedder, Model2VecEmbedder};
use crate::project::Project;

fn cam_project_env() -> Option<PathBuf> {
    let value = std::env::var("CAM_PROJECT").ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

/// Resolve a project root.
///
/// Order: explicit path, `CAM_PROJECT`, then `.cam` / `.git` walk-up.
pub fn resolve_project(explicit: Option<&Path>) -> Result<Project> {
    if let Some(path) = explicit {
        return Project::resolve(Some(path));
    }
    if let Some(env_path) = cam_project_env() {
        return Project::resolve(Some(&env_path));
    }
    Project::resolve(None)
}

/// Resolve a project root for an MCP tool call.
///
/// Order: tool `path`, `CAM_PROJECT`, the server `--path`, then `.cam` / `.git` walk-up.
pub fn resolve_project_from_strings(
    tool_path: Option<&str>,
    default_path: Option<&Path>,
) -> Result<Project> {
    if let Some(path) = tool_path.filter(|s| !s.is_empty()) {
        return resolve_project(Some(Path::new(path)));
    }
    if let Some(env_path) = cam_project_env() {
        return Project::resolve(Some(&env_path));
    }
    resolve_project(default_path)
}

/// Initialize a project from an MCP tool call.
///
/// Order: tool `path`, `CAM_PROJECT`, the server `--path`, then the server cwd.
pub fn init_project_from_strings(
    tool_path: Option<&str>,
    default_path: Option<&Path>,
) -> Result<Project> {
    if let Some(path) = tool_path.filter(|s| !s.is_empty()) {
        return Project::init(Some(Path::new(path)));
    }
    if let Some(env_path) = cam_project_env() {
        return Project::init(Some(&env_path));
    }
    Project::init(default_path)
}

pub fn load_embedder(hash: bool) -> Result<Box<dyn Embedder>> {
    if hash || env_flag("CAM_HASH_EMBED") {
        return Ok(Box::new(HashEmbedder::default()));
    }
    let require = std::env::var_os("CAM_REQUIRE_MODEL2VEC").is_some();
    match Model2VecEmbedder::load() {
        Ok(model) => Ok(Box::new(model)),
        Err(err) if require => {
            Err(err).context("CAM_REQUIRE_MODEL2VEC is set; refusing hash fallback")
        }
        Err(err) => {
            eprintln!("warn: model2vec unavailable ({err}); falling back to hash embedder");
            Ok(Box::new(HashEmbedder::default()))
        }
    }
}

pub fn env_flag(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => {
            let v = v.trim();
            !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false")
        }
        Err(_) => false,
    }
}
