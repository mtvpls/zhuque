use crate::models::Task;
use crate::services::{EnvService, ConfigService};
use crate::services::dependence::aggressive_memory_reclaim;
use crate::utils::python_detector::PYTHON_CMD;
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, RwLock};
use tracing::{error, info};
use uuid::Uuid;

#[derive(Clone, Serialize)]
pub struct ExecutionInfo {
    pub execution_id: String,
    pub task_id: i64,
    pub task_name: String,
    pub pid: Option<u32>,
    pub started_at: chrono::DateTime<chrono::Utc>,
}

pub struct Executor {
    env_service: Arc<EnvService>,
    config_service: Arc<ConfigService>,
    running_tasks: Arc<RwLock<HashMap<i64, u32>>>, // task_id -> PID
    log_channels: Arc<RwLock<HashMap<String, broadcast::Sender<String>>>>, // execution_id -> log channel
    log_buffers: Arc<RwLock<HashMap<String, Vec<String>>>>, // execution_id -> log buffer
    executions: Arc<RwLock<HashMap<String, ExecutionInfo>>>, // execution_id -> execution info
}

impl Executor {
    pub fn new(env_service: Arc<EnvService>, config_service: Arc<ConfigService>) -> Self {
        Self {
            env_service,
            config_service,
            running_tasks: Arc::new(RwLock::new(HashMap::new())),
            log_channels: Arc::new(RwLock::new(HashMap::new())),
            log_buffers: Arc::new(RwLock::new(HashMap::new())),
            executions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 根据任务获取工作目录
    fn get_working_directory(&self, task: &Task) -> std::path::PathBuf {
        let project_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let scripts_dir = project_root.join("data/scripts");

        // 如果任务设置了自定义工作目录
        if let Some(working_dir) = &task.working_dir {
            let working_dir = working_dir.trim();
            if !working_dir.is_empty() {
                let path = std::path::Path::new(working_dir);
                // 如果是绝对路径，直接使用
                if path.is_absolute() {
                    return path.to_path_buf();
                } else {
                    // 相对路径，以 scripts 目录为基准
                    return scripts_dir.join(path);
                }
            }
        }

        // 没有设置工作目录，使用原有逻辑
        self.get_working_directory_from_command(&task.command)
    }

    /// 根据命令获取工作目录（用于 debug 执行等没有 task 对象的场景）
    fn get_working_directory_from_command(&self, command: &str) -> std::path::PathBuf {
        let project_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let scripts_dir = project_root.join("data/scripts");

        info!("get_working_directory_from_command - command: {}", command);

        // 检查是否是单行命令
        if command.lines().count() != 1 {
            info!("Multi-line command, using scripts_dir");
            return scripts_dir;
        }

        // 解析命令，提取脚本路径
        let parts: Vec<&str> = command.trim().split_whitespace().collect();
        info!("Command parts: {:?}", parts);

        if parts.is_empty() {
            info!("Empty command, using scripts_dir");
            return scripts_dir;
        }

        // 查找脚本文件（从第一个参数开始查找，因为可能直接是脚本路径）
        let script_path = parts.iter().find(|part| {
            part.ends_with(".py") || part.ends_with(".js") || part.ends_with(".sh")
        });

        info!("Found script_path: {:?}", script_path);

        if let Some(script) = script_path {
            let script_path = std::path::Path::new(script);

            // 如果是绝对路径，返回脚本所在目录
            if script_path.is_absolute() {
                if let Some(parent) = script_path.parent() {
                    info!("Absolute path, parent: {:?}", parent);
                    return parent.to_path_buf();
                }
            } else {
                // 相对路径，以 scripts 为基础
                let full_path = scripts_dir.join(script_path);
                info!("Relative path, full_path: {:?}", full_path);
                if let Some(parent) = full_path.parent() {
                    info!("Returning parent: {:?}", parent);
                    return parent.to_path_buf();
                }
            }
        }

        info!("No script found, using scripts_dir");
        scripts_dir
    }

    fn adjust_command_for_working_dir(&self, command: &str, working_dir: &std::path::Path) -> String {
        let project_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let scripts_dir = project_root.join("data/scripts");

        info!("adjust_command_for_working_dir - command: {}, working_dir: {:?}, scripts_dir: {:?}", command, working_dir, scripts_dir);

        // 检查是否是单行命令
        if command.lines().count() != 1 {
            info!("Multi-line command, no adjustment");
            return command.to_string();
        }

        // 解析命令
        let parts: Vec<&str> = command.trim().split_whitespace().collect();
        if parts.is_empty() {
            info!("Empty command, no adjustment");
            return command.to_string();
        }

        // 查找脚本文件并调整路径（从第一个参数开始，因为可能直接是脚本路径）
        let mut adjusted_parts: Vec<String> = parts.iter().map(|s| s.to_string()).collect();
        let mut found_script = false;
        let mut is_python_script = false;
        let mut script_index = 0;

        for (i, part) in parts.iter().enumerate() {
            if part.ends_with(".py") || part.ends_with(".js") || part.ends_with(".sh") {
                let script_path = std::path::Path::new(part);
                info!("Found script at index {}: {}, is_absolute: {}", i, part, script_path.is_absolute());
                found_script = true;
                is_python_script = part.ends_with(".py");
                script_index = i;

                if !script_path.is_absolute() {
                    // 相对路径
                    if working_dir == scripts_dir {
                        // 工作目录是 scripts，不需要调整路径，但如果是第一个参数（直接执行）需要添加 ./
                        if i == 0 {
                            if !part.starts_with("./") {
                                let adjusted = format!("./{}", part);
                                info!("Adding ./ prefix: {} to {}", part, adjusted);
                                adjusted_parts[i] = adjusted;
                            }
                        }
                    } else {
                        // 工作目录不是 scripts，需要提取文件名
                        if let Some(file_name) = script_path.file_name() {
                            if let Some(name_str) = file_name.to_str() {
                                // 如果是第一个参数（没有执行器），添加 ./
                                let adjusted = if i == 0 {
                                    if name_str.starts_with("./") {
                                        name_str.to_string()
                                    } else {
                                        format!("./{}", name_str)
                                    }
                                } else {
                                    name_str.to_string()
                                };
                                info!("Adjusting {} to {}", part, adjusted);
                                adjusted_parts[i] = adjusted;
                            }
                        }
                    }
                }
                break;
            }
        }

        // 如果是Python脚本，确保使用 python -u 执行
        if is_python_script {
            let has_python_cmd = adjusted_parts.iter().any(|p|
                p == "python" || p == "python3" || p.ends_with("/python") || p.ends_with("/python3")
            );

            if !has_python_cmd && script_index == 0 {
                // 脚本是第一个参数（直接执行），转换为 python -u script.py
                let script_path = adjusted_parts[0].clone();
                adjusted_parts.clear();
                adjusted_parts.push(PYTHON_CMD.as_str().to_string());
                adjusted_parts.push("-u".to_string());
                adjusted_parts.push(script_path);
                info!("Converted direct Python script execution to: {} -u", PYTHON_CMD.as_str());
            } else if has_python_cmd {
                // 命令中已有python，添加-u参数
                for (i, part) in adjusted_parts.clone().iter().enumerate() {
                    if part == "python" || part == "python3" || part.ends_with("/python") || part.ends_with("/python3") {
                        if i + 1 < adjusted_parts.len() && adjusted_parts[i + 1] != "-u" {
                            adjusted_parts.insert(i + 1, "-u".to_string());
                            info!("Added -u flag to python command");
                        }
                        break;
                    }
                }
            }
        }

        let result = adjusted_parts.join(" ");
        info!("Adjusted command result: {}", result);
        result
    }

    /// 确保脚本文件有执行权限
    async fn ensure_script_executable(&self, command: &str, working_dir: &std::path::Path) {
        // 解析命令，提取脚本路径
        let parts: Vec<&str> = command.trim().split_whitespace().collect();
        if parts.is_empty() {
            return;
        }

        // 查找脚本文件（从第一个参数开始）
        let script_path = parts.iter().find(|part| {
            part.ends_with(".py") || part.ends_with(".js") || part.ends_with(".sh")
        });

        if let Some(script) = script_path {
            let script_path = std::path::Path::new(script);

            // 构建完整路径
            let full_path = if script_path.is_absolute() {
                script_path.to_path_buf()
            } else {
                // 相对路径，基于工作目录
                working_dir.join(script_path.file_name().unwrap_or(script_path.as_os_str()))
            };

            // 添加执行权限
            if full_path.exists() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(metadata) = tokio::fs::metadata(&full_path).await {
                        let mut perms = metadata.permissions();
                        let mode = perms.mode();
                        perms.set_mode(mode | 0o111); // 添加执行权限
                        let _ = tokio::fs::set_permissions(&full_path, perms).await;
                        info!("Set executable permission for: {:?}", full_path);
                    }
                }
            }
        }
    }

