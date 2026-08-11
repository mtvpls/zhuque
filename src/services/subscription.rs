use crate::models::{CreateSubscription, DependenceType, Subscription, UpdateSubscription};
use crate::services::DependenceService;
use anyhow::Result;
use chrono::Utc;
use serde_json::Value;
use sqlx::SqlitePool;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::RwLock;

pub struct SubscriptionService {
    db_pool: Arc<RwLock<SqlitePool>>,
    scripts_path: PathBuf,
    dependence_service: Arc<DependenceService>,
}

impl SubscriptionService {
    pub fn new(
        db_pool: Arc<RwLock<SqlitePool>>,
        scripts_path: PathBuf,
        dependence_service: Arc<DependenceService>,
    ) -> Self {
        Self { db_pool, scripts_path, dependence_service }
    }

    fn validate_type(subscription_type: &str) -> Result<()> {
        if matches!(subscription_type, "git" | "single_file") {
            Ok(())
        } else {
            Err(anyhow::anyhow!("不支持的订阅类型: {}", subscription_type))
        }
    }

    fn validate_save_path(path: &str) -> Result<()> {
        let path = Path::new(path.trim());
        if path.as_os_str().is_empty()
            || path.is_absolute()
            || path.components().any(|component| matches!(component, Component::ParentDir))
        {
            return Err(anyhow::anyhow!("保存路径必须是 scripts 目录下的相对路径"));
        }
        Ok(())
    }

    fn default_save_path(url: &str) -> Result<String> {
        let raw_url = url.split(['?', '#']).next().unwrap_or(url).trim_end_matches('/');
        let file_name = raw_url.rsplit('/').next().unwrap_or("").trim();
        if file_name.is_empty() || file_name == "." || file_name == ".." {
            return Err(anyhow::anyhow!("无法从订阅地址中识别文件名，请手动填写保存路径"));
        }
        let file_name = urlencoding::decode(file_name)
            .map_err(|_| anyhow::anyhow!("订阅地址中的文件名编码无效"))?
            .into_owned();
        Self::validate_save_path(&file_name)?;
        Ok(file_name)
    }

    fn git_path(&self, name: &str) -> PathBuf {
        self.scripts_path.join("git").join(name)
    }

