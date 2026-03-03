use anyhow::{Context, Result};
use reqwest::Client;
use std::path::Path;
use tokio::fs;

#[derive(Debug, Clone)]
pub struct WebDavFile {
    pub name: String,
    pub path: String,
    pub modified: Option<String>,
}

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

    /// 列出目录中的文件
    pub async fn list_files(&self, remote_path: &str) -> Result<Vec<WebDavFile>> {
        let url = format!("{}/{}", self.base_url, remote_path.trim_start_matches('/'));

        let response = self
            .client
            .request(reqwest::Method::from_bytes(b"PROPFIND")?, &url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Depth", "1")
            .send()
            .await
            .context("Failed to list files")?;

        if !response.status().is_success() {
            anyhow::bail!("List files failed with status: {}", response.status());
        }

        let body = response.text().await?;

        tracing::debug!("WebDAV PROPFIND response body:\n{}", body);

        // 简单解析 WebDAV XML 响应
        let mut files = Vec::new();
        for line in body.lines() {
            if line.contains("<d:href>") || line.contains("<D:href>") {
                let (start_tag, end_tag) = if line.contains("<d:href>") {
                    ("<d:href>", "</d:href>")
                } else {
                    ("<D:href>", "</D:href>")
                };

                if let Some(start) = line.find(start_tag) {
                    if let Some(end) = line.find(end_tag) {
                        let path = &line[start + start_tag.len()..end];

                        // 跳过目录（以 / 结尾）
                        if path.ends_with('/') {
                            continue;
                        }

                        let name = path.split('/').last().unwrap_or(path).to_string();

                        tracing::debug!("Found file: name={}, path={}", name, path);

                        files.push(WebDavFile {
                            name: name.clone(),
                            path: path.to_string(),
                            modified: None,
                        });
                    }
                }
            }
        }

        tracing::debug!("Listed {} files from WebDAV path: {}", files.len(), remote_path);
        Ok(files)
    }

    /// 删除文件
    pub async fn delete_file(&self, remote_path: &str) -> Result<()> {
        // WebDAV 返回的 path 是服务器路径（如 /dav/zqtest/file.tar.gz）
        // 需要提取出相对于 base_url 的部分
        let base_path = self.base_url.trim_start_matches("https://").trim_start_matches("http://");
        let base_path = base_path.split_once('/').map(|(_, p)| format!("/{}", p)).unwrap_or_default();

        let relative_path = if !base_path.is_empty() && remote_path.starts_with(&base_path) {
            remote_path.trim_start_matches(&base_path)
        } else {
            remote_path
        };

        let url = format!("{}/{}", self.base_url, relative_path.trim_start_matches('/'));

        tracing::debug!("Deleting file from WebDAV: {} (URL: {})", remote_path, url);

        let response = self
            .client
            .delete(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .context("Failed to delete file")?;

        if !response.status().is_success() {
            anyhow::bail!("Delete failed with status: {}", response.status());
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
