use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tracing::info;

#[derive(Error, Debug)]
pub enum DownloadError {
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub struct DownloadResult {
    pub content: String,
    pub hash: String,
}

fn get_http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent("ProxyD/1.0")
            .timeout(Duration::from_secs(300))
            .connect_timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client")
    })
}

pub async fn download_csv(url: &str) -> Result<DownloadResult, DownloadError> {
    info!("Downloading CSV from {}", url);

    let client = get_http_client();

    let response = client.get(url).send().await?.error_for_status()?;
    let content = response.text().await?;

    let hash = compute_hash(&content);
    info!("Downloaded CSV, hash: {}", hash);

    Ok(DownloadResult { content, hash })
}

pub async fn save_csv(path: &Path, content: &str) -> Result<(), DownloadError> {
    atomic_write(path, content.as_bytes()).await
}

pub async fn save_hash(path: &Path, hash: &str) -> Result<(), DownloadError> {
    atomic_write(path, hash.as_bytes()).await
}

async fn atomic_write(path: &Path, content: &[u8]) -> Result<(), DownloadError> {
    let temp_path = path.with_extension("tmp");

    let mut file = tokio::fs::File::create(&temp_path).await?;
    file.write_all(content).await?;
    file.sync_all().await?;
    drop(file);

    tokio::fs::rename(&temp_path, path).await?;
    Ok(())
}

pub async fn load_hash(path: &Path) -> Option<String> {
    tokio::fs::read_to_string(path).await.ok()
}

pub async fn load_csv(path: &Path) -> Result<String, DownloadError> {
    Ok(tokio::fs::read_to_string(path).await?)
}

pub fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}
