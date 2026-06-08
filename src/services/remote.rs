use crate::models::{
    CreateRemoteAgentRequest, CreateRemoteAgentResponse, CreateRemoteCommandRequest,
    RegisterAgentRequest, RegisterAgentResponse, RemoteAgent, RemoteAgentMessage, RemoteCommand,
    RemoteCommandLog, RemoteServerMessage,
};
use anyhow::{anyhow, Result};
use chrono::{Duration, Utc};
use rand::{rngs::OsRng, RngCore};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, oneshot, RwLock};
use uuid::Uuid;

#[derive(Clone)]
pub struct RemoteSession {
    pub agent_id: i64,
    pub session_id: String,
    pub sender: mpsc::UnboundedSender<RemoteServerMessage>,
}

pub struct RemoteService {
    pool: Arc<RwLock<SqlitePool>>,
    sessions: Arc<RwLock<HashMap<i64, RemoteSession>>>,
    log_channels: Arc<RwLock<HashMap<String, broadcast::Sender<String>>>>,
    pending_requests: Arc<RwLock<HashMap<String, oneshot::Sender<serde_json::Value>>>>,
    terminal_channels: Arc<RwLock<HashMap<String, mpsc::UnboundedSender<RemoteAgentMessage>>>>,
}

impl RemoteService {
    pub fn new(pool: Arc<RwLock<SqlitePool>>) -> Self {
        Self {
            pool,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            log_channels: Arc::new(RwLock::new(HashMap::new())),
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
            terminal_channels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn register_agent(
        &self,
        bootstrap_token: Option<&str>,
        payload: RegisterAgentRequest,
    ) -> Result<RegisterAgentResponse> {
        let pool = self.pool.read().await;
        let expected = sqlx::query_scalar::<_, String>(
            "SELECT value FROM system_configs WHERE key = 'remote_register_token'",
        )
        .fetch_optional(&*pool)
        .await?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
        drop(pool);

        match (expected.as_deref(), bootstrap_token.map(str::trim)) {
            (Some(expected), Some(actual)) if expected == actual => {}
            (Some(_), _) => return Err(anyhow!("invalid register token")),
            (None, _) => return Err(anyhow!("remote register token is not configured in system settings")),
        }

        let token = Self::new_token();
        let capabilities = payload
            .capabilities
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let tags = payload.tags.as_ref().map(serde_json::to_string).transpose()?;
        let now = Utc::now();
        let pool = self.pool.read().await;
        let result = sqlx::query(
            r#"
            INSERT INTO remote_agents
                (name, hostname, os, arch, version, status, registered_at, token_hash, capabilities, tags)
            VALUES (?, ?, ?, ?, ?, 'offline', ?, ?, ?, ?)
            "#,
        )
        .bind(payload.name)
        .bind(payload.hostname)
        .bind(payload.os)
        .bind(payload.arch)
        .bind(payload.version)
        .bind(now)
        .bind(&token)
        .bind(capabilities)
        .bind(tags)
        .execute(&*pool)
        .await?;

        Ok(RegisterAgentResponse {
            agent_id: result.last_insert_rowid(),
            token,
        })
    }

    pub async fn create_agent(&self, payload: CreateRemoteAgentRequest) -> Result<CreateRemoteAgentResponse> {
        let token = Self::new_token();
        let now = Utc::now();
        let pool = self.pool.read().await;
        let result = sqlx::query(
            r#"
            INSERT INTO remote_agents
                (name, hostname, os, arch, version, status, registered_at, token_hash, capabilities, tags, remark)
            VALUES (?, NULL, NULL, NULL, NULL, 'offline', ?, ?, NULL, NULL, ?)
            "#,
        )
        .bind(payload.name)
        .bind(now)
        .bind(&token)
        .bind(payload.remark)
        .execute(&*pool)
        .await?;
        let id = result.last_insert_rowid();
        drop(pool);

        let agent = self
            .get_agent(id)
            .await?
            .ok_or_else(|| anyhow!("agent not found after create"))?;
        Ok(CreateRemoteAgentResponse { agent, token })
    }

    pub async fn regenerate_agent_token(&self, agent_id: i64) -> Result<CreateRemoteAgentResponse> {
        let token = Self::new_token();
        let pool = self.pool.read().await;
        let result = sqlx::query("UPDATE remote_agents SET token_hash = ? WHERE id = ?")
            .bind(&token)
            .bind(agent_id)
            .execute(&*pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(anyhow!("agent not found"));
        }
        drop(pool);

        let agent = self
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| anyhow!("agent not found after token regeneration"))?;
        Ok(CreateRemoteAgentResponse { agent, token })
    }

    pub async fn authenticate_agent(&self, agent_id: i64, token: &str) -> Result<RemoteAgent> {
        let pool = self.pool.read().await;
        let agent = sqlx::query_as::<_, RemoteAgent>("SELECT * FROM remote_agents WHERE id = ?")
            .bind(agent_id)
            .fetch_optional(&*pool)
            .await?
            .ok_or_else(|| anyhow!("agent not found"))?;

        if agent.disabled {
            return Err(anyhow!("agent is disabled"));
        }

        if token != agent.token_hash {
            return Err(anyhow!("invalid agent token"));
        }

        Ok(agent)
    }

    pub async fn attach_session(
        &self,
        agent_id: i64,
        sender: mpsc::UnboundedSender<RemoteServerMessage>,
        remote_addr: Option<String>,
    ) -> Result<String> {
        let session_id = Uuid::new_v4().to_string();
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(
                agent_id,
                RemoteSession {
                    agent_id,
                    session_id: session_id.clone(),
                    sender,
                },
            );
        }

        let now = Utc::now();
        let pool = self.pool.read().await;
        sqlx::query("UPDATE remote_agents SET status = 'online', last_seen_at = ? WHERE id = ?")
            .bind(now)
            .bind(agent_id)
            .execute(&*pool)
            .await?;

        sqlx::query(
            "INSERT INTO remote_agent_sessions (id, agent_id, connected_at, remote_addr) VALUES (?, ?, ?, ?)",
        )
        .bind(&session_id)
        .bind(agent_id)
        .bind(now)
        .bind(remote_addr)
        .execute(&*pool)
        .await?;

        Ok(session_id)
    }

