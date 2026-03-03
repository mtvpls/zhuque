use axum::{
    body::Body,
    extract::{Multipart, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use flate2::{write::GzEncoder, read::GzDecoder, Compression};
use serde_json::json;
use std::sync::Arc;
use tar::{Archive, Builder};
use tokio::fs;
use tracing::{error, info, warn};

use crate::api::AppState;

#[cfg(unix)]
async fn fix_permissions(dir: &str) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    use tokio::fs;

    let mut entries = fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let metadata = fs::metadata(&path).await?;

        if metadata.is_dir() {
            // 目录权限: 0o755 (rwxr-xr-x)
            let mut perms = metadata.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).await?;
            // 递归处理子目录
            Box::pin(fix_permissions(path.to_str().unwrap())).await?;
        } else {
            // 文件权限: 0o644 (rw-r--r--)
            let mut perms = metadata.permissions();
            perms.set_mode(0o644);
            fs::set_permissions(&path, perms).await?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
async fn fix_permissions(_dir: &str) -> std::io::Result<()> {
    // Windows 不需要修改权限
    Ok(())
}

pub async fn create_backup(
    State(_state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, StatusCode> {
    let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "./data".into());
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let backup_filename = format!("zhuque_backup_{}.tar.gz", timestamp);

    info!("Creating backup from: {}", data_dir);

    // 在内存中创建 tar.gz
    let mut tar_gz_data = Vec::new();
    {
        let encoder = GzEncoder::new(&mut tar_gz_data, Compression::default());
        let mut tar = Builder::new(encoder);

        // 递归添加 data 目录下的所有文件
        let data_path = std::path::Path::new(&data_dir);
        if data_path.exists() {
            tar.append_dir_all("data", &data_dir).map_err(|e| {
                error!("Failed to add directory to tar: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
        }

        tar.finish().map_err(|e| {
            error!("Failed to finish tar: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    }

    info!("Backup created successfully: {} bytes", tar_gz_data.len());

    // 返回文件下载响应
    let content_disposition = format!("attachment; filename=\"{}\"", backup_filename);
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/gzip".to_string()),
            (header::CONTENT_DISPOSITION, content_disposition),
        ],
        Body::from(tar_gz_data),
    ))
}

pub async fn restore_backup(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, StatusCode> {
    let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "./data".into());
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");

    info!("Starting restore process");

    let mut backup_data = Vec::new();

    // 接收上传的文件
    while let Some(field) = multipart.next_field().await.map_err(|e| {
        error!("Failed to read multipart field: {}", e);
        StatusCode::BAD_REQUEST
    })? {
        let name = field.name().unwrap_or("");

        if name == "file" {
            backup_data = field.bytes().await.map_err(|e| {
                error!("Failed to read file data: {}", e);
                StatusCode::BAD_REQUEST
            })?.to_vec();

            info!("Received backup file: {} bytes", backup_data.len());
            break;
        }
    }

    if backup_data.is_empty() {
        error!("No file uploaded");
        return Err(StatusCode::BAD_REQUEST);
    }

    // 创建当前数据的备份（在内存中）
    info!("Creating backup of current data");
    let mut current_backup_data = Vec::new();
    {
        let encoder = GzEncoder::new(&mut current_backup_data, Compression::default());
        let mut tar = Builder::new(encoder);

        let data_path = std::path::Path::new(&data_dir);
        if data_path.exists() {
            if let Err(e) = tar.append_dir_all("data", &data_dir) {
                warn!("Failed to backup current data: {}, continuing anyway", e);
            }
        }

        if let Err(e) = tar.finish() {
            warn!("Failed to finish current backup: {}, continuing anyway", e);
        }
    }

    // 保存当前备份到临时文件
    let parent_dir = std::path::Path::new(&data_dir)
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or(".");
    let current_backup_path = format!("{}/zhuque_before_restore_{}.tar.gz", parent_dir, timestamp);

    if !current_backup_data.is_empty() {
        if let Err(e) = fs::write(&current_backup_path, &current_backup_data).await {
            warn!("Failed to save current backup: {}", e);
        }
    }

    // 清空 data 目录
    info!("Cleaning data directory");
    let data_path = std::path::Path::new(&data_dir);
    if data_path.exists() {
        if let Err(e) = tokio::fs::remove_dir_all(&data_dir).await {
            error!("Failed to clean data directory: {}", e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    // 创建 data 目录
    if let Err(e) = tokio::fs::create_dir_all(&data_dir).await {
        error!("Failed to create data directory: {}", e);
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    // 解压备份文件
    info!("Extracting backup");
    let decoder = GzDecoder::new(&backup_data[..]);
    let mut archive = Archive::new(decoder);

    // 获取父目录路径
    let parent_path = std::path::Path::new(parent_dir);

    if let Err(e) = archive.unpack(parent_path) {
        error!("Failed to extract backup: {}", e);
        return Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": "Failed to extract backup file",
                "current_backup": current_backup_path
            }))
        ));
    }

    // 修复文件权限
    info!("Fixing file permissions");
    if let Err(e) = fix_permissions(&data_dir).await {
        warn!("Failed to fix permissions: {}", e);
    }

    // 重新初始化数据库连接
    info!("Reinitializing database connections");
    if let Err(e) = state.reinit_database().await {
        error!("Failed to reinitialize database: {}", e);
        return Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": "备份恢复成功，但数据库重新初始化失败，请手动重启服务"
            }))
        ));
    }

    // 删除当前数据的备份（恢复成功）
    let _ = fs::remove_file(&current_backup_path).await;

    info!("Restore completed successfully");

    Ok((
        StatusCode::OK,
        Json(json!({
            "success": true,
            "message": "备份恢复成功，数据库已重新初始化。"
        }))
    ))
}
