use anyhow::Result;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::path::Path;

pub async fn init_db(database_url: &str) -> Result<SqlitePool> {
    // 确保数据库文件所在目录存在
    if let Some(parent) = Path::new(database_url.trim_start_matches("sqlite://")).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(20)
        .acquire_timeout(std::time::Duration::from_secs(30))
        .idle_timeout(std::time::Duration::from_secs(300))
        .connect(&format!("{}?mode=rwc", database_url))
        .await?;

    // 启用 WAL 模式（Write-Ahead Logging）- 关键！允许并发读写
    sqlx::query("PRAGMA journal_mode = WAL")
        .execute(&pool)
        .await?;

    // 设置繁忙超时（毫秒）- 避免立即失败
    sqlx::query("PRAGMA busy_timeout = 5000")
        .execute(&pool)
        .await?;

    // 同步模式设置为 NORMAL（平衡性能和安全性）
    sqlx::query("PRAGMA synchronous = NORMAL")
        .execute(&pool)
        .await?;

    // 启用增量自动压缩
    sqlx::query("PRAGMA auto_vacuum = INCREMENTAL")
        .execute(&pool)
        .await?;

    // 设置页面大小（在创建表之前设置，只对新数据库生效）
    sqlx::query("PRAGMA page_size = 4096")
        .execute(&pool)
        .await?;

    // 创建任务分组表
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS task_groups (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            description TEXT,
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
        )
        "#,
    )
    .execute(&pool)
    .await?;

    // 创建表
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS tasks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            command TEXT NOT NULL,
            cron TEXT NOT NULL,
            type TEXT NOT NULL DEFAULT 'cron',
            enabled BOOLEAN NOT NULL DEFAULT 1,
            env TEXT,
            pre_command TEXT,
            post_command TEXT,
            group_id INTEGER,
            last_run_at DATETIME,
            last_run_duration INTEGER,
            next_run_at DATETIME,
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (group_id) REFERENCES task_groups(id) ON DELETE SET NULL
        )
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id INTEGER NOT NULL,
            output TEXT NOT NULL,
            status TEXT NOT NULL,
            duration INTEGER,
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (task_id) REFERENCES tasks(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(&pool)
    .await?;

    // 创建索引
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_logs_task_id ON logs(task_id)")
        .execute(&pool)
        .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_logs_created_at ON logs(created_at)")
        .execute(&pool)
        .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_tasks_group_id ON tasks(group_id)")
        .execute(&pool)
        .await?;

    // 创建依赖表
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS dependences (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            type INTEGER NOT NULL,
            status INTEGER NOT NULL DEFAULT 0,
            log TEXT,
            remark TEXT,
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
        )
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_dependences_type ON dependences(type)")
        .execute(&pool)
        .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_dependences_status ON dependences(status)")
        .execute(&pool)
        .await?;

    // 创建环境变量表
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS env_vars (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            key TEXT NOT NULL UNIQUE,
            value TEXT NOT NULL,
            remark TEXT,
            enabled BOOLEAN NOT NULL DEFAULT 1,
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
        )
        "#,
    )
    .execute(&pool)
    .await?;

    // 添加enabled字段（如果表已存在）
    sqlx::query("ALTER TABLE env_vars ADD COLUMN enabled BOOLEAN NOT NULL DEFAULT 1")
        .execute(&pool)
        .await
        .ok(); // 忽略错误，字段可能已存在

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_env_vars_key ON env_vars(key)")
        .execute(&pool)
        .await?;

    // 创建订阅表
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS subscriptions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            url TEXT NOT NULL,
            branch TEXT NOT NULL DEFAULT 'main',
            schedule TEXT NOT NULL,
            enabled BOOLEAN NOT NULL DEFAULT 1,
            last_run_time DATETIME,
            last_run_status TEXT,
            last_run_log TEXT,
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
        )
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_subscriptions_enabled ON subscriptions(enabled)")
        .execute(&pool)
        .await?;

    // 创建系统配置表
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS system_configs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            key TEXT NOT NULL UNIQUE,
            value TEXT NOT NULL,
            description TEXT,
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
        )
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_system_configs_key ON system_configs(key)")
        .execute(&pool)
        .await?;

    // 迁移：添加 type 列（如果不存在）
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN type TEXT NOT NULL DEFAULT 'cron'")
        .execute(&pool)
        .await;

    // 数据库迁移：添加 working_dir 字段到 tasks 表
    sqlx::query("ALTER TABLE tasks ADD COLUMN working_dir TEXT")
        .execute(&pool)
        .await
        .ok(); // 忽略错误，字段可能已存在

    // 数据库迁移：添加 duration 字段到 logs 表
    sqlx::query("ALTER TABLE logs ADD COLUMN duration INTEGER")
        .execute(&pool)
        .await
        .ok(); // 忽略错误，字段可能已存在

    // 执行增量压缩回收空间
    sqlx::query("PRAGMA incremental_vacuum")
        .execute(&pool)
        .await
        .ok(); // 忽略错误

    Ok(pool)
}
