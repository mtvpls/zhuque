mod api;
mod middleware;
mod models;
mod scheduler;
mod services;
mod utils;

use anyhow::Result;
use api::AppState;
use models::db::init_db;
use scheduler::{Scheduler, SubscriptionScheduler, BackupScheduler};
use services::{AuthService, ConfigService, DependenceService, EnvService, Executor, LogService, LoginLogService, NotificationService, NotifyTokenRegistry, ScriptService, SubscriptionService, SystemLogCollector, TaskService, TaskGroupService, TotpService, UserService};

#[cfg(not(target_os = "android"))]
use services::TerminalService;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[cfg(feature = "jemalloc")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[cfg(feature = "jemalloc")]
#[export_name = "malloc_conf"]
pub static MALLOC_CONF: &[u8] = b"dirty_decay_ms:10000,muzzy_decay_ms:10000,background_thread:true\0";

#[tokio::main]
async fn main() -> Result<()> {
    // 创建日志目录
    let log_dir = PathBuf::from("./logs");
    tokio::fs::create_dir_all(&log_dir).await?;

    // 创建文件日志 appender（每天滚动）
    let file_appender = tracing_appender::rolling::daily(&log_dir, "zhuque.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    // 创建系统日志收集器
    let system_log_collector = SystemLogCollector::new(100);
    let log_layer = services::system_log::SystemLogLayer::new(system_log_collector.clone());

    // 初始化日志
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "zhuque=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer()) // 控制台输出
        .with(tracing_subscriber::fmt::layer().with_writer(non_blocking)) // 文件输出
        .with(log_layer)
        .init();

    info!("Starting Zhuque...");

    // 配置
    let data_dir = PathBuf::from(std::env::var("DATA_DIR").unwrap_or_else(|_| "./data".into()));
    let data_dir = if data_dir.is_absolute() {
        data_dir
    } else {
        std::env::current_dir().unwrap_or_default().join(&data_dir)
    };
    let database_url = format!("sqlite://{}/app.db", data_dir.display());
    let scripts_dir = data_dir.join("scripts");
    let port = std::env::var("PORT")
        .unwrap_or_else(|_| "3000".into())
        .parse::<u16>()?;

    // 检查是否需要自动恢复备份（在初始化数据库之前）
    info!("Checking auto restore configuration...");

    let auto_restore_enabled = std::env::var("AUTO_RESTORE_ON_STARTUP")
        .ok()
        .and_then(|v| v.parse::<bool>().ok())
        .unwrap_or(false);

    let env_webdav_url = std::env::var("WEBDAV_URL").ok();
    let env_webdav_username = std::env::var("WEBDAV_USERNAME").ok();
    let env_webdav_password = std::env::var("WEBDAV_PASSWORD").ok();
    let env_remote_path = std::env::var("WEBDAV_REMOTE_PATH").ok();

    // S3 兼容对象存储（Cloudflare R2 / MinIO / Wasabi 等）
    let env_s3_endpoint = std::env::var("S3_ENDPOINT").ok();
    let env_s3_region = std::env::var("S3_REGION").ok();
    let env_s3_bucket = std::env::var("S3_BUCKET").ok();
    let env_s3_access_key_id = std::env::var("S3_ACCESS_KEY_ID").ok();
    let env_s3_secret_access_key = std::env::var("S3_SECRET_ACCESS_KEY").ok();
    let env_s3_remote_path = std::env::var("S3_REMOTE_PATH").ok();

    if auto_restore_enabled {
        // 备份目标：BACKUP_PROVIDER 显式指定优先，否则按已提供的环境变量推断
        let provider = std::env::var("BACKUP_PROVIDER")
            .ok()
            .map(|v| v.trim().to_ascii_lowercase())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| {
                if env_webdav_url.is_some() {
                    "webdav".to_string()
                } else if env_s3_endpoint.is_some() {
                    "s3".to_string()
                } else {
                    "webdav".to_string()
                }
            });

        let backup_config = if provider == "s3" {
            let missing: Vec<&str> = [
                ("S3_ENDPOINT", env_s3_endpoint.as_deref()),
                ("S3_BUCKET", env_s3_bucket.as_deref()),
                ("S3_ACCESS_KEY_ID", env_s3_access_key_id.as_deref()),
                ("S3_SECRET_ACCESS_KEY", env_s3_secret_access_key.as_deref()),
            ]
            .iter()
            .filter(|(_, value)| value.map(|v| v.trim().is_empty()).unwrap_or(true))
            .map(|(name, _)| *name)
            .collect();

            if !missing.is_empty() {
                warn!(
                    "AUTO_RESTORE_ON_STARTUP with BACKUP_PROVIDER=s3 requires {}, skipping auto restore",
                    missing.join(", ")
                );
                None
            } else {
                Some(models::config::AutoBackupConfig {
                    provider: "s3".to_string(),
                    s3_endpoint: env_s3_endpoint.unwrap(),
                    s3_region: env_s3_region.unwrap_or_else(|| "auto".to_string()),
                    s3_bucket: env_s3_bucket.unwrap(),
                    s3_access_key_id: env_s3_access_key_id.unwrap(),
                    s3_secret_access_key: env_s3_secret_access_key.unwrap(),
                    remote_path: env_s3_remote_path,
                    ..Default::default()
                })
            }
        } else {
            match (env_webdav_url, env_webdav_username, env_webdav_password) {
                (Some(url), Some(username), Some(password)) => {
                    Some(models::config::AutoBackupConfig {
                        provider: "webdav".to_string(),
                        webdav_url: url,
                        webdav_username: username,
                        webdav_password: password,
                        remote_path: env_remote_path,
                        ..Default::default()
                    })
                }
                _ => {
                    warn!(
                        "AUTO_RESTORE_ON_STARTUP with provider webdav requires WEBDAV_URL, WEBDAV_USERNAME, WEBDAV_PASSWORD, skipping auto restore"
                    );
                    None
                }
            }
        };

        if let Some(backup_config) = backup_config {
            info!(
                "Auto restore is enabled via environment variables (provider: {}), restoring latest backup...",
                backup_config.provider
            );
            restore_latest_backup(&backup_config, &data_dir).await?;
            info!("Startup backup restore handling completed");
        }
    }

    // 初始化数据库
    info!("Initializing database...");
    let pool = init_db(&database_url).await?;
    let shared_pool = Arc::new(tokio::sync::RwLock::new(pool));

    // 初始化服务
    let task_service = Arc::new(TaskService::new(shared_pool.clone()));
    let log_service = Arc::new(LogService::new(shared_pool.clone()));
    let login_log_service = Arc::new(LoginLogService::new(shared_pool.clone()));
    let env_service = Arc::new(EnvService::new(shared_pool.clone()));
    let dependence_service = Arc::new(DependenceService::new(shared_pool.clone()));
    let task_group_service = Arc::new(TaskGroupService::new(shared_pool.clone()));
    let subscription_service = Arc::new(SubscriptionService::new(
        shared_pool.clone(),
        scripts_dir.clone(),
        dependence_service.clone(),
    ));
    let config_service = Arc::new(ConfigService::new(shared_pool.clone()));
    let script_service = Arc::new(ScriptService::new(
        scripts_dir.clone(),
        data_dir.join("helpers"),
        env_service.clone(),
        config_service.clone(),
    ));
    let user_service = Arc::new(UserService::new(shared_pool.clone()));
    let mut auth_service = AuthService::new(user_service.clone())?;
    auth_service.set_config_service(config_service.clone());
    let auth_service = Arc::new(auth_service);

    #[cfg(not(target_os = "android"))]
    let terminal_service = Arc::new(TerminalService::new(scripts_dir.clone()));

    // 初始化通知服务
    let token_registry: NotifyTokenRegistry = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    let notification_service = Arc::new(NotificationService::new(
        config_service.clone(),
        token_registry.clone(),
    ));

    let totp_service = Arc::new(TotpService::new(config_service.clone()));
    let executor = Arc::new(Executor::new(
        env_service.clone(),
        config_service.clone(),
        Some(notification_service.clone()),
        token_registry,
        data_dir.join("helpers"),
    ));

    script_service.init().await?;

    // 加载并应用镜像配置
    info!("Loading mirror configuration...");
    if let Err(e) = config_service.load_and_apply_mirror_config().await {
        error!("Failed to load mirror config: {}", e);
    }

    // 启动时安装待安装的依赖（异步）
    info!("Installing pending dependencies...");
    let deps_done_rx = dependence_service.install_on_startup().await?;

    // 初始化调度器
    info!("Initializing scheduler...");
    let scheduler = Arc::new(Scheduler::new(task_service.clone(), log_service.clone(), executor.clone()).await?);
    scheduler.start().await?;

    // 初始化订阅调度器
    info!("Initializing subscription scheduler...");
    let subscription_scheduler = Arc::new(SubscriptionScheduler::new(subscription_service.clone()).await?);
    subscription_scheduler.start().await?;

    // 初始化自动备份调度器
    info!("Initializing backup scheduler...");
    let backup_scheduler = match BackupScheduler::new(config_service.clone()).await {
        Ok(scheduler) => {
            scheduler.start().await?;
            Some(Arc::new(scheduler))
        }
        Err(e) => {
            error!("Failed to initialize backup scheduler: {}", e);
            None
        }
    };

    // 启动日志清理定时任务
    info!("Starting log cleanup task...");
    let log_service_cleanup = log_service.clone();
    let login_log_service_cleanup = login_log_service.clone();
    let config_service_cleanup = config_service.clone();
    let db_pool_cleanup = shared_pool.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(86400)); // 每24小时
        loop {
            interval.tick().await;

            // 获取日志保留天数配置
            let retention_days = match config_service_cleanup.get_by_key("log_retention_days").await {
                Ok(Some(config)) => config.value.parse::<i64>().unwrap_or(30),
                _ => 30, // 默认30天
            };

            // 清理执行日志
            info!("Running log cleanup, retention days: {}", retention_days);
            match log_service_cleanup.delete_old_logs(retention_days).await {
                Ok(count) => info!("Deleted {} old log entries", count),
                Err(e) => error!("Failed to delete old logs: {}", e),
            }

            // 清理登录日志
            info!("Running login log cleanup, retention days: {}", retention_days);
            match login_log_service_cleanup.delete_old_logs(retention_days).await {
                Ok(count) => info!("Deleted {} old login log entries", count),
                Err(e) => error!("Failed to delete old login logs: {}", e),
            }

            // 与日志清理共用定时器，清理长期未活跃且没有运行任务的 AI 会话
            let session_retention_days = match config_service_cleanup.get_by_key("ai_session_retention_days").await {
                Ok(Some(config)) => config.value.parse::<i64>().unwrap_or(7).max(1),
                _ => 7,
            };
            info!("Running AI session cleanup, retention days: {}", session_retention_days);
            let pool = db_pool_cleanup.read().await;
            match sqlx::query(
                "DELETE FROM ai_sessions WHERE active_job_id IS NULL AND updated_at < datetime('now', '-' || ? || ' days')",
            )
            .bind(session_retention_days)
            .execute(&*pool)
            .await {
                Ok(result) => info!("Deleted {} inactive AI sessions", result.rows_affected()),
                Err(e) => error!("Failed to delete inactive AI sessions: {}", e),
            }
        }
    });

    // 创建应用状态
    let state = Arc::new(AppState {
        task_service: task_service.clone(),
        log_service: log_service.clone(),
        script_service,
        dependence_service,
        env_service,
        task_group_service,
        subscription_service,
        config_service,
        auth_service,
        user_service,
        login_log_service,
        #[cfg(not(target_os = "android"))]
        terminal_service,
        totp_service,
        scheduler: scheduler.clone(),
        subscription_scheduler,
        backup_scheduler,
        db_pool: shared_pool,
        system_log_collector,
        notification_service,
    });

    // 创建路由
    let app = api::create_router(state).layer(CorsLayer::permissive());

    // 启动服务器
    let addr = format!("0.0.0.0:{}", port);
    info!("Server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;

    // 在后台等待依赖安装完成后执行开机任务
    let task_service_clone = task_service.clone();
    let log_service_clone = log_service.clone();
    let executor_clone = executor.clone();
    let scheduler_clone = scheduler.clone();
    tokio::spawn(async move {
        // 等待依赖安装完成
        if let Ok(_) = deps_done_rx.await {
            info!("Dependencies installation completed, running startup tasks...");

            match task_service_clone.get_startup_tasks().await {
                Ok(startup_tasks) => {
                    if !startup_tasks.is_empty() {
                        info!("Found {} startup tasks", startup_tasks.len());
                        for task in startup_tasks {
                            info!("Executing startup task: {}", task.name);
                            let start_time = chrono::Utc::now();

                            match executor_clone.execute(&task).await {
                                Ok((_execution_id, output, status)) => {
                                    let duration = (chrono::Utc::now() - start_time).num_milliseconds();
                                    info!("Startup task {} completed with status: {}", task.name, status);

                                    // 更新任务执行信息
                                    if let Err(e) = task_service_clone.update_run_info(task.id, start_time, duration).await {
                                        error!("Failed to update startup task run info: {}", e);
                                    }

                                    // 记录日志
                                    if let Err(e) = log_service_clone.create(task.id, output, status.to_string(), Some(duration), start_time).await {
                                        error!("Failed to save startup task log: {}", e);
                                    }
                                }
                                Err(e) => {
                                    error!("Failed to execute startup task {}: {}", task.name, e);
                                }
                            }
                        }
                    } else {
                        info!("No startup tasks to run");
                    }
                }
                Err(e) => {
                    error!("Failed to get startup tasks: {}", e);
                }
            }

            if let Err(e) = scheduler_clone.run_startup_supplement_tasks().await {
                error!("Failed to run startup supplement tasks: {}", e);
            }
        }
    });

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>()
    ).await?;

    Ok(())
}