    pub async fn detach_session(&self, agent_id: i64, session_id: &str) -> Result<()> {
        let mut detached_current_session = false;
        {
            let mut sessions = self.sessions.write().await;
            if sessions
                .get(&agent_id)
                .map(|s| s.session_id.as_str() == session_id)
                .unwrap_or(false)
            {
                sessions.remove(&agent_id);
                detached_current_session = true;
            }
        }

        if !detached_current_session {
            return Ok(());
        }

        let now = Utc::now();
        let pool = self.pool.read().await;
        sqlx::query("UPDATE remote_agents SET status = 'offline', last_seen_at = ? WHERE id = ?")
            .bind(now)
            .bind(agent_id)
            .execute(&*pool)
            .await?;
        sqlx::query("UPDATE remote_agent_sessions SET disconnected_at = ? WHERE id = ?")
            .bind(now)
            .bind(session_id)
            .execute(&*pool)
            .await?;

        Ok(())
    }

    pub async fn list_agents(&self) -> Result<Vec<RemoteAgent>> {
        self.mark_stale_agents_offline().await?;
        let pool = self.pool.read().await;
        Ok(sqlx::query_as::<_, RemoteAgent>(
            "SELECT * FROM remote_agents ORDER BY id DESC",
        )
        .fetch_all(&*pool)
        .await?)
    }

    pub async fn get_agent(&self, id: i64) -> Result<Option<RemoteAgent>> {
        self.mark_stale_agents_offline().await?;
        let pool = self.pool.read().await;
        Ok(sqlx::query_as::<_, RemoteAgent>("SELECT * FROM remote_agents WHERE id = ?")
            .bind(id)
            .fetch_optional(&*pool)
            .await?)
    }

    pub async fn delete_agent(&self, agent_id: i64) -> Result<()> {
        self.sessions.write().await.remove(&agent_id);
        let pool = self.pool.read().await;
        let result = sqlx::query("DELETE FROM remote_agents WHERE id = ?")
            .bind(agent_id)
            .execute(&*pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(anyhow!("agent not found"));
        }
        Ok(())
    }

