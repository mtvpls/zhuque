use crate::models::{CreateDependence, Dependence, DependenceStatus, DependenceType, UpdateDependence};
use crate::utils::python_detector::PIP_CMD;
use anyhow::Result;
use chrono::Utc;
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::{Semaphore, oneshot, RwLock};
use tracing::{error, info};

pub struct DependenceService {
    pool: Arc<RwLock<SqlitePool>>,
    install_semaphore: Arc<Semaphore>, // 限制同时安装的依赖数量
}

impl DependenceService {
    pub fn new(pool: Arc<RwLock<SqlitePool>>) -> Self {
        Self {
            pool,
            install_semaphore: Arc::new(Semaphore::new(1)), // 一次只安装一个依赖
        }
    }

    /// 启动时重置并安装所有依赖，返回一个 receiver 用于等待安装完成
    pub async fn install_on_startup(&self) -> Result<oneshot::Receiver<()>> {
        info!("Resetting and installing all dependencies on startup...");

        // 1. 重置所有依赖为installing状态（除了removed）
        {
            let pool = self.pool.read().await;
            sqlx::query(
                "UPDATE dependences SET status = ?, updated_at = ?
                 WHERE status != ?",
            )
            .bind(DependenceStatus::Installing.to_i32())
            .bind(Utc::now())
            .bind(DependenceStatus::Removed.to_i32())
            .execute(&*pool)
            .await?;
        }

        // 2. 获取所有需要安装的依赖
        let deps = {
            let pool = self.pool.read().await;
            sqlx::query_as::<_, Dependence>(
                "SELECT * FROM dependences WHERE status = ? ORDER BY id ASC",
            )
            .bind(DependenceStatus::Installing.to_i32())
            .fetch_all(&*pool)
            .await?
        };

        let (tx, rx) = oneshot::channel();

        if deps.is_empty() {
            info!("No dependencies to install");
            let _ = tx.send(());
            return Ok(rx);
        }

        info!("Found {} dependencies to install", deps.len());

        // 3. 启动后台任务处理队列
        let pool = self.pool.clone();
        let semaphore = self.install_semaphore.clone();
        tokio::spawn(async move {
            for dep in deps {
                // 获取信号量，确保一次只安装一个
                let _permit = semaphore.acquire().await.unwrap();

                info!("Processing dependency from queue: {}", dep.name);
                if let Err(e) = Self::install_dependency(&pool, dep).await {
                    error!("Failed to install dependency: {}", e);
                }

                // permit自动释放，继续下一个
            }
            info!("All startup dependencies processed");

            let _ = tx.send(()); // 通知安装完成
        });

        Ok(rx)
    }

    /// 获取依赖列表
    pub async fn list(&self, dep_type: Option<DependenceType>) -> Result<Vec<Dependence>> {
        let deps = if let Some(t) = dep_type {
            sqlx::query_as::<_, Dependence>(
                "SELECT * FROM dependences WHERE type = ? ORDER BY created_at DESC",
            )
            .bind(t.to_i32())
            .fetch_all(&*self.pool.read().await)
            .await?
        } else {
            sqlx::query_as::<_, Dependence>("SELECT * FROM dependences ORDER BY created_at DESC")
                .fetch_all(&*self.pool.read().await)
                .await?
        };
        Ok(deps)
    }

    /// 获取单个依赖
    pub async fn get(&self, id: i64) -> Result<Option<Dependence>> {
        let dep = sqlx::query_as::<_, Dependence>("SELECT * FROM dependences WHERE id = ?")
            .bind(id)
            .fetch_optional(&*self.pool.read().await)
            .await?;
        Ok(dep)
    }

