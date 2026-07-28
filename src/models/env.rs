use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct EnvVar {
    pub id: i64,
    pub key: String,
    pub value: String,
    pub remark: Option<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateEnvVar {
    pub key: String,
    pub value: String,
    pub remark: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateEnvVar {
    pub key: Option<String>,
    pub value: Option<String>,
    pub remark: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BatchVar {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BatchImportRequest {
    pub vars: Vec<BatchVar>,
    pub overwrite: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BatchImportResponse {
    pub created: usize,
    pub updated: usize,
    pub conflicts: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BatchDeleteRequest {
    pub ids: Vec<i64>,
}
