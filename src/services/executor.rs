use crate::models::Task;
use crate::models::config::TaskNotificationConfig;
use crate::services::{EnvService, ConfigService, NotificationService, NotifyTokenRegistry};
use crate::utils::python_detector::PYTHON_CMD;
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, error, info};
use uuid::Uuid;
#[cfg(unix)]
use libc;

/// 辅助结构体：处理 \r 和 \n 作为行分隔符的读取器
struct LineReader<R> {
    reader: BufReader<R>,
    buffer: Vec<u8>,
}

impl<R: AsyncReadExt + Unpin> LineReader<R> {
    fn new(reader: R) -> Self {
        Self {
            reader: BufReader::new(reader),
            buffer: Vec::new(),
        }
    }

    async fn next_line(&mut self) -> std::io::Result<Option<String>> {
        self.buffer.clear();

        loop {
            let mut byte = [0u8; 1];
            match self.reader.read(&mut byte).await? {
                0 => {
                    // EOF
                    if self.buffer.is_empty() {
                        return Ok(None);
                    } else {
                        let line = String::from_utf8_lossy(&self.buffer).to_string();
                        self.buffer.clear();
                        return Ok(Some(line));
                    }
                }
                _ => {
                    match byte[0] {
                        b'\n' | b'\r' => {
                            // 遇到 \n 或 \r，返回当前行
                            if !self.buffer.is_empty() {
                                let line = String::from_utf8_lossy(&self.buffer).to_string();
                                self.buffer.clear();
                                return Ok(Some(line));
                            }
                            // 如果 buffer 为空，继续读取下一个字符
                        }
                        _ => {
                            self.buffer.push(byte[0]);
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, Serialize)]
pub struct ExecutionInfo {
    pub execution_id: String,
    pub task_id: i64,
    pub task_name: String,
    pub pid: Option<u32>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub killed_externally: Arc<AtomicBool>,
    pub task_notification_json: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunningTasksUpdate {
    pub running_ids: Vec<i64>,
    pub changed_task_id: i64,
    pub change_type: String, // "started" or "finished"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_data: Option<serde_json::Value>, // 任务结束时包含更新后的任务数据
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<chrono::DateTime<chrono::Utc>>, // 任务执行时间
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run_duration: Option<i64>, // 任务执行耗时（毫秒）
}

pub struct Executor {
    env_service: Arc<EnvService>,
    config_service: Arc<ConfigService>,
    running_tasks: Arc<RwLock<HashMap<i64, u32>>>, // task_id -> PID
    log_channels: Arc<RwLock<HashMap<String, broadcast::Sender<String>>>>, // execution_id -> log channel
    log_buffers: Arc<RwLock<HashMap<String, Vec<String>>>>, // execution_id -> log buffer
    executions: Arc<RwLock<HashMap<String, ExecutionInfo>>>, // execution_id -> execution info
    running_tasks_notifier: broadcast::Sender<RunningTasksUpdate>, // 运行任务状态变化通知
    notification_service: Option<Arc<NotificationService>>,
    token_registry: NotifyTokenRegistry,
    helpers_dir: std::path::PathBuf,
}

impl Executor {
    pub fn new(
        env_service: Arc<EnvService>,
        config_service: Arc<ConfigService>,
        notification_service: Option<Arc<NotificationService>>,
        token_registry: NotifyTokenRegistry,
        helpers_dir: std::path::PathBuf,
    ) -> Self {
        let (tx, _) = broadcast::channel(100);

        let helpers_dir_clone = helpers_dir.clone();
        tokio::spawn(async move {
            if let Err(e) = Self::init_notify_helpers(&helpers_dir_clone).await {
                error!("Failed to init notify helpers: {}", e);
            }
        });

        Self {
            env_service,
            config_service,
            running_tasks: Arc::new(RwLock::new(HashMap::new())),
            log_channels: Arc::new(RwLock::new(HashMap::new())),
            log_buffers: Arc::new(RwLock::new(HashMap::new())),
            executions: Arc::new(RwLock::new(HashMap::new())),
            running_tasks_notifier: tx,
            notification_service,
            token_registry,
            helpers_dir,
        }
    }

    fn is_script_path(path: &str) -> bool {
        path.ends_with(".py") || path.ends_with(".js") || path.ends_with(".ts") || path.ends_with(".sh")
    }

    fn is_python_command(command: &str) -> bool {
        command == "python" || command == "python3" || command.ends_with("/python") || command.ends_with("/python3")
    }

    fn is_node_command(command: &str) -> bool {
        command == "node"
            || command.ends_with("/node")
            || command == "bun"
            || command.ends_with("/bun")
            || command == "tsx"
            || command.ends_with("/tsx")
            || command == "ts-node"
            || command.ends_with("/ts-node")
    }

    fn shell_command(command: &str) -> Command {
        #[cfg(windows)]
        {
            let mut cmd = Command::new("cmd");
            cmd.arg("/C").arg(command);
            cmd
        }

        #[cfg(not(windows))]
        {
            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg(command);
            cmd.process_group(0);
            cmd
        }
    }

    fn prepend_path_var(existing: Option<&String>, prefix: &std::path::Path) -> String {
        let mut paths = vec![prefix.to_path_buf()];
        if let Some(existing) = existing {
            paths.extend(std::env::split_paths(existing));
        }
        std::env::join_paths(paths)
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|_| {
                let separator = if cfg!(windows) { ";" } else { ":" };
                match existing {
                    Some(value) if !value.is_empty() => {
                        format!("{}{}{}", prefix.display(), separator, value)
                    }
                    _ => prefix.to_string_lossy().into_owned(),
                }
            })
    }

    fn schedule_token_cleanup(&self, execution_id: &str) {
        let token_registry = self.token_registry.clone();
        let execution_id = execution_id.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            token_registry.write().await.remove(&execution_id);
        });
    }

    async fn kill_process_tree(pid: u32) -> bool {
        #[cfg(windows)]
        {
            Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .status()
                .await
                .map(|status| status.success())
                .unwrap_or(false)
        }

        #[cfg(not(windows))]
        {
            unsafe { libc::kill(-(pid as i32), libc::SIGKILL) == 0 }
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

        debug!("get_working_directory_from_command - command: {}", command);

        // 检查是否是单行命令
        if command.lines().count() != 1 {
            debug!("Multi-line command, using scripts_dir");
            return scripts_dir;
        }

        // 解析命令，提取脚本路径
        let parts: Vec<&str> = command.trim().split_whitespace().collect();
        debug!("Command parts: {:?}", parts);

        if parts.is_empty() {
            debug!("Empty command, using scripts_dir");
            return scripts_dir;
        }

        // 查找脚本文件（从第一个参数开始查找，因为可能直接是脚本路径）
        let script_path = parts.iter().find(|part| Self::is_script_path(part));

        debug!("Found script_path: {:?}", script_path);

        if let Some(script) = script_path {
            let script_path = std::path::Path::new(script);

            // 如果是绝对路径，返回脚本所在目录
            if script_path.is_absolute() {
                if let Some(parent) = script_path.parent() {
                    debug!("Absolute path, parent: {:?}", parent);
                    return parent.to_path_buf();
                }
            } else {
                // 相对路径，以 scripts 为基础
                let full_path = scripts_dir.join(script_path);
                debug!("Relative path, full_path: {:?}", full_path);
                if let Some(parent) = full_path.parent() {
                    debug!("Returning parent: {:?}", parent);
                    return parent.to_path_buf();
                }
            }
        }

        debug!("No script found, using scripts_dir");
        scripts_dir
    }

    fn adjust_command_for_working_dir(&self, command: &str, working_dir: &std::path::Path) -> String {
        let project_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let scripts_dir = project_root.join("data/scripts");

        debug!("adjust_command_for_working_dir - command: {}, working_dir: {:?}, scripts_dir: {:?}", command, working_dir, scripts_dir);

        // 检查是否是单行命令
        if command.lines().count() != 1 {
            debug!("Multi-line command, no adjustment");
            return command.to_string();
        }

        // 解析命令
        let parts: Vec<&str> = command.trim().split_whitespace().collect();
        if parts.is_empty() {
            debug!("Empty command, no adjustment");
            return command.to_string();
        }

        // 查找脚本文件并调整路径（从第一个参数开始，因为可能直接是脚本路径）
        let mut adjusted_parts: Vec<String> = parts.iter().map(|s| s.to_string()).collect();
        let mut is_python_script = false;
        let mut is_node_script = false;
        let mut script_index = 0;

        for (i, part) in parts.iter().enumerate() {
            if Self::is_script_path(part) {
                let script_path = std::path::Path::new(part);
                debug!("Found script at index {}: {}, is_absolute: {}", i, part, script_path.is_absolute());
                is_python_script = part.ends_with(".py");
                is_node_script = part.ends_with(".js") || part.ends_with(".ts");
                script_index = i;

                if !script_path.is_absolute() {
                    // 相对路径
                    if working_dir == scripts_dir {
                        // 工作目录是 scripts，不需要调整路径，但如果是第一个参数（直接执行）需要添加 ./
                        if i == 0 {
                            if !part.starts_with("./") {
                                let adjusted = format!("./{}", part);
                                debug!("Adding ./ prefix: {} to {}", part, adjusted);
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
                                debug!("Adjusting {} to {}", part, adjusted);
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
            let has_python_cmd = adjusted_parts.iter().any(|p| Self::is_python_command(p));

            if !has_python_cmd && script_index == 0 {
                // 脚本是第一个参数（直接执行），转换为 python -u script.py [args...]
                let script_path = adjusted_parts[0].clone();
                let remaining_args: Vec<String> = adjusted_parts.iter().skip(1).cloned().collect();
                adjusted_parts.clear();
                adjusted_parts.push(PYTHON_CMD.as_str().to_string());
                adjusted_parts.push("-u".to_string());
                adjusted_parts.push(script_path);
                adjusted_parts.extend(remaining_args);
                debug!("Converted direct Python script execution to: {} -u", PYTHON_CMD.as_str());
            } else if has_python_cmd {
                // 命令中已有python，添加-u参数
                for (i, part) in adjusted_parts.clone().iter().enumerate() {
                    if Self::is_python_command(part) {
                        if i + 1 < adjusted_parts.len() && adjusted_parts[i + 1] != "-u" {
                            adjusted_parts.insert(i + 1, "-u".to_string());
                            debug!("Added -u flag to python command");
                        }
                        break;
                    }
                }
            }
        }

        if is_node_script {
            let has_node_cmd = adjusted_parts.iter().any(|p| Self::is_node_command(p));

            if !has_node_cmd && script_index == 0 {
                let script_path = adjusted_parts[0].clone();
                let remaining_args: Vec<String> = adjusted_parts.iter().skip(1).cloned().collect();
                let runner = if script_path.ends_with(".ts") { "bun" } else { "node" };
                adjusted_parts.clear();
                adjusted_parts.push(runner.to_string());
                adjusted_parts.push(script_path);
                adjusted_parts.extend(remaining_args);
                debug!("Converted direct Node.js/TypeScript script execution to {}", runner);
            }
        }

        let result = adjusted_parts.join(" ");
        debug!("Adjusted command result: {}", result);
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
        let script_path = parts.iter().find(|part| Self::is_script_path(part));

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
                        debug!("Set executable permission for: {:?}", full_path);
                    }
                }
            }
        }
    }

    /// 初始化脚本通知 helper 文件（启动时调用一次，或在 helper 丢失时懒重建）
    async fn init_notify_helpers(dir: &std::path::Path) -> Result<()> {
        tokio::fs::create_dir_all(dir).await?;

        let notify_bin = r#"#!/usr/bin/env python3
import sys, os, json
from urllib import request as _req

url   = os.environ.get("ZHUQUE_NOTIFY_URL", "")
token = os.environ.get("ZHUQUE_NOTIFY_TOKEN", "")
title   = sys.argv[1] if len(sys.argv) > 1 else ""
content = sys.argv[2] if len(sys.argv) > 2 else ""

if not url or not token:
    sys.exit(0)

payload = json.dumps({"title": title, "content": content}).encode()
req = _req.Request(
    url, data=payload,
    headers={"Authorization": "Bearer " + token,
             "Content-Type": "application/json"}
)
try:
    _req.urlopen(req, timeout=10)
except Exception as e:
    print("[notify] send failed:", e, file=sys.stderr)
"#;
        let bin_path = dir.join("notify");
        tokio::fs::write(&bin_path, notify_bin).await?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = tokio::fs::metadata(&bin_path).await?.permissions();
            perms.set_mode(0o755);
            tokio::fs::set_permissions(&bin_path, perms).await?;
        }

        tokio::fs::write(
            dir.join("notify.py"),
            r#"import subprocess as _sp
import os as _os

def send(title: str, content: str) -> None:
    _sp.run(['notify.cmd' if _os.name == 'nt' else 'notify', title, content])

sendNotify = send
"#,
        )
        .await?;

        tokio::fs::write(
            dir.join("sendNotify.js"),
            r#"'use strict';
const { spawnSync } = require('child_process');

function sendNotify(title, content) {
    const command = process.platform === 'win32' ? 'notify.cmd' : 'notify';
    spawnSync(command, [String(title), String(content)], { stdio: 'inherit' });
}

module.exports = { sendNotify };
module.exports.default = sendNotify;
"#,
        )
        .await?;

        tokio::fs::write(
            dir.join("sendNotify.ts"),
            r#"import { spawnSync } from 'child_process';

export function sendNotify(title: string, content: string): void {
    const command = process.platform === 'win32' ? 'notify.cmd' : 'notify';
    spawnSync(command, [String(title), String(content)], { stdio: 'inherit' });
}

export default sendNotify;
"#,
        )
        .await?;

        tokio::fs::write(
            dir.join("notify.cmd"),
            "@echo off\r\npython \"%~dp0notify\" %*\r\n",
        )
        .await?;

        Ok(())
    }

    /// 将 notify 工具注入到任务进程的环境变量中
    async fn inject_notify_env(
        env_vars: &mut HashMap<String, String>,
        token: &str,
        helpers_dir: &std::path::Path,
    ) {
        // 懒重建：如果 helper 文件丢失（如数据目录被清理），在此重建
        let bin_path = helpers_dir.join("notify");
        if !bin_path.exists() {
            if let Err(e) = Self::init_notify_helpers(helpers_dir).await {
                error!("Failed to re-init notify helpers: {}", e);
            }
        }

        for key in ["PATH", "PYTHONPATH", "NODE_PATH"] {
            let value = Self::prepend_path_var(env_vars.get(key), helpers_dir);
            env_vars.insert(key.to_string(), value);
        }

        env_vars.insert(
            "ZHUQUE_NOTIFY_URL".to_string(),
            format!(
                "http://127.0.0.1:{}/api/notify/send",
                std::env::var("PORT").unwrap_or_else(|_| "3000".to_string())
            ),
        );
        env_vars.insert("ZHUQUE_NOTIFY_TOKEN".to_string(), token.to_string());
    }

    /// 执行任务并返回 (execution_id, output, success)
    pub async fn execute(&self, task: &Task) -> Result<(String, String, &'static str)> {
        // 防止并发重入：如果该任务已在运行中，跳过本次触发
        if self.running_tasks.read().await.contains_key(&task.id) {
            info!(
                "Task '{}' (id={}) is already running, skipping concurrent trigger",
                task.name, task.id
            );
            return Ok((
                String::new(),
                format!("[SKIPPED] Task '{}' is already running, concurrent trigger was skipped.", task.name),
                "success",
            ));
        }

        let execution_id = Uuid::new_v4().to_string();
        let start_time = std::time::Instant::now();
        debug!("Executing task: {} ({}) with execution_id: {}", task.name, task.command, execution_id);

        // 创建广播通道和日志缓存
        let (tx, _) = broadcast::channel(100);
        self.log_channels.write().await.insert(execution_id.clone(), tx.clone());
        self.log_buffers.write().await.insert(execution_id.clone(), Vec::new());

        // 记录执行信息
        let killed_externally = Arc::new(AtomicBool::new(false));
        let exec_info = ExecutionInfo {
            execution_id: execution_id.clone(),
            task_id: task.id,
            task_name: task.name.clone(),
            pid: None,
            started_at: chrono::Utc::now(),
            killed_externally: killed_externally.clone(),
            task_notification_json: task.notification.clone(),
        };
        self.executions.write().await.insert(execution_id.clone(), exec_info);

        // 解析环境变量
        let mut env_vars = self.parse_env(&task.env).await;

        // 获取工作目录（提前计算，供前置、主命令、后置命令使用）
        let working_dir = self.get_working_directory(&task);

        // 确保工作目录存在
        if !working_dir.exists() {
            tokio::fs::create_dir_all(&working_dir).await?;
        }

        // 注册 notify token 并注入环境变量
        self.token_registry.write().await.insert(execution_id.clone(), task.id);
        Self::inject_notify_env(&mut env_vars, &execution_id, &self.helpers_dir).await;

        debug!("Working directory: {:?}", working_dir);

        let mut output = String::new();
        let mut overall_success = true;
        let mut final_status: &'static str = "success";

        // 执行前置命令
        if let Some(pre_cmd) = &task.pre_command {
            if !pre_cmd.trim().is_empty() {
                debug!("Executing pre-command: {}", pre_cmd);
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
                            self.schedule_token_cleanup(&execution_id);
                            return Ok((execution_id, output, "failed"));
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
                        self.schedule_token_cleanup(&execution_id);
                        return Ok((execution_id, output, "failed"));
                    }
                }
            }
        }

        // 执行主命令
        debug!("Executing main command: {}", task.command);
        let _ = tx.send(format!("[MAIN] Executing: {}", task.command));

        // 给脚本文件添加执行权限
        self.ensure_script_executable(&task.command, &working_dir).await;

        // 调整命令以适应工作目录
        let adjusted_command = self.adjust_command_for_working_dir(&task.command, &working_dir);
        debug!("Adjusted command: {}", adjusted_command);

        let mut cmd = Self::shell_command(&adjusted_command);
        cmd.current_dir(&working_dir);
        cmd.env_clear();
        cmd.envs(env_vars.clone())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                self.schedule_token_cleanup(&execution_id);
                return Err(e.into());
            }
        };

        // 注册进程
        let pid = match child.id() {
            Some(pid) => pid,
            None => {
                self.schedule_token_cleanup(&execution_id);
                return Err(anyhow!("Failed to get process ID"));
            }
        };
        self.running_tasks.write().await.insert(task.id, pid);

        // 通知运行状态变化
        let running_list: Vec<i64> = self.running_tasks.read().await.keys().copied().collect();
        let update = RunningTasksUpdate {
            running_ids: running_list,
            changed_task_id: task.id,
            change_type: "started".to_string(),
            task_data: None,
            last_run_at: None,
            last_run_duration: None,
        };
        let _ = self.running_tasks_notifier.send(update);

        // 更新执行信息中的 PID
        if let Some(info) = self.executions.write().await.get_mut(&execution_id) {
            info.pid = Some(pid);
        }

        // 超时看门狗：task.timeout > 0 时，在后台计时，超时后杀掉进程
        // execute 方法 - 超时看门狗（用 libc::kill 替代 shell kill 命令）
        let timed_out = Arc::new(AtomicBool::new(false));
        let timeout_handle = if task.timeout > 0 {
            let timeout_secs = task.timeout as u64;
            let pid_to_kill = pid;
            let timed_out_flag = timed_out.clone();
            Some(tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(timeout_secs)).await;
                timed_out_flag.store(true, Ordering::SeqCst);
                Self::kill_process_tree(pid_to_kill).await;
            }))
        } else {
            None
        };

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        // 并发读取stdout和stderr，保持时序
        let log_buffers = self.log_buffers.clone();
        let exec_id_clone = execution_id.clone();
        let mut stdout_reader = LineReader::new(stdout);
        let mut stderr_reader = LineReader::new(stderr);

        let mut stdout_done = false;
        let mut stderr_done = false;

        while !stdout_done || !stderr_done {
            tokio::select! {
                result = stdout_reader.next_line(), if !stdout_done => {
                    match result {
                        Ok(Some(line)) => {
                            output.push_str(&line);
                            output.push('\n');
                            let _ = tx.send(line.clone());
                            // 缓存日志
                            if let Some(buffer) = log_buffers.write().await.get_mut(&exec_id_clone) {
                                buffer.push(line);
                                if buffer.len() > 1000 { buffer.remove(0); }
                            }
                        }
                        Ok(None) => stdout_done = true,
                        Err(_) => stdout_done = true,
                    }
                }
                result = stderr_reader.next_line(), if !stderr_done => {
                    match result {
                        Ok(Some(line)) => {
                            output.push_str(&line);
                            output.push('\n');
                            let _ = tx.send(line.clone());
                            // 缓存日志
                            if let Some(buffer) = log_buffers.write().await.get_mut(&exec_id_clone) {
                                buffer.push(line);
                                if buffer.len() > 1000 { buffer.remove(0); }
                            }
                        }
                        Ok(None) => stderr_done = true,
                        Err(_) => stderr_done = true,
                    }
                }
            }
        }

        // 等待进程结束
        let status_code = match child.wait().await {
            Ok(status) => status,
            Err(e) => {
                self.schedule_token_cleanup(&execution_id);
                return Err(e.into());
            }
        };
        let success = status_code.success();

        // 检查是否被手动终止
        if killed_externally.load(Ordering::SeqCst) {
            // kill_task_with_log 已发送通知，跳过后置命令和通知，直接返回
            output.push_str(&format!(
                "\n[KILLED] Task '{}' was manually terminated (PID: {})",
                task.name, pid
            ));
            self.schedule_token_cleanup(&execution_id);
            return Ok((execution_id, output, "killed"));
        }

        // 检查是否超时
        if timed_out.load(Ordering::SeqCst) {
            overall_success = false;
            let timeout_msg = format!(
                "[TIMEOUT] Task exceeded {}s timeout limit and was killed",
                task.timeout
            );
            let _ = tx.send(timeout_msg.clone());
            if let Some(buffer) = log_buffers.write().await.get_mut(&exec_id_clone) {
                buffer.push(timeout_msg.clone());
            }
            output.push_str(&timeout_msg);
            output.push('\n');
        } else {
            // 发送退出消息
            let exit_msg = if success {
                "[MAIN] Process exited with code 0".to_string()
            } else {
                format!("[MAIN] Process exited with code {}", status_code.code().unwrap_or(-1))
            };
            let _ = tx.send(exit_msg.clone());
            if let Some(buffer) = log_buffers.write().await.get_mut(&exec_id_clone) {
                buffer.push(exit_msg.clone());
            }
            output.push_str(&exit_msg);
            output.push('\n');
        }

        // 清理进程记录
        let task_id = task.id;
        self.running_tasks.write().await.remove(&task_id);

        if !success {
            overall_success = false;
        }

        // 执行后置命令（无论主命令成功与否都执行，用于清理工作）
        if let Some(post_cmd) = &task.post_command {
            if !post_cmd.trim().is_empty() {
                debug!("Executing post-command: {}", post_cmd);
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

        // 计算总耗时并发送
        let duration = start_time.elapsed().as_millis() as i64;
        let duration_msg = format!("[执行耗时: {}ms ({:.2}s)]", duration, duration as f64 / 1000.0);
        let _ = tx.send(duration_msg.clone());
        // 缓存耗时消息
        if let Some(buffer) = log_buffers.write().await.get_mut(&exec_id_clone) {
            buffer.push(duration_msg.clone());
        }
        output.push_str(&duration_msg);
        output.push('\n');

        // 获取任务开始时间
        let started_at = self.executions.read().await.get(&execution_id).map(|e| e.started_at);

        self.log_channels.write().await.remove(&execution_id);
        self.executions.write().await.remove(&execution_id);

        // 通知运行状态变化（包含执行信息）
        let running_list: Vec<i64> = self.running_tasks.read().await.keys().copied().collect();
        let update = RunningTasksUpdate {
            running_ids: running_list,
            changed_task_id: task_id,
            change_type: "finished".to_string(),
            task_data: None,
            last_run_at: started_at,
            last_run_duration: Some(duration),
        };
        let _ = self.running_tasks_notifier.send(update);

        // 清理 notify token
        self.schedule_token_cleanup(&execution_id);

        // 先确定最终状态，再发通知
        final_status = if overall_success { "success" } else { "failed" };

        // 平台级任务通知（后台发送，不阻塞返回）
        if let Some(notif_svc) = &self.notification_service {
            let status = final_status;
            let output_tail: String = output
                .lines()
                .rev()
                .take(20)
                .collect::<Vec<&str>>()
                .into_iter()
                .rev()
                .collect::<Vec<&str>>()
                .join("\n");
            let notif_svc = notif_svc.clone();
            let task_name = task.name.clone();
            let status = status.to_string();
            let task_notification = task.notification.as_deref()
                .and_then(|s| serde_json::from_str::<TaskNotificationConfig>(s).ok());
            tokio::spawn(async move {
                notif_svc
                    .notify_task_result(&task_name, &status, duration, &output_tail, task_notification)
                    .await;
            });
        }

        if overall_success {
            info!("Task {} completed successfully", task.name);
        } else {
            error!("Task {} failed", task.name);
        }

        Ok((execution_id, output, final_status))
    }
    pub async fn kill_task(&self, task_id: i64) -> Result<()> {
        let mut tasks = self.running_tasks.write().await;

        if let Some(pid) = tasks.remove(&task_id) {
            if Self::kill_process_tree(pid).await {
                Ok(())
            } else {
                Err(anyhow!("Failed to kill process group {}: errno={}", pid, std::io::Error::last_os_error()))
            }
        } else {
            Err(anyhow!("Task not running"))
        }
    }

    /// 中止正在执行的任务并记录日志
    pub async fn kill_task_with_log(&self, task_id: i64, log_service: Arc<crate::services::LogService>) -> Result<()> {
        // 获取执行信息和 PID（在删除之前）
        let exec_info = {
            let executions = self.executions.read().await;
            executions.values()
                .find(|e| e.task_id == task_id)
                .cloned()
        };

        // 提前 clone killed_externally，在删除 exec_info 前设置标志
        let killed_externally_flag = exec_info.as_ref().map(|i| i.killed_externally.clone());

        // 获取 PID 并从 running_tasks 中移除（快速释放锁）
        let pid = {
            let mut tasks = self.running_tasks.write().await;
            tasks.remove(&task_id)
        };

        if let Some(pid) = pid {
            // 先设置标志，再杀进程，防止 execute/execute_stream 后台任务在检查前已退出
            if let Some(flag) = &killed_externally_flag {
                flag.store(true, Ordering::SeqCst);
            }

            let kill_ok = Self::kill_process_tree(pid).await;

            if kill_ok {
                if let Some(info) = exec_info {
                    let duration = (chrono::Utc::now() - info.started_at).num_milliseconds();

                    let existing_output = {
                        let buffers = self.log_buffers.read().await;
                        buffers.get(&info.execution_id)
                            .map(|lines| lines.join("\n"))
                            .unwrap_or_default()
                    };

                    let mut log_output = String::new();
                    if !existing_output.is_empty() {
                        log_output.push_str(&existing_output);
                        log_output.push('\n');
                    }
                    log_output.push_str(&format!("[KILLED] Task '{}' was manually terminated (PID: {})", info.task_name, pid));

                    self.executions.write().await.remove(&info.execution_id);
                    self.log_channels.write().await.remove(&info.execution_id);
                    self.schedule_token_cleanup(&info.execution_id);
                    self.log_buffers.write().await.remove(&info.execution_id);

                    let running_list: Vec<i64> = self.running_tasks.read().await.keys().copied().collect();
                    let update = RunningTasksUpdate {
                        running_ids: running_list,
                        changed_task_id: task_id,
                        change_type: "finished".to_string(),
                        task_data: None,
                        last_run_at: Some(info.started_at),
                        last_run_duration: Some(duration),
                    };
                    let _ = self.running_tasks_notifier.send(update);

                    // 发送 "killed" 推送通知
                    if let Some(notif_svc) = &self.notification_service {
                        let notif_svc = notif_svc.clone();
                        let task_name = info.task_name.clone();
                        let task_notification = info.task_notification_json.as_deref()
                            .and_then(|s| serde_json::from_str::<TaskNotificationConfig>(s).ok());
                        let output_tail = log_output.lines().rev().take(20)
                            .collect::<Vec<_>>().into_iter().rev()
                            .collect::<Vec<_>>().join("\n");
                        tokio::spawn(async move {
                            notif_svc.notify_task_result(&task_name, "killed", duration, &output_tail, task_notification).await;
                        });
                    }
                }
                Ok(())
            } else {
                Err(anyhow!("Failed to kill process group {}: errno={}", pid, std::io::Error::last_os_error()))
            }
        } else {
            Err(anyhow!("Task not running"))
        }
    }

    /// 列出正在执行的任务
    pub async fn list_running(&self) -> Vec<i64> {
        self.running_tasks.read().await.keys().copied().collect()
    }

    /// 订阅运行任务状态变化
    pub fn subscribe_running_tasks(&self) -> broadcast::Receiver<RunningTasksUpdate> {
        self.running_tasks_notifier.subscribe()
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

        // Windows runtime components require system variables in the child environment.
        #[cfg(windows)]
        env_vars.extend(std::env::vars());

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

        #[cfg(windows)]
        for canonical in ["PATH", "PYTHONPATH", "NODE_PATH", "HOME"] {
            if let Some(value) = env_vars
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(canonical))
                .map(|(_, value)| value.clone())
            {
                env_vars.retain(|key, _| !key.eq_ignore_ascii_case(canonical));
                env_vars.insert(canonical.to_string(), value);
            }
        }

        #[cfg(windows)]
        {
            let system_root = env_vars
                .get("SystemRoot")
                .cloned()
                .or_else(|| env_vars.get("WINDIR").cloned())
                .or_else(|| {
                    env_vars
                        .get("ComSpec")
                        .and_then(|value| std::path::Path::new(value).parent()?.parent())
                        .map(|path| path.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "C:\\Windows".to_string());
            env_vars.insert("SystemRoot".to_string(), system_root.clone());
            env_vars.insert("WINDIR".to_string(), system_root);

            let temp = env_vars
                .get("TEMP")
                .cloned()
                .or_else(|| env_vars.get("TMP").cloned())
                .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().into_owned());
            env_vars.insert("TEMP".to_string(), temp.clone());
            env_vars.insert("TMP".to_string(), temp);
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

        let mut command = Self::shell_command(&adjusted_command);
        command.current_dir(&working_dir);
        command.env_clear();
        let mut child = command
            .envs(env_vars.clone())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let mut output = String::new();

        // 读取stdout
        let mut stdout_reader = LineReader::new(stdout);
        while let Ok(Some(line)) = stdout_reader.next_line().await {
            output.push_str(&line);
            output.push('\n');
            let _ = tx.send(line);
        }

        // 读取stderr
        let mut stderr_reader = LineReader::new(stderr);
        while let Ok(Some(line)) = stderr_reader.next_line().await {
            output.push_str(&line);
            output.push('\n');
            let _ = tx.send(line);
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
