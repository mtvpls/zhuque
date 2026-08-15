use axum::{
    body::Body,
    extract::{Multipart, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use serde_json::json;
use sqlx::{sqlite::SqliteConnectOptions, Connection, SqliteConnection};
use std::{
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
use tar::{Archive, Builder};
use tokio::{fs, io::AsyncWriteExt, sync::Mutex};
use tracing::{error, info, warn};

use crate::api::AppState;
use crate::models::db::init_db;

const MAX_UNPACKED_BACKUP_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_BACKUP_ENTRIES: usize = 100_000;
const RESTORE_STAGING_PREFIX: &str = ".zhuque_restore_";

// ponytail: Process-local lock; use a filesystem lock if DATA_DIR is shared by multiple instances.
static BACKUP_OPERATION_LOCK: Mutex<()> = Mutex::const_new(());

#[cfg(unix)]
async fn fix_permissions(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut entries = fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).await?;

        if metadata.is_dir() {
            let mut perms = metadata.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).await?;
            Box::pin(fix_permissions(&path)).await?;
        } else if !metadata.file_type().is_symlink() {
            let mut perms = metadata.permissions();
            perms.set_mode(0o644);
            fs::set_permissions(&path, perms).await?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
async fn fix_permissions(_dir: &Path) -> io::Result<()> {
    Ok(())
}

fn write_backup_archive(data_dir: &Path, backup_path: &Path) -> io::Result<()> {
    let backup_file = std::fs::File::create(backup_path)?;
    let encoder = GzEncoder::new(backup_file, Compression::default());
    let mut archive = Builder::new(encoder);

    if data_dir.exists() {
        archive.append_dir_all("data", data_dir)?;
    }

    archive.finish()?;
    archive.into_inner()?.finish()?;
    Ok(())
}

async fn create_backup_file_unlocked(data_dir: PathBuf, backup_path: PathBuf) -> io::Result<()> {
    cleanup_restore_staging(&data_dir).await?;
    tokio::task::spawn_blocking(move || write_backup_archive(&data_dir, &backup_path))
        .await
        .map_err(io::Error::other)?
}

pub(crate) async fn create_backup_file(
    data_dir: PathBuf,
    backup_path: PathBuf,
) -> io::Result<()> {
    let _guard = BACKUP_OPERATION_LOCK.lock().await;
    create_backup_file_unlocked(data_dir, backup_path).await
}

fn validate_archive_limits(
    backup_path: &Path,
    max_bytes: u64,
    max_entries: usize,
) -> io::Result<()> {
    let backup_file = std::fs::File::open(backup_path)?;
    let decoder = GzDecoder::new(backup_file);
    let mut archive = Archive::new(decoder);
    let mut total_bytes = 0_u64;
    let mut entry_count = 0_usize;

    for entry in archive
        .entries()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
    {
        let entry = entry.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        entry_count = entry_count.checked_add(1).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "backup has too many entries")
        })?;
        if entry_count > max_entries {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "backup has too many entries",
            ));
        }

        total_bytes = total_bytes.checked_add(entry.size()).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "backup is too large")
        })?;
        if total_bytes > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "backup expands beyond the allowed size",
            ));
        }
    }

    Ok(())
}

fn unpack_backup_archive(backup_path: &Path, destination: &Path) -> io::Result<()> {
    validate_archive_limits(
        backup_path,
        MAX_UNPACKED_BACKUP_BYTES,
        MAX_BACKUP_ENTRIES,
    )?;
    let backup_file = std::fs::File::open(backup_path)?;
    let decoder = GzDecoder::new(backup_file);
    Archive::new(decoder).unpack(destination)
}

async fn validate_database(path: &Path) -> io::Result<()> {
    let options = SqliteConnectOptions::new().filename(path);
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let check = sqlx::query_scalar::<_, String>("PRAGMA quick_check")
        .fetch_all(&mut connection)
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if check.as_slice() != ["ok"] {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("SQLite quick_check failed: {}", check.join(", ")),
        ));
    }

    let has_tasks = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'tasks'",
    )
    .fetch_one(&mut connection)
    .await
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if has_tasks != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "backup database does not contain the tasks table",
        ));
    }

    connection
        .close()
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