    fn validate_subscription_name(name: &str) -> Result<()> {
        let path = Path::new(name.trim());
        if name.trim().is_empty()
            || path.is_absolute()
            || path.components().any(|component| matches!(component, Component::ParentDir))
            || path.components().count() != 1
        {
            return Err(anyhow::anyhow!("订阅名称只能用于 Git 目录下的单层目录名"));
        }
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<Subscription>> {
        let pool = self.db_pool.read().await;
        Ok(sqlx::query_as::<_, Subscription>(
            "SELECT id, name, url, subscription_type, branch, save_path, auto_resolve_dependencies, schedule, enabled, last_run_time, last_run_status, last_run_log, created_at, updated_at FROM subscriptions ORDER BY created_at DESC",
        )
        .fetch_all(&*pool)
        .await?)
    }

    pub async fn get(&self, id: i64) -> Result<Option<Subscription>> {
        let pool = self.db_pool.read().await;
        Ok(sqlx::query_as::<_, Subscription>("SELECT id, name, url, subscription_type, branch, save_path, auto_resolve_dependencies, schedule, enabled, last_run_time, last_run_status, last_run_log, created_at, updated_at FROM subscriptions WHERE id = ?")
            .bind(id)
            .fetch_optional(&*pool)
            .await?)
    }

    pub async fn create(&self, payload: CreateSubscription) -> Result<Subscription> {
        Self::validate_type(&payload.subscription_type)?;
        Self::validate_subscription_name(&payload.name)?;
        let save_path = if payload.subscription_type == "single_file" {
            match payload.save_path.as_deref().map(str::trim).filter(|path| !path.is_empty()) {
                Some(path) => {
                    Self::validate_save_path(path)?;
                    Some(path.to_string())
                }
                None => Some(Self::default_save_path(&payload.url)?),
            }
        } else {
            payload.save_path.clone()
        };

        let branch = payload.branch.unwrap_or_else(|| "main".to_string());
        let enabled = payload.enabled.unwrap_or(true);
        let pool = self.db_pool.read().await;
        let result = sqlx::query(
            "INSERT INTO subscriptions (name, url, subscription_type, branch, save_path, auto_resolve_dependencies, schedule, enabled) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&payload.name)
        .bind(&payload.url)
        .bind(&payload.subscription_type)
        .bind(&branch)
.bind(&save_path)
        .bind(payload.auto_resolve_dependencies)
        .bind(&payload.schedule)
        .bind(enabled)
        .execute(&*pool)
        .await?;
        drop(pool);

        self.get(result.last_insert_rowid())
            .await?
            .ok_or_else(|| anyhow::anyhow!("创建订阅后读取记录失败"))
    }

    pub async fn update(&self, id: i64, payload: UpdateSubscription) -> Result<Option<Subscription>> {
        if let Some(subscription_type) = &payload.subscription_type {
            Self::validate_type(subscription_type)?;
        }
        if let Some(name) = &payload.name {
            Self::validate_subscription_name(name)?;
        }
        let current = self.get(id).await?.ok_or_else(|| anyhow::anyhow!("订阅不存在"))?;
        let subscription_type = payload.subscription_type.as_deref().unwrap_or(&current.subscription_type);
        let requested_save_path = payload
            .save_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty());
        let should_default_save_path = subscription_type == "single_file"
            && requested_save_path.is_none()
            && (payload.subscription_type.as_deref() == Some("single_file")
                || (current.subscription_type == "single_file" && payload.url.is_some()));
        let save_path = if let Some(path) = requested_save_path {
            Self::validate_save_path(path)?;
            Some(path.to_string())
        } else if should_default_save_path {
            Some(Self::default_save_path(
                payload.url.as_deref().unwrap_or(&current.url)
            )?)
        } else {
            None
        };

        let mut updates = Vec::new();
        if payload.name.is_some() { updates.push("name = ?"); }
        if payload.url.is_some() { updates.push("url = ?"); }
        if payload.subscription_type.is_some() { updates.push("subscription_type = ?"); }
        if payload.branch.is_some() { updates.push("branch = ?"); }
        if save_path.is_some() { updates.push("save_path = ?"); }
        if payload.auto_resolve_dependencies.is_some() { updates.push("auto_resolve_dependencies = ?"); }
        if payload.schedule.is_some() { updates.push("schedule = ?"); }
        if payload.enabled.is_some() { updates.push("enabled = ?"); }

        if updates.is_empty() {
            return self.get(id).await;
        }

        updates.push("updated_at = CURRENT_TIMESTAMP");
        let sql = format!("UPDATE subscriptions SET {} WHERE id = ?", updates.join(", "));
        let mut query = sqlx::query(&sql);
        if let Some(value) = payload.name { query = query.bind(value); }
        if let Some(value) = payload.url { query = query.bind(value); }
        if let Some(value) = payload.subscription_type { query = query.bind(value); }
        if let Some(value) = payload.branch { query = query.bind(value); }
        if let Some(value) = save_path { query = query.bind(value); }
        if let Some(value) = payload.auto_resolve_dependencies { query = query.bind(value); }
        if let Some(value) = payload.schedule { query = query.bind(value); }
        if let Some(value) = payload.enabled { query = query.bind(value); }
        let pool = self.db_pool.read().await;
        query.bind(id).execute(&*pool).await?;
        drop(pool);
        self.get(id).await
    }

    pub async fn delete(&self, id: i64) -> Result<bool> {
        let sub = self.get(id).await?;
        if let Some(sub) = &sub {
            if sub.subscription_type == "git" {
                let path = self.git_path(&sub.name);
                if path.exists() {
                    let _ = tokio::fs::remove_dir_all(path).await;
                }
            }
        }
        let pool = self.db_pool.read().await;
        Ok(sqlx::query("DELETE FROM subscriptions WHERE id = ?")
            .bind(id)
            .execute(&*pool)
            .await?
            .rows_affected() > 0)
    }

    pub async fn run(&self, id: i64) -> Result<()> {
        let sub = self.get(id).await?.ok_or_else(|| anyhow::anyhow!("订阅不存在"))?;
        self.update_run_status(id, "running", None).await?;
        let scripts_path = self.scripts_path.clone();
        let db_pool = self.db_pool.clone();
        let dependence_service = self.dependence_service.clone();
        tokio::spawn(async move {
            if let Err(error) = Self::run_operation(id, sub, scripts_path, db_pool, dependence_service).await {
                tracing::error!("Subscription {} run failed: {}", id, error);
            }
        });
        Ok(())
    }

    async fn run_operation(
        id: i64,
        sub: Subscription,
        scripts_path: PathBuf,
        db_pool: Arc<RwLock<SqlitePool>>,
        dependence_service: Arc<DependenceService>,
    ) -> Result<()> {
        let result =         if sub.subscription_type == "single_file" {
            Self::download_file(&sub, &scripts_path).await
        } else {
            Self::sync_git(&sub, &scripts_path).await
        };

        match result {
            Ok((source_path, mut log)) => {
                if sub.subscription_type == "git" && sub.auto_resolve_dependencies {
                    if let Err(error) = Self::resolve_dependencies(&source_path, &dependence_service, &mut log).await {
                        log.push_str(&format!("\n自动解析依赖失败: {}\n", error));
                    }
                }
                Self::update_status(&db_pool, id, "success", Some(&log)).await?;
                Ok(())
            }
            Err((error, log)) => {
                let message = if log.is_empty() { error.clone() } else { log };
                Self::update_status(&db_pool, id, "failed", Some(&message)).await?;
                Err(anyhow::anyhow!(error))
            }
        }
    }

    async fn download_file(sub: &Subscription, scripts_path: &Path) -> std::result::Result<(PathBuf, String), (String, String)> {
        let save_path = match sub.save_path.as_deref().map(str::trim).filter(|path| !path.is_empty()) {
            Some(path) => path.to_string(),
            None => match Self::default_save_path(&sub.url) {
                Ok(path) => path,
                Err(error) => return Err((error.to_string(), String::new())),
            },
        };
        if let Err(error) = Self::validate_save_path(&save_path) {
            return Err((error.to_string(), String::new()));
        }
        let target = scripts_path.join(&save_path);
        if let Some(parent) = target.parent() {
            if let Err(error) = tokio::fs::create_dir_all(parent).await {
                return Err((format!("创建保存目录失败: {}", error), String::new()));
            }
        }
        let response = match reqwest::get(&sub.url).await {
            Ok(response) => response,
            Err(error) => return Err((format!("下载文件失败: {}", error), String::new())),
        };
        if !response.status().is_success() {
            return Err((format!("下载文件失败，HTTP 状态码: {}", response.status()), String::new()));
        }
        let bytes = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(error) => return Err((format!("读取下载内容失败: {}", error), String::new())),
        };
        if let Err(error) = tokio::fs::write(&target, bytes).await {
            return Err((format!("保存文件失败: {}", error), String::new()));
        }
        let target_display = target.display().to_string();
        Ok((target, format!("Downloaded {} to {}\n", sub.url, target_display)))
    }

