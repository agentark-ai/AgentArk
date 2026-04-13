use anyhow::{Context, Result};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::path::PathBuf;

fn main() -> Result<()> {
    let cache_dir = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/app/prebuilt-embeddings-cache"));
    std::fs::create_dir_all(&cache_dir)
        .with_context(|| format!("failed to create embedding cache dir {:?}", cache_dir))?;

    let options = InitOptions::new(EmbeddingModel::BGESmallENV15)
        .with_cache_dir(cache_dir)
        .with_show_download_progress(true);
    let mut model = TextEmbedding::try_new(options)?;
    let _ = model.embed(vec!["agentark embedding prefetch"], None)?;
    Ok(())
}