async fn remove_path(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path).await?;
    if metadata.is_dir() {
        fs::remove_dir_all(path).await
    } else {
        fs::remove_file(path).await
    }
}

async fn cleanup_restore_staging(data_dir: &Path) -> io::Result<()> {
    let mut entries = match fs::read_dir(data_dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };

    while let Some(entry) = entries.next_entry().await? {
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(RESTORE_STAGING_PREFIX)
        {
            remove_path(&entry.path()).await?;
        }
    }

    Ok(())
}

async fn clear_directory_contents_except(dir: &Path, keep: Option<&Path>) -> io::Result<()> {
    fs::create_dir_all(dir).await?;
    let mut entries = fs::read_dir(dir).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if keep == Some(path.as_path()) {
            continue;
        }
        remove_path(&path).await?;
    }

    Ok(())
}

pub(crate) async fn prepare_backup_file(
    backup_path: &Path,
    data_dir: &Path,
) -> io::Result<PathBuf> {
    cleanup_restore_staging(data_dir).await?;
    fs::create_dir_all(data_dir).await?;
    let staging_dir = data_dir.join(format!("{}{}", RESTORE_STAGING_PREFIX, uuid::Uuid::new_v4()));
    fs::create_dir(&staging_dir).await?;

    let archive_path = backup_path.to_path_buf();
    let extract_path = staging_dir.clone();
    let unpack_result = match tokio::task::spawn_blocking(move || {
        unpack_backup_archive(&archive_path, &extract_path)
    })
    .await
    {
        Ok(result) => result,
        Err(e) => Err(io::Error::other(e)),
    };

    if let Err(e) = unpack_result {
        let _ = fs::remove_dir_all(&staging_dir).await;
        return Err(e);
    }

    let validation_result = async {
        let staged_data = staging_dir.join("data");
        let data_metadata = fs::symlink_metadata(&staged_data).await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "backup archive does not contain a data directory",
            )
        })?;
        let database_metadata = fs::symlink_metadata(staged_data.join("app.db"))
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "backup archive does not contain data/app.db",
                )
            })?;

        if !data_metadata.is_dir()
            || data_metadata.file_type().is_symlink()
            || !database_metadata.is_file()
            || database_metadata.file_type().is_symlink()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "backup archive has an invalid data layout",
            ));
        }

        validate_database(&staged_data.join("app.db")).await?;

        let mut top_level = fs::read_dir(&staging_dir).await?;
        while let Some(entry) = top_level.next_entry().await? {
            if entry.file_name() != OsStr::new("data") {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "backup archive contains files outside the data directory",
                ));
            }
        }

        Ok(())
    }
    .await;

    if let Err(e) = validation_result {
        let _ = fs::remove_dir_all(&staging_dir).await;
        return Err(e);
    }

    Ok(staging_dir)
}

pub(crate) async fn activate_prepared_backup(
    data_dir: &Path,
    staging_dir: &Path,
) -> io::Result<()> {
    clear_directory_contents_except(data_dir, Some(staging_dir)).await?;

    let staged_data = staging_dir.join("data");
    let mut entries = fs::read_dir(&staged_data).await?;
    while let Some(entry) = entries.next_entry().await? {
        fs::rename(entry.path(), data_dir.join(entry.file_name())).await?;
    }

    fs::remove_dir_all(staging_dir).await
}

pub(crate) async fn restore_backup_file(backup_path: &Path, data_dir: &Path) -> io::Result<()> {
    let staging_dir = prepare_backup_file(backup_path, data_dir).await?;
    activate_prepared_backup(data_dir, &staging_dir).await
}

fn database_url(data_dir: &Path) -> String {
    format!("sqlite://{}/app.db", data_dir.display())
}

