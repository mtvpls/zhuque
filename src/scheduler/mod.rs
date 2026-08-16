mod subscription_scheduler;
mod backup_scheduler;

pub use subscription_scheduler::SubscriptionScheduler;
pub use backup_scheduler::BackupScheduler;

use crate::services::{Executor, LogService, TaskService};
use anyhow::Result;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{error, info};

/// 标准化cron表达式：如果是5字段格式，自动补充秒字段
fn normalize_cron_expr(expr: &str) -> String {
    let parts: Vec<&str> = expr.trim().split_whitespace().collect();
    if parts.len() == 5 {
        // 5字段格式（分 时 日 月 周），补充秒字段
        format!("0 {}", expr)
    } else {
        // 已经是6字段或其他格式，保持原样
        expr.to_string()
    }
}

pub struct Scheduler {
    scheduler: JobScheduler,
    task_service: Arc<TaskService>,
    log_service: Arc<LogService>,
    executor: Arc<Executor>,
    job_ids: Arc<RwLock<Vec<(i64, uuid::Uuid)>>>, // (task_id, job_id)
}

impl Scheduler {
    pub async fn new(
        task_service: Arc<TaskService>,
        log_service: Arc<LogService>,
        executor: Arc<Executor>,
    ) -> Result<Self> {
        let scheduler = JobScheduler::new().await?;

        Ok(Self {
            scheduler,
            task_service,
            log_service,
            executor,
            job_ids: Arc::new(RwLock::new(Vec::new())),
        })
    }

    pub async fn start(&self) -> Result<()> {
        info!("Starting scheduler...");
        self.scheduler.start().await?;
        self.reload_tasks().await?;
        info!("Scheduler started");
        Ok(())
    }

    pub async fn reload_tasks(&self) -> Result<()> {
        info!("Reloading tasks...");

        // 清除现有任务
        let mut job_ids = self.job_ids.write().await;
        for (_, job_id) in job_ids.drain(..) {
            let _ = self.scheduler.remove(&job_id).await;
        }

        // 加载启用的任务
        let tasks = self.task_service.get_enabled_tasks().await?;
        info!("Found {} enabled tasks", tasks.len());

        for task in tasks {
            match self.add_task(task).await {
                Ok(task_job_ids) => {
                    job_ids.extend(task_job_ids);
                }
                Err(e) => {
                    error!("Failed to add task: {}", e);
                }
            }
        }

        info!("Tasks reloaded");
        Ok(())
    }

    /// 从调度器移除指定任务的所有 Job
    pub async fn remove_task_from_scheduler(&self, task_id: i64) -> Result<()> {
        let mut job_ids = self.job_ids.write().await;
        let mut remaining = Vec::new();
        for (tid, job_id) in job_ids.drain(..) {
            if tid == task_id {
                let _ = self.scheduler.remove(&job_id).await;
            } else {
                remaining.push((tid, job_id));
            }
        }
        *job_ids = remaining;
        info!("Removed task {} from scheduler", task_id);
        Ok(())
    }

    /// 从 DB 加载指定任务并注册到调度器（若已启用且为 cron 类型）
    pub async fn add_task_to_scheduler(&self, task_id: i64) -> Result<()> {
        let task = self.task_service.get(task_id).await?
            .ok_or_else(|| anyhow::anyhow!("Task not found: {}", task_id))?;

        if task.enabled && task.task_type == "cron" {
            let new_job_ids = self.add_task(task).await?;
            self.job_ids.write().await.extend(new_job_ids);
            info!("Added task {} to scheduler", task_id);
        }
        Ok(())
    }

    /// 更新调度器中的指定任务（先移除旧 Job，再按最新状态重新注册）
    pub async fn update_task_in_scheduler(&self, task_id: i64) -> Result<()> {
        self.remove_task_from_scheduler(task_id).await?;
        self.add_task_to_scheduler(task_id).await?;
        Ok(())
    }

