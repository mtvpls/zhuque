use crate::models::ScriptFile;
use crate::models::config::NotificationConfig;
use crate::services::{ConfigService, EnvService};
use crate::utils::python_detector::PYTHON_CMD;
use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use flate2::read::GzDecoder;
use std::collections::HashMap;
use std::fs::File;
use std::io::Cursor;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tar::Archive;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::RwLock;
use zip::ZipArchive;

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

pub struct ScriptService {
    base_path: PathBuf,
    helpers_dir: PathBuf,
    running_processes: Arc<RwLock<HashMap<String, u32>>>, // execution_id -> PID
    env_service: Arc<EnvService>,
    config_service: Arc<ConfigService>,
}

impl ScriptService {
    pub fn new(
        base_path: PathBuf,
        helpers_dir: PathBuf,
        env_service: Arc<EnvService>,
        config_service: Arc<ConfigService>,
    ) -> Self {
        Self {
            base_path,
            helpers_dir,
            running_processes: Arc::new(RwLock::new(HashMap::new())),
            env_service,
            config_service,
        }
    }

    fn normalize_script_type(script_type: &str) -> &str {
        match script_type {
            "javascript" | "node" => "js",
            "typescript" => "ts",
            other => other,
        }
    }

    fn prepend_path_var(existing: Option<&String>, prefix: &Path) -> String {
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

    /// 获取基础环境变量
    fn get_base_env(&self) -> HashMap<String, String> {
        let mut env_vars = HashMap::new();
        #[cfg(windows)]
        env_vars.extend(std::env::vars());
        env_vars.insert(
            "PATH".to_string(),
            Self::prepend_path_var(std::env::var("PATH").ok().as_ref(), &self.helpers_dir),
        );
        env_vars.insert(
            "PYTHONPATH".to_string(),
            Self::prepend_path_var(std::env::var("PYTHONPATH").ok().as_ref(), &self.helpers_dir),
        );
        env_vars.insert(
            "NODE_PATH".to_string(),
            Self::prepend_path_var(std::env::var("NODE_PATH").ok().as_ref(), &self.helpers_dir),
        );
        env_vars.insert("HOME".to_string(), std::env::var("HOME").unwrap_or_default());
        env_vars.insert("USER".to_string(), std::env::var("USER").unwrap_or_default());
        env_vars.insert("SHELL".to_string(), std::env::var("SHELL").unwrap_or_default());
        env_vars.insert("LANG".to_string(), std::env::var("LANG").unwrap_or_else(|_| "en_US.UTF-8".to_string()));
        env_vars
    }

    /// 解析环境变量JSON
    async fn parse_env(&self, env_json: Option<&str>) -> HashMap<String, String> {
        let mut env_vars = self.get_base_env();

        // 从数据库读取全局环境变量
        if let Ok(global_vars) = self.env_service.get_all_as_map().await {
            env_vars.extend(global_vars);
        }

        // 自定义环境变量会覆盖全局变量
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
                        .and_then(|value| Path::new(value).parent()?.parent())
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

    pub async fn manual_notification_enabled(&self) -> bool {
        let config = match self.config_service.get_by_key("notification_config").await {
            Ok(Some(value)) => serde_json::from_str::<NotificationConfig>(&value.value).ok(),
            _ => None,
        };
        config.map(|value| value.enabled).unwrap_or(false)
    }

    pub async fn manual_result_notification_allowed(&self, status: &str) -> bool {
        let config = match self.config_service.get_by_key("notification_config").await {
            Ok(Some(value)) => serde_json::from_str::<NotificationConfig>(&value.value).ok(),
            _ => None,
        };
        config.map(|value| {
            value.enabled
                && match status {
                    "success" => value.on_success,
                    "failed" => value.on_failure,
                    "killed" => value.on_killed,
                    _ => false,
                }
        }).unwrap_or(false)
    }

    pub async fn init(&self) -> Result<()> {
        tokio::fs::create_dir_all(&self.base_path).await?;
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<ScriptFile>> {
        // 保留原有方法用于兼容
        self.list_dir("").await
    }

    /// 列出指定目录下的直接子项（文件和文件夹）
    pub async fn list_dir(&self, dir_path: &str) -> Result<Vec<ScriptFile>> {
        self.validate_path(dir_path)?;
        let target_path = if dir_path.is_empty() {
            self.base_path.clone()
        } else {
            self.base_path.join(dir_path)
        };

        if !target_path.exists() {
            return Ok(Vec::new());
        }

        let mut files = Vec::new();
        let mut entries = tokio::fs::read_dir(&target_path).await?;

        while let Some(entry) = entries.next_entry().await? {
            let metadata = entry.metadata().await?;
            let file_name = entry.file_name().to_string_lossy().to_string();

            let relative_path = if dir_path.is_empty() {
                file_name.clone()
            } else {
                format!("{}/{}", dir_path, file_name)
            };

            let modified = metadata
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs();
            let modified_dt = DateTime::from_timestamp(modified as i64, 0)
                .unwrap_or_else(|| Utc::now());

            files.push(ScriptFile {
                name: file_name,
                path: relative_path,
                size: if metadata.is_file() { metadata.len() } else { 0 },
                modified: modified_dt,
                is_directory: metadata.is_dir(),
            });
        }

        // 排序：文件夹在前，然后按名称排序
        files.sort_by(|a, b| {
            match (a.is_directory, b.is_directory) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            }
        });

        Ok(files)
    }

    pub async fn read(&self, path: &str) -> Result<String> {
        self.validate_path(path)?;
        let full_path = self.base_path.join(path);
        let content = tokio::fs::read_to_string(full_path).await?;
        Ok(content)
    }

    /// Read a file without assuming it is UTF-8 text.
    pub async fn read_bytes(&self, path: &str) -> Result<Vec<u8>> {
        self.validate_path(path)?;
        let full_path = self.base_path.join(path);
        Ok(tokio::fs::read(full_path).await?)
    }

    pub async fn write(&self, path: &str, content: &str) -> Result<()> {
        self.write_bytes(path, content.as_bytes()).await
    }

    pub async fn write_bytes(&self, path: &str, content: &[u8]) -> Result<()> {
        self.validate_path(path)?;
        let full_path = self.base_path.join(path);

        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::write(&full_path, content).await?;

        // 如果是.sh脚本，添加执行权限
        #[cfg(unix)]
        if path.ends_with(".sh") {
            let metadata = tokio::fs::metadata(&full_path).await?;
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o755); // rwxr-xr-x
            tokio::fs::set_permissions(&full_path, permissions).await?;
        }

        Ok(())
    }

    pub async fn create_directory(&self, path: &str) -> Result<()> {
        self.validate_path(path)?;
        let full_path = self.base_path.join(path);
        tokio::fs::create_dir_all(full_path).await?;
        Ok(())
    }

    pub async fn delete_directory(&self, path: &str) -> Result<()> {
        self.validate_path(path)?;
        let full_path = self.base_path.join(path);

        // 检查是否为目录
        let metadata = tokio::fs::metadata(&full_path).await?;
        if !metadata.is_dir() {
            return Err(anyhow!("Not a directory"));
        }

        tokio::fs::remove_dir_all(full_path).await?;
        Ok(())
    }

    pub async fn rename(&self, old_path: &str, new_path: &str) -> Result<()> {
        self.validate_path(old_path)?;
        self.validate_path(new_path)?;

        let old_full_path = self.base_path.join(old_path);
        let new_full_path = self.base_path.join(new_path);

        // 确保新路径的父目录存在
        if let Some(parent) = new_full_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::rename(old_full_path, new_full_path).await?;
        Ok(())
    }

    pub async fn copy(&self, source_path: &str, target_path: &str) -> Result<()> {
        self.validate_path(source_path)?;
        self.validate_path(target_path)?;

        let source_full = self.base_path.join(source_path);
        let target_full = self.base_path.join(target_path);

        // 检查源路径是否存在
        if !source_full.exists() {
            return Err(anyhow!("Source path does not exist"));
        }

        // 检查目标路径是否已存在
        if target_full.exists() {
            return Err(anyhow!("Target path already exists"));
        }

        // 确保目标路径的父目录存在
        if let Some(parent) = target_full.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let metadata = tokio::fs::metadata(&source_full).await?;

        if metadata.is_dir() {
            // 目录复制：使用 spawn_blocking 避免阻塞
            let source_clone = source_full.clone();
            let target_clone = target_full.clone();

            tokio::task::spawn_blocking(move || {
                Self::copy_dir_recursive(&source_clone, &target_clone)
            })
            .await??;
        } else {
            // 文件复制：直接使用 tokio::fs::copy
            tokio::fs::copy(&source_full, &target_full).await?;

            // 保留执行权限
            #[cfg(unix)]
            {
                let source_permissions = metadata.permissions();
                tokio::fs::set_permissions(&target_full, source_permissions).await?;
            }
        }

        Ok(())
    }

    /// 递归复制目录（同步函数，在 spawn_blocking 中调用）
    fn copy_dir_recursive(source: &std::path::Path, target: &std::path::Path) -> Result<()> {
        use std::fs;

        // 创建目标目录
        fs::create_dir_all(target)?;

        // 复制目录权限
        #[cfg(unix)]
        {
            let metadata = fs::metadata(source)?;
            fs::set_permissions(target, metadata.permissions())?;
        }

        // 遍历源目录
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let source_path = entry.path();
            let file_name = entry.file_name();
            let target_path = target.join(&file_name);

            if file_type.is_dir() {
                // 递归复制子目录
                Self::copy_dir_recursive(&source_path, &target_path)?;
            } else {
                // 复制文件
                fs::copy(&source_path, &target_path)?;

                // 保留文件权限
                #[cfg(unix)]
                {
                    let metadata = fs::metadata(&source_path)?;
                    fs::set_permissions(&target_path, metadata.permissions())?;
                }
            }
        }

        Ok(())
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        self.validate_path(path)?;
        let full_path = self.base_path.join(path);
        tokio::fs::remove_file(full_path).await?;
        Ok(())
    }

