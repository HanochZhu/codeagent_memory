use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

/// Built-in model2vec model. Cache stays at `~/.cam/models/potion-multilingual-128M`.
pub const DEFAULT_EMBED_MODEL: &str = "minishlab/potion-multilingual-128M";
pub const HASH_EMBED_MODEL: &str = "hash";

const MODEL_FILES: &[&str] = &["tokenizer.json", "config.json", "model.safetensors"];

pub trait Embedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>>;
    /// Identity stored with each embedding so a later model is not compared against it.
    fn model_id(&self) -> &str;
}

pub struct HashEmbedder {
    dim: usize,
}

impl Default for HashEmbedder {
    fn default() -> Self {
        Self { dim: 64 }
    }
}

impl Embedder for HashEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut vec = vec![0f32; self.dim];
        for token in tokenize(text) {
            let digest = Sha256::digest(token.as_bytes());
            let idx = u32::from_le_bytes(digest[0..4].try_into().unwrap()) as usize % self.dim;
            vec[idx] += 1.0;
        }
        Ok(l2_normalize(vec))
    }

    fn model_id(&self) -> &str {
        HASH_EMBED_MODEL
    }
}

pub struct Model2VecEmbedder {
    model: model2vec_rs::model::StaticModel,
    id: String,
}

impl Model2VecEmbedder {
    pub fn load() -> Result<Self> {
        Self::load_spec(DEFAULT_EMBED_MODEL)
    }

    /// Load a model2vec model. `spec` is a local directory or a Hugging Face repo id.
    pub fn load_spec(spec: &str) -> Result<Self> {
        let path = Path::new(spec);
        if path.is_dir() {
            return Self::load_dir(path);
        }
        Self::load_repo(spec)
    }

    fn load_dir(dir: &Path) -> Result<Self> {
        let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        require_model_files(&dir)?;
        let model = model2vec_rs::model::StaticModel::from_pretrained(&dir, None, Some(true), None)
            .with_context(|| format!("load model2vec model from {}", dir.display()))?;
        Ok(Self {
            model,
            id: dir.to_string_lossy().into_owned(),
        })
    }

    fn load_repo(repo: &str) -> Result<Self> {
        let cache = cache_dir(repo)?;
        std::fs::create_dir_all(&cache)?;
        std::env::set_var("HF_HOME", crate::config::Config::models_dir()?);
        if !model_files_present(&cache) {
            fetch_model_files(&cache, repo)?;
        }
        let model = if model_files_present(&cache) {
            model2vec_rs::model::StaticModel::from_pretrained(&cache, None, Some(true), None)
                .with_context(|| format!("load cached model2vec model {repo}"))?
        } else {
            model2vec_rs::model::StaticModel::from_pretrained(repo, None, Some(true), None)
                .with_context(|| {
                    format!("load model2vec model {repo} (needs network on first run)")
                })?
        };
        Ok(Self {
            model,
            id: repo.to_string(),
        })
    }
}

fn cache_dir(repo: &str) -> Result<PathBuf> {
    let base = crate::config::Config::models_dir()?;
    if repo == DEFAULT_EMBED_MODEL {
        Ok(base.join("potion-multilingual-128M"))
    } else {
        Ok(base.join(repo))
    }
}

fn model_files_present(dir: &Path) -> bool {
    MODEL_FILES.iter().all(|name| dir.join(name).is_file())
}

fn require_model_files(dir: &Path) -> Result<()> {
    for name in MODEL_FILES {
        if !dir.join(name).is_file() {
            anyhow::bail!(
                "embedding model directory {} is missing {name}",
                dir.display()
            );
        }
    }
    Ok(())
}

fn fetch_model_files(dir: &Path, repo: &str) -> Result<()> {
    let base = std::env::var("HF_ENDPOINT").unwrap_or_else(|_| "https://hf-mirror.com".into());
    let root = format!(
        "{}/{}/resolve/main",
        base.trim_end_matches('/'),
        repo.trim_matches('/')
    );
    for name in MODEL_FILES {
        let dest = dir.join(name);
        if dest.exists() {
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let url = format!("{root}/{name}");
        let status = std::process::Command::new("curl")
            .args(["-4", "-fsSL", "--retry", "3", "--max-time", "180", "-o"])
            .arg(&dest)
            .arg(&url)
            .status()
            .with_context(|| format!("spawn curl for {url}"))?;
        if !status.success() {
            let _ = std::fs::remove_file(&dest);
            anyhow::bail!("curl failed for {url} (status {status})");
        }
    }
    Ok(())
}

impl Embedder for Model2VecEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let out = self.model.encode(&[text.to_string()]);
        out.into_iter()
            .next()
            .context("model2vec returned no embedding")
    }

    fn model_id(&self) -> &str {
        &self.id
    }
}

/// A stored vector is usable only when its dimension matches and it was produced
/// by the same model. Rows written before model ids existed stay usable for the
/// built-in potion model and the hash embedder; a custom model ignores them.
pub(crate) fn embedding_usable(
    stored_model: Option<&str>,
    stored: &[f32],
    current_id: &str,
    query: &[f32],
) -> bool {
    if stored.is_empty() || stored.len() != query.len() {
        return false;
    }
    match stored_model.map(str::trim).filter(|id| !id.is_empty()) {
        Some(id) => id == current_id,
        None => current_id == DEFAULT_EMBED_MODEL || current_id == HASH_EMBED_MODEL,
    }
}

pub fn encode_f32(vec: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vec.len() * 4);
    for v in vec {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

pub fn decode_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0;
    let mut na = 0.0;
    let mut nb = 0.0;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect()
}

fn l2_normalize(mut vec: Vec<f32>) -> Vec<f32> {
    let norm = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut vec {
            *v /= norm;
        }
    }
    vec
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn similar_texts_have_higher_cosine() {
        let e = HashEmbedder::default();
        let a = e.embed("bm25 vector hybrid recall").unwrap();
        let b = e.embed("hybrid bm25 and vector recall").unwrap();
        let c = e.embed("unrelated cooking recipe").unwrap();
        assert!(cosine(&a, &b) > cosine(&a, &c));
    }

    #[test]
    fn custom_model_does_not_reuse_legacy_or_other_vectors() {
        let query = vec![1.0, 0.0];
        let same = vec![1.0, 0.0];
        let other_dim = vec![1.0, 0.0, 0.0];
        assert!(embedding_usable(None, &same, DEFAULT_EMBED_MODEL, &query));
        assert!(embedding_usable(None, &same, HASH_EMBED_MODEL, &query));
        assert!(!embedding_usable(None, &same, "/models/custom", &query));
        assert!(!embedding_usable(
            Some("minishlab/potion-base-8M"),
            &same,
            DEFAULT_EMBED_MODEL,
            &query
        ));
        assert!(embedding_usable(
            Some(DEFAULT_EMBED_MODEL),
            &same,
            DEFAULT_EMBED_MODEL,
            &query
        ));
        assert!(!embedding_usable(
            Some(DEFAULT_EMBED_MODEL),
            &other_dim,
            DEFAULT_EMBED_MODEL,
            &query
        ));
    }
}