    /// 执行任务并返回 (execution_id, output, success)
    pub async fn execute(&self, task: &Task) -> Result<(String, String, bool)> {
        let execution_id = Uuid::new_v4().to_string();
        info!("Executing task: {} ({}) with execution_id: {}", task.name, task.command, execution_id);

        // 创建广播通道和日志缓存
        let (tx, _) = broadcast::channel(100);
        self.log_channels.write().await.insert(execution_id.clone(), tx.clone());
        self.log_buffers.write().await.insert(execution_id.clone(), Vec::new());

        // 记录执行信息
        let exec_info = ExecutionInfo {
            execution_id: execution_id.clone(),
            task_id: task.id,
            task_name: task.name.clone(),
            pid: None,
            started_at: chrono::Utc::now(),
        };
        self.executions.write().await.insert(execution_id.clone(), exec_info);

        // 解析环境变量
        let env_vars = self.parse_env(&task.env).await;

        // 获取工作目录（提前计算，供前置、主命令、后置命令使用）
        let working_dir = self.get_working_directory(&task);

        // 确保工作目录存在
        if !working_dir.exists() {
            tokio::fs::create_dir_all(&working_dir).await?;
        }

        info!("Working directory: {:?}", working_dir);

        let mut output = String::new();
        let mut overall_success = true;

        // 执行前置命令
        if let Some(pre_cmd) = &task.pre_command {
            if !pre_cmd.trim().is_empty() {
                info!("Executing pre-command: {}", pre_cmd);
                let _ = tx.send(format!("[PRE] Executing: {}", pre_cmd));

                match self.execute_command(pre_cmd, &env_vars, &tx, &working_dir).await {
                    Ok((cmd_output, success)) => {
                        output.push_str(&cmd_output);
                        if !success {
                            overall_success = false;
                            let msg = "[PRE] Pre-command failed, stopping execution".to_string();
                            let _ = tx.send(msg.clone());
                            output.push_str(&msg);
                            output.push('\n');

                            self.log_channels.write().await.remove(&execution_id);
                            self.log_buffers.write().await.remove(&execution_id);
                            self.executions.write().await.remove(&execution_id);
                            return Ok((execution_id, output, false));
                        }
                    }
                    Err(e) => {
                        overall_success = false;
                        let msg = format!("[PRE] Pre-command error: {}", e);
                        let _ = tx.send(msg.clone());
                        output.push_str(&msg);
                        output.push('\n');

                        self.log_channels.write().await.remove(&execution_id);
                        self.executions.write().await.remove(&execution_id);
                        return Ok((execution_id, output, false));
                    }
                }
            }
        }

        // 执行主命令
        info!("Executing main command: {}", task.command);
        let _ = tx.send(format!("[MAIN] Executing: {}", task.command));

        // 给脚本文件添加执行权限
        self.ensure_script_executable(&task.command, &working_dir).await;

        // 调整命令以适应工作目录
        let adjusted_command = self.adjust_command_for_working_dir(&task.command, &working_dir);
        info!("Adjusted command: {}", adjusted_command);

        let mut child = Command::new("sh")
            .arg("-c")
            .arg(&adjusted_command)
            .current_dir(&working_dir)
            .env_clear()
            .envs(env_vars.clone())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        // 注册进程
        let pid = child.id().ok_or_else(|| anyhow!("Failed to get process ID"))?;
        self.running_tasks.write().await.insert(task.id, pid);

        // 更新执行信息中的 PID
        if let Some(info) = self.executions.write().await.get_mut(&execution_id) {
            info.pid = Some(pid);
        }

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        // 读取stdout
        let log_buffers = self.log_buffers.clone();
        let exec_id_clone = execution_id.clone();
        let mut stdout_reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = stdout_reader.next_line().await {
            output.push_str(&line);
            output.push('\n');
            let _ = tx.send(line.clone());
            // 缓存日志
            if let Some(buffer) = log_buffers.write().await.get_mut(&exec_id_clone) {
                buffer.push(line);
                if buffer.len() > 1000 { buffer.remove(0); }
            }
        }

        // 读取stderr
        let mut stderr_reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = stderr_reader.next_line().await {
            let stderr_line = format!("[STDERR] {}", line);
            output.push_str(&stderr_line);
            output.push('\n');
            let _ = tx.send(stderr_line.clone());
            // 缓存日志
            if let Some(buffer) = log_buffers.write().await.get_mut(&exec_id_clone) {
                buffer.push(stderr_line);
                if buffer.len() > 1000 { buffer.remove(0); }
            }
        }

        // 等待进程结束
        let status = child.wait().await?;
        let success = status.success();

        // 发送退出消息
        let exit_msg = if success {
            "[MAIN] Process exited with code 0".to_string()
        } else {
            format!("[MAIN] Process exited with code {}", status.code().unwrap_or(-1))
        };
        let _ = tx.send(exit_msg.clone());
        // 缓存退出消息
        if let Some(buffer) = log_buffers.write().await.get_mut(&exec_id_clone) {
            buffer.push(exit_msg.clone());
        }
        output.push_str(&exit_msg);
        output.push('\n');

        // 清理进程记录
        self.running_tasks.write().await.remove(&task.id);

        if !success {
            overall_success = false;
            // 不要在这里 return，继续执行后置命令
        }

        // 执行后置命令（无论主命令成功与否都执行，用于清理工作）
        if let Some(post_cmd) = &task.post_command {
            if !post_cmd.trim().is_empty() {
                info!("Executing post-command: {}", post_cmd);
                let _ = tx.send(format!("[POST] Executing: {}", post_cmd));

                match self.execute_command(post_cmd, &env_vars, &tx, &working_dir).await {
                    Ok((cmd_output, success)) => {
                        output.push_str(&cmd_output);
                        if !success {
                            overall_success = false;
                        }
                    }
                    Err(e) => {
                        overall_success = false;
                        let msg = format!("[POST] Post-command error: {}", e);
                        let _ = tx.send(msg.clone());
                        output.push_str(&msg);
                        output.push('\n');
                    }
                }
            }
        }

        self.log_channels.write().await.remove(&execution_id);
        self.executions.write().await.remove(&execution_id);

        if overall_success {
            info!("Task {} completed successfully", task.name);
        } else {
            error!("Task {} failed", task.name);
        }

        // 脚本执行完成后，激进回收内存
        aggressive_memory_reclaim();

        Ok((execution_id, output, overall_success))
    }