pub async fn create_backup(
    State(_state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, StatusCode> {
    let data_dir = PathBuf::from(std::env::var("DATA_DIR").unwrap_or_else(|_| "./data".into()));
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let backup_filename = format!("zhuque_backup_{}.tar.gz", timestamp);

    info!("Creating backup from: {}", data_dir.display());

    let parent_dir = data_dir.parent().unwrap_or(Path::new("."));
    let backup_path = parent_dir.join(&backup_filename);

    create_backup_file(data_dir, backup_path.clone())
        .await
        .map_err(|e| {
            error!("Failed to create backup: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // 读取备份文件
    let backup_data = fs::read(&backup_path).await.map_err(|e| {
        error!("Failed to read backup file: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    info!("Backup created successfully: {} bytes", backup_data.len());

    // 删除临时备份文件
    let _ = fs::remove_file(&backup_path).await;

    // 返回文件下载响应
    let content_disposition = format!("attachment; filename=\"{}\"", backup_filename);
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/gzip".to_string()),
            (header::CONTENT_DISPOSITION, content_disposition),
        ],
        Body::from(backup_data),
    ))
}

pub async fn restore_backup(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let data_dir = PathBuf::from(std::env::var("DATA_DIR").unwrap_or_else(|_| "./data".into()));
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let request_id = uuid::Uuid::new_v4();

    info!("Starting restore process");

    let data_dir_abs = std::fs::canonicalize(&data_dir)
        .unwrap_or_else(|_| std::env::current_dir().unwrap().join(&data_dir));
    let parent_dir = data_dir_abs
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();

    let uploaded_backup_path = parent_dir.join(format!(".zhuque_uploaded_{}.tar.gz", request_id));
    let mut file_received = false;
    let mut totp_code: Option<String> = None;

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(e) => {
                error!(
                    "Failed to read multipart field: {}; cause: {}",
                    e,
                    e.body_text()
                );
                let _ = fs::remove_file(&uploaded_backup_path).await;
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "success": false,
                        "message": "上传中断或请求格式错误，请重新选择备份文件后重试"
                    })),
                );
            }
        };
        let name = field.name().unwrap_or("").to_string();
        info!("Processing multipart field: {}", name);

        if name == "file" {
            let mut field = field;
            let mut output = match fs::File::create(&uploaded_backup_path).await {
                Ok(output) => output,
                Err(e) => {
                    error!("Failed to create uploaded file: {}", e);
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "success": false,
                            "message": format!("保存上传文件失败: {}", e)
                        })),
                    );
                }
            };
            let mut received_bytes = 0_u64;

            loop {
                let chunk = match field.chunk().await {
                    Ok(Some(chunk)) => chunk,
                    Ok(None) => break,
                    Err(e) => {
                        error!("Failed to read file data: {}; cause: {}", e, e.body_text());
                        drop(output);
                        let _ = fs::remove_file(&uploaded_backup_path).await;
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(json!({
                                "success": false,
                                "message": "备份文件上传不完整，请重试"
                            })),
                        );
                    }
                };

                if let Err(e) = output.write_all(&chunk).await {
                    error!("Failed to write uploaded file: {}", e);
                    drop(output);
                    let _ = fs::remove_file(&uploaded_backup_path).await;
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "success": false,
                            "message": format!("保存上传文件失败: {}", e)
                        })),
                    );
                }
                received_bytes += chunk.len() as u64;
            }

            if let Err(e) = output.flush().await {
                error!("Failed to flush uploaded file: {}", e);
                drop(output);
                let _ = fs::remove_file(&uploaded_backup_path).await;
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "success": false,
                        "message": format!("保存上传文件失败: {}", e)
                    })),
                );
            }
            drop(output);

            info!("Received backup file: {} bytes", received_bytes);
            file_received = true;
        } else if name == "totp_code" {
            let text = match field.text().await {
                Ok(text) => text,
                Err(e) => {
                    error!(
                        "Failed to read totp_code field: {}; cause: {}",
                        e,
                        e.body_text()
                    );
                    let _ = fs::remove_file(&uploaded_backup_path).await;
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({
                            "success": false,
                            "message": "TOTP验证码请求格式错误"
                        })),
                    );
                }
            };
            totp_code = Some(text);
            info!("Received TOTP code");
        }
    }

    if !file_received {
        error!("No file uploaded");
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "未接收到备份文件，请选择一个有效的 .tar.gz 文件"
            })),
        );
    }

    let totp_enabled = match state.totp_service.is_enabled().await {
        Ok(enabled) => enabled,
        Err(e) => {
            error!("Failed to check TOTP status: {}", e);
            let _ = fs::remove_file(&uploaded_backup_path).await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "success": false,
                    "message": "读取TOTP配置失败，恢复已取消"
                })),
            );
        }
    };

    if totp_enabled {
        match totp_code {
            Some(code) => match state.totp_service.verify_code(&code).await {
                Ok(true) => info!("TOTP verification successful"),
                Ok(false) => {
                    error!("Invalid TOTP code");
                    let _ = fs::remove_file(&uploaded_backup_path).await;
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({
                            "success": false,
                            "message": "TOTP验证码错误"
                        })),
                    );
                }
                Err(e) => {
                    error!("Failed to verify TOTP: {}", e);
                    let _ = fs::remove_file(&uploaded_backup_path).await;
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "success": false,
                            "message": "TOTP验证失败"
                        })),
                    );
                }
            },
            None => {
                error!("TOTP is enabled but no code provided");
                let _ = fs::remove_file(&uploaded_backup_path).await;
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "success": false,
                        "message": "需要提供TOTP验证码",
                        "requires_totp": true
                    })),
                );
            }
        }
    }

    info!("Creating backup of current data");
    let current_backup_path = parent_dir.join(format!(
        "zhuque_before_restore_{}_{}.tar.gz",
        timestamp,
        request_id.simple()
    ));
    let current_backup_display = current_backup_path.display().to_string();

    let operation_guard = BACKUP_OPERATION_LOCK.lock().await;
    let mut pool = state.db_pool.write().await;
    match sqlx::query_as::<_, (i64, i64, i64)>("PRAGMA wal_checkpoint(TRUNCATE)")
        .fetch_one(&*pool)
        .await
    {
        Ok((0, _, _)) => {}
        Ok((busy, _, _)) => {
            error!(
                "Failed to checkpoint database before restore: busy={}",
                busy
            );
            let _ = fs::remove_file(&uploaded_backup_path).await;
            return (
                StatusCode::CONFLICT,
                Json(json!({
                    "success": false,
                    "message": "数据库正在使用中，请稍后重试"
                })),
            );
        }
        Err(e) => {
            error!("Failed to checkpoint database before restore: {}", e);
            let _ = fs::remove_file(&uploaded_backup_path).await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "success": false,
                    "message": format!("创建恢复点失败: {}，未修改现有数据", e)
                })),
            );
        }
    }

    if let Err(e) =
        create_backup_file_unlocked(data_dir_abs.clone(), current_backup_path.clone()).await
    {
        error!("Failed to create current backup: {}", e);
        let _ = fs::remove_file(&uploaded_backup_path).await;
        let _ = fs::remove_file(&current_backup_path).await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": format!("创建当前数据回滚备份失败: {}，未修改现有数据", e)
            })),
        );
    }

    info!("Validating uploaded backup");
    let staging_dir = match prepare_backup_file(&uploaded_backup_path, &data_dir_abs).await {
        Ok(path) => path,
        Err(e) => {
            error!("Failed to validate backup: {}", e);
            let _ = fs::remove_file(&uploaded_backup_path).await;
            let _ = fs::remove_file(&current_backup_path).await;
            let status = if e.kind() == io::ErrorKind::InvalidData {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            return (
                status,
                Json(json!({
                    "success": false,
                    "message": format!("备份文件校验失败: {}", e)
                })),
            );
        }
    };

    info!("Closing database connections");
    pool.close().await;

    info!("Activating restored data");
    let restore_result = async {
        activate_prepared_backup(&data_dir_abs, &staging_dir).await?;
        if let Err(e) = fix_permissions(&data_dir_abs).await {
            warn!("Failed to fix permissions: {}", e);
        }
        init_db(&database_url(&data_dir_abs))
            .await
            .map_err(io::Error::other)
    }
    .await;

    let new_pool = match restore_result {
        Ok(pool) => pool,
        Err(restore_error) => {
            error!("Failed to activate restored data: {}", restore_error);
            warn!("Rolling back current data from: {}", current_backup_display);

            let rollback_result = async {
                restore_backup_file(&current_backup_path, &data_dir_abs).await?;
                if let Err(e) = fix_permissions(&data_dir_abs).await {
                    warn!("Failed to fix permissions after rollback: {}", e);
                }
                init_db(&database_url(&data_dir_abs))
                    .await
                    .map_err(io::Error::other)
            }
            .await;

            let _ = fs::remove_file(&uploaded_backup_path).await;
            return match rollback_result {
                Ok(rollback_pool) => {
                    *pool = rollback_pool;
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "success": false,
                            "message": format!("备份恢复失败: {}。当前数据已自动回滚。", restore_error),
                            "current_backup": current_backup_display
                        })),
                    )
                }
                Err(rollback_error) => {
                    error!("Failed to roll back current data: {}", rollback_error);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "success": false,
                            "message": format!("备份恢复失败: {}；自动回滚失败: {}。数据库访问已停止，请使用恢复点修复数据后重启服务", restore_error, rollback_error),
                            "current_backup": current_backup_display
                        })),
                    )
                }
            };
        }
    };

    *pool = new_pool;
    drop(pool);
    let _ = fs::remove_file(&uploaded_backup_path).await;

    let _ = fs::remove_file(&current_backup_path).await;
    drop(operation_guard);

    if let Err(e) = state.scheduler.reload_tasks().await {
        warn!("Failed to reload tasks after restore: {}", e);
    }
    if let Err(e) = state.subscription_scheduler.reload_subscriptions().await {
        warn!("Failed to reload subscriptions after restore: {}", e);
    }
    if let Some(scheduler) = &state.backup_scheduler {
        if let Err(e) = scheduler.reload_backup_job().await {
            warn!("Failed to reload backup schedule after restore: {}", e);
        }
    }

    info!("Restore completed successfully");

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "message": "备份恢复成功，数据库已重新初始化。"
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn restore_replaces_contents_without_replacing_data_directory() {
        let root =
            std::env::temp_dir().join(format!("zhuque-restore-test-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        let data_dir = root.join("mounted-data");
        let archive = root.join("backup.tar.gz");

        fs::create_dir_all(source.join("scripts")).await.unwrap();
        let options = SqliteConnectOptions::new()
            .filename(source.join("app.db"))
            .create_if_missing(true);
        let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
        sqlx::query("CREATE TABLE tasks (id INTEGER PRIMARY KEY)")
            .execute(&mut connection)
            .await
            .unwrap();
        sqlx::query("INSERT INTO tasks (id) VALUES (42)")
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();
        fs::write(source.join("scripts/task.py"), b"print('ok')")
            .await
            .unwrap();
        fs::create_dir_all(&data_dir).await.unwrap();
        fs::write(data_dir.join("old.txt"), b"old").await.unwrap();

        let stale_staging = data_dir.join(format!("{}stale", RESTORE_STAGING_PREFIX));
        fs::create_dir_all(&stale_staging).await.unwrap();
        fs::write(stale_staging.join("leftover"), b"stale")
            .await
            .unwrap();
        create_backup_file(data_dir.clone(), root.join("current.tar.gz"))
            .await
            .unwrap();
        assert!(!stale_staging.exists());

        let invalid_archive = root.join("invalid.tar.gz");
        fs::write(&invalid_archive, b"not a backup").await.unwrap();
        assert!(restore_backup_file(&invalid_archive, &data_dir)
            .await
            .is_err());
        assert_eq!(fs::read(data_dir.join("old.txt")).await.unwrap(), b"old");
        assert_eq!(std::fs::read_dir(&data_dir).unwrap().count(), 1);

        #[cfg(unix)]
        let inode_before = {
            use std::os::unix::fs::MetadataExt;
            std::fs::metadata(&data_dir).unwrap().ino()
        };

        write_backup_archive(&source, &archive).unwrap();
        assert_eq!(
            validate_archive_limits(&archive, 1, MAX_BACKUP_ENTRIES)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        restore_backup_file(&archive, &data_dir).await.unwrap();

        assert!(data_dir.is_dir());
        assert!(!data_dir.join("old.txt").exists());
        validate_database(&data_dir.join("app.db")).await.unwrap();
        assert!(data_dir.join("scripts/task.py").is_file());

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(inode_before, std::fs::metadata(&data_dir).unwrap().ino());
        }

        fs::remove_dir_all(root).await.unwrap();
    }
}
