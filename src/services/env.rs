use crate::models::{BatchImportResponse, BatchVar, CreateEnvVar, EnvVar, UpdateEnvVar};
use anyhow::Result;
use chrono::Utc;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct EnvService {
    pool: Arc<RwLock<SqlitePool>>,
}

impl EnvService {
    pub fn new(pool: Arc<RwLock<SqlitePool>>) -> Self {
        Self { pool }
    }

    /// 获取所有环境变量
    pub async fn list(&self) -> Result<Vec<EnvVar>> {
        let pool = self.pool.read().await;
        let vars = sqlx::query_as::<_, EnvVar>("SELECT * FROM env_vars ORDER BY key ASC")
            .fetch_all(&*pool)
            .await?;
        Ok(vars)
    }

    /// 获取环境变量列表（支持关键字搜索）
    pub async fn list_with_search(&self, search: Option<&str>) -> Result<Vec<EnvVar>> {
        let pool = self.pool.read().await;
        match search.map(|s| s.trim()).filter(|s| !s.is_empty()) {
            Some(kw) => {
                let pattern = format!("%{}%", kw.to_lowercase());
                let vars = sqlx::query_as::<_, EnvVar>(
                    "SELECT * FROM env_vars WHERE LOWER(key) LIKE ? OR LOWER(value) LIKE ? OR LOWER(COALESCE(remark,'')) LIKE ? ORDER BY key ASC",
                )
                .bind(&pattern)
                .bind(&pattern)
                .bind(&pattern)
                .fetch_all(&*pool)
                .await?;
                Ok(vars)
            }
            None => {
                let vars = sqlx::query_as::<_, EnvVar>("SELECT * FROM env_vars ORDER BY key ASC")
                    .fetch_all(&*pool)
                    .await?;
                Ok(vars)
            }
        }
    }

    /// 获取所有环境变量作为HashMap（只返回启用的）
    pub async fn get_all_as_map(&self) -> Result<HashMap<String, String>> {
        let vars = self.list().await?;
        let map = vars
            .into_iter()
            .filter(|v| v.enabled)
            .map(|v| (v.key, v.value))
            .collect();
        Ok(map)
    }

    /// 获取单个环境变量
    pub async fn get(&self, id: i64) -> Result<Option<EnvVar>> {
        let pool = self.pool.read().await;
        let var = sqlx::query_as::<_, EnvVar>("SELECT * FROM env_vars WHERE id = ?")
            .bind(id)
            .fetch_optional(&*pool)
            .await?;
        Ok(var)
    }

    /// 根据key获取环境变量
    pub async fn get_by_key(&self, key: &str) -> Result<Option<EnvVar>> {
        let pool = self.pool.read().await;
        let var = sqlx::query_as::<_, EnvVar>("SELECT * FROM env_vars WHERE key = ?")
            .bind(key)
            .fetch_optional(&*pool)
            .await?;
        Ok(var)
    }

