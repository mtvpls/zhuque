use crate::models::{Claims, LoginRequest, LoginResponse, LoginStepOneResponse};
use crate::services::{ConfigService, UserService};
use anyhow::{anyhow, Result};
use chrono::Utc;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

const TOKEN_EXPIRATION_DAYS: i64 = 7;
const SESSION_TOKEN_EXPIRATION: i64 = 300; // 5分钟

pub struct AuthService {
    jwt_secret: String,
    config_service: Option<Arc<ConfigService>>,
    user_service: Arc<UserService>,
}

impl AuthService {
    pub fn new(user_service: Arc<UserService>) -> Result<Self> {
        let jwt_secret = match std::env::var("JWT_SECRET") {
            Ok(secret) if !secret.is_empty() => {
                info!("Using JWT_SECRET from environment variable");
                secret
            }
            _ => {
                let generated_secret = Uuid::new_v4().to_string();
                info!("JWT_SECRET not set, generated random secret: {}", generated_secret);
                generated_secret
            }
        };

        Ok(Self {
            jwt_secret,
            config_service: None,
            user_service,
        })
    }

    pub fn set_config_service(&mut self, config_service: Arc<ConfigService>) {
        self.config_service = Some(config_service);
    }

    /// 第一步登录：验证用户名密码
    pub async fn login_step_one(&self, request: &LoginRequest) -> Result<LoginStepOneResponse> {
        // 使用 UserService 验证用户名密码
        if !self.user_service.verify_password(&request.username, &request.password).await? {
            return Err(anyhow!("Invalid username or password"));
        }

        // 检查是否启用TOTP
        let totp_enabled = if let Some(config_service) = &self.config_service {
            match config_service.get_by_key("totp_enabled").await {
                Ok(Some(config)) => config.value == "true",
                _ => false,
            }
        } else {
            false
        };

        if totp_enabled {
            // 生成临时session token
            let session_token = self.generate_session_token(&request.username)?;
            Ok(LoginStepOneResponse {
                requires_totp: true,
                session_token: Some(session_token),
                token: None,
                expires_in: None,
            })
        } else {
            // 直接生成JWT token
            let response = self.generate_jwt_token(&request.username)?;
            Ok(LoginStepOneResponse {
                requires_totp: false,
                session_token: None,
                token: Some(response.token),
                expires_in: Some(response.expires_in),
            })
        }
    }

    /// 第二步登录：验证TOTP（由API层调用TotpService验证）
    pub fn login_step_two(&self, username: &str) -> Result<LoginResponse> {
        self.generate_jwt_token(username)
    }

    /// 生成临时session token（5分钟有效）
    fn generate_session_token(&self, username: &str) -> Result<String> {
        let now = Utc::now().timestamp();
        let exp = now + SESSION_TOKEN_EXPIRATION;

        let claims = Claims {
            sub: format!("session:{}", username),
            exp,
            iat: now,
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )?;

        Ok(token)
    }

    /// 验证session token
    pub fn verify_session_token(&self, token: &str) -> Result<String> {
        let claims = self.verify_token(token)?;

        // 确保是session token
        if !claims.sub.starts_with("session:") {
            return Err(anyhow!("Invalid session token"));
        }

        // 提取用户名
        let username = claims.sub.strip_prefix("session:")
            .ok_or_else(|| anyhow!("Invalid session token format"))?;

        Ok(username.to_string())
    }

    /// 生成JWT token
    fn generate_jwt_token(&self, username: &str) -> Result<LoginResponse> {
        let now = Utc::now().timestamp();
        let expires_in = TOKEN_EXPIRATION_DAYS * 24 * 60 * 60;
        let exp = now + expires_in;

        let claims = Claims {
            sub: username.to_string(),
            exp,
            iat: now,
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )?;

        Ok(LoginResponse {
            token,
            expires_in,
        })
    }

    pub fn verify_token(&self, token: &str) -> Result<Claims> {
        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret.as_bytes()),
            &Validation::default(),
        )?;

        Ok(token_data.claims)
    }
}