    async fn sync_git(sub: &Subscription, scripts_path: &Path) -> std::result::Result<(PathBuf, String), (String, String)> {
        let git_path = scripts_path.join("git");
        if let Err(error) = tokio::fs::create_dir_all(&git_path).await {
            return Err((format!("创建 Git 目录失败: {}", error), String::new()));
        }
        let target = git_path.join(&sub.name);
        let mut log = String::new();
        let output = if target.join(".git").exists() {
            log.push_str(&format!("Pulling updates from {}...\n", sub.url));
            Self::git_command(&["-C", target.to_str().unwrap_or_default(), "pull"]).await
        } else {
            log.push_str(&format!("Cloning repository from {}...\n", sub.url));
            Self::git_command(&[
                "clone", "--depth=1", "--branch", &sub.branch, &sub.url, target.to_str().unwrap_or_default(),
            ]).await
        };
        let (success, output_text) = match output {
            Ok(output) => (output.status.success(), format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))),
            Err(error) => return Err((error.to_string(), log)),
        };
        log.push_str(&output_text);
        if success {
            return Ok((target, log));
        }

        if target.join(".git").exists() {
            log.push_str("\nPull failed, resetting and retrying...\n");
            let reset = Self::git_command(&[
                "-C", target.to_str().unwrap_or_default(), "reset", "--hard", &format!("origin/{}", sub.branch),
            ]).await;
            if reset.map(|output| output.status.success()).unwrap_or(false) {
                if let Ok(retry) = Self::git_command(&["-C", target.to_str().unwrap_or_default(), "pull"]).await {
                    log.push_str(&format!("{}{}", String::from_utf8_lossy(&retry.stdout), String::from_utf8_lossy(&retry.stderr)));
                    if retry.status.success() {
                        return Ok((target, log));
                    }
                }
            }
            let _ = tokio::fs::remove_dir_all(&target).await;
            log.push_str("\nRetry failed, cloning fresh...\n");
        }

        let clone = Self::git_command(&[
            "clone", "--depth=1", "--branch", &sub.branch, &sub.url, target.to_str().unwrap_or_default(),
        ]).await;
        match clone {
            Ok(output) => {
                log.push_str(&format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)));
                if output.status.success() {
                    Ok((target, log))
                } else {
                    Err(("Git 拉取失败".to_string(), log))
                }
            }
            Err(error) => Err((error.to_string(), log)),
        }
    }

    async fn git_command(args: &[&str]) -> Result<std::process::Output> {
        Ok(Command::new("git").args(args).output().await?)
    }

    async fn resolve_dependencies(
        source_path: &Path,
        dependence_service: &DependenceService,
        log: &mut String,
    ) -> Result<()> {
        let mut dependencies = Vec::new();
        let package_json = source_path.join("package.json");
        if package_json.is_file() {
            let content = tokio::fs::read_to_string(&package_json).await?;
            let package: Value = serde_json::from_str(&content)?;
            for section in ["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"] {
                if let Some(items) = package.get(section).and_then(Value::as_object) {
                    dependencies.extend(items.keys().cloned().map(|name| (name, DependenceType::NodeJS)));
                }
            }
        }

        let requirements = source_path.join("requirements.txt");
        if requirements.is_file() {
            let content = tokio::fs::read_to_string(&requirements).await?;
            for line in content.lines() {
                let line = line.split(" #").next().unwrap_or(line).trim();
                if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
                    continue;
                }
                dependencies.push((line.to_string(), DependenceType::Python));
            }
        }

        if dependencies.is_empty() {
            return Ok(());
        }
        let added = dependence_service.ensure_dependencies(dependencies).await?;
        if !added.is_empty() {
            log.push_str(&format!("\n自动添加依赖: {}\n", added.join(", ")));
        } else {
            log.push_str("\n依赖解析完成，没有新增依赖。\n");
        }
        Ok(())
    }

    async fn update_status(
        db_pool: &Arc<RwLock<SqlitePool>>,
        id: i64,
        status: &str,
        log: Option<&str>,
    ) -> Result<()> {
        let pool = db_pool.read().await;
        sqlx::query("UPDATE subscriptions SET last_run_time = ?, last_run_status = ?, last_run_log = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(Utc::now())
            .bind(status)
            .bind(log)
            .bind(id)
            .execute(&*pool)
            .await?;
        Ok(())
    }

    async fn update_run_status(&self, id: i64, status: &str, log: Option<&str>) -> Result<()> {
        Self::update_status(&self.db_pool, id, status, log).await
    }

    pub async fn list_enabled(&self) -> Result<Vec<Subscription>> {
        let pool = self.db_pool.read().await;
        Ok(sqlx::query_as::<_, Subscription>("SELECT id, name, url, subscription_type, branch, save_path, auto_resolve_dependencies, schedule, enabled, last_run_time, last_run_status, last_run_log, created_at, updated_at FROM subscriptions WHERE enabled = 1")
            .fetch_all(&*pool)
            .await?)
    }
}