    /// 流式执行任务，返回 execution_id 和 stream
    pub async fn execute_stream(
        &self,
        task: &Task,
    ) -> Result<(String, impl tokio_stream::Stream<Item = Result<String>>)> {
        let execution_id = Uuid::new_v4().to_string();
        info!("Executing task with stream: {} ({}) with execution_id: {}", task.name, task.command, execution_id);

        // 创建广播通道和日志缓存
        let (tx, _) = broadcast::channel(100);
        self.log_channels.write().await.insert(execution_id.clone(), tx.clone());
        self.log_buffers.write().await.insert(execution_id.clone(), Vec::new());

        // 记录执行信息
        let exec_info = ExecutionInfo {
            execution_id: execution_id.clone(),
            task_id: task.id,
            task_name: task.name.clone(),
            pid: None,
            started_at: chrono::Utc::now(),
        };
        self.executions.write().await.insert(execution_id.clone(), exec_info);

        // 解析环境变量
        let env_vars = self.parse_env(&task.env).await;

        // 获取工作目录
        let working_dir = self.get_working_directory(&task);

        // 确保工作目录存在
        if !working_dir.exists() {
            tokio::fs::create_dir_all(&working_dir).await?;
        }

        // 给脚本文件添加执行权限
        self.ensure_script_executable(&task.command, &working_dir).await;

        // 调整命令以适应工作目录
        let adjusted_command = self.adjust_command_for_working_dir(&task.command, &working_dir);

        // 执行命令
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(&adjusted_command)
            .current_dir(&working_dir)
            .env_clear()
            .envs(env_vars)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        // 注册进程
        let pid = child.id().ok_or_else(|| anyhow!("Failed to get process ID"))?;
        self.running_tasks.write().await.insert(task.id, pid);

        // 更新执行信息中的 PID
        if let Some(info) = self.executions.write().await.get_mut(&execution_id) {
            info.pid = Some(pid);
        }

        let stdout = child.stdout.take().ok_or_else(|| anyhow!("Failed to capture stdout"))?;
        let stderr = child.stderr.take().ok_or_else(|| anyhow!("Failed to capture stderr"))?;

        let task_id = task.id;
        let running_tasks = self.running_tasks.clone();
        let log_channels = self.log_channels.clone();
        let executions = self.executions.clone();
        let exec_id = execution_id.clone();

        let stream = async_stream::stream! {
            let mut stdout_reader = BufReader::new(stdout).lines();
            let mut stderr_reader = BufReader::new(stderr).lines();

            loop {
                tokio::select! {
                    result = stdout_reader.next_line() => {
                        match result {
                            Ok(Some(line)) => {
                                let _ = tx.send(line.clone());
                                yield Ok(line);
                            },
                            Ok(None) => break,
                            Err(e) => {
                                let err_msg = format!("Stdout error: {}", e);
                                let _ = tx.send(err_msg.clone());
                                yield Err(anyhow!(err_msg));
                            },
                        }
                    }
                    result = stderr_reader.next_line() => {
                        match result {
                            Ok(Some(line)) => {
                                let stderr_line = format!("[STDERR] {}", line);
                                let _ = tx.send(stderr_line.clone());
                                yield Ok(stderr_line);
                            },
                            Ok(None) => {},
                            Err(e) => {
                                let err_msg = format!("Stderr error: {}", e);
                                let _ = tx.send(err_msg.clone());
                                yield Err(anyhow!(err_msg));
                            },
                        }
                    }
                }
            }

            // 等待进程结束
            match child.wait().await {
                Ok(status) => {
                    let exit_msg = if status.success() {
                        "[EXIT] Process exited with code 0".to_string()
                    } else {
                        format!("[EXIT] Process exited with code {}", status.code().unwrap_or(-1))
                    };
                    let _ = tx.send(exit_msg.clone());
                    yield Ok(exit_msg);
                }
                Err(e) => {
                    let err_msg = format!("Failed to wait for process: {}", e);
                    let _ = tx.send(err_msg.clone());
                    yield Err(anyhow!(err_msg));
                }
            }

            // 清理进程记录
            running_tasks.write().await.remove(&task_id);
            log_channels.write().await.remove(&exec_id);
            executions.write().await.remove(&exec_id);
        };

        Ok((execution_id, stream))
    }