    /// 创建依赖
    pub async fn create(&self, create: CreateDependence) -> Result<Dependence> {
        // 检查依赖是否已存在（同名同类型）
        let existing = sqlx::query_as::<_, Dependence>(
            "SELECT * FROM dependences WHERE name = ? AND type = ?",
        )
        .bind(&create.name)
        .bind(create.dep_type.to_i32())
        .fetch_optional(&*self.pool.read().await)
        .await?;

        if existing.is_some() {
            return Err(anyhow::anyhow!(
                "依赖 '{}' (类型: {:?}) 已存在",
                create.name,
                create.dep_type
            ));
        }

        let now = Utc::now();
        let result = sqlx::query(
            "INSERT INTO dependences (name, type, status, remark, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&create.name)
        .bind(create.dep_type.to_i32())
        .bind(DependenceStatus::Installing.to_i32())
        .bind(&create.remark)
        .bind(now)
        .bind(now)
        .execute(&*self.pool.read().await)
        .await?;

        let dep = self.get(result.last_insert_rowid()).await?.unwrap();

        // 加入安装队列（使用信号量控制并发）
        let pool = self.pool.clone();
        let semaphore = self.install_semaphore.clone();
        let dep_clone = dep.clone();
        tokio::spawn(async move {
            let _permit = semaphore.acquire().await.unwrap();
            info!("Installing dependency from queue: {}", dep_clone.name);
            if let Err(e) = Self::install_dependency(&pool, dep_clone).await {
                error!("Failed to install dependency: {}", e);
            }
        });

        Ok(dep)
    }

    /// 批量创建依赖
    pub async fn create_batch(&self, creates: Vec<CreateDependence>) -> Result<Vec<Dependence>> {
        let now = Utc::now();
        let mut deps = Vec::new();

        // 批量插入数据库
        for create in creates {
            // 检查依赖是否已存在（同名同类型）
            let existing = sqlx::query_as::<_, Dependence>(
                "SELECT * FROM dependences WHERE name = ? AND type = ?",
            )
            .bind(&create.name)
            .bind(create.dep_type.to_i32())
            .fetch_optional(&*self.pool.read().await)
            .await?;

            if existing.is_some() {
                return Err(anyhow::anyhow!(
                    "依赖 '{}' (类型: {:?}) 已存在",
                    create.name,
                    create.dep_type
                ));
            }

            let result = sqlx::query(
                "INSERT INTO dependences (name, type, status, remark, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&create.name)
            .bind(create.dep_type.to_i32())
            .bind(DependenceStatus::Installing.to_i32())
            .bind(&create.remark)
            .bind(now)
            .bind(now)
            .execute(&*self.pool.read().await)
            .await?;

            let dep = self.get(result.last_insert_rowid()).await?.unwrap();
            deps.push(dep);
        }

        // 批量加入安装队列（使用信号量控制并发）
        let pool = self.pool.clone();
        let semaphore = self.install_semaphore.clone();
        for dep in deps.clone() {
            let pool_clone = pool.clone();
            let semaphore_clone = semaphore.clone();
            tokio::spawn(async move {
                let _permit = semaphore_clone.acquire().await.unwrap();
                info!("Installing dependency from batch queue: {}", dep.name);
                if let Err(e) = Self::install_dependency(&pool_clone, dep).await {
                    error!("Failed to install dependency: {}", e);
                }
            });
        }

        Ok(deps)
    }

    /// 更新依赖
    pub async fn update(&self, id: i64, update: UpdateDependence) -> Result<Option<Dependence>> {
        let mut query = String::from("UPDATE dependences SET updated_at = ?");
        let mut params: Vec<String> = vec![Utc::now().to_rfc3339()];

        if let Some(name) = &update.name {
            query.push_str(", name = ?");
            params.push(name.clone());
        }
        if let Some(dep_type) = &update.dep_type {
            query.push_str(", type = ?");
            params.push(dep_type.to_i32().to_string());
        }
        if let Some(remark) = &update.remark {
            query.push_str(", remark = ?");
            params.push(remark.clone());
        }

        query.push_str(" WHERE id = ?");
        params.push(id.to_string());

        let mut q = sqlx::query(&query).bind(Utc::now());

        if let Some(name) = &update.name {
            q = q.bind(name);
        }
        if let Some(dep_type) = &update.dep_type {
            q = q.bind(dep_type.to_i32());
        }
        if let Some(remark) = &update.remark {
            q = q.bind(remark);
        }

        q = q.bind(id);
        q.execute(&*self.pool.read().await).await?;

        self.get(id).await
    }

    /// 删除依赖
    pub async fn delete(&self, id: i64) -> Result<bool> {
        // 获取依赖信息
        let dep = match self.get(id).await? {
            Some(d) => d,
            None => return Ok(false),
        };

        // 同步卸载系统依赖（使用信号量排队）
        let _permit = self.install_semaphore.acquire().await.unwrap();

        info!("Uninstalling dependency before deletion: {}", dep.name);
        if let Err(e) = Self::uninstall_dependency(&self.pool, dep.clone()).await {
            error!("Failed to uninstall dependency: {}", e);
            // 即使卸载失败，也继续删除数据库记录
        }

        // 删除数据库记录
        let result = sqlx::query("DELETE FROM dependences WHERE id = ?")
            .bind(id)
            .execute(&*self.pool.read().await)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// 重新安装依赖
    pub async fn reinstall(&self, id: i64) -> Result<()> {
        if let Some(dep) = self.get(id).await? {
            // 更新状态为installing
            sqlx::query("UPDATE dependences SET status = ?, updated_at = ? WHERE id = ?")
                .bind(DependenceStatus::Installing.to_i32())
                .bind(Utc::now())
                .bind(id)
                .execute(&*self.pool.read().await)
                .await?;

            let pool = self.pool.clone();
            let semaphore = self.install_semaphore.clone();
            tokio::spawn(async move {
                let _permit = semaphore.acquire().await.unwrap();
                info!("Reinstalling dependency: {}", dep.name);
                if let Err(e) = Self::install_dependency(&pool, dep).await {
                    error!("Failed to reinstall dependency: {}", e);
                }
            });
        }
        Ok(())
    }

    /// 安装依赖（内部方法）
    async fn install_dependency(pool: &Arc<RwLock<SqlitePool>>, dep: Dependence) -> Result<()> {
        info!("Installing dependency: {} ({})", dep.name, dep.id);

        let dep_type = DependenceType::from_i32(dep.dep_type).unwrap();
        let result = match dep_type {
            DependenceType::NodeJS => Self::install_nodejs(&dep.name).await,
            DependenceType::Python => Self::install_python(&dep.name).await,
            DependenceType::Linux => Self::install_linux(&dep.name).await,
        };

        // 执行完系统命令后再获取连接更新数据库
        match result {
            Ok(log_lines) => {
                info!("Dependency {} installed successfully", dep.name);
                let log_json = Dependence::set_log_lines(log_lines);
                {
                    let pool_guard = pool.read().await;
                    sqlx::query(
                        "UPDATE dependences SET status = ?, log = ?, updated_at = ? WHERE id = ?",
                    )
                    .bind(DependenceStatus::Installed.to_i32())
                    .bind(log_json)
                    .bind(Utc::now())
                    .bind(dep.id)
                    .execute(&*pool_guard)
                    .await?;
                }
            }
            Err(e) => {
                error!("Failed to install dependency {}: {}", dep.name, e);
                let error_lines = vec![format!("Error: {}", e)];
                let log_json = Dependence::set_log_lines(error_lines);
                {
                    let pool_guard = pool.read().await;
                    sqlx::query(
                        "UPDATE dependences SET status = ?, log = ?, updated_at = ? WHERE id = ?",
                    )
                    .bind(DependenceStatus::Failed.to_i32())
                    .bind(log_json)
                    .bind(Utc::now())
                    .bind(dep.id)
                    .execute(&*pool_guard)
                    .await?;
                }
            }
        }

        Ok(())
    }

    /// 卸载依赖（内部方法）
    async fn uninstall_dependency(pool: &Arc<RwLock<SqlitePool>>, dep: Dependence) -> Result<()> {
        info!("Uninstalling dependency: {} ({})", dep.name, dep.id);

        // 更新状态为removing
        sqlx::query("UPDATE dependences SET status = ?, updated_at = ? WHERE id = ?")
            .bind(DependenceStatus::Removing.to_i32())
            .bind(Utc::now())
            .bind(dep.id)
            .execute(&*pool.read().await)
            .await?;

        let dep_type = DependenceType::from_i32(dep.dep_type).unwrap();
        let result = match dep_type {
            DependenceType::NodeJS => Self::uninstall_nodejs(&dep.name).await,
            DependenceType::Python => Self::uninstall_python(&dep.name).await,
            DependenceType::Linux => Self::uninstall_linux(&dep.name).await,
        };

        match result {
            Ok(_) => {
                info!("Dependency {} uninstalled successfully", dep.name);
                sqlx::query("UPDATE dependences SET status = ?, updated_at = ? WHERE id = ?")
                    .bind(DependenceStatus::Removed.to_i32())
                    .bind(Utc::now())
                    .bind(dep.id)
                    .execute(&*pool.read().await)
                    .await?;
            }
            Err(e) => {
                error!("Failed to uninstall dependency {}: {}", dep.name, e);
            }
        }

        Ok(())
    }

    /// 安装Node.js依赖
    async fn install_nodejs(name: &str) -> Result<Vec<String>> {
        let output = Command::new("npm")
            .args(["install", "-g", name])
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let mut lines = Vec::new();
        for line in stdout.lines() {
            lines.push(line.to_string());
        }
        for line in stderr.lines() {
            lines.push(line.to_string());
        }

        if !output.status.success() {
            return Err(anyhow::anyhow!("npm install failed"));
        }

        Ok(lines)
    }

    /// 安装Python依赖
    async fn install_python(name: &str) -> Result<Vec<String>> {
        let output = Command::new(PIP_CMD.as_str())
            .args(["install", "--break-system-packages", name])
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let mut lines = Vec::new();
        for line in stdout.lines() {
            lines.push(line.to_string());
        }
        for line in stderr.lines() {
            lines.push(line.to_string());
        }

        if !output.status.success() {
            return Err(anyhow::anyhow!("pip install failed"));
        }

        Ok(lines)
    }

    /// 安装Linux依赖
    async fn install_linux(name: &str) -> Result<Vec<String>> {
        // 尝试使用apt-get或pkg（Termux）
        let output = if cfg!(target_os = "android") {
            Command::new("pkg")
                .args(["install", "-y", name])
                .output()
                .await?
        } else {
            Command::new("apt-get")
                .args(["install", "-y", name])
                .output()
                .await?
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let mut lines = Vec::new();
        for line in stdout.lines() {
            lines.push(line.to_string());
        }
        for line in stderr.lines() {
            lines.push(line.to_string());
        }

        if !output.status.success() {
            return Err(anyhow::anyhow!("package install failed"));
        }

        Ok(lines)
    }

    /// 卸载Node.js依赖
    async fn uninstall_nodejs(name: &str) -> Result<String> {
        let output = Command::new("npm")
            .args(["uninstall", "-g", name])
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        Ok(format!("{}\n{}", stdout, stderr))
    }

    /// 卸载Python依赖
    async fn uninstall_python(name: &str) -> Result<String> {
        let output = Command::new(PIP_CMD.as_str())
            .args(["uninstall", "-y", name])
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        Ok(format!("{}\n{}", stdout, stderr))
    }

    /// 卸载Linux依赖
    async fn uninstall_linux(name: &str) -> Result<String> {
        let output = if cfg!(target_os = "android") {
            Command::new("pkg")
                .args(["uninstall", "-y", name])
                .output()
                .await?
        } else {
            Command::new("apt-get")
                .args(["remove", "-y", name])
                .output()
                .await?
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        Ok(format!("{}\n{}", stdout, stderr))
    }
}
