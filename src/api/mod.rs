pub mod auth;
pub mod backup;
pub mod config;
pub mod dependence;
pub mod env;
pub mod log;
pub mod script;
pub mod subscription;
pub mod system;
pub mod task;
pub mod task_group;
pub mod terminal;

use crate::middleware::{auth_middleware, webhook_auth_middleware};
use crate::scheduler::{Scheduler, SubscriptionScheduler, BackupScheduler};
use crate::services::{AuthService, ConfigService, DependenceService, EnvService, LogService, ScriptService, SubscriptionService, TaskService, TaskGroupService, TerminalService, TotpService};
use axum::{
    http::{StatusCode, Uri},
    middleware,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;
use sqlx::SqlitePool;
use anyhow::Result;
use tower_http::services::ServeDir;

pub struct AppState {
    pub task_service: Arc<TaskService>,
    pub log_service: Arc<LogService>,
    pub script_service: Arc<ScriptService>,
    pub dependence_service: Arc<DependenceService>,
    pub env_service: Arc<EnvService>,
    pub task_group_service: Arc<TaskGroupService>,
    pub subscription_service: Arc<SubscriptionService>,
    pub config_service: Arc<ConfigService>,
    pub auth_service: Arc<AuthService>,
    pub terminal_service: Arc<TerminalService>,
    pub totp_service: Arc<TotpService>,
    pub scheduler: Arc<Scheduler>,
    pub subscription_scheduler: Arc<SubscriptionScheduler>,
    pub backup_scheduler: Option<Arc<BackupScheduler>>,
    pub db_pool: Arc<RwLock<SqlitePool>>,
}

impl AppState {
    pub async fn reinit_database(&self) -> Result<()> {
        use crate::models::db::init_db;

        let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "./data".into());
        let database_url = format!("sqlite://{}/app.db", data_dir);

        // 关闭旧连接池
        {
            let old_pool = self.db_pool.read().await;
            old_pool.close().await;
        }

        // 创建新连接池
        let new_pool = init_db(&database_url).await?;

        // 更新连接池
        {
            let mut pool = self.db_pool.write().await;
            *pool = new_pool;
        }

        Ok(())
    }
}

async fn index() -> impl IntoResponse {
    Json(json!({
        "name": "Zhuque",
        "version": env!("CARGO_PKG_VERSION"),
        "status": "running"
    }))
}

async fn not_found() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({
            "error": "Not Found",
            "message": "The requested resource was not found"
        }))
    )
}

async fn spa_fallback(uri: Uri) -> impl IntoResponse {
    let path = uri.path();

    if path.starts_with("/api/") {
        return (
            StatusCode::NOT_FOUND,
            axum::response::Html("API endpoint not found".to_string())
        ).into_response();
    }

    let static_dir = std::path::PathBuf::from("./web/dist");
    let index_path = static_dir.join("index.html");

    match tokio::fs::read_to_string(index_path).await {
        Ok(content) => axum::response::Html(content).into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to load index.html"
        ).into_response(),
    }
}

