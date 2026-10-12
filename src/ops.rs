use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::memory::embed::{DEFAULT_EMBED_MODEL, HASH_EMBED_MODEL};
use crate::memory::{Embedder, HashEmbedder, Model2VecEmbedder};
use crate::project::Project;
use crate::Config;

const MODEL_FILES: &[&str] = &["tokenizer.json", "config.json", "model.safetensors"];

/// How to embed for this process.
pub enum EmbedChoice {
    Hash,
    /// Local model2vec directory or Hugging Face repo id.
    Model(String),
}

fn cam_project_env() -> Option<PathBuf> {
    let value = std::env::var("CAM_PROJECT").ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

/// Resolve a project root and create `.cam/` plus the database if missing.
///
/// Order: explicit path, `CAM_PROJECT`, then `.cam` / `.git` walk-up, else cwd.
pub fn resolve_project(explicit: Option<&Path>) -> Result<Project> {
    let env_path = cam_project_env();
    let project = Project::resolve(explicit.or(env_path.as_deref()))?;
    project.ensure_initialized()?;
    Ok(project)
}

/// Resolve a project root for an MCP tool call and create `.cam/` if missing.
///
/// Order: tool `path`, `CAM_PROJECT`, the server `--project`, then `.cam` / `.git` walk-up, else cwd.
pub fn resolve_project_from_strings(
    tool_path: Option<&str>,
    default_path: Option<&Path>,
) -> Result<Project> {
    let tool = tool_path.filter(|s| !s.is_empty()).map(Path::new);
    let env = cam_project_env();
    resolve_project(tool.or(env.as_deref()).or(default_path))
}

pub fn load_embedder(hash: bool) -> Result<Box<dyn Embedder>> {
    let (choice, explicit) = resolve_embed_choice(hash)?;
    match choice {
        EmbedChoice::Hash => Ok(Box::new(HashEmbedder::default())),
        EmbedChoice::Model(spec) => load_model2vec(&spec, explicit),
    }
}

/// Model id that `add` / `recall` will use, without loading weights.
pub fn active_embed_label(hash: bool) -> Result<String> {
    if hash || env_flag("CAM_HASH_EMBED") {
        return Ok(HASH_EMBED_MODEL.to_string());
    }
    if let Some(raw) = env_nonempty("CAM_EMBED_MODEL") {
        return Ok(raw);
    }
    let cfg = Config::load()?;
    Ok(cfg
        .embedding_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("default"))
        .unwrap_or(DEFAULT_EMBED_MODEL)
        .to_string())
}

/// Parse a config or env value. `default` is the built-in potion model.
pub fn parse_embed_model(value: &str) -> Result<EmbedChoice> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("default") {
        return Ok(EmbedChoice::Model(DEFAULT_EMBED_MODEL.to_string()));
    }
    if value.eq_ignore_ascii_case("hash") {
        return Ok(EmbedChoice::Hash);
    }
    if looks_like_path(value) {
        let path = expand_user_path(value);
        if !path.is_dir() {
            bail!(
                "embedding model directory does not exist: {}",
                path.display()
            );
        }
        for name in MODEL_FILES {
            if !path.join(name).is_file() {
                bail!(
                    "embedding model directory {} is missing {name}; it must be a model2vec folder (tokenizer.json, config.json, model.safetensors)",
                    path.display()
                );
            }
        }
        let stored = path.canonicalize().unwrap_or(path);
        return Ok(EmbedChoice::Model(stored.to_string_lossy().into_owned()));
    }
    if !is_repo_id(value) {
        bail!(
            "embedding_model must be `default`, `hash`, a model2vec directory, or a Hugging Face repo id like `minishlab/potion-base-8M`"
        );
    }
    Ok(EmbedChoice::Model(value.to_string()))
}

fn resolve_embed_choice(force_hash: bool) -> Result<(EmbedChoice, bool)> {
    if force_hash || env_flag("CAM_HASH_EMBED") {
        return Ok((EmbedChoice::Hash, false));
    }
    if let Some(raw) = env_nonempty("CAM_EMBED_MODEL") {
        return Ok((parse_embed_model(&raw)?, true));
    }
    let cfg = Config::load()?;
    if let Some(raw) = cfg
        .embedding_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("default"))
    {
        return Ok((parse_embed_model(raw)?, true));
    }
    Ok((EmbedChoice::Model(DEFAULT_EMBED_MODEL.to_string()), false))
}

fn load_model2vec(spec: &str, explicit: bool) -> Result<Box<dyn Embedder>> {
    let require = std::env::var_os("CAM_REQUIRE_MODEL2VEC").is_some();
    match Model2VecEmbedder::load_spec(spec) {
        Ok(model) => Ok(Box::new(model)),
        Err(err) if explicit || require => Err(err).with_context(|| {
            if require {
                "CAM_REQUIRE_MODEL2VEC is set; refusing hash fallback".to_string()
            } else {
                format!("embedding model `{spec}` failed to load; refusing hash fallback")
            }
        }),
        Err(err) => {
            eprintln!("warn: model2vec unavailable ({err}); falling back to hash embedder");
            Ok(Box::new(HashEmbedder::default()))
        }
    }
}

fn env_nonempty(name: &str) -> Option<String> {
    let value = std::env::var(name).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn looks_like_path(value: &str) -> bool {
    let path = Path::new(value);
    path.is_absolute() || value.starts_with('.') || value.starts_with('~') || path.is_dir()
}

fn expand_user_path(value: &str) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(value)
}

fn is_repo_id(value: &str) -> bool {
    let Some((owner, name)) = value.split_once('/') else {
        return false;
    };
    let ok = |part: &str| {
        !part.is_empty()
            && !part.contains("..")
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    };
    !value.split('/').nth(2).is_some() && ok(owner) && ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_embed_model_accepts_default_hash_and_repo() {
        match parse_embed_model("default").unwrap() {
            EmbedChoice::Model(spec) => assert_eq!(spec, DEFAULT_EMBED_MODEL),
            EmbedChoice::Hash => panic!("default is not hash"),
        }
        assert!(matches!(
            parse_embed_model("hash").unwrap(),
            EmbedChoice::Hash
        ));
        match parse_embed_model("minishlab/potion-base-8M").unwrap() {
            EmbedChoice::Model(spec) => assert_eq!(spec, "minishlab/potion-base-8M"),
            EmbedChoice::Hash => panic!("repo is not hash"),
        }
        assert!(parse_embed_model("not a model").is_err());
        assert!(parse_embed_model("/no/such/embedding-model").is_err());
    }

    #[test]
    fn parse_embed_model_accepts_a_local_model2vec_dir() {
        let dir = tempfile::tempdir().unwrap();
        for name in MODEL_FILES {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        match parse_embed_model(dir.path().to_str().unwrap()).unwrap() {
            EmbedChoice::Model(spec) => {
                assert_eq!(spec, dir.path().canonicalize().unwrap().to_string_lossy());
            }
            EmbedChoice::Hash => panic!("directory is not hash"),
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
