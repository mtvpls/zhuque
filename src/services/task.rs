use crate::models::{CreateTask, Task, UpdateTask};
use anyhow::Result;
use chrono::Utc;
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct TaskService {
    pool: Arc<RwLock<SqlitePool>>,
}

impl TaskService {
    pub fn new(pool: Arc<RwLock<SqlitePool>>) -> Self {
        Self { pool }
    }

    pub async fn list(&self) -> Result<Vec<Task>> {
        let pool = self.pool.read().await;
        let tasks = sqlx::query_as::<_, Task>("SELECT * FROM tasks ORDER BY id DESC")
            .fetch_all(&*pool)
            .await?;
        Ok(tasks)
    }

    pub async fn list_with_search(&self, search: Option<&str>) -> Result<Vec<Task>> {
        let pool = self.pool.read().await;
        match search.map(|s| s.trim()).filter(|s| !s.is_empty()) {
            Some(kw) => {
                let pattern = format!("%{}%", kw.to_lowercase());
                let tasks = sqlx::query_as::<_, Task>(
                    "SELECT * FROM tasks WHERE LOWER(name) LIKE ? OR LOWER(command) LIKE ? ORDER BY id DESC",
                )
                .bind(&pattern)
                .bind(&pattern)
                .fetch_all(&*pool)
                .await?;
                Ok(tasks)
            }
            None => {
                let tasks = sqlx::query_as::<_, Task>("SELECT * FROM tasks ORDER BY id DESC")
                    .fetch_all(&*pool)
                    .await?;
                Ok(tasks)
            }
        }
    }

    pub async fn get(&self, id: i64) -> Result<Option<Task>> {
        let pool = self.pool.read().await;
        let task = sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE id = ?")
            .bind(id)
            .fetch_optional(&*pool)
            .await?;
        Ok(task)
    }

