use crate::models::{CreateSubscription, Subscription, UpdateSubscription};
use anyhow::Result;
use chrono::Utc;
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::RwLock;

pub struct SubscriptionService {
    db_pool: Arc<RwLock<SqlitePool>>,
    base_path: PathBuf,
}

impl SubscriptionService {
    pub fn new(db_pool: Arc<RwLock<SqlitePool>>, base_path: PathBuf) -> Self {
        // 克隆的仓库统一放到 scripts/git 目录下
        let git_path = base_path.join("git");
        Self { db_pool, base_path: git_path }
    }

    pub async fn list(&self) -> Result<Vec<Subscription>> {
        let pool = self.db_pool.read().await;
        let subs = sqlx::query_as::<_, Subscription>(
            "SELECT * FROM subscriptions ORDER BY created_at DESC"
        )
        .fetch_all(&*pool)
        .await?;
        Ok(subs)
    }

    pub async fn get(&self, id: i64) -> Result<Option<Subscription>> {
        let pool = self.db_pool.read().await;
        let sub = sqlx::query_as::<_, Subscription>(
            "SELECT * FROM subscriptions WHERE id = ?"
        )
        .bind(id)
        .fetch_optional(&*pool)
        .await?;
        Ok(sub)
    }

    pub async fn create(&self, payload: CreateSubscription) -> Result<Subscription> {
        let pool = self.db_pool.read().await;
        let branch = payload.branch.unwrap_or_else(|| "main".to_string());
        let enabled = payload.enabled.unwrap_or(true);

        let result = sqlx::query(
            r#"
            INSERT INTO subscriptions (name, url, branch, schedule, enabled)
            VALUES (?, ?, ?, ?, ?)
            "#
        )
        .bind(&payload.name)
        .bind(&payload.url)
        .bind(&branch)
        .bind(&payload.schedule)
        .bind(enabled)
        .execute(&*pool)
        .await?;

        let sub = self.get(result.last_insert_rowid()).await?
            .ok_or_else(|| anyhow::anyhow!("Failed to get created subscription"))?;

        Ok(sub)
    }

    pub async fn update(&self, id: i64, payload: UpdateSubscription) -> Result<Option<Subscription>> {
        let pool = self.db_pool.read().await;

        let mut sql = String::from("UPDATE subscriptions SET ");
        let mut updates = Vec::new();
        let mut has_update = false;

        if payload.name.is_some() {
            updates.push("name = ?");
            has_update = true;
        }
        if payload.url.is_some() {
            updates.push("url = ?");
            has_update = true;
        }
        if payload.branch.is_some() {
            updates.push("branch = ?");
            has_update = true;
        }
        if payload.schedule.is_some() {
            updates.push("schedule = ?");
            has_update = true;
        }
        if payload.enabled.is_some() {
            updates.push("enabled = ?");
            has_update = true;
        }

        if !has_update {
            return self.get(id).await;
        }

        updates.push("updated_at = CURRENT_TIMESTAMP");
        sql.push_str(&updates.join(", "));
        sql.push_str(" WHERE id = ?");

        let mut query = sqlx::query(&sql);

        if let Some(name) = payload.name {
            query = query.bind(name);
        }
        if let Some(url) = payload.url {
            query = query.bind(url);
        }
        if let Some(branch) = payload.branch {
            query = query.bind(branch);
        }
        if let Some(schedule) = payload.schedule {
            query = query.bind(schedule);
        }
        if let Some(enabled) = payload.enabled {
            query = query.bind(enabled);
        }

        query = query.bind(id);
        query.execute(&*pool).await?;

        self.get(id).await
    }

