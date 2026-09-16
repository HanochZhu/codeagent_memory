use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

pub trait Embedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>>;
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
}

pub struct Model2VecEmbedder {
    model: model2vec_rs::model::StaticModel,
}

impl Model2VecEmbedder {
    pub fn load() -> Result<Self> {
        let cache = crate::config::Config::models_dir()?.join("potion-multilingual-128M");
        std::fs::create_dir_all(&cache)?;
        std::env::set_var("HF_HOME", crate::config::Config::models_dir()?);
        let tokenizer = cache.join("tokenizer.json");
        let weights = cache.join("model.safetensors");
        let config = cache.join("config.json");
        if !(tokenizer.exists() && weights.exists() && config.exists()) {
            fetch_potion_files(&cache)?;
        }
        let model = if tokenizer.exists() && weights.exists() && config.exists() {
            model2vec_rs::model::StaticModel::from_pretrained(&cache, None, Some(true), None)
                .context("load local potion-multilingual-128M")?
        } else {
            model2vec_rs::model::StaticModel::from_pretrained(
                "minishlab/potion-multilingual-128M",
                None,
                Some(true),
                None,
            )
            .context("load model2vec potion-multilingual-128M (needs network on first run)")?
        };
        Ok(Self { model })
    }
}

fn fetch_potion_files(dir: &std::path::Path) -> Result<()> {
    let base = std::env::var("HF_ENDPOINT").unwrap_or_else(|_| "https://hf-mirror.com".into());
    let repo = format!(
        "{}/minishlab/potion-multilingual-128M/resolve/main",
        base.trim_end_matches('/')
    );
    for name in ["tokenizer.json", "config.json", "model.safetensors"] {
        let dest = dir.join(name);
        if dest.exists() {
            continue;
        }
        let url = format!("{repo}/{name}");
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
}
