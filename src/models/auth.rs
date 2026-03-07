use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginResponse {
    pub token: String,
    pub expires_in: i64, // 秒
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginStepOneResponse {
    pub requires_totp: bool,
    pub session_token: Option<String>, // 临时token，用于第二步验证
    pub token: Option<String>,         // 如果不需要TOTP，直接返回JWT
    pub expires_in: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TotpVerifyRequest {
    pub session_token: String,
    pub code: String, // 6位验证码或16位备用码
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TotpSetupResponse {
    pub secret: String,           // Base32编码的密钥
    pub qr_code: String,          // Base64编码的二维码图片
    pub backup_codes: Vec<String>, // 备用恢复码
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TotpEnableRequest {
    pub code: String, // 验证码，用于确认设置
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TotpStatusResponse {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // username or session:username
    pub exp: i64,    // 过期时间
    pub iat: i64,    // 签发时间
}

// Axum extractor for Claims
#[async_trait]
impl<S> FromRequestParts<S> for Claims
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Claims>()
            .cloned()
            .ok_or((StatusCode::UNAUTHORIZED, "Unauthorized"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct User {
    pub id: i64,
    pub username: String,
    #[serde(skip_serializing)]  // 永远不序列化密码哈希
    pub password_hash: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InitialSetupRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InitialSetupStatusResponse {
    pub needs_setup: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdatePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}