    fn validate_path(&self, path: &str) -> Result<()> {
        let candidate = Path::new(path);
        if candidate.is_absolute()
            || candidate.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(anyhow!("Invalid path"));
        }
        Ok(())
    }

    pub fn get_full_path(&self, path: &str) -> PathBuf {
        self.base_path.join(path)
    }

    /// 执行脚本并流式返回输出
    pub async fn execute_script(
        &self,
        path: &str,
        env_json: Option<&str>,
    ) -> Result<(String, impl tokio_stream::Stream<Item = Result<String>>)> {
        self.validate_path(path)?;
        let full_path = self.base_path.join(path);

        // 检查文件是否存在
        if !tokio::fs::metadata(&full_path).await?.is_file() {
            return Err(anyhow!("File not found"));
        }

        // 转换为绝对路径
        let absolute_path = std::fs::canonicalize(&full_path)?;

        // 生成执行ID
        let execution_id = uuid::Uuid::new_v4().to_string();

        // 获取脚本所在目录作为工作目录
        let working_dir = absolute_path.parent().ok_or_else(|| anyhow!("Invalid path"))?;

        // 根据文件扩展名选择执行方式
        let mut cmd = if path.ends_with(".sh") {
            let mut c = Command::new("bash");
            c.arg(&absolute_path);
            c
        } else if path.ends_with(".py") {
            let mut c = Command::new(PYTHON_CMD.as_str());
            c.arg("-u");  // 禁用输出缓冲
            c.arg(&absolute_path);
            c
        } else if path.ends_with(".js") {
            let mut c = Command::new("node");
            c.arg(&absolute_path);
            c
        } else if path.ends_with(".ts") {
            let mut c = Command::new("bun");
            c.arg(&absolute_path);
            c
        } else {
            return Err(anyhow!("Unsupported file type"));
        };

        // 设置工作目录为脚本所在目录
        cmd.current_dir(working_dir);
        cmd.env_clear();
        let mut env_vars = self.parse_env(env_json).await;
        let notify_mode = if self.manual_notification_enabled().await { "simulate" } else { "off" };
        env_vars.insert("ZHUQUE_NOTIFY_MODE".to_string(), notify_mode.to_string());
        cmd.envs(env_vars);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn()?;

        // 获取进程ID并注册
        let pid = child.id().ok_or_else(|| anyhow!("Failed to get process ID"))?;
        self.running_processes.write().await.insert(execution_id.clone(), pid);

        let stdout = child.stdout.take().ok_or_else(|| anyhow!("Failed to capture stdout"))?;
        let stderr = child.stderr.take().ok_or_else(|| anyhow!("Failed to capture stderr"))?;

        let exec_id_clone = execution_id.clone();
        let processes = self.running_processes.clone();

        let stream = async_stream::stream! {
            let mut stdout_reader = LineReader::new(stdout);
            let mut stderr_reader = LineReader::new(stderr);

            loop {
                tokio::select! {
                    result = stdout_reader.next_line() => {
                        match result {
                            Ok(Some(line)) => yield Ok(line),
                            Ok(None) => break,
                            Err(e) => yield Err(anyhow!("Stdout error: {}", e)),
                        }
                    }
                    result = stderr_reader.next_line() => {
                        match result {
                            Ok(Some(line)) => yield Ok(line),
                            Ok(None) => {},
                            Err(e) => yield Err(anyhow!("Stderr error: {}", e)),
                        }
                    }
                }
            }

            // 等待进程结束
            match child.wait().await {
                Ok(status) => {
                    if status.success() {
                        yield Ok(format!("[EXIT] Process exited with code 0"));
                    } else {
                        yield Ok(format!("[EXIT] Process exited with code {}", status.code().unwrap_or(-1)));
                    }
                }
                Err(e) => yield Err(anyhow!("Failed to wait for process: {}", e)),
            }

            // 清理进程记录
            processes.write().await.remove(&exec_id_clone);
        };

        Ok((execution_id, stream))
    }

    /// 执行临时脚本内容（用于调试）
    pub async fn execute_content(
        &self,
        content: &str,
        script_type: &str,
        env_json: Option<&str>,
        file_path: Option<&str>,
    ) -> Result<(String, impl tokio_stream::Stream<Item = Result<String>>)> {
        let script_type = Self::normalize_script_type(script_type);
        let execution_id = uuid::Uuid::new_v4().to_string();

        // 先计算工作目录，临时文件也写到这里
        // 这样 Python 的 sys.path[0] 和 Node.js 的 require('./') 都能正确解析同目录模块
        let work_dir = if let Some(path) = file_path {
            let file_full_path = self.base_path.join(path);
            if let Some(parent) = file_full_path.parent() {
                parent.to_path_buf()
            } else {
                self.base_path.clone()
            }
        } else {
            self.base_path.clone()
        };
        tokio::fs::create_dir_all(&work_dir).await?;

        // 临时文件写到 work_dir，文件名以 .__debug_ 开头避免与用户文件冲突
        let temp_file = work_dir.join(format!(".__debug_{}.{}", execution_id, script_type));
        tokio::fs::write(&temp_file, content).await?;

        // 如果是shell脚本，添加执行权限
        #[cfg(unix)]
        if script_type == "sh" {
            let metadata = tokio::fs::metadata(&temp_file).await?;
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o755);
            tokio::fs::set_permissions(&temp_file, permissions).await?;
        }

        // 获取临时文件的绝对路径
        let temp_file_abs = temp_file.canonicalize()?;

        // 执行脚本
        let mut cmd = match script_type {
            "sh" => {
                let mut c = Command::new("bash");
                c.arg(&temp_file_abs);
                c
            }
            "py" => {
                let mut c = Command::new(PYTHON_CMD.as_str());
                c.arg("-u");  // 禁用输出缓冲
                c.arg(&temp_file_abs);
                c
            }
            "js" => {
                let mut c = Command::new("node");
                c.arg(&temp_file_abs);
                c
            }
            "ts" => {
                let mut c = Command::new("bun");
                c.arg(&temp_file_abs);
                c
            }
            _ => return Err(anyhow!("Unsupported script type")),
        };

        cmd.current_dir(&work_dir);
        cmd.env_clear();
        let mut env_vars = self.parse_env(env_json).await;
        let notify_mode = if self.manual_notification_enabled().await { "simulate" } else { "off" };
        env_vars.insert("ZHUQUE_NOTIFY_MODE".to_string(), notify_mode.to_string());
        cmd.envs(env_vars);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn()?;

        // 获取进程ID并注册
        let pid = child.id().ok_or_else(|| anyhow!("Failed to get process ID"))?;
        self.running_processes.write().await.insert(execution_id.clone(), pid);

        let stdout = child.stdout.take().ok_or_else(|| anyhow!("Failed to capture stdout"))?;
        let stderr = child.stderr.take().ok_or_else(|| anyhow!("Failed to capture stderr"))?;

        let temp_file_clone = temp_file.clone();
        let exec_id_clone = execution_id.clone();
        let processes = self.running_processes.clone();

        let stream = async_stream::stream! {
            let mut stdout_reader = LineReader::new(stdout);
            let mut stderr_reader = LineReader::new(stderr);

            loop {
                tokio::select! {
                    result = stdout_reader.next_line() => {
                        match result {
                            Ok(Some(line)) => yield Ok(line),
                            Ok(None) => break,
                            Err(e) => yield Err(anyhow!("Stdout error: {}", e)),
                        }
                    }
                    result = stderr_reader.next_line() => {
                        match result {
                            Ok(Some(line)) => yield Ok(line),
                            Ok(None) => {},
                            Err(e) => yield Err(anyhow!("Stderr error: {}", e)),
                        }
                    }
                }
            }

            // 等待进程结束
            match child.wait().await {
                Ok(status) => {
                    if status.success() {
                        yield Ok(format!("[EXIT] Process exited with code 0"));
                    } else {
                        yield Ok(format!("[EXIT] Process exited with code {}", status.code().unwrap_or(-1)));
                    }
                }
                Err(e) => yield Err(anyhow!("Failed to wait for process: {}", e)),
            }

            // 清理临时文件和进程记录
            let _ = tokio::fs::remove_file(&temp_file_clone).await;
            processes.write().await.remove(&exec_id_clone);
        };

        Ok((execution_id, stream))
    }

    /// 中止正在执行的脚本
    pub async fn kill_execution(&self, execution_id: &str) -> Result<()> {
        let mut processes = self.running_processes.write().await;

        if let Some(pid) = processes.remove(execution_id) {
            // 使用kill命令终止进程
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
            Err(anyhow!("Execution ID not found or already finished"))
        }
    }

    /// 列出正在执行的脚本
    pub async fn list_running(&self) -> Vec<String> {
        self.running_processes.read().await.keys().cloned().collect()
    }

    /// 解压 ZIP 文件到指定目录
    pub async fn extract_zip(&self, data: &[u8], target_path: &str) -> Result<()> {
        self.validate_path(target_path)?;
        let full_path = self.base_path.join(target_path);

        // 创建目标目录
        tokio::fs::create_dir_all(&full_path).await?;

        // 在阻塞任务中执行解压操作
        let full_path_clone = full_path.clone();
        let data_vec = data.to_vec();

        tokio::task::spawn_blocking(move || {
            let cursor = Cursor::new(data_vec);
            let mut archive = ZipArchive::new(cursor)?;

            for i in 0..archive.len() {
                let mut file = archive.by_index(i)?;
                let outpath = match file.enclosed_name() {
                    Some(path) => full_path_clone.join(path),
                    None => continue,
                };

                if file.name().ends_with('/') {
                    std::fs::create_dir_all(&outpath)?;
                } else {
                    if let Some(p) = outpath.parent() {
                        if !p.exists() {
                            std::fs::create_dir_all(p)?;
                        }
                    }
                    let mut outfile = File::create(&outpath)?;
                    std::io::copy(&mut file, &mut outfile)?;

                    // 保留执行权限
                    #[cfg(unix)]
                    if let Some(mode) = file.unix_mode() {
                        use std::fs::Permissions;
                        std::fs::set_permissions(&outpath, Permissions::from_mode(mode))?;
                    }
                }
            }
            Ok::<(), anyhow::Error>(())
        })
        .await??;

        Ok(())
    }

    /// 解压 TAR.GZ 文件到指定目录
    pub async fn extract_tar_gz(&self, data: &[u8], target_path: &str) -> Result<()> {
        self.validate_path(target_path)?;
        let full_path = self.base_path.join(target_path);

        // 创建目标目录
        tokio::fs::create_dir_all(&full_path).await?;

        // 在阻塞任务中执行解压操作
        let full_path_clone = full_path.clone();
        let data_vec = data.to_vec();

        tokio::task::spawn_blocking(move || {
            let cursor = Cursor::new(data_vec);
            let tar = GzDecoder::new(cursor);
            let mut archive = Archive::new(tar);
            archive.unpack(&full_path_clone)?;
            Ok::<(), anyhow::Error>(())
        })
        .await??;

        Ok(())
    }

    /// 解压 TAR 文件到指定目录
    pub async fn extract_tar(&self, data: &[u8], target_path: &str) -> Result<()> {
        self.validate_path(target_path)?;
        let full_path = self.base_path.join(target_path);

        // 创建目标目录
        tokio::fs::create_dir_all(&full_path).await?;

        // 在阻塞任务中执行解压操作
        let full_path_clone = full_path.clone();
        let data_vec = data.to_vec();

        tokio::task::spawn_blocking(move || {
            let cursor = Cursor::new(data_vec);
            let mut archive = Archive::new(cursor);
            archive.unpack(&full_path_clone)?;
            Ok::<(), anyhow::Error>(())
        })
        .await??;

        Ok(())
    }
}