async fn restore_latest_backup(
    backup_config: &models::config::AutoBackupConfig,
    data_dir: &PathBuf,
) -> Result<()> {
    // 先把最新的备份文件下载到本地临时文件（按 provider 分派）
    let Some(temp_file) = download_latest_backup(backup_config).await? else {
        return Ok(());
    };

    let restore_result: Result<()> = async {
        let rollback_path = data_dir.parent().unwrap_or(std::path::Path::new(".")).join(format!(
            "zhuque_before_startup_restore_{}.tar.gz",
            uuid::Uuid::new_v4().simple()
        ));
        let has_current_data = data_dir.join("app.db").is_file();

        if has_current_data {
            api::backup::create_backup_file(data_dir.clone(), rollback_path.clone()).await?;
        }

        let staging_dir = match api::backup::prepare_backup_file(&temp_file, data_dir).await {
            Ok(path) => path,
            Err(e) => {
                let _ = tokio::fs::remove_file(&rollback_path).await;
                return Err(e.into());
            }
        };

        match api::backup::activate_prepared_backup(data_dir, &staging_dir).await {
            Ok(()) => {
                let _ = tokio::fs::remove_file(&rollback_path).await;
                Ok(())
            }
            Err(restore_error) if has_current_data => {
                match api::backup::restore_backup_file(&rollback_path, data_dir).await {
                    Ok(()) => {
                        let _ = tokio::fs::remove_file(&rollback_path).await;
                        warn!(
                            "Startup restore failed, current data was rolled back: {}",
                            restore_error
                        );
                        Ok(())
                    }
                    Err(rollback_error) => Err(anyhow::anyhow!(
                        "restore failed: {}; rollback failed: {}; rollback archive: {}",
                        restore_error,
                        rollback_error,
                        rollback_path.display()
                    )),
                }
            }
            Err(e) => Err(e.into()),
        }
    }
    .await;
    let _ = tokio::fs::remove_file(&temp_file).await;
    restore_result?;

    info!("Backup restored successfully");

    Ok(())
}