    pub async fn create(&self, create: CreateTask) -> Result<Task> {
        let pool = self.pool.read().await;
        // 检查任务名称是否已存在
        let existing = sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE name = ?")
            .bind(&create.name)
            .fetch_optional(&*pool)
            .await?;

        if existing.is_some() {
            return Err(anyhow::anyhow!("任务名称 '{}' 已存在", create.name));
        }

        // 将 cron 数组序列化为 JSON 字符串
        let cron_vec = create.cron.to_vec();
        let cron_json = serde_json::to_string(&cron_vec)?;

        let now = Utc::now();
        let target_type = create.target_type.unwrap_or_else(|| "local".to_string());
        let result = sqlx::query(
            "INSERT INTO tasks (name, command, cron, type, enabled, env, pre_command, post_command, group_id, working_dir, notification, timeout, target_type, target_agent_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&create.name)
        .bind(&create.command)
        .bind(&cron_json)
        .bind(&create.task_type)
        .bind(create.enabled)
        .bind(&create.env)
        .bind(&create.pre_command)
        .bind(&create.post_command)
        .bind(create.group_id)
        .bind(&create.working_dir)
        .bind(&create.notification)
        .bind(create.timeout)
        .bind(&target_type)
        .bind(create.target_agent_id)
        .bind(now)
        .bind(now)
        .execute(&*pool)
        .await?;

        let task = self.get(result.last_insert_rowid()).await?.unwrap();
        Ok(task)
    }

    pub async fn update(&self, id: i64, update: UpdateTask) -> Result<Option<Task>> {
        let pool = self.pool.read().await;
        let mut query = String::from("UPDATE tasks SET updated_at = ?");
        let mut params: Vec<String> = vec![Utc::now().to_rfc3339()];

        if let Some(name) = &update.name {
            query.push_str(", name = ?");
            params.push(name.clone());
        }
        if let Some(command) = &update.command {
            query.push_str(", command = ?");
            params.push(command.clone());
        }
        if let Some(cron) = &update.cron {
            query.push_str(", cron = ?");
            // 将 cron 数组序列化为 JSON 字符串
            let cron_vec = match cron {
                crate::models::CronInput::Single(s) => vec![s.clone()],
                crate::models::CronInput::Multiple(v) => v.clone(),
            };
            params.push(serde_json::to_string(&cron_vec)?);
        }
        if let Some(task_type) = &update.task_type {
            query.push_str(", type = ?");
            params.push(task_type.clone());
        }
        if let Some(enabled) = update.enabled {
            query.push_str(", enabled = ?");
            params.push(enabled.to_string());
        }
        if let Some(env) = &update.env {
            query.push_str(", env = ?");
            params.push(env.clone());
        }
        if let Some(pre_command) = &update.pre_command {
            query.push_str(", pre_command = ?");
            params.push(pre_command.clone());
        }
        if let Some(post_command) = &update.post_command {
            query.push_str(", post_command = ?");
            params.push(post_command.clone());
        }
        if let Some(group_id) = update.group_id {
            query.push_str(", group_id = ?");
            params.push(group_id.to_string());
        }
        if let Some(working_dir) = &update.working_dir {
            query.push_str(", working_dir = ?");
            params.push(working_dir.clone());
        }
        if let Some(notification) = &update.notification {
            query.push_str(", notification = ?");
            params.push(notification.clone());
        }
        if let Some(timeout) = update.timeout {
            query.push_str(", timeout = ?");
            params.push(timeout.to_string());
        }
        if let Some(target_type) = &update.target_type {
            query.push_str(", target_type = ?");
            params.push(target_type.clone());
        }
        if let Some(target_agent_id) = update.target_agent_id {
            query.push_str(", target_agent_id = ?");
            params.push(target_agent_id.to_string());
        }

        query.push_str(" WHERE id = ?");
        params.push(id.to_string());

        let mut q = sqlx::query(&query).bind(Utc::now());

        if let Some(name) = &update.name {
            q = q.bind(name);
        }
        if let Some(command) = &update.command {
            q = q.bind(command);
        }
        if let Some(cron) = &update.cron {
            let cron_vec = match cron {
                crate::models::CronInput::Single(s) => vec![s.clone()],
                crate::models::CronInput::Multiple(v) => v.clone(),
            };
            q = q.bind(serde_json::to_string(&cron_vec)?);
        }
        if let Some(task_type) = &update.task_type {
            q = q.bind(task_type);
        }
        if let Some(enabled) = update.enabled {
            q = q.bind(enabled);
        }
        if let Some(env) = &update.env {
            q = q.bind(env);
        }
        if let Some(pre_command) = &update.pre_command {
            q = q.bind(pre_command);
        }
        if let Some(post_command) = &update.post_command {
            q = q.bind(post_command);
        }
        if let Some(group_id) = update.group_id {
            q = q.bind(group_id);
        }
        if let Some(working_dir) = &update.working_dir {
            q = q.bind(working_dir);
        }
        if let Some(notification) = &update.notification {
            q = q.bind(notification);
        }
        if let Some(timeout) = update.timeout {
            q = q.bind(timeout);
        }
        if let Some(target_type) = &update.target_type {
            q = q.bind(target_type);
        }
        if let Some(target_agent_id) = update.target_agent_id {
            q = q.bind(target_agent_id);
        }

        q = q.bind(id);
        q.execute(&*pool).await?;

        drop(pool);
        self.get(id).await
    }

    pub async fn delete(&self, id: i64) -> Result<bool> {
        let pool = self.pool.read().await;
        let result = sqlx::query("DELETE FROM tasks WHERE id = ?")
            .bind(id)
            .execute(&*pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn get_enabled_tasks(&self) -> Result<Vec<Task>> {
        let pool = self.pool.read().await;
        let tasks = sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE enabled = 1 AND type = 'cron'")
            .fetch_all(&*pool)
            .await?;
        Ok(tasks)
    }

    pub async fn get_startup_tasks(&self) -> Result<Vec<Task>> {
        let pool = self.pool.read().await;
        let tasks = sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE enabled = 1 AND type = 'startup'")
            .fetch_all(&*pool)
            .await?;
        Ok(tasks)
    }

    pub async fn list_by_group(&self, group_id: i64, search: Option<&str>) -> Result<Vec<Task>> {
        let pool = self.pool.read().await;
        let kw = search.map(|s| s.trim()).filter(|s| !s.is_empty());
        if let Some(kw) = kw {
            let pattern = format!("%{}%", kw.to_lowercase());
            let tasks = sqlx::query_as::<_, Task>(
                "SELECT * FROM tasks WHERE group_id = ? AND (LOWER(name) LIKE ? OR LOWER(command) LIKE ?) ORDER BY id DESC",
            )
            .bind(group_id)
            .bind(&pattern)
            .bind(&pattern)
            .fetch_all(&*pool)
            .await?;
            Ok(tasks)
        } else {
            let tasks = sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE group_id = ? ORDER BY id DESC")
                .bind(group_id)
                .fetch_all(&*pool)
                .await?;
            Ok(tasks)
        }
    }

    pub async fn update_run_info(&self, id: i64, last_run_at: chrono::DateTime<chrono::Utc>, duration_ms: i64) -> Result<()> {
        let pool = self.pool.read().await;
        sqlx::query(
            "UPDATE tasks SET last_run_at = ?, last_run_duration = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?"
        )
        .bind(last_run_at)
        .bind(duration_ms)
        .bind(id)
        .execute(&*pool)
        .await?;
        Ok(())
    }

    pub async fn update_next_run_at(&self, id: i64, next_run_at: chrono::DateTime<chrono::Utc>) -> Result<()> {
        let pool = self.pool.read().await;
        sqlx::query(
            "UPDATE tasks SET next_run_at = ? WHERE id = ?"
        )
        .bind(next_run_at)
        .bind(id)
        .execute(&*pool)
        .await?;
        Ok(())
    }
}