    /// 创建环境变量
    pub async fn create(&self, create: CreateEnvVar) -> Result<EnvVar> {
        let pool = self.pool.read().await;
        let now = Utc::now();
        let enabled = create.enabled.unwrap_or(true);
        let result = sqlx::query(
            "INSERT INTO env_vars (key, value, remark, enabled, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&create.key)
        .bind(&create.value)
        .bind(&create.remark)
        .bind(enabled)
        .bind(now)
        .bind(now)
        .execute(&*pool)
        .await?;

        drop(pool);
        let var = self.get(result.last_insert_rowid()).await?.unwrap();
        Ok(var)
    }

    /// 更新环境变量
    pub async fn update(&self, id: i64, update: UpdateEnvVar) -> Result<Option<EnvVar>> {
        let pool = self.pool.read().await;
        let mut query = String::from("UPDATE env_vars SET updated_at = ?");
        let mut has_update = false;

        if update.key.is_some() {
            query.push_str(", key = ?");
            has_update = true;
        }
        if update.value.is_some() {
            query.push_str(", value = ?");
            has_update = true;
        }
        if update.remark.is_some() {
            query.push_str(", remark = ?");
            has_update = true;
        }
        if update.enabled.is_some() {
            query.push_str(", enabled = ?");
            has_update = true;
        }

        if !has_update {
            drop(pool);
            return self.get(id).await;
        }

        query.push_str(" WHERE id = ?");

        let mut q = sqlx::query(&query).bind(Utc::now());

        if let Some(key) = &update.key {
            q = q.bind(key);
        }
        if let Some(value) = &update.value {
            q = q.bind(value);
        }
        if let Some(remark) = &update.remark {
            q = q.bind(remark);
        }
        if let Some(enabled) = update.enabled {
            q = q.bind(enabled);
        }

        q = q.bind(id);
        q.execute(&*pool).await?;

        drop(pool);
        self.get(id).await
    }

    /// 批量导入环境变量
    pub async fn batch_import(
        &self,
        vars: Vec<BatchVar>,
        overwrite: bool,
    ) -> Result<BatchImportResponse> {
        let existing = self.list().await?;
        let existing_map: std::collections::HashMap<String, i64> =
            existing.iter().map(|v| (v.key.clone(), v.id)).collect();

        let conflicts: Vec<String> = vars
            .iter()
            .filter(|v| existing_map.contains_key(&v.key))
            .map(|v| v.key.clone())
            .collect();

        if !overwrite && !conflicts.is_empty() {
            return Ok(BatchImportResponse {
                created: 0,
                updated: 0,
                conflicts,
            });
        }

        let mut created = 0usize;
        let mut updated = 0usize;

        for var in vars {
            if let Some(&id) = existing_map.get(&var.key) {
                self.update(
                    id,
                    UpdateEnvVar {
                        key: None,
                        value: Some(var.value),
                        remark: None,
                        enabled: None,
                    },
                )
                .await?;
                updated += 1;
            } else {
                self.create(CreateEnvVar {
                    key: var.key,
                    value: var.value,
                    remark: None,
                    enabled: Some(true),
                })
                .await?;
                created += 1;
            }
        }

        Ok(BatchImportResponse {
            created,
            updated,
            conflicts: vec![],
        })
    }

    /// 删除环境变量
    pub async fn delete(&self, id: i64) -> Result<bool> {
        let pool = self.pool.read().await;
        let result = sqlx::query("DELETE FROM env_vars WHERE id = ?")
            .bind(id)
            .execute(&*pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// 批量删除环境变量
    pub async fn batch_delete(&self, ids: Vec<i64>) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let pool = self.pool.read().await;
        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!("DELETE FROM env_vars WHERE id IN ({})", placeholders);
        let mut q = sqlx::query(&sql);
        for id in &ids {
            q = q.bind(*id);
        }
        let result = q.execute(&*pool).await?;
        Ok(result.rows_affected() as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_service() -> EnvService {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE env_vars (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                key TEXT NOT NULL UNIQUE,
                value TEXT NOT NULL,
                remark TEXT,
                enabled BOOLEAN NOT NULL DEFAULT 1,
                created_at DATETIME NOT NULL,
                updated_at DATETIME NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        EnvService::new(Arc::new(RwLock::new(pool)))
    }

    #[tokio::test]
    async fn update_can_rename_key() {
        let service = test_service().await;
        let created = service
            .create(CreateEnvVar {
                key: "OLD_KEY".to_string(),
                value: "value".to_string(),
                remark: None,
                enabled: Some(true),
            })
            .await
            .unwrap();

        let updated = service
            .update(
                created.id,
                UpdateEnvVar {
                    key: Some("NEW_KEY".to_string()),
                    value: Some("updated".to_string()),
                    remark: None,
                    enabled: None,
                },
            )
            .await
            .unwrap()
            .unwrap();

        assert_eq!(updated.key, "NEW_KEY");
        assert_eq!(updated.value, "updated");
        assert!(service.get_by_key("OLD_KEY").await.unwrap().is_none());
        assert_eq!(
            service.get_by_key("NEW_KEY").await.unwrap().unwrap().id,
            created.id
        );
    }
}
