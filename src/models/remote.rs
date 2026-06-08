use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct RemoteAgent {
    pub id: i64,
    pub name: String,
    pub hostname: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub version: Option<String>,
    pub status: String,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub registered_at: DateTime<Utc>,
    pub token_hash: String,
    pub capabilities: Option<String>,
    pub tags: Option<String>,
    pub remark: Option<String>,
    pub disabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct RemoteCommand {
    pub id: String,
    pub agent_id: i64,
    pub kind: String,
    pub payload: String,
    pub status: String,
    pub exit_code: Option<i64>,
    pub output: Option<String>,
    pub error: Option<String>,
    pub timeout: Option<i64>,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct RemoteCommandLog {
    pub id: i64,
    pub command_id: String,
    pub stream: String,
    pub line: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterAgentRequest {
    pub name: String,
    pub hostname: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub version: Option<String>,
    pub capabilities: Option<serde_json::Value>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct RegisterAgentResponse {
    pub agent_id: i64,
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateRemoteAgentRequest {
    pub name: String,
    pub remark: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateRemoteAgentResponse {
    pub agent: RemoteAgent,
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateRemoteCommandRequest {
    pub kind: Option<String>,
    pub command: String,
    pub working_dir: Option<String>,
    pub env: Option<std::collections::HashMap<String, String>>,
    pub timeout: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RunRemoteScriptRequest {
    pub path: String,
    pub command: Option<String>,
    pub working_dir: Option<String>,
    pub env: Option<std::collections::HashMap<String, String>>,
    pub timeout: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RemoteWriteFileRequest {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct RemotePathRequest {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct RemoteMoveFileRequest {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RemoteFileEntry {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
    pub size: Option<u64>,
    pub modified: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RemoteServerMessage {
    #[serde(rename = "command.start")]
    CommandStart {
        command_id: String,
        command: String,
        working_dir: Option<String>,
        env: Option<std::collections::HashMap<String, String>>,
        timeout: Option<i64>,
        script_content: Option<String>,
        script_name: Option<String>,
    },
    #[serde(rename = "command.kill")]
    CommandKill { command_id: String },
    #[serde(rename = "file.list")]
    FileList { request_id: String, path: String },
    #[serde(rename = "file.read")]
    FileRead { request_id: String, path: String },
    #[serde(rename = "file.write")]
    FileWrite { request_id: String, path: String, content: String },
    #[serde(rename = "file.delete")]
    FileDelete { request_id: String, path: String },
    #[serde(rename = "file.mkdir")]
    FileMkdir { request_id: String, path: String },
    #[serde(rename = "file.rename")]
    FileRename { request_id: String, from: String, to: String },
    #[serde(rename = "terminal.open")]
    TerminalOpen {
        terminal_id: String,
        rows: u16,
        cols: u16,
    },
    #[serde(rename = "terminal.input")]
    TerminalInput { terminal_id: String, data: String },
    #[serde(rename = "terminal.resize")]
    TerminalResize {
        terminal_id: String,
        rows: u16,
        cols: u16,
    },
    #[serde(rename = "terminal.close")]
    TerminalClose { terminal_id: String },
    #[serde(rename = "status.request")]
    StatusRequest { request_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RemoteAgentMessage {
    #[serde(rename = "agent.hello")]
    Hello {
        hostname: Option<String>,
        os: Option<String>,
        arch: Option<String>,
        version: Option<String>,
        capabilities: Option<serde_json::Value>,
    },
    #[serde(rename = "agent.heartbeat")]
    Heartbeat,
    #[serde(rename = "agent.status")]
    Status {
        request_id: Option<String>,
        metrics: serde_json::Value,
    },
    #[serde(rename = "command.started")]
    CommandStarted { command_id: String },
    #[serde(rename = "command.output")]
    CommandOutput {
        command_id: String,
        stream: String,
        line: String,
    },
    #[serde(rename = "command.finished")]
    CommandFinished {
        command_id: String,
        status: String,
        exit_code: Option<i64>,
        error: Option<String>,
        duration_ms: Option<i64>,
    },
    #[serde(rename = "file.list.result")]
    FileListResult {
        request_id: String,
        entries: serde_json::Value,
        error: Option<String>,
    },
    #[serde(rename = "file.read.result")]
    FileReadResult {
        request_id: String,
        content: Option<String>,
        error: Option<String>,
    },
    #[serde(rename = "file.action.result")]
    FileActionResult {
        request_id: String,
        success: bool,
        error: Option<String>,
    },
    #[serde(rename = "terminal.opened")]
    TerminalOpened { terminal_id: String },
    #[serde(rename = "terminal.output")]
    TerminalOutput { terminal_id: String, data: String },
    #[serde(rename = "terminal.closed")]
    TerminalClosed {
        terminal_id: String,
        error: Option<String>,
    },
}
