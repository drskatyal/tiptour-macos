// fastembed-rs wrapper. Quantized MiniLM-L6-v2 (384 dims), lazy-init
// behind a `Mutex<Option<...>>` so the first `embed_text` call pays the
// model-download + ONNX-Runtime warmup cost, not app startup.
//
// Only compiled under the `agent-memory-vector` feature. The default
// build uses the bag-of-words ranker in `store.rs`.

use std::path::PathBuf;
use std::sync::Mutex;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use once_cell::sync::Lazy;

pub const EMBEDDING_DIMENSIONS: usize = 384;

static EMBEDDING_MODEL: Lazy<Mutex<Option<TextEmbedding>>> = Lazy::new(|| Mutex::new(None));

fn cache_directory() -> Result<PathBuf, String> {
    let base = dirs::cache_dir().ok_or("no cache_dir on this OS")?;
    Ok(base.join("TipTour").join("embeddings"))
}

fn ensure_model_initialized() -> Result<(), String> {
    let mut guard = EMBEDDING_MODEL
        .lock()
        .map_err(|_| "embedder mutex poisoned".to_string())?;
    if guard.is_some() {
        return Ok(());
    }
    let cache_dir = cache_directory()?;
    std::fs::create_dir_all(&cache_dir).map_err(|e| format!("create embed cache dir: {e}"))?;
    let init_options = InitOptions::new(EmbeddingModel::AllMiniLML6V2Q)
        .with_cache_dir(cache_dir)
        .with_show_download_progress(false);
    let model = TextEmbedding::try_new(init_options)
        .map_err(|e| format!("fastembed init: {e}"))?;
    *guard = Some(model);
    Ok(())
}

pub fn embed_text(text: &str) -> Result<Vec<f32>, String> {
    ensure_model_initialized()?;
    let mut guard = EMBEDDING_MODEL
        .lock()
        .map_err(|_| "embedder mutex poisoned".to_string())?;
    let model = guard.as_mut().ok_or("embedder not initialized")?;
    let embeddings = model
        .embed(vec![text.to_string()], None)
        .map_err(|e| format!("fastembed embed: {e}"))?;
    embeddings
        .into_iter()
        .next()
        .ok_or_else(|| "fastembed returned zero embeddings".to_string())
}
