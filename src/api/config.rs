use crate::api::AppState;
use crate::models::{AutoBackupConfig, MirrorConfig, UpdateSystemConfig, WebSearchConfig};
use crate::services::{S3Client, WebDavClient};
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
    // 先尝试更新
    let config = state
        .config_service
        .update(&key, update.clone())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // 如果配置不存在，则创建
    match config {
        Some(c) => Ok(Json(c)),
        None => {
            let created = state
                .config_service
                .create(crate::models::CreateSystemConfig {
                    key,
                    value: update.value,
                    description: update.description,
                })
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            Ok(Json(created))
        }
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

// 获取联网搜索配置
pub async fn get_web_search_config(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let config = state
        .config_service
        .get_web_search_config()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(config))
}

// 更新联网搜索配置
pub async fn update_web_search_config(
    State(state): State<Arc<AppState>>,
    Json(config): Json<WebSearchConfig>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    state
        .config_service
        .update_web_search_config(&config)
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

// 测试备份目标连接（按 provider 分派：WebDAV / S3 兼容对象存储）
pub async fn test_backup_connection(
    Json(backup_config): Json<AutoBackupConfig>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if backup_config.is_s3() {
        let client = S3Client::new(
            backup_config.s3_endpoint,
            backup_config.s3_region,
            backup_config.s3_bucket,
            backup_config.s3_access_key_id,
            backup_config.s3_secret_access_key,
        )
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("S3 客户端初始化失败: {}", e)))?;

        client
            .test_connection()
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("连接失败: {}", e)))?;
    } else {
        let client = WebDavClient::new(
            backup_config.webdav_url,
            backup_config.webdav_username,
            backup_config.webdav_password,
        );

        client
            .test_connection()
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("连接失败: {}", e)))?;
    }

    Ok(Json(serde_json::json!({ "success": true, "message": "连接成功" })))
}

// 立即备份到所选存储（WebDAV / S3 兼容对象存储）
pub async fn backup_now(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // 获取自动备份配置
    let backup_config = state
        .config_service
        .get_auto_backup_config()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // 验证配置（按 provider 校验必填项）
    let missing = backup_config.missing_fields();
    if !missing.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "{} 配置不完整，缺少: {}",
                if backup_config.is_s3() { "S3" } else { "WebDAV" },
                missing.join(", ")
            ),
        ));
    }

    // 在后台执行备份
    tokio::spawn(async move {
        use crate::scheduler::BackupScheduler;
        use tracing::{error, info};

        info!("Manual backup triggered");
        match BackupScheduler::perform_backup_static(&backup_config).await {
            Ok(_) => info!("Manual backup completed successfully"),
            Err(e) => error!("Manual backup failed: {}", e),
        }
    });

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "备份任务已启动，正在后台执行"
    })))
}
