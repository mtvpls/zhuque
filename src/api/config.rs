use crate::api::AppState;
use crate::models::{MirrorConfig, UpdateSystemConfig, AutoBackupConfig};
use crate::services::WebDavClient;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

// 获取所有配置
pub async fn list_configs(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let configs = state
        .config_service
        .list()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(configs))
}

// 获取指定配置
pub async fn get_config(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let config = state
        .config_service
        .get_by_key(&key)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    match config {
        Some(c) => Ok(Json(c)),
        None => Err((StatusCode::NOT_FOUND, "配置不存在".to_string())),
    }
}

// 更新配置
pub async fn update_config(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    Json(update): Json<UpdateSystemConfig>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let config = state
        .config_service
        .update(&key, update)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    match config {
        Some(c) => Ok(Json(c)),
        None => Err((StatusCode::NOT_FOUND, "配置不存在".to_string())),
    }
}

// 删除配置
pub async fn delete_config(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let deleted = state
        .config_service
        .delete(&key)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, "配置不存在".to_string()))
    }
}

// 获取镜像源配置
pub async fn get_mirror_config(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let config = state
        .config_service
        .get_mirror_config()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(config))
}

// 更新镜像源配置
pub async fn update_mirror_config(
    State(state): State<Arc<AppState>>,
    Json(mirror_config): Json<MirrorConfig>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let config = state
        .config_service
        .update_mirror_config(mirror_config)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(config))
}

// 获取自动备份配置
pub async fn get_auto_backup_config(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let config = state
        .config_service
        .get_auto_backup_config()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(config))
}

// 更新自动备份配置
pub async fn update_auto_backup_config(
    State(state): State<Arc<AppState>>,
    Json(backup_config): Json<AutoBackupConfig>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    state
        .config_service
        .update_auto_backup_config(&backup_config)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // 重新加载备份调度器
    if let Some(backup_scheduler) = &state.backup_scheduler {
        backup_scheduler
            .reload_backup_job()
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    Ok(Json(backup_config))
}

// 测试 WebDAV 连接
pub async fn test_webdav_connection(
    Json(backup_config): Json<AutoBackupConfig>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let client = WebDavClient::new(
        backup_config.webdav_url,
        backup_config.webdav_username,
        backup_config.webdav_password,
    );

    client
        .test_connection()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("连接失败: {}", e)))?;

    Ok(Json(serde_json::json!({ "success": true, "message": "连接成功" })))
}