pub fn create_router(state: Arc<AppState>) -> Router {
    // Webhook路由（使用webhook token认证）
    let webhook_routes = Router::new()
        .route("/api/webhook/tasks/:id/trigger", post(task::webhook_trigger_task))
        .layer(middleware::from_fn(webhook_auth_middleware));

    // 需要认证的路由
    let protected_routes = Router::new()
        // 任务管理
        .route("/api/tasks", get(task::list_tasks).post(task::create_task))
        .route("/api/tasks/running", get(task::list_running_tasks))
        .route(
            "/api/tasks/:id",
            get(task::get_task)
                .put(task::update_task)
                .delete(task::delete_task),
        )
        .route("/api/tasks/:id/run", post(task::run_task))
        .route("/api/tasks/:id/run-stream", get(task::run_task_stream))
        .route("/api/tasks/:id/kill", delete(task::kill_task))
        // 执行管理
        .route("/api/executions", get(task::list_executions))
        .route(
            "/api/executions/:execution_id/logs",
            get(task::subscribe_execution_logs),
        )
        // 日志管理
        .route("/api/logs", get(log::list_logs))
        .route("/api/logs/cleanup/:days", delete(log::delete_old_logs))
        // 环境变量管理
        .route(
            "/api/env",
            get(env::list_env_vars).post(env::create_env_var),
        )
        .route(
            "/api/env/:id",
            get(env::get_env_var)
                .put(env::update_env_var)
                .delete(env::delete_env_var),
        )
        // 脚本管理
        .route(
            "/api/scripts",
            get(script::list_scripts).post(script::upload_script),
        )
        .route("/api/scripts/archive", post(script::upload_archive))
        .route(
            "/api/scripts/directories/*path",
            post(script::create_directory).delete(script::delete_directory),
        )
        .route("/api/scripts/execute/*path", get(script::execute_script))
        .route("/api/scripts/debug", post(script::execute_content))
        .route("/api/scripts/running", get(script::list_running))
        .route(
            "/api/scripts/kill/:execution_id",
            delete(script::kill_execution),
        )
        .route(
            "/api/scripts/*path",
            get(script::get_script)
                .put(script::update_script)
                .delete(script::delete_script),
        )
        .route("/api/scripts/rename/*path", post(script::rename_script))
        // 依赖管理
        .route(
            "/api/dependences",
            get(dependence::list_dependences).post(dependence::create_dependence),
        )
        .route(
            "/api/dependences/batch",
            post(dependence::create_dependences_batch),
        )
        .route(
            "/api/dependences/:id",
            get(dependence::get_dependence)
                .put(dependence::update_dependence)
                .delete(dependence::delete_dependence),
        )
        .route(
            "/api/dependences/:id/reinstall",
            post(dependence::reinstall_dependence),
        )
        // 任务分组管理
        .route(
            "/api/task-groups",
            get(task_group::list_groups).post(task_group::create_group),
        )
        .route(
            "/api/task-groups/:id",
            get(task_group::get_group)
                .put(task_group::update_group)
                .delete(task_group::delete_group),
        )
        .route(
            "/api/task-groups/:id/tasks",
            get(task_group::get_group_tasks),
        )
        .route(
            "/api/task-groups/:id/stats",
            get(task_group::get_group_stats),
        )
        // 备份管理
        .route("/api/backup", get(backup::create_backup))
        .route("/api/backup/restore", post(backup::restore_backup))
        // 系统配置管理
        .route("/api/configs", get(config::list_configs))
        .route(
            "/api/configs/:key",
            get(config::get_config)
                .post(config::update_config)
                .delete(config::delete_config),
        )
        .route("/api/configs/mirror/config", get(config::get_mirror_config))
        .route("/api/configs/mirror/config", post(config::update_mirror_config))
        // 自动备份配置
        .route("/api/configs/auto-backup/config", get(config::get_auto_backup_config))
        .route("/api/configs/auto-backup/config", post(config::update_auto_backup_config))
        .route("/api/configs/auto-backup/test", post(config::test_webdav_connection))
        // 订阅管理
        .route(
            "/api/subscriptions",
            get(subscription::list_subscriptions).post(subscription::create_subscription),
        )
        .route(
            "/api/subscriptions/:id",
            get(subscription::get_subscription)
                .put(subscription::update_subscription)
                .delete(subscription::delete_subscription),
        )
        .route("/api/subscriptions/:id/run", post(subscription::run_subscription))
        // 系统信息
        .route("/api/system/info", get(system::get_system_info))
        .route("/api/system/webhook-config", get(system::get_webhook_config))
        // 终端
        .route("/api/terminal/connect", get(terminal::connect_terminal))
        // TOTP管理
        .route("/api/auth/totp/status", get(auth::get_totp_status))
        .route("/api/auth/totp/setup", post(auth::setup_totp))
        .route("/api/auth/totp/enable", post(auth::enable_totp))
        .route("/api/auth/totp/disable", post(auth::disable_totp))
        .route("/api/auth/totp/regenerate-backup-codes", post(auth::regenerate_backup_codes))
        .layer(middleware::from_fn_with_state(
            state.auth_service.clone(),
            auth_middleware,
        ));

    // 公开路由
    let app = Router::new()
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/totp/verify", post(auth::verify_totp))
        .merge(webhook_routes)
        .merge(protected_routes)
        .with_state(state);

    // 静态文件服务
    let static_dir = std::path::PathBuf::from("./web/dist");

    if static_dir.exists() {
        app.nest_service("/assets", ServeDir::new(static_dir.join("assets")))
            .fallback(spa_fallback)
    } else {
        app.fallback(not_found)
    }
}
