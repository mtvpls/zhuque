use crate::models::{SystemConfig, CreateSystemConfig, UpdateSystemConfig, MirrorConfig, AutoBackupConfig};
use anyhow::Result;
use sqlx::SqlitePool;
use std::process::Command;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, error};

pub struct ConfigService {
    pool: Arc<RwLock<SqlitePool>>,
}

impl ConfigService {
    pub fn new(pool: Arc<RwLock<SqlitePool>>) -> Self {
        Self { pool }
    }

    pub async fn get_by_key(&self, key: &str) -> Result<Option<SystemConfig>> {
        let pool = self.pool.read().await;
        let config = sqlx::query_as::<_, SystemConfig>(
            "SELECT * FROM system_configs WHERE key = ?"
        )
        .bind(key)
        .fetch_optional(&*pool)
        .await?;
        Ok(config)
    }

    pub async fn list(&self) -> Result<Vec<SystemConfig>> {
        let pool = self.pool.read().await;
        let configs = sqlx::query_as::<_, SystemConfig>(
            "SELECT * FROM system_configs ORDER BY created_at DESC"
        )
        .fetch_all(&*pool)
        .await?;
        Ok(configs)
    }

    pub async fn create(&self, config: CreateSystemConfig) -> Result<SystemConfig> {
        let pool = self.pool.read().await;
        let result = sqlx::query(
            "INSERT INTO system_configs (key, value, description) VALUES (?, ?, ?)"
        )
        .bind(&config.key)
        .bind(&config.value)
        .bind(&config.description)
        .execute(&*pool)
        .await?;

        let id = result.last_insert_rowid();
        drop(pool);
        let created = self.get_by_id(id).await?;
        Ok(created.unwrap())
    }

    pub async fn update(&self, key: &str, update: UpdateSystemConfig) -> Result<Option<SystemConfig>> {
        let pool = self.pool.read().await;
        sqlx::query(
            "UPDATE system_configs SET value = ?, description = ?, updated_at = CURRENT_TIMESTAMP WHERE key = ?"
        )
        .bind(&update.value)
        .bind(&update.description)
        .bind(key)
        .execute(&*pool)
        .await?;

        drop(pool);
        self.get_by_key(key).await
    }