    pub async fn create_command(
        &self,
        agent_id: i64,
        payload: CreateRemoteCommandRequest,
        created_by: Option<String>,
    ) -> Result<RemoteCommand> {
        let command_id = Uuid::new_v4().to_string();
        let kind = payload.kind.unwrap_or_else(|| "command".to_string());
        let timeout = payload.timeout;
        let wire_payload = serde_json::json!({
            "command": payload.command,
            "working_dir": payload.working_dir,
            "env": payload.env,
            "timeout": timeout,
        });
        let payload_json = serde_json::to_string(&wire_payload)?;
        let now = Utc::now();

        let (tx, _) = broadcast::channel(500);
        self.log_channels
            .write()
            .await
            .insert(command_id.clone(), tx);

        let pool = self.pool.read().await;
        sqlx::query(
            r#"
            INSERT INTO remote_commands
                (id, agent_id, kind, payload, status, timeout, created_by, created_at)
            VALUES (?, ?, ?, ?, 'queued', ?, ?, ?)
            "#,
        )
        .bind(&command_id)
        .bind(agent_id)
        .bind(&kind)
        .bind(&payload_json)
        .bind(timeout)
        .bind(created_by)
        .bind(now)
        .execute(&*pool)
        .await?;
        drop(pool);

        self.send_to_agent(
            agent_id,
            RemoteServerMessage::CommandStart {
                command_id: command_id.clone(),
                command: wire_payload["command"].as_str().unwrap_or_default().to_string(),
                working_dir: wire_payload["working_dir"].as_str().map(ToOwned::to_owned),
                env: serde_json::from_value(wire_payload["env"].clone()).ok().flatten(),
                timeout,
                script_content: None,
                script_name: None,
            },
        )
        .await?;

        self.get_command(&command_id)
            .await?
            .ok_or_else(|| anyhow!("command not found after create"))
    }

    pub async fn create_script_command(
        &self,
        agent_id: i64,
        script_name: String,
        script_content: String,
        command: String,
        working_dir: Option<String>,
        env: Option<HashMap<String, String>>,
        timeout: Option<i64>,
    ) -> Result<RemoteCommand> {
        let command_id = Uuid::new_v4().to_string();
        let payload_json = serde_json::to_string(&serde_json::json!({
            "command": command,
            "working_dir": working_dir,
            "env": env,
            "timeout": timeout,
            "script_name": script_name,
        }))?;
        let now = Utc::now();
        let (tx, _) = broadcast::channel(500);
        self.log_channels
            .write()
            .await
            .insert(command_id.clone(), tx);

        let pool = self.pool.read().await;
        sqlx::query(
            r#"
            INSERT INTO remote_commands
                (id, agent_id, kind, payload, status, timeout, created_by, created_at)
            VALUES (?, ?, 'script', ?, 'queued', ?, 'web', ?)
            "#,
        )
        .bind(&command_id)
        .bind(agent_id)
        .bind(&payload_json)
        .bind(timeout)
        .bind(now)
        .execute(&*pool)
        .await?;
        drop(pool);

        self.send_to_agent(
            agent_id,
            RemoteServerMessage::CommandStart {
                command_id: command_id.clone(),
                command,
                working_dir,
                env,
                timeout,
                script_content: Some(script_content),
                script_name: Some(script_name),
            },
        )
        .await?;

        self.get_command(&command_id)
            .await?
            .ok_or_else(|| anyhow!("command not found after create"))
    }