    /// 中止正在执行的任务
    pub async fn kill_task(&self, task_id: i64) -> Result<()> {
        let mut tasks = self.running_tasks.write().await;

        if let Some(pid) = tasks.remove(&task_id) {
            let output = Command::new("kill")
                .arg("-9")
                .arg(pid.to_string())
                .output()
                .await?;

            if output.status.success() {
                Ok(())
            } else {
                Err(anyhow!("Failed to kill process {}", pid))
            }
        } else {
            Err(anyhow!("Task not running"))
        }
    }

    /// 列出正在执行的任务
    pub async fn list_running(&self) -> Vec<i64> {
        self.running_tasks.read().await.keys().copied().collect()
    }

    /// 订阅执行日志
    pub async fn subscribe_logs(&self, execution_id: &str) -> Result<broadcast::Receiver<String>> {
        let channels = self.log_channels.read().await;
        let tx = channels
            .get(execution_id)
            .ok_or_else(|| anyhow!("Execution not found or already completed"))?;
        Ok(tx.subscribe())
    }

    /// 获取历史日志
    pub async fn get_log_history(&self, execution_id: &str) -> Vec<String> {
        self.log_buffers
            .read()
            .await
            .get(execution_id)
            .cloned()
            .unwrap_or_default()
    }

    /// 发送日志并缓存
    async fn send_and_cache_log(&self, execution_id: &str, tx: &broadcast::Sender<String>, log: String) {
        // 发送到广播频道
        let _ = tx.send(log.clone());

        // 缓存日志（限制最多1000行）
        if let Some(buffer) = self.log_buffers.write().await.get_mut(execution_id) {
            buffer.push(log);
            if buffer.len() > 1000 {
                buffer.remove(0);
            }
        }
    }