    async fn add_task(&self, task: crate::models::Task) -> Result<Vec<(i64, uuid::Uuid)>> {
        let task_id = task.id;
        let mut job_ids = Vec::new();

        // 为每个 cron 表达式创建一个 Job
        for (idx, cron_expr) in task.cron.iter().enumerate() {
            // 兼容5字段cron表达式：如果只有5个字段，自动在前面补0（秒）
            let normalized_cron = normalize_cron_expr(cron_expr);
            let cron_expr = &normalized_cron;
            let task_service = self.task_service.clone();
            let log_service = self.log_service.clone();
            let executor = self.executor.clone();
            let task_clone = task.clone();
            let _cron_expr_clone = cron_expr.clone();

            let job = Job::new_async_tz(cron_expr.as_str(), chrono::Local, move |_uuid, _l| {
                let task = task_clone.clone();
                let task_service = task_service.clone();
                let log_service = log_service.clone();
                let executor = executor.clone();

                Box::pin(async move {
                    info!("Running scheduled task: {}", task.name);

                    // 防止调度重入：如果上一次执行尚未完成，跳过本次触发
                    if executor.list_running().await.contains(&task.id) {
                        info!(
                            "Task '{}' (id={}) is still running, skipping this scheduled trigger",
                            task.name, task.id
                        );
                        return;
                    }

                    let start_time = chrono::Utc::now();

                    // 执行任务（cron 调度处）
                    let (_execution_id, output, status) = match executor.execute(&task).await {
                        Ok(result) => result,
                        Err(e) => {
                            error!("Task execution error: {}", e);
                            (String::new(), format!("Execution error: {}", e), "failed")
                        }
                    };

                    let duration = (chrono::Utc::now() - start_time).num_milliseconds();

                    // 更新任务执行信息
                    if let Err(e) = task_service.update_run_info(task.id, start_time, duration).await {
                        error!("Failed to update task run info: {}", e);
                    }

                    // 更新下次执行时间（使用所有 cron 表达式中最早的时间）
                    if let Some(next) = task.cron.iter()
                        .map(|c| normalize_cron_expr(c))
                        .filter_map(|c| cron::Schedule::from_str(&c).ok())
                        .filter_map(|s| s.upcoming(chrono::Local).next())
                        .min() {
                        let _ = task_service.update_next_run_at(task.id, next.with_timezone(&chrono::Utc)).await;
                    }

                    // 保存日志
                    if let Err(e) = log_service.create(task.id, output, status.to_string(), Some(duration), start_time).await {
                        error!("Failed to save log: {}", e);
                    }
                })
            })?;

            let job_id = self.scheduler.add(job).await?;
            info!("Added task {} with cron[{}]: {}", task_id, idx, cron_expr);
            job_ids.push((task_id, job_id));
        }

        // 计算并更新下次执行时间（使用所有 cron 表达式中最早的时间）
        if let Some(next) = task.cron.iter()
            .map(|c| normalize_cron_expr(c))
            .filter_map(|c| cron::Schedule::from_str(&c).ok())
            .filter_map(|s| s.upcoming(chrono::Local).next())
            .min() {
            let _ = self.task_service.update_next_run_at(task_id, next.with_timezone(&chrono::Utc)).await;
        }

        Ok(job_ids)
    }

    pub async fn run_task_now(&self, task_id: i64) -> Result<()> {
        let task = self
            .task_service
            .get(task_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Task not found"))?;

        info!("Running task immediately: {}", task.name);

        let task_service = self.task_service.clone();
        let log_service = self.log_service.clone();
        let executor = self.executor.clone();

        tokio::spawn(async move {
            let start_time = chrono::Utc::now();

            let (_execution_id, output, status) = match executor.execute(&task).await {
                Ok(result) => result,
                Err(e) => {
                    error!("Task execution error: {}", e);
                    (String::new(), format!("Execution error: {}", e), "failed")
                }
            };

            let duration = (chrono::Utc::now() - start_time).num_milliseconds();

            // 更新任务执行信息
            if let Err(e) = task_service.update_run_info(task.id, start_time, duration).await {
                error!("Failed to update task run info: {}", e);
            }

            let status_str = status;
            if let Err(e) = log_service.create(task.id, output, status_str.to_string(), Some(duration), start_time).await {
                error!("Failed to save log: {}", e);
            }
        });

        Ok(())
    }

    /// 中止正在执行的任务
    pub async fn kill_task(&self, task_id: i64) -> anyhow::Result<()> {
        self.executor.kill_task_with_log(task_id, self.log_service.clone()).await
    }

    /// 列出正在执行的任务
    pub async fn list_running(&self) -> Vec<i64> {
        self.executor.list_running().await
    }

    /// 订阅运行任务状态变化（包含任务数据）
    pub async fn subscribe_running_tasks_with_data(&self) -> tokio::sync::broadcast::Receiver<crate::services::executor::RunningTasksUpdate> {
        let mut rx = self.executor.subscribe_running_tasks();
        let (tx, rx_out) = tokio::sync::broadcast::channel(100);
        let task_service = self.task_service.clone();

        tokio::spawn(async move {
            while let Ok(mut update) = rx.recv().await {
                // 如果任务结束，查询任务基础数据并合并执行信息
                if update.change_type == "finished" {
                    if let Ok(Some(mut task)) = task_service.get(update.changed_task_id).await {
                        // 使用通知中的执行信息更新任务数据
                        if let Some(last_run_at) = update.last_run_at {
                            task.last_run_at = Some(last_run_at);
                        }
                        if let Some(duration) = update.last_run_duration {
                            task.last_run_duration = Some(duration);
                        }

                        if let Ok(task_json) = serde_json::to_value(&task) {
                            update.task_data = Some(task_json);
                        }
                    }
                }
                let _ = tx.send(update);
            }
        });

        rx_out
    }

    /// 订阅运行任务状态变化
    pub fn subscribe_running_tasks(&self) -> tokio::sync::broadcast::Receiver<crate::services::executor::RunningTasksUpdate> {
        self.executor.subscribe_running_tasks()
    }

    /// 订阅执行日志
    pub async fn subscribe_logs(&self, execution_id: &str) -> anyhow::Result<tokio::sync::broadcast::Receiver<String>> {
        self.executor.subscribe_logs(execution_id).await
    }

    /// 获取历史日志
    pub async fn get_log_history(&self, execution_id: &str) -> Vec<String> {
        self.executor.get_log_history(execution_id).await
    }

    /// 获取执行信息
    pub async fn get_execution(&self, execution_id: &str) -> Option<crate::services::executor::ExecutionInfo> {
        self.executor.get_execution(execution_id).await
    }

    /// 列出所有活跃的执行
    pub async fn list_executions(&self) -> Vec<crate::services::executor::ExecutionInfo> {
        self.executor.list_executions().await
    }
}
