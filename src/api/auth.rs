use crate::api::AppState;
use crate::models::{LoginRequest, TotpVerifyRequest, InitialSetupRequest, InitialSetupStatusResponse, UpdatePasswordRequest, Claims};
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize)]
pub struct TotpEnableRequestFull {
    pub secret: String,
    pub backup_codes: Vec<String>,
    pub code: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TotpCodeRequest {
    pub code: String,
}

// 检查是否需要初始设置
pub async fn check_initial_setup(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    match state.user_service.needs_initial_setup().await {
        Ok(needs_setup) => Ok(Json(InitialSetupStatusResponse { needs_setup })),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

// 初始设置（创建第一个用户）
pub async fn initial_setup(
    State(state): State<Arc<AppState>>,
    Json(request): Json<InitialSetupRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    match state.user_service.create_initial_user(&request.username, &request.password).await {
        Ok(_) => Ok(Json(serde_json::json!({ "success": true }))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

// 修改密码（需要认证）
pub async fn change_password(
    State(state): State<Arc<AppState>>,
    claims: Claims,  // 从中间件注入
    Json(request): Json<UpdatePasswordRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    match state.user_service.update_password(&claims.sub, &request.old_password, &request.new_password).await {
        Ok(_) => Ok(Json(serde_json::json!({ "success": true }))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

/// 第一步登录：验证用户名密码
pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(request): Json<LoginRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    match state.auth_service.login_step_one(&request).await {
        Ok(response) => Ok(Json(response)),
        Err(e) => Err((StatusCode::UNAUTHORIZED, e.to_string())),
    }
}

/// 第二步登录：验证TOTP码
pub async fn verify_totp(
    State(state): State<Arc<AppState>>,
    Json(request): Json<TotpVerifyRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // 验证session token
    let username = match state.auth_service.verify_session_token(&request.session_token) {
        Ok(username) => username,
        Err(e) => return Err((StatusCode::UNAUTHORIZED, e.to_string())),
    };

    // 验证TOTP码
    match state.totp_service.verify_code(&request.code).await {
        Ok(true) => {
            // 生成JWT token
            match state.auth_service.login_step_two(&username) {
                Ok(response) => Ok(Json(response)),
                Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
            }
        }
        Ok(false) => Err((StatusCode::UNAUTHORIZED, "Invalid verification code".to_string())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// 获取TOTP状态
pub async fn get_totp_status(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    match state.totp_service.is_enabled().await {
        Ok(enabled) => Ok(Json(serde_json::json!({ "enabled": enabled }))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// 初始化TOTP设置
pub async fn setup_totp(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    match state.totp_service.generate_setup(&claims.sub).await {
        Ok(response) => Ok(Json(response)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// 启用TOTP
pub async fn enable_totp(
    State(state): State<Arc<AppState>>,
    Json(request): Json<TotpEnableRequestFull>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    match state.totp_service.enable_totp(&request.secret, &request.backup_codes, &request.code).await {
        Ok(_) => Ok(Json(serde_json::json!({ "success": true }))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

/// 禁用TOTP
pub async fn disable_totp(
    State(state): State<Arc<AppState>>,
    Json(request): Json<TotpCodeRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // 验证TOTP码
    match state.totp_service.verify_code(&request.code).await {
        Ok(true) => {
            match state.totp_service.disable_totp().await {
                Ok(_) => Ok(Json(serde_json::json!({ "success": true }))),
                Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
            }
        }
        Ok(false) => Err((StatusCode::BAD_REQUEST, "验证码错误".to_string())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// 重新生成备用码
pub async fn regenerate_backup_codes(
    State(state): State<Arc<AppState>>,
    Json(request): Json<TotpCodeRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // 验证TOTP码
    match state.totp_service.verify_code(&request.code).await {
        Ok(true) => {
            match state.totp_service.regenerate_backup_codes().await {
                Ok(codes) => Ok(Json(serde_json::json!({ "backup_codes": codes }))),
                Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
            }
        }
        Ok(false) => Err((StatusCode::BAD_REQUEST, "验证码错误".to_string())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}
