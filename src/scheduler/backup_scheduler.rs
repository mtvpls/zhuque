use crate::services::{ConfigService, WebDavClient};
use anyhow::Result;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::sync::Arc;
use tar::Builder;
use tokio::sync::RwLock;
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{error, info};

/// 标准化cron表达式：如果是5字段格式，自动补充秒字段
fn normalize_cron_expr(expr: &str) -> String {
    let parts: Vec<&str> = expr.trim().split_whitespace().collect();
    if parts.len() == 5 {
        format!("0 {}", expr)
    } else {
        expr.to_string()
    }
}

pub struct BackupScheduler {
    scheduler: JobScheduler,
    config_service: Arc<ConfigService>,
    job_id: Arc<RwLock<Option<uuid::Uuid>>>,
}

impl BackupScheduler {
    pub async fn new(config_service: Arc<ConfigService>) -> Result<Self> {
        let scheduler = JobScheduler::new().await?;

        Ok(Self {
            scheduler,
            config_service,
            job_id: Arc::new(RwLock::new(None)),
        })
    }

    pub async fn start(&self) -> Result<()> {
        info!("Starting backup scheduler...");
        self.scheduler.start().await?;
        self.reload_backup_job().await?;
        info!("Backup scheduler started");
        Ok(())
    }

    pub async fn reload_backup_job(&self) -> Result<()> {
        info!("Reloading backup job...");

        // 清除现有任务
        let mut job_id = self.job_id.write().await;
        if let Some(id) = job_id.take() {
            let _ = self.scheduler.remove(&id).await;
        }

        // 加载自动备份配置
        let backup_config = self.config_service.get_auto_backup_config().await?;

        if !backup_config.enabled {
            info!("Auto backup is disabled");
            return Ok(());
        }

        // 验证配置
        if backup_config.webdav_url.is_empty()
            || backup_config.webdav_username.is_empty()
            || backup_config.webdav_password.is_empty()
        {
            error!("Auto backup is enabled but WebDAV configuration is incomplete");
            return Ok(());
        }

        let cron_expr = normalize_cron_expr(&backup_config.cron);
        let webdav_url = backup_config.webdav_url.clone();
        let webdav_username = backup_config.webdav_username.clone();
        let webdav_password = backup_config.webdav_password.clone();
        let remote_path = backup_config.remote_path.clone();

        match Job::new_async_tz(cron_expr.as_str(), chrono::Local, move |_uuid, _l| {
            let url = webdav_url.clone();
            let username = webdav_username.clone();
            let password = webdav_password.clone();
            let path = remote_path.clone();

            Box::pin(async move {
                info!("Running scheduled backup...");
                if let Err(e) = Self::perform_backup(&url, &username, &password, path.as_deref()).await {
                    error!("Failed to perform scheduled backup: {}", e);
                } else {
                    info!("Scheduled backup completed successfully");
                }
            })
        }) {
            Ok(job) => {
                match self.scheduler.add(job).await {
                    Ok(id) => {
                        info!("Added backup job with schedule: {}", backup_config.cron);
                        *job_id = Some(id);
                    }
                    Err(e) => error!("Failed to add backup job: {}", e),
                }
            }
            Err(e) => error!("Failed to create backup job: {}", e),
        }

        info!("Backup job reloaded");
        Ok(())
    }

    async fn perform_backup(
        webdav_url: &str,
        webdav_username: &str,
        webdav_password: &str,
        remote_path: Option<&str>,
    ) -> Result<()> {
        let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "./data".into());
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let backup_filename = format!("xuanwu_backup_{}.tar.gz", timestamp);

        // 创建备份文件
        info!("Creating backup archive...");
        let mut tar_gz_data = Vec::new();
        {
            let encoder = GzEncoder::new(&mut tar_gz_data, Compression::default());
            let mut tar = Builder::new(encoder);
            tar.append_dir_all("data", &data_dir)
                .map_err(|e| anyhow::anyhow!("Failed to create tar archive: {}", e))?;
            tar.finish()
                .map_err(|e| anyhow::anyhow!("Failed to finish tar archive: {}", e))?;
        }

        // 保存到临时文件
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(&backup_filename);
        tokio::fs::write(&temp_file, &tar_gz_data).await?;

        info!("Backup archive created: {} bytes", tar_gz_data.len());

        // 上传到 WebDAV
        info!("Uploading to WebDAV...");
        let client = WebDavClient::new(
            webdav_url.to_string(),
            webdav_username.to_string(),
            webdav_password.to_string(),
        );

        let remote_file_path = if let Some(path) = remote_path {
            format!("{}/{}", path.trim_end_matches('/'), backup_filename)
        } else {
            backup_filename
        };

        client.upload_file(&temp_file, &remote_file_path).await?;

        // 删除临时文件
        let _ = tokio::fs::remove_file(&temp_file).await;

        info!("Backup uploaded to WebDAV: {}", remote_file_path);
        Ok(())
    }
}