    pub async fn delete(&self, key: &str) -> Result<bool> {
        let pool = self.pool.read().await;
        let result = sqlx::query("DELETE FROM system_configs WHERE key = ?")
            .bind(key)
            .execute(&*pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn get_by_id(&self, id: i64) -> Result<Option<SystemConfig>> {
        let pool = self.pool.read().await;
        let config = sqlx::query_as::<_, SystemConfig>(
            "SELECT * FROM system_configs WHERE id = ?"
        )
        .bind(id)
        .fetch_optional(&*pool)
        .await?;
        Ok(config)
    }

    // 镜像源配置相关方法
    pub async fn get_mirror_config(&self) -> Result<MirrorConfig> {
        if let Some(config) = self.get_by_key("mirror").await? {
            let mirror_config: MirrorConfig = serde_json::from_str(&config.value)?;
            Ok(mirror_config)
        } else {
            Ok(MirrorConfig {
                linux: None,
                nodejs: None,
                python: None,
            })
        }
    }

    pub async fn update_mirror_config(&self, mirror_config: MirrorConfig) -> Result<SystemConfig> {
        let value = serde_json::to_string(&mirror_config)?;

        // 应用镜像配置到系统
        self.apply_mirror_config(&mirror_config).await?;

        if let Some(_) = self.get_by_key("mirror").await? {
            let updated = self.update("mirror", UpdateSystemConfig {
                value,
                description: Some("镜像源配置".to_string()),
            }).await?;
            Ok(updated.unwrap())
        } else {
            self.create(CreateSystemConfig {
                key: "mirror".to_string(),
                value,
                description: Some("镜像源配置".to_string()),
            }).await
        }
    }

    // 应用镜像配置到系统
    async fn apply_mirror_config(&self, config: &MirrorConfig) -> Result<()> {
        info!("Applying mirror configuration...");

        // 配置 Node.js 镜像
        if let Some(nodejs) = &config.nodejs {
            if nodejs.enabled {
                if let Some(registry) = &nodejs.registry {
                    info!("Setting npm registry to: {}", registry);
                    let output = Command::new("npm")
                        .args(&["config", "set", "registry", registry])
                        .output();

                    match output {
                        Ok(out) if out.status.success() => {
                            info!("npm registry configured successfully");
                        }
                        Ok(out) => {
                            error!("Failed to set npm registry: {}", String::from_utf8_lossy(&out.stderr));
                        }
                        Err(e) => {
                            error!("npm command not found or failed: {}", e);
                        }
                    }
                }
            }
        }

        // 配置 Python 镜像
        if let Some(python) = &config.python {
            if python.enabled {
                if let Some(index_url) = &python.index_url {
                    info!("Setting pip index to: {}", index_url);

                    // 创建 pip 配置目录
                    let pip_config_dir = std::env::var("HOME")
                        .map(|h| format!("{}/.pip", h))
                        .unwrap_or_else(|_| ".pip".to_string());

                    if let Err(e) = std::fs::create_dir_all(&pip_config_dir) {
                        error!("Failed to create pip config directory: {}", e);
                    } else {
                        let pip_config_file = format!("{}/pip.conf", pip_config_dir);
                        let pip_config_content = format!(
                            "[global]\nindex-url = {}\n[install]\ntrusted-host = {}\n",
                            index_url,
                            index_url.replace("https://", "").replace("http://", "").split('/').next().unwrap_or("")
                        );

                        match std::fs::write(&pip_config_file, pip_config_content) {
                            Ok(_) => info!("pip config written successfully"),
                            Err(e) => error!("Failed to write pip config: {}", e),
                        }
                    }
                }
            }
        }

        // 配置 Linux 镜像
        if let Some(linux) = &config.linux {
            if linux.enabled {
                // APT 源配置 (Debian/Ubuntu)
                if let Some(apt_source) = &linux.apt_source {
                    info!("Setting APT source to: {}", apt_source);

                    // 备份原有配置
                    let _ = Command::new("cp")
                        .args(&["/etc/apt/sources.list", "/etc/apt/sources.list.bak"])
                        .output();

                    // 写入新的源配置
                    match std::fs::write("/etc/apt/sources.list", apt_source) {
                        Ok(_) => {
                            info!("APT sources updated successfully");
                            // 更新软件包列表
                            let _ = Command::new("apt-get")
                                .arg("update")
                                .output();
                        }
                        Err(e) => {
                            error!("Failed to update APT sources (may need root): {}", e);
                        }
                    }
                }

                // YUM 源配置 (CentOS/RHEL)
                if let Some(yum_source) = &linux.yum_source {
                    info!("Setting YUM source to: {}", yum_source);

                    // 备份原有配置
                    let _ = Command::new("cp")
                        .args(&["-r", "/etc/yum.repos.d", "/etc/yum.repos.d.bak"])
                        .output();

                    // 写入新的源配置
                    match std::fs::write("/etc/yum.repos.d/custom.repo", yum_source) {
                        Ok(_) => {
                            info!("YUM sources updated successfully");
                            // 清理缓存
                            let _ = Command::new("yum")
                                .arg("clean")
                                .arg("all")
                                .output();
                        }
                        Err(e) => {
                            error!("Failed to update YUM sources (may need root): {}", e);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    // 在应用启动时加载并应用镜像配置
    pub async fn load_and_apply_mirror_config(&self) -> Result<()> {
        info!("Loading mirror configuration on startup...");
        let config = self.get_mirror_config().await?;
        self.apply_mirror_config(&config).await?;
        Ok(())
    }

    // 获取自动备份配置
    pub async fn get_auto_backup_config(&self) -> Result<AutoBackupConfig> {
        if let Some(config) = self.get_by_key("auto_backup").await? {
            let backup_config: AutoBackupConfig = serde_json::from_str(&config.value)?;
            Ok(backup_config)
        } else {
            Ok(AutoBackupConfig::default())
        }
    }

    // 更新自动备份配置
    pub async fn update_auto_backup_config(&self, config: &AutoBackupConfig) -> Result<()> {
        let value = serde_json::to_string(config)?;

        if self.get_by_key("auto_backup").await?.is_some() {
            self.update("auto_backup", UpdateSystemConfig {
                value,
                description: Some("自动备份配置".to_string()),
            }).await?;
        } else {
            self.create(CreateSystemConfig {
                key: "auto_backup".to_string(),
                value,
                description: Some("自动备份配置".to_string()),
            }).await?;
        }

        Ok(())
    }
}
