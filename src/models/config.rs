use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SystemConfig {
    pub id: i64,
    pub key: String,
    pub value: String, // JSON格式
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateSystemConfig {
    pub key: String,
    pub value: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSystemConfig {
    pub value: String,
    pub description: Option<String>,
}

// 镜像源配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirrorConfig {
    pub linux: Option<LinuxMirror>,
    pub nodejs: Option<NodejsMirror>,
    pub python: Option<PythonMirror>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinuxMirror {
    pub enabled: bool,
    pub apt_source: Option<String>, // Debian/Ubuntu
    pub yum_source: Option<String>, // CentOS/RHEL
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodejsMirror {
    pub enabled: bool,
    pub registry: Option<String>, // npm registry
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PythonMirror {
    pub enabled: bool,
    pub index_url: Option<String>, // pip index
}

// AI Provider 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    pub enabled: bool,
    pub provider: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default = "default_ai_context_window_tokens")]
    pub context_window_tokens: u32,
    #[serde(default = "default_ai_compression_ratio")]
    pub compression_ratio: u8,
}

fn default_ai_context_window_tokens() -> u32 {
    131_072
}

fn default_ai_compression_ratio() -> u8 {
    90
}

impl AiConfig {
    pub fn normalized(&self) -> Self {
        let mut value = self.clone();
        value.context_window_tokens = value.context_window_tokens.clamp(8_192, 131_072);
        value.compression_ratio = value.compression_ratio.clamp(10, 95);
        value
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "OpenAI Compatible".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            model: String::new(),
            context_window_tokens: default_ai_context_window_tokens(),
            compression_ratio: default_ai_compression_ratio(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoBackupConfig {
    pub enabled: bool,
    pub webdav_url: String,
    pub webdav_username: String,
    pub webdav_password: String,
    pub cron: String,
    pub remote_path: Option<String>, // WebDAV 远程路径，默认为根目录
    pub max_backups: Option<u32>,    // 最大保留备份数量，None 表示不限制
}

impl Default for AutoBackupConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            webdav_url: String::new(),
            webdav_username: String::new(),
            webdav_password: String::new(),
            cron: "0 2 * * *".to_string(), // 默认每天凌晨2点（5字段格式）
            remote_path: None,
            max_backups: Some(10), // 默认保留10个备份
        }
    }
}

// ─── 通知配置 ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationConfig {
    pub enabled: bool,
    pub on_success: bool,
    pub on_failure: bool,
    pub on_killed: bool,
    #[serde(default)]
    pub on_login: bool,
    pub channels: Vec<ChannelConfig>,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            on_success: false,
            on_failure: true,
            on_killed: true,
            on_login: false,
            channels: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    #[serde(default = "new_uuid")]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type")]
    pub channel_type: String,
    pub enabled: bool,
    pub config: serde_json::Value,
}

fn new_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TelegramConfig {
    pub bot_token: String,
    pub chat_id: String,
    pub proxy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PushPlusConfig {
    pub token: String,
    pub topic: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub from: String,
    pub to: Vec<String>,
    pub use_tls: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResendConfig {
    pub api_key: String,
    pub from: String,
    pub to: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WeComConfig {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub webhook_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub url: String,
    #[serde(default = "default_method")]
    pub method: String,
    #[serde(default)]
    pub headers: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub body_template: String,
}

fn default_method() -> String {
    "POST".to_string()
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            method: "POST".to_string(),
            headers: std::collections::HashMap::new(),
            body_template: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DingTalkConfig {
    pub access_token: String,
    pub secret: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FeishuConfig {
    pub webhook_url: String,
    pub sign_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BarkConfig {
    pub server_url: String,
    pub device_key: String,
    pub sound: Option<String>,
    pub group: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NtfyConfig {
    pub server_url: String,
    pub topic: String,
    pub token: Option<String>,
    #[serde(default)]
    pub priority: u8,
}

// ─── 任务级通知配置（可覆盖全局通知配置）─────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNotificationConfig {
    /// 是否启用任务级别覆盖
    pub enabled: bool,
    /// 覆盖：成功时是否通知（None = 跟随全局）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_success: Option<bool>,
    /// 覆盖：失败时是否通知（None = 跟随全局）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_failure: Option<bool>,
    /// 覆盖：终止时是否通知（None = 跟随全局）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_killed: Option<bool>,
    /// 覆盖：指定生效的渠道 ID 列表（None 或空 = 使用全局所有已启用渠道）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_ids: Option<Vec<String>>,
}
