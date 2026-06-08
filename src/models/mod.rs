pub mod auth;
pub mod config;
pub mod db;
pub mod dependence;
pub mod env;
pub mod remote;
pub mod subscription;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

pub use auth::*;
pub use config::*;
pub use dependence::*;
pub use env::*;
pub use remote::*;
pub use subscription::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: i64,
    pub name: String,
    pub command: String,
    pub cron: Vec<String>, // 支持多个 cron 表达式
    #[serde(rename = "type")]
    pub task_type: String, // cron/manual/startup
    pub enabled: bool,
    pub env: Option<String>, // JSON格式的环境变量
    pub pre_command: Option<String>,
    pub post_command: Option<String>,
    pub group_id: Option<i64>,
    pub working_dir: Option<String>, // 自定义工作目录
    pub notification: Option<String>, // JSON 格式的任务级通知配置（TaskNotificationConfig）
    pub timeout: i64, // 执行超时（秒），0 表示不超时
    pub target_type: String, // local/remote
    pub target_agent_id: Option<i64>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_run_duration: Option<i64>, // 毫秒
    pub next_run_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// 手动实现 FromRow，以便处理 cron 字段的 JSON 反序列化
impl<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> for Task {
    fn from_row(row: &'r sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;

        let cron_str: String = row.try_get("cron")?;
        let cron: Vec<String> = serde_json::from_str(&cron_str)
            .unwrap_or_else(|_| vec![cron_str.clone()]);

        Ok(Task {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            command: row.try_get("command")?,
            cron,
            task_type: row.try_get("type")?,
            enabled: row.try_get("enabled")?,
            env: row.try_get("env")?,
            pre_command: row.try_get("pre_command")?,
            post_command: row.try_get("post_command")?,
            group_id: row.try_get("group_id")?,
            working_dir: row.try_get("working_dir").ok().flatten(),
            notification: row.try_get("notification").ok().flatten(),
            timeout: row.try_get::<Option<i64>, _>("timeout").ok().flatten().unwrap_or(0),
            target_type: row
                .try_get::<Option<String>, _>("target_type")
                .ok()
                .flatten()
                .unwrap_or_else(|| "local".to_string()),
            target_agent_id: row.try_get("target_agent_id").ok().flatten(),
            last_run_at: row.try_get("last_run_at")?,
            last_run_duration: row.try_get("last_run_duration")?,
            next_run_at: row.try_get("next_run_at")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateTask {
    pub name: String,
    pub command: String,
    pub cron: CronInput, // 支持多个 cron 表达式
    #[serde(rename = "type")]
    pub task_type: String, // cron/manual/startup
    pub enabled: bool,
    pub env: Option<String>,
    pub pre_command: Option<String>,
    pub post_command: Option<String>,
    pub group_id: Option<i64>,
    pub working_dir: Option<String>,
    pub notification: Option<String>, // JSON 格式的任务级通知配置
    #[serde(default)]
    pub timeout: i64, // 执行超时（秒），0 表示不超时
    pub target_type: Option<String>,
    pub target_agent_id: Option<i64>,
}

// 用于接收前端输入的 cron，支持字符串或数组
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CronInput {
    Single(String),
    Multiple(Vec<String>),
}

impl CronInput {
    pub fn to_vec(self) -> Vec<String> {
        match self {
            CronInput::Single(s) => vec![s],
            CronInput::Multiple(v) => v,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateTask {
    pub name: Option<String>,
    pub command: Option<String>,
    pub cron: Option<CronInput>, // 支持多个 cron 表达式
    #[serde(rename = "type")]
    pub task_type: Option<String>, // cron/manual/startup
    pub enabled: Option<bool>,
    pub env: Option<String>,
    pub pre_command: Option<String>,
    pub post_command: Option<String>,
    pub group_id: Option<i64>,
    pub working_dir: Option<String>,
    pub notification: Option<String>, // JSON 格式的任务级通知配置
    pub timeout: Option<i64>, // 执行超时（秒），0 表示不超时
    pub target_type: Option<String>,
    pub target_agent_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Log {
    pub id: i64,
    pub task_id: i64,
    pub output: String,
    pub status: String, // success/failed
    pub duration: Option<i64>, // 执行耗时（毫秒）
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ScriptFile {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified: DateTime<Utc>,
    pub is_directory: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TaskGroup {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateTaskGroup {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateTaskGroup {
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct LoginLog {
    pub id: i64,
    pub username: String,
    pub ip_address: String,
    pub created_at: DateTime<Utc>,
}
