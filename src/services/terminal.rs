// 非 Android 平台：完整的终端功能
#[cfg(not(target_os = "android"))]
mod terminal_impl {
    use anyhow::{anyhow, Result};
    use portable_pty::{native_pty_system, CommandBuilder, PtySize, PtySystem};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    pub struct TerminalSession {
        pub id: String,
        pub child: Box<dyn portable_pty::Child + Send + Sync>,
    }

    pub struct TerminalService {
        sessions: Arc<Mutex<HashMap<String, TerminalSession>>>,
        working_dir: PathBuf,
    }

    impl TerminalService {
        pub fn new(working_dir: PathBuf) -> Self {
            Self {
                sessions: Arc::new(Mutex::new(HashMap::new())),
                working_dir,
            }
        }

        /// 创建新的终端会话并返回 IO 句柄
        pub async fn create_session(
            &self,
            env_vars: HashMap<String, String>,
            rows: u16,
            cols: u16,
        ) -> Result<(
            String,
            Box<dyn std::io::Read + Send>,
            Box<dyn std::io::Write + Send>,
            Arc<Mutex<Box<dyn portable_pty::MasterPty + Send>>>,
        )> {
            let pty_system = native_pty_system();

            // 创建 PTY
            let pair = pty_system
                .openpty(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| anyhow!("Failed to create PTY: {}", e))?;

            // 配置 Shell 命令
            let shell = if cfg!(target_os = "windows") {
                "powershell.exe".to_string()
            } else {
                std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
            };

            let mut cmd = CommandBuilder::new(shell);
            cmd.env("TERM", "xterm-256color");

            // 设置工作目录为 scripts 目录
            cmd.cwd(&self.working_dir);

            // 注入环境变量
            for (key, value) in env_vars {
                cmd.env(key, value);
            }

            // 启动 Shell
            let child = pair
                .slave
                .spawn_command(cmd)
                .map_err(|e| anyhow!("Failed to spawn shell: {}", e))?;

            // 生成会话 ID
            let session_id = Uuid::new_v4().to_string();

            // 获取读写器
            let reader = pair
                .master
                .try_clone_reader()
                .map_err(|e| anyhow!("Failed to clone reader: {}", e))?;

            let writer = pair
                .master
                .take_writer()
                .map_err(|e| anyhow!("Failed to take writer: {}", e))?;

            let master = Arc::new(Mutex::new(pair.master));

            // 保存会话
            let session = TerminalSession {
                id: session_id.clone(),
                child,
            };

            self.sessions
                .lock()
                .await
                .insert(session_id.clone(), session);

            tracing::info!("Created terminal session: {} in directory: {:?}", session_id, self.working_dir);

            Ok((session_id, reader, writer, master))
        }

    /// 移除会话
    pub async fn remove_session(&self, session_id: &str) -> Result<()> {
        let mut sessions = self.sessions.lock().await;
        if let Some(mut session) = sessions.remove(session_id) {
            // 尝试杀死子进程
            let _ = session.child.kill();
            tracing::info!("Removed terminal session: {}", session_id);
        }
        Ok(())
    }
    }
}

#[cfg(not(target_os = "android"))]
pub use terminal_impl::*;

// Android 平台：空实现
#[cfg(target_os = "android")]
mod terminal_impl {
    use anyhow::{anyhow, Result};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    pub struct TerminalSession {
        pub id: String,
    }

    pub struct TerminalService {
        _working_dir: PathBuf,
    }

    impl TerminalService {
        pub fn new(working_dir: PathBuf) -> Self {
            Self {
                _working_dir: working_dir,
            }
        }

        pub async fn create_session(
            &self,
            _env_vars: HashMap<String, String>,
            _rows: u16,
            _cols: u16,
        ) -> Result<(
            String,
            Box<dyn std::io::Read + Send>,
            Box<dyn std::io::Write + Send>,
            Arc<Mutex<Box<dyn std::io::Read + Send>>>,
        )> {
            Err(anyhow!("Terminal is not supported on Android platform"))
        }

        pub async fn remove_session(&self, _session_id: &str) -> Result<()> {
            Err(anyhow!("Terminal is not supported on Android platform"))
        }
    }
}

#[cfg(target_os = "android")]
pub use terminal_impl::*;