    pub async fn execute_task_command(
        &self,
        agent_id: i64,
        command: String,
        working_dir: Option<String>,
        env: Option<HashMap<String, String>>,
        timeout: Option<i64>,
    ) -> Result<(String, String, String)> {
        let created = self
            .create_command(
                agent_id,
                CreateRemoteCommandRequest {
                    kind: Some("task".to_string()),
                    command,
                    working_dir,
                    env,
                    timeout,
                },
                Some("scheduler".to_string()),
            )
            .await?;

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let command = self
                .get_command(&created.id)
                .await?
                .ok_or_else(|| anyhow!("remote command disappeared"))?;
            match command.status.as_str() {
                "success" | "failed" | "killed" | "timeout" => {
                    return Ok((
                        command.id,
                        command.output.unwrap_or_default(),
                        command.status,
                    ));
                }
                _ => {}
            }
        }
    }

    pub async fn kill_command(&self, command_id: &str) -> Result<()> {
        let command = self
            .get_command(command_id)
            .await?
            .ok_or_else(|| anyhow!("command not found"))?;
        self.send_to_agent(
            command.agent_id,
            RemoteServerMessage::CommandKill {
                command_id: command_id.to_string(),
            },
        )
        .await
    }

    pub async fn request_status(&self, agent_id: i64) -> Result<serde_json::Value> {
        let request_id = Uuid::new_v4().to_string();
        let rx = self
            .register_pending_request(&request_id)
            .await;
        self.send_to_agent(
            agent_id,
            RemoteServerMessage::StatusRequest {
                request_id: request_id.clone(),
            },
        )
        .await?;
        Self::await_pending(rx).await
    }

    pub async fn list_files(&self, agent_id: i64, path: String) -> Result<serde_json::Value> {
        let request_id = Uuid::new_v4().to_string();
        let rx = self
            .register_pending_request(&request_id)
            .await;
        self.send_to_agent(
            agent_id,
            RemoteServerMessage::FileList {
                request_id: request_id.clone(),
                path,
            },
        )
        .await?;
        Self::await_pending(rx).await
    }

    pub async fn read_file(&self, agent_id: i64, path: String) -> Result<serde_json::Value> {
        let request_id = Uuid::new_v4().to_string();
        let rx = self
            .register_pending_request(&request_id)
            .await;
        self.send_to_agent(
            agent_id,
            RemoteServerMessage::FileRead {
                request_id: request_id.clone(),
                path,
            },
        )
        .await?;
        Self::await_pending(rx).await
    }

    pub async fn write_file(&self, agent_id: i64, path: String, content: String) -> Result<serde_json::Value> {
        let request_id = Uuid::new_v4().to_string();
        let rx = self.register_pending_request(&request_id).await;
        self.send_to_agent(
            agent_id,
            RemoteServerMessage::FileWrite {
                request_id: request_id.clone(),
                path,
                content,
            },
        )
        .await?;
        Self::await_pending(rx).await
    }

    pub async fn delete_file(&self, agent_id: i64, path: String) -> Result<serde_json::Value> {
        let request_id = Uuid::new_v4().to_string();
        let rx = self.register_pending_request(&request_id).await;
        self.send_to_agent(
            agent_id,
            RemoteServerMessage::FileDelete {
                request_id: request_id.clone(),
                path,
            },
        )
        .await?;
        Self::await_pending(rx).await
    }

    pub async fn create_dir(&self, agent_id: i64, path: String) -> Result<serde_json::Value> {
        let request_id = Uuid::new_v4().to_string();
        let rx = self.register_pending_request(&request_id).await;
        self.send_to_agent(
            agent_id,
            RemoteServerMessage::FileMkdir {
                request_id: request_id.clone(),
                path,
            },
        )
        .await?;
        Self::await_pending(rx).await
    }

    pub async fn rename_file(&self, agent_id: i64, from: String, to: String) -> Result<serde_json::Value> {
        let request_id = Uuid::new_v4().to_string();
        let rx = self.register_pending_request(&request_id).await;
        self.send_to_agent(
            agent_id,
            RemoteServerMessage::FileRename {
                request_id: request_id.clone(),
                from,
                to,
            },
        )
        .await?;
        Self::await_pending(rx).await
    }

    pub async fn open_terminal(
        &self,
        agent_id: i64,
        rows: u16,
        cols: u16,
    ) -> Result<(String, mpsc::UnboundedReceiver<RemoteAgentMessage>)> {
        let terminal_id = Uuid::new_v4().to_string();
        let (tx, rx) = mpsc::unbounded_channel();
        self.terminal_channels
            .write()
            .await
            .insert(terminal_id.clone(), tx);

        if let Err(error) = self
            .send_to_agent(
                agent_id,
                RemoteServerMessage::TerminalOpen {
                    terminal_id: terminal_id.clone(),
                    rows,
                    cols,
                },
            )
            .await
        {
            self.terminal_channels.write().await.remove(&terminal_id);
            return Err(error);
        }

        Ok((terminal_id, rx))
    }

    pub async fn send_terminal_input(&self, agent_id: i64, terminal_id: String, data: String) -> Result<()> {
        self.send_to_agent(
            agent_id,
            RemoteServerMessage::TerminalInput { terminal_id, data },
        )
        .await
    }

    pub async fn resize_terminal(&self, agent_id: i64, terminal_id: String, rows: u16, cols: u16) -> Result<()> {
        self.send_to_agent(
            agent_id,
            RemoteServerMessage::TerminalResize {
                terminal_id,
                rows,
                cols,
            },
        )
        .await
    }

    pub async fn close_terminal(&self, agent_id: i64, terminal_id: &str) -> Result<()> {
        self.terminal_channels.write().await.remove(terminal_id);
        self.send_to_agent(
            agent_id,
            RemoteServerMessage::TerminalClose {
                terminal_id: terminal_id.to_string(),
            },
        )
        .await
    }

    pub async fn get_command(&self, id: &str) -> Result<Option<RemoteCommand>> {
        let pool = self.pool.read().await;
        Ok(sqlx::query_as::<_, RemoteCommand>("SELECT * FROM remote_commands WHERE id = ?")
            .bind(id)
            .fetch_optional(&*pool)
            .await?)
    }

    pub async fn list_commands(&self, agent_id: Option<i64>) -> Result<Vec<RemoteCommand>> {
        let pool = self.pool.read().await;
        if let Some(agent_id) = agent_id {
            Ok(sqlx::query_as::<_, RemoteCommand>(
                "SELECT * FROM remote_commands WHERE agent_id = ? ORDER BY created_at DESC LIMIT 200",
            )
            .bind(agent_id)
            .fetch_all(&*pool)
            .await?)
        } else {
            Ok(sqlx::query_as::<_, RemoteCommand>(
                "SELECT * FROM remote_commands ORDER BY created_at DESC LIMIT 200",
            )
            .fetch_all(&*pool)
            .await?)
        }
    }

    pub async fn list_command_logs(&self, command_id: &str) -> Result<Vec<RemoteCommandLog>> {
        let pool = self.pool.read().await;
        Ok(sqlx::query_as::<_, RemoteCommandLog>(
            "SELECT * FROM remote_command_logs WHERE command_id = ? ORDER BY id ASC",
        )
        .bind(command_id)
        .fetch_all(&*pool)
        .await?)
    }

    pub async fn subscribe_logs(&self, command_id: &str) -> broadcast::Receiver<String> {
        let mut channels = self.log_channels.write().await;
        channels
            .entry(command_id.to_string())
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(500);
                tx
            })
            .subscribe()
    }

    pub async fn handle_agent_message(&self, agent_id: i64, message: RemoteAgentMessage) -> Result<()> {
        match message {
            RemoteAgentMessage::Hello {
                hostname,
                os,
                arch,
                version,
                capabilities,
            } => {
                let pool = self.pool.read().await;
                sqlx::query(
                    "UPDATE remote_agents SET hostname = ?, os = ?, arch = ?, version = ?, capabilities = ?, status = 'online', last_seen_at = ? WHERE id = ?",
                )
                .bind(hostname)
                .bind(os)
                .bind(arch)
                .bind(version)
                .bind(capabilities.map(|v| v.to_string()))
                .bind(Utc::now())
                .bind(agent_id)
                .execute(&*pool)
                .await?;
            }
            RemoteAgentMessage::Heartbeat => {
                let pool = self.pool.read().await;
                sqlx::query("UPDATE remote_agents SET status = 'online', last_seen_at = ? WHERE id = ?")
                    .bind(Utc::now())
                    .bind(agent_id)
                    .execute(&*pool)
                    .await?;
            }
            RemoteAgentMessage::Status {
                request_id,
                metrics,
            } => {
                let pool = self.pool.read().await;
                sqlx::query("UPDATE remote_agents SET status = 'online', last_seen_at = ? WHERE id = ?")
                    .bind(Utc::now())
                    .bind(agent_id)
                    .execute(&*pool)
                    .await?;
                if let Some(request_id) = request_id {
                    self.resolve_pending_request(&request_id, metrics).await;
                }
            }
            RemoteAgentMessage::CommandStarted { command_id } => {
                let pool = self.pool.read().await;
                sqlx::query("UPDATE remote_commands SET status = 'running', started_at = ? WHERE id = ?")
                    .bind(Utc::now())
                    .bind(&command_id)
                    .execute(&*pool)
                    .await?;
                self.publish_log(&command_id, "system", "[started]").await?;
            }
            RemoteAgentMessage::CommandOutput {
                command_id,
                stream,
                line,
            } => {
                self.publish_log(&command_id, &stream, &line).await?;
            }
            RemoteAgentMessage::CommandFinished {
                command_id,
                status,
                exit_code,
                error,
                ..
            } => {
                let output = self
                    .list_command_logs(&command_id)
                    .await?
                    .into_iter()
                    .map(|l| l.line)
                    .collect::<Vec<_>>()
                    .join("\n");
                let pool = self.pool.read().await;
                sqlx::query(
                    "UPDATE remote_commands SET status = ?, exit_code = ?, output = ?, error = ?, finished_at = ? WHERE id = ?",
                )
                .bind(&status)
                .bind(exit_code)
                .bind(output)
                .bind(error)
                .bind(Utc::now())
                .bind(&command_id)
                .execute(&*pool)
                .await?;
                self.publish_log(&command_id, "system", &format!("[finished] {}", status)).await?;
                self.log_channels.write().await.remove(&command_id);
            }
            RemoteAgentMessage::FileListResult {
                request_id,
                entries,
                error,
            } => {
                self.resolve_pending_request(
                    &request_id,
                    serde_json::json!({ "entries": entries, "error": error }),
                )
                .await;
            }
            RemoteAgentMessage::FileReadResult {
                request_id,
                content,
                error,
            } => {
                self.resolve_pending_request(
                    &request_id,
                    serde_json::json!({ "content": content, "error": error }),
                )
                .await;
            }
            RemoteAgentMessage::FileActionResult {
                request_id,
                success,
                error,
            } => {
                self.resolve_pending_request(
                    &request_id,
                    serde_json::json!({ "success": success, "error": error }),
                )
                .await;
            }
            RemoteAgentMessage::TerminalOpened { terminal_id } => {
                self.forward_terminal_message(
                    &terminal_id,
                    RemoteAgentMessage::TerminalOpened { terminal_id: terminal_id.clone() },
                )
                .await;
            }
            RemoteAgentMessage::TerminalOutput { terminal_id, data } => {
                self.forward_terminal_message(
                    &terminal_id,
                    RemoteAgentMessage::TerminalOutput {
                        terminal_id: terminal_id.clone(),
                        data,
                    },
                )
                .await;
            }
            RemoteAgentMessage::TerminalClosed { terminal_id, error } => {
                self.forward_terminal_message(
                    &terminal_id,
                    RemoteAgentMessage::TerminalClosed {
                        terminal_id: terminal_id.clone(),
                        error,
                    },
                )
                .await;
                self.terminal_channels.write().await.remove(&terminal_id);
            }
        }

        Ok(())
    }

    async fn publish_log(&self, command_id: &str, stream: &str, line: &str) -> Result<()> {
        let pool = self.pool.read().await;
        sqlx::query(
            "INSERT INTO remote_command_logs (command_id, stream, line, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(command_id)
        .bind(stream)
        .bind(line)
        .bind(Utc::now())
        .execute(&*pool)
        .await?;

        if let Some(tx) = self.log_channels.read().await.get(command_id) {
            let _ = tx.send(format!("[{}] {}", stream, line));
        }
        Ok(())
    }

    async fn send_to_agent(&self, agent_id: i64, message: RemoteServerMessage) -> Result<()> {
        let sessions = self.sessions.read().await;
        let session = sessions
            .get(&agent_id)
            .ok_or_else(|| anyhow!("agent is not online"))?;
        session
            .sender
            .send(message)
            .map_err(|_| anyhow!("agent session is closed"))
    }

    async fn mark_stale_agents_offline(&self) -> Result<()> {
        let cutoff = Utc::now() - Duration::seconds(70);
        let pool = self.pool.read().await;
        sqlx::query("UPDATE remote_agents SET status = 'offline' WHERE status = 'online' AND last_seen_at < ?")
            .bind(cutoff)
            .execute(&*pool)
            .await?;
        Ok(())
    }

    async fn register_pending_request(&self, request_id: &str) -> oneshot::Receiver<serde_json::Value> {
        let (tx, rx) = oneshot::channel();
        self.pending_requests
            .write()
            .await
            .insert(request_id.to_string(), tx);
        rx
    }

    async fn resolve_pending_request(&self, request_id: &str, value: serde_json::Value) {
        if let Some(tx) = self.pending_requests.write().await.remove(request_id) {
            let _ = tx.send(value);
        }
    }

    async fn forward_terminal_message(&self, terminal_id: &str, message: RemoteAgentMessage) {
        if let Some(tx) = self.terminal_channels.read().await.get(terminal_id) {
            let _ = tx.send(message);
        }
    }

    async fn await_pending(rx: oneshot::Receiver<serde_json::Value>) -> Result<serde_json::Value> {
        tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .map_err(|_| anyhow!("remote request timed out"))?
            .map_err(|_| anyhow!("remote request was canceled"))
    }

    fn new_token() -> String {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        hex::encode(bytes)
    }
}