    /// 列出所有活跃的执行
    pub async fn list_executions(&self) -> Vec<ExecutionInfo> {
        self.executions.read().await.values().cloned().collect()
    }

    /// 获取执行信息
    pub async fn get_execution(&self, execution_id: &str) -> Option<ExecutionInfo> {
        self.executions.read().await.get(execution_id).cloned()
    }

    async fn parse_env(&self, env_json: &Option<String>) -> HashMap<String, String> {
        let mut env_vars = HashMap::new();

        // 添加基础环境变量
        env_vars.insert("PATH".to_string(), std::env::var("PATH").unwrap_or_default());
        env_vars.insert("HOME".to_string(), std::env::var("HOME").unwrap_or_default());

        // 从数据库读取全局环境变量
        if let Ok(global_vars) = self.env_service.get_all_as_map().await {
            env_vars.extend(global_vars);
        }

        // 解析自定义环境变量（会覆盖全局变量）
        if let Some(json_str) = env_json {
            if let Ok(custom_vars) = serde_json::from_str::<HashMap<String, String>>(json_str) {
                env_vars.extend(custom_vars);
            }
        }

        env_vars
    }

    /// 执行单个命令并返回输出和成功状态
    async fn execute_command(
        &self,
        command: &str,
        env_vars: &HashMap<String, String>,
        tx: &broadcast::Sender<String>,
        working_dir: &std::path::Path,
    ) -> Result<(String, bool)> {
        // 确保工作目录存在
        if !working_dir.exists() {
            tokio::fs::create_dir_all(&working_dir).await?;
        }

        // 给脚本文件添加执行权限
        self.ensure_script_executable(command, &working_dir).await;

        // 调整命令以适应工作目录
        let adjusted_command = self.adjust_command_for_working_dir(command, &working_dir);

        let mut child = Command::new("sh")
            .arg("-c")
            .arg(&adjusted_command)
            .current_dir(&working_dir)
            .env_clear()
            .envs(env_vars.clone())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let mut output = String::new();

        // 读取stdout
        let mut stdout_reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = stdout_reader.next_line().await {
            output.push_str(&line);
            output.push('\n');
            let _ = tx.send(line);
        }

        // 读取stderr
        let mut stderr_reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = stderr_reader.next_line().await {
            let stderr_line = format!("[STDERR] {}", line);
            output.push_str(&stderr_line);
            output.push('\n');
            let _ = tx.send(stderr_line);
        }

        // 等待进程结束
        let status = child.wait().await?;
        let success = status.success();

        let exit_msg = if success {
            "Process exited with code 0".to_string()
        } else {
            format!("Process exited with code {}", status.code().unwrap_or(-1))
        };
        let _ = tx.send(exit_msg.clone());
        output.push_str(&exit_msg);
        output.push('\n');

        Ok((output, success))
    }
}
