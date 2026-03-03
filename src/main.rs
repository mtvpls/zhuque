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
use services::{AuthService, ConfigService, DependenceService, EnvService, Executor, LogService, ScriptService, SubscriptionService, TaskService, TaskGroupService, TotpService};

#[cfg(not(target_os = "android"))]
use services::TerminalService;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[cfg(feature = "jemalloc")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[cfg(feature = "jemalloc")]
#[export_name = "malloc_conf"]
pub static MALLOC_CONF: &[u8] = b"dirty_decay_ms:10000,muzzy_decay_ms:10000,background_thread:true\0";

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "zhuque=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting Zhuque...");

    // 配置
    let data_dir = PathBuf::from(std::env::var("DATA_DIR").unwrap_or_else(|_| "./data".into()));
    let database_url = format!("sqlite://{}/app.db", data_dir.display());
    let scripts_dir = data_dir.join("scripts");
    let port = std::env::var("PORT")
        .unwrap_or_else(|_| "3000".into())
        .parse::<u16>()?;

    // 初始化数据库
    info!("Initializing database...");
    let pool = init_db(&database_url).await?;
    let shared_pool = Arc::new(tokio::sync::RwLock::new(pool));

    // 初始化服务
    let task_service = Arc::new(TaskService::new(shared_pool.clone()));
    let log_service = Arc::new(LogService::new(shared_pool.clone()));
    let env_service = Arc::new(EnvService::new(shared_pool.clone()));
    let script_service = Arc::new(ScriptService::new(scripts_dir.clone(), env_service.clone()));
    let dependence_service = Arc::new(DependenceService::new(shared_pool.clone()));
    let task_group_service = Arc::new(TaskGroupService::new(shared_pool.clone()));
    let subscription_service = Arc::new(SubscriptionService::new(shared_pool.clone(), scripts_dir.clone()));
    let config_service = Arc::new(ConfigService::new(shared_pool.clone()));
    let mut auth_service = AuthService::new()?;
    auth_service.set_config_service(config_service.clone());
    let auth_service = Arc::new(auth_service);

    #[cfg(not(target_os = "android"))]
    let terminal_service = Arc::new(TerminalService::new(scripts_dir.clone()));

    let totp_service = Arc::new(TotpService::new(config_service.clone()));
    let executor = Arc::new(Executor::new(env_service.clone(), config_service.clone()));

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
    let config_service_cleanup = config_service.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(86400)); // 每24小时
        loop {
            interval.tick().await;

            // 获取日志保留天数配置
            let retention_days = match config_service_cleanup.get_by_key("log_retention_days").await {
                Ok(Some(config)) => config.value.parse::<i64>().unwrap_or(30),
                _ => 30, // 默认30天
            };

            info!("Running log cleanup, retention days: {}", retention_days);
            match log_service_cleanup.delete_old_logs(retention_days).await {
                Ok(count) => info!("Deleted {} old log entries", count),
                Err(e) => error!("Failed to delete old logs: {}", e),
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
        #[cfg(not(target_os = "android"))]
        terminal_service,
        totp_service,
        scheduler,
        subscription_scheduler,
        backup_scheduler,
        db_pool: shared_pool,
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
                            match executor_clone.execute(&task).await {
                                Ok((_execution_id, output, success)) => {
                                    let status = if success { "success" } else { "failed" };
                                    info!("Startup task {} completed with status: {}", task.name, status);

                                    // 记录日志
                                    if let Err(e) = log_service_clone.create(task.id, output, status.to_string()).await {
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
        }
    });

    axum::serve(listener, app).await?;

    Ok(())
}