    pub async fn delete(&self, id: i64) -> Result<bool> {
        // 先获取订阅信息（不持有锁）
        let sub = self.get(id).await?;

        // 删除订阅目录
        if let Some(sub) = sub {
            let sub_dir = self.base_path.join(&sub.name);
            if sub_dir.exists() {
                tokio::fs::remove_dir_all(sub_dir).await.ok();
            }
        }

        // 再获取连接执行删除操作
        let pool = self.db_pool.read().await;
        let result = sqlx::query("DELETE FROM subscriptions WHERE id = ?")
            .bind(id)
            .execute(&*pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn run(&self, id: i64) -> Result<()> {
        // 检查订阅是否存在
        let sub = self.get(id).await?
            .ok_or_else(|| anyhow::anyhow!("Subscription not found"))?;

        tracing::info!("Starting subscription run for id: {}, name: {}", id, sub.name);

        // 更新状态为 running
        if let Err(e) = self.update_run_status(id, "running", None).await {
            tracing::error!("Failed to update status to running: {}", e);
            return Err(e);
        }

        tracing::info!("Status updated to running for subscription {}", id);

        // 克隆必要的数据用于异步任务
        let base_path = self.base_path.clone();
        let db_pool = self.db_pool.clone();

        // 在后台异步执行 git 操作
        tokio::spawn(async move {
            tracing::info!("Background task started for subscription {}", id);
            let result = Self::run_git_operation(id, sub, base_path, db_pool).await;
            match result {
                Ok(_) => tracing::info!("Subscription {} completed successfully", id),
                Err(e) => tracing::error!("Subscription {} run failed: {}", id, e),
            }
        });

        tracing::info!("Background task spawned for subscription {}", id);
        Ok(())
    }

    async fn run_git_operation(
        id: i64,
        sub: Subscription,
        base_path: PathBuf,
        db_pool: Arc<RwLock<SqlitePool>>,
    ) -> Result<()> {
        tracing::info!("run_git_operation started for subscription {}", id);

        // 确保 git 目录存在
        if let Err(e) = tokio::fs::create_dir_all(&base_path).await {
            let error_msg = format!("Failed to create git directory: {}", e);
            tracing::error!("Subscription {}: {}", id, error_msg);
            let _ = Self::update_status(&db_pool, id, "failed", Some(&error_msg)).await;
            return Err(anyhow::anyhow!(error_msg));
        }

        tracing::info!("Git directory ensured for subscription {}", id);

        let sub_dir = base_path.join(&sub.name);
        let mut log = String::new();

        let result = if sub_dir.exists() {
            tracing::info!("Directory exists, pulling for subscription {}", id);
            // 目录存在，执行 git pull
            log.push_str(&format!("Pulling updates from {}...\n", sub.url));

            // 先尝试清理可能的锁文件
            let git_dir = sub_dir.join(".git");
            if git_dir.exists() {
                let lock_files = vec![
                    git_dir.join("index.lock"),
                    git_dir.join("HEAD.lock"),
                    git_dir.join("refs/heads/main.lock"),
                    git_dir.join("refs/heads/master.lock"),
                ];
                for lock_file in lock_files {
                    if lock_file.exists() {
                        tracing::warn!("Removing stale lock file: {:?}", lock_file);
                        let _ = tokio::fs::remove_file(lock_file).await;
                    }
                }
            }

            let output = match Command::new("git")
                .args(&["-C", sub_dir.to_str().unwrap(), "pull"])
                .output()
                .await
            {
                Ok(output) => {
                    tracing::info!("Git pull command executed for subscription {}", id);
                    output
                },
                Err(e) => {
                    let error_msg = format!("Failed to execute git pull: {}", e);
                    tracing::error!("Subscription {}: {}", id, error_msg);
                    log.push_str(&error_msg);
                    let _ = Self::update_status(&db_pool, id, "failed", Some(&log)).await;
                    return Err(anyhow::anyhow!(error_msg));
                }
            };

            log.push_str(&String::from_utf8_lossy(&output.stdout));
            log.push_str(&String::from_utf8_lossy(&output.stderr));

            if output.status.success() {
                tracing::info!("Git pull succeeded for subscription {}", id);
                let _ = Self::update_status(&db_pool, id, "success", Some(&log)).await;
                Ok(())
            } else {
                tracing::error!("Git pull failed for subscription {}, exit code: {:?}", id, output.status.code());

                // 如果 pull 失败，尝试重置并重新拉取
                log.push_str("\nPull failed, attempting to reset and retry...\n");

                // 先重置到远程分支
                let reset_output = Command::new("git")
                    .args(&["-C", sub_dir.to_str().unwrap(), "reset", "--hard", &format!("origin/{}", sub.branch)])
                    .output()
                    .await;

                if let Ok(reset_out) = reset_output {
                    log.push_str(&String::from_utf8_lossy(&reset_out.stdout));
                    log.push_str(&String::from_utf8_lossy(&reset_out.stderr));

                    if reset_out.status.success() {
                        // 重置成功，再次尝试 pull
                        let retry_output = Command::new("git")
                            .args(&["-C", sub_dir.to_str().unwrap(), "pull"])
                            .output()
                            .await;

                        if let Ok(retry_out) = retry_output {
                            log.push_str(&String::from_utf8_lossy(&retry_out.stdout));
                            log.push_str(&String::from_utf8_lossy(&retry_out.stderr));

                            if retry_out.status.success() {
                                tracing::info!("Git pull retry succeeded for subscription {}", id);
                                let _ = Self::update_status(&db_pool, id, "success", Some(&log)).await;
                                return Ok(());
                            }
                        }
                    }
                }

                // 如果重试还是失败，删除目录并重新克隆
                log.push_str("\nRetry failed, removing directory and cloning fresh...\n");
                if let Err(e) = tokio::fs::remove_dir_all(&sub_dir).await {
                    log.push_str(&format!("Failed to remove directory: {}\n", e));
                }

                // 重新克隆
                let clone_output = Command::new("git")
                    .args(&[
                        "clone",
                        "--depth=1",
                        "--branch",
                        &sub.branch,
                        &sub.url,
                        sub_dir.to_str().unwrap(),
                    ])
                    .output()
                    .await;

                if let Ok(clone_out) = clone_output {
                    log.push_str(&String::from_utf8_lossy(&clone_out.stdout));
                    log.push_str(&String::from_utf8_lossy(&clone_out.stderr));

                    if clone_out.status.success() {
                        tracing::info!("Fresh clone succeeded for subscription {}", id);
                        let _ = Self::update_status(&db_pool, id, "success", Some(&log)).await;
                        return Ok(());
                    }
                }

                let _ = Self::update_status(&db_pool, id, "failed", Some(&log)).await;
                Err(anyhow::anyhow!("Git pull failed and recovery attempts failed"))
            }
        } else {
            tracing::info!("Directory does not exist, cloning for subscription {}", id);
            // 目录不存在，执行 git clone
            log.push_str(&format!("Cloning repository from {}...\n", sub.url));

            let output = match Command::new("git")
                .args(&[
                    "clone",
                    "--depth=1",
                    "--branch",
                    &sub.branch,
                    &sub.url,
                    sub_dir.to_str().unwrap(),
                ])
                .output()
                .await
            {
                Ok(output) => {
                    tracing::info!("Git clone command executed for subscription {}", id);
                    output
                },
                Err(e) => {
                    let error_msg = format!("Failed to execute git clone: {}", e);
                    tracing::error!("Subscription {}: {}", id, error_msg);
                    log.push_str(&error_msg);
                    let _ = Self::update_status(&db_pool, id, "failed", Some(&log)).await;
                    return Err(anyhow::anyhow!(error_msg));
                }
            };

            log.push_str(&String::from_utf8_lossy(&output.stdout));
            log.push_str(&String::from_utf8_lossy(&output.stderr));

            if output.status.success() {
                tracing::info!("Git clone succeeded for subscription {}", id);
                let _ = Self::update_status(&db_pool, id, "success", Some(&log)).await;
                Ok(())
            } else {
                tracing::error!("Git clone failed for subscription {}, exit code: {:?}", id, output.status.code());
                let _ = Self::update_status(&db_pool, id, "failed", Some(&log)).await;
                Err(anyhow::anyhow!("Git clone failed"))
            }
        };

        result
    }

    async fn update_status(
        db_pool: &Arc<RwLock<SqlitePool>>,
        id: i64,
        status: &str,
        log: Option<&str>,
    ) -> Result<()> {
        // 获取连接，执行更新，立即释放
        {
            let pool = db_pool.read().await;
            sqlx::query(
                r#"
                UPDATE subscriptions
                SET last_run_time = ?, last_run_status = ?, last_run_log = ?, updated_at = CURRENT_TIMESTAMP
                WHERE id = ?
                "#
            )
            .bind(Utc::now())
            .bind(status)
            .bind(log)
            .bind(id)
            .execute(&*pool)
            .await?;
        } // 锁在这里被释放
        Ok(())
    }

    async fn update_run_status(&self, id: i64, status: &str, log: Option<&str>) -> Result<()> {
        let pool = self.db_pool.read().await;
        sqlx::query(
            r#"
            UPDATE subscriptions
            SET last_run_time = ?, last_run_status = ?, last_run_log = ?, updated_at = CURRENT_TIMESTAMP
            WHERE id = ?
            "#
        )
        .bind(Utc::now())
        .bind(status)
        .bind(log)
        .bind(id)
        .execute(&*pool)
        .await?;
        Ok(())
    }

    pub async fn list_enabled(&self) -> Result<Vec<Subscription>> {
        let pool = self.db_pool.read().await;
        let subs = sqlx::query_as::<_, Subscription>(
            "SELECT * FROM subscriptions WHERE enabled = 1"
        )
        .fetch_all(&*pool)
        .await?;
        Ok(subs)
    }
}
