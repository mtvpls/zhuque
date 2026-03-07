use crate::models::User;
use anyhow::{anyhow, Result};
use bcrypt::{hash, verify, DEFAULT_COST};
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct UserService {
    pool: Arc<RwLock<SqlitePool>>,
}

impl UserService {
    pub fn new(pool: Arc<RwLock<SqlitePool>>) -> Self {
        Self { pool }
    }

    // 检查是否需要初始设置
    pub async fn needs_initial_setup(&self) -> Result<bool> {
        let pool = self.pool.read().await;
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&*pool)
            .await?;
        Ok(count.0 == 0)
    }

    // 创建初始用户
    pub async fn create_initial_user(&self, username: &str, password: &str) -> Result<User> {
        // 检查是否已有用户
        if !self.needs_initial_setup().await? {
            return Err(anyhow!("Initial setup already completed"));
        }

        // 验证输入
        if username.len() < 3 {
            return Err(anyhow!("Username must be at least 3 characters"));
        }
        if password.len() < 6 {
            return Err(anyhow!("Password must be at least 6 characters"));
        }

        let password_hash = hash(password, DEFAULT_COST)?;
        let pool = self.pool.read().await;

        let result = sqlx::query(
            "INSERT INTO users (username, password_hash) VALUES (?, ?)"
        )
        .bind(username)
        .bind(&password_hash)
        .execute(&*pool)
        .await?;

        let id = result.last_insert_rowid();
        drop(pool);

        self.get_by_id(id).await?.ok_or_else(|| anyhow!("Failed to create user"))
    }

    // 根据用户名获取用户
    pub async fn get_by_username(&self, username: &str) -> Result<Option<User>> {
        let pool = self.pool.read().await;
        let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE username = ?")
            .bind(username)
            .fetch_optional(&*pool)
            .await?;
        Ok(user)
    }

    // 验证密码
    pub async fn verify_password(&self, username: &str, password: &str) -> Result<bool> {
        let user = self.get_by_username(username).await?
            .ok_or_else(|| anyhow!("User not found"))?;
        Ok(verify(password, &user.password_hash)?)
    }

    // 更新密码
    pub async fn update_password(&self, username: &str, old_password: &str, new_password: &str) -> Result<()> {
        // 验证旧密码
        if !self.verify_password(username, old_password).await? {
            return Err(anyhow!("Invalid old password"));
        }

        // 验证新密码
        if new_password.len() < 6 {
            return Err(anyhow!("Password must be at least 6 characters"));
        }

        let new_hash = hash(new_password, DEFAULT_COST)?;
        let pool = self.pool.read().await;

        sqlx::query("UPDATE users SET password_hash = ?, updated_at = CURRENT_TIMESTAMP WHERE username = ?")
            .bind(&new_hash)
            .bind(username)
            .execute(&*pool)
            .await?;

        Ok(())
    }

    // 根据ID获取用户
    async fn get_by_id(&self, id: i64) -> Result<Option<User>> {
        let pool = self.pool.read().await;
        let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(&*pool)
            .await?;
        Ok(user)
    }
}
