use anyhow::{Context, Result};
use s3::bucket::Bucket;
use s3::creds::Credentials;
use s3::region::Region;
use std::path::Path;
use tokio::fs;

#[derive(Debug, Clone)]
pub struct S3File {
    pub name: String,
    pub key: String,
}

pub struct S3Client {
    bucket: Box<Bucket>,
}

impl S3Client {
    /// 创建 S3 客户端（兼容 Cloudflare R2 / MinIO / Wasabi 等 S3 协议实现）
    pub fn new(
        endpoint: String,
        region: String,
        bucket: String,
        access_key_id: String,
        secret_access_key: String,
    ) -> Result<Self> {
        let region = Region::Custom {
            region: if region.trim().is_empty() {
                "auto".to_string()
            } else {
                region.trim().to_string()
            },
            endpoint: endpoint.trim_end_matches('/').to_string(),
        };

        let credentials = Credentials::new(
            Some(access_key_id.trim()),
            Some(secret_access_key.trim()),
            None,
            None,
            None,
        )
        .context("Failed to build S3 credentials")?;

        let bucket = Bucket::new(bucket.trim(), region, credentials)
            .context("Failed to create S3 bucket client")?
            .with_path_style();

        Ok(Self { bucket })
    }

    /// 上传文件到 S3
    pub async fn upload_file(&self, local_path: &Path, key: &str) -> Result<()> {
        let file_data = fs::read(local_path)
            .await
            .context("Failed to read local file")?;

        let key = key.trim_start_matches('/');
        tracing::debug!("Uploading {} bytes to S3 key: {}", file_data.len(), key);

        self.bucket
            .put_object(key, &file_data)
            .await
            .with_context(|| format!("Failed to upload to S3: {}", key))?;

        Ok(())
    }

    /// 列出指定前缀下的对象
    pub async fn list_files(&self, prefix: &str) -> Result<Vec<S3File>> {
        let prefix = prefix.trim_start_matches('/').to_string();

        let results = self
            .bucket
            .list(prefix.clone(), None)
            .await
            .context("Failed to list S3 objects")?;

        let mut files = Vec::new();
        for result in results {
            for object in result.contents {
                let key = object.key;

                // 跳过以 / 结尾的目录占位对象
                if key.ends_with('/') {
                    continue;
                }

                let name = key.rsplit('/').next().unwrap_or(&key).to_string();

                tracing::debug!("Found S3 object: name={}, key={}", name, key);

                files.push(S3File { name, key });
            }
        }

        tracing::debug!("Listed {} objects from S3 prefix: {}", files.len(), prefix);
        Ok(files)
    }

    /// 删除对象
    pub async fn delete_file(&self, key: &str) -> Result<()> {
        let key = key.trim_start_matches('/');

        tracing::debug!("Deleting S3 object: {}", key);

        self.bucket
            .delete_object(key)
            .await
            .with_context(|| format!("Failed to delete S3 object: {}", key))?;

        Ok(())
    }

    /// 下载对象到本地文件
    pub async fn download_file(&self, key: &str, local_path: &Path) -> Result<()> {
        let key = key.trim_start_matches('/');

        tracing::info!("Downloading object from S3: {}", key);

        let response = self
            .bucket
            .get_object(key)
            .await
            .with_context(|| format!("Failed to download S3 object: {}", key))?;

        fs::write(local_path, response.as_slice())
            .await
            .context("Failed to write downloaded file")?;

        Ok(())
    }

    /// 测试连接（列出根前缀）
    pub async fn test_connection(&self) -> Result<()> {
        self.bucket
            .list(String::new(), None)
            .await
            .context("Failed to connect to S3 storage")?;

        Ok(())
    }
}
