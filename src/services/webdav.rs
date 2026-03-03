use anyhow::{Context, Result};
use reqwest::Client;
use std::path::Path;
use tokio::fs;

pub struct WebDavClient {
    client: Client,
    base_url: String,
    username: String,
    password: String,
}

impl WebDavClient {
    pub fn new(base_url: String, username: String, password: String) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap();

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            username,
            password,
        }
    }

    /// 上传文件到 WebDAV
    pub async fn upload_file(&self, local_path: &Path, remote_path: &str) -> Result<()> {
        let file_data = fs::read(local_path)
            .await
            .context("Failed to read local file")?;

        let url = format!("{}/{}", self.base_url, remote_path.trim_start_matches('/'));

        let response = self
            .client
            .put(&url)
            .basic_auth(&self.username, Some(&self.password))
            .body(file_data)
            .send()
            .await
            .context("Failed to send upload request")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Upload failed with status: {} - {}",
                response.status(),
                response.text().await.unwrap_or_default()
            );
        }

        Ok(())
    }

    /// 测试连接
    pub async fn test_connection(&self) -> Result<()> {
        let response = self
            .client
            .request(reqwest::Method::from_bytes(b"PROPFIND")?, &self.base_url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Depth", "0")
            .send()
            .await
            .context("Failed to connect to WebDAV server")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Connection test failed with status: {}",
                response.status()
            );
        }

        Ok(())
    }
}