/// 按 provider（WebDAV / S3 兼容对象存储）列出远端备份并下载最新的一个到本地临时文件。
/// 远端没有任何备份时返回 `Ok(None)`。
async fn download_latest_backup(
    backup_config: &models::config::AutoBackupConfig,
) -> Result<Option<PathBuf>> {
    let list_path = backup_config.remote_path.as_deref().unwrap_or("").to_string();

    if backup_config.is_s3() {
        use services::S3Client;

        let client = S3Client::new(
            backup_config.s3_endpoint.clone(),
            backup_config.s3_region.clone(),
            backup_config.s3_bucket.clone(),
            backup_config.s3_access_key_id.clone(),
            backup_config.s3_secret_access_key.clone(),
        )?;

        let mut files = client.list_files(&list_path).await?;
        files.retain(|f| f.name.starts_with("zhuque_backup_") && f.name.ends_with(".tar.gz"));

        if files.is_empty() {
            info!("No backup files found on S3");
            return Ok(None);
        }

        // 按文件名排序（文件名包含时间戳），取最新的
        files.sort_by(|a, b| b.name.cmp(&a.name));
        let latest_file = files.remove(0);

        info!("Found latest backup: {}", latest_file.name);

        let temp_file = std::env::temp_dir().join(&latest_file.name);
        client.download_file(&latest_file.key, &temp_file).await?;

        info!(
            "Downloaded backup file: {} bytes",
            tokio::fs::metadata(&temp_file).await?.len()
        );

        Ok(Some(temp_file))
    } else {
        use services::WebDavClient;

        let client = WebDavClient::new(
            backup_config.webdav_url.clone(),
            backup_config.webdav_username.clone(),
            backup_config.webdav_password.clone(),
        );

        let mut files = client.list_files(&list_path).await?;
        files.retain(|f| f.name.starts_with("zhuque_backup_") && f.name.ends_with(".tar.gz"));

        if files.is_empty() {
            info!("No backup files found on WebDAV");
            return Ok(None);
        }

        // 按文件名排序（文件名包含时间戳），取最新的
        files.sort_by(|a, b| b.name.cmp(&a.name));
        let latest_file = files.remove(0);

        info!("Found latest backup: {}", latest_file.name);

        let temp_file = std::env::temp_dir().join(&latest_file.name);
        client.download_file(&latest_file.path, &temp_file).await?;

        info!(
            "Downloaded backup file: {} bytes",
            tokio::fs::metadata(&temp_file).await?.len()
        );

        Ok(Some(temp_file))
    }
}
