use anyhow::{anyhow, Result};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[derive(Debug, Deserialize)]
struct RegisterResponse {
    agent_id: i64,
    token: String,
}

#[derive(Debug, Serialize)]
struct RegisterRequest {
    name: String,
    hostname: Option<String>,
    os: String,
    arch: String,
    version: String,
    capabilities: serde_json::Value,
    tags: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AgentCredentials {
    agent_id: i64,
    token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
enum ServerMessage {
    #[serde(rename = "command.start")]
    CommandStart {
        command_id: String,
        command: String,
        working_dir: Option<String>,
        env: Option<HashMap<String, String>>,
        timeout: Option<i64>,
        script_content: Option<String>,
        script_name: Option<String>,
    },
    #[serde(rename = "command.kill")]
    CommandKill { command_id: String },
    #[serde(rename = "status.request")]
    StatusRequest { request_id: String },
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
enum AgentMessage {
    #[serde(rename = "agent.hello")]
    Hello {
        hostname: Option<String>,
        os: String,
        arch: String,
        version: String,
        capabilities: serde_json::Value,
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
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    apply_allowed_roots_arg(&args);
    match args.get(1).map(|s| s.as_str()) {
        Some("register") => register(&args).await,
        Some("start") => start(&args).await,
        Some("run") => run(&args).await,
        _ => {
            eprintln!(
                "Usage:\n  zhuque-agent start --server http://127.0.0.1:3000 --agent-id <id> --token <agent-token> [--config zhuque-agent.json] [--allowed-roots <paths>]\n  zhuque-agent register --server http://127.0.0.1:3000 --token <register-token> --name <name>\n  zhuque-agent run --server http://127.0.0.1:3000 --agent-id <id> --token <agent-token> [--allowed-roots <paths>]"
            );
            Ok(())
        }
    }
}

async fn register(args: &[String]) -> Result<()> {
    let data = register_agent(args).await?;
    println!("agent_id={}", data.agent_id);
    println!("token={}", data.token);
    Ok(())
}

async fn start(args: &[String]) -> Result<()> {
    let server = arg(args, "--server")?;
    let config_path = arg_optional(args, "--config").unwrap_or_else(|| "zhuque-agent.json".to_string());

    if let Ok(credentials) = load_credentials(&config_path).await {
        println!("using saved agent_id={}", credentials.agent_id);
        return run_agent(&server, credentials.agent_id, &credentials.token).await;
    }

    if let Some(agent_id_raw) = arg_optional(args, "--agent-id") {
        let token = arg(args, "--token")?;
        let agent_id = parse_agent_id(&agent_id_raw)?;
        let credentials = AgentCredentials { agent_id, token };
        save_credentials(&config_path, &credentials).await?;
        println!("saved credentials to {}", config_path);
        return run_agent(&server, credentials.agent_id, &credentials.token).await;
    }

    let data = register_agent(args).await?;
    let credentials = AgentCredentials {
        agent_id: data.agent_id,
        token: data.token,
    };
    save_credentials(&config_path, &credentials).await?;
    println!("registered agent_id={}", data.agent_id);
    println!("saved credentials to {}", config_path);
    run_agent(&server, credentials.agent_id, &credentials.token).await
}

async fn register_agent(args: &[String]) -> Result<RegisterResponse> {
    let server = arg(args, "--server")?;
    let token = arg(args, "--token")?;
    let name = arg(args, "--name").unwrap_or_else(|_| hostname());
    let payload = RegisterRequest {
        name,
        hostname: Some(hostname()),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        capabilities: serde_json::json!({
            "command": true,
            "status": true,
            "file": false,
            "terminal": false
        }),
        tags: Vec::new(),
    };

    let url = format!("{}/api/remote/agent/register", server.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .post(url)
        .header("x-register-token", token)
        .json(&payload)
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(anyhow!("register failed: {}", resp.text().await?));
    }

    Ok(resp.json().await?)
}

async fn run(args: &[String]) -> Result<()> {
    let server = arg(args, "--server")?;
    let agent_id_raw = arg(args, "--agent-id")?;
    let agent_id = parse_agent_id(&agent_id_raw)?;
    let token = arg(args, "--token")?;
    run_agent(&server, agent_id, &token).await
}

async fn run_agent(server: &str, agent_id: i64, token: &str) -> Result<()> {
    let ws_url = ws_url(&server, agent_id, &token);
    let (socket, _) = connect_async(ws_url).await?;
    let (writer, mut reader) = socket.split();
    let writer = Arc::new(Mutex::new(writer));
    let running_commands: Arc<Mutex<HashMap<String, u32>>> = Arc::new(Mutex::new(HashMap::new()));

    send(
        &writer,
        AgentMessage::Hello {
            hostname: Some(hostname()),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            capabilities: serde_json::json!({
                "command": true,
                "status": true,
                "file": false,
                "terminal": false
            }),
        },
    )
    .await?;

    let heartbeat_writer = writer.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(20));
        loop {
            interval.tick().await;
            if send(&heartbeat_writer, AgentMessage::Heartbeat).await.is_err() {
                break;
            }
        }
    });

    while let Some(message) = reader.next().await {
        match message? {
            Message::Text(text) => {
                let msg: ServerMessage = serde_json::from_str(&text)?;
                match msg {
                    ServerMessage::CommandStart {
                        command_id,
                        command,
                        working_dir,
                        env,
                        timeout,
                        script_content,
                        script_name,
                    } => {
                        let command_writer = writer.clone();
                        let command_processes = running_commands.clone();
                        tokio::spawn(async move {
                            let _ = execute_command(
                                command_writer,
                                command_processes,
                                command_id,
                                command,
                                working_dir,
                                env.unwrap_or_default(),
                                timeout.unwrap_or(0),
                                script_content,
                                script_name,
                            )
                            .await;
                        });
                    }
                    ServerMessage::StatusRequest { request_id } => {
                        send(
                            &writer,
                            AgentMessage::Status {
                                request_id: Some(request_id),
                                metrics: metrics(),
                            },
                        )
                        .await?;
                    }
                    ServerMessage::FileList { request_id, path } => {
                        let result = list_files(&path).await;
                        let (entries, error) = match result {
                            Ok(entries) => (entries, None),
                            Err(e) => (serde_json::json!([]), Some(e.to_string())),
                        };
                        send(
                            &writer,
                            AgentMessage::FileListResult {
                                request_id,
                                entries,
                                error,
                            },
                        )
                        .await?;
                    }
                    ServerMessage::FileRead { request_id, path } => {
                        let result = read_file(&path).await;
                        let (content, error) = match result {
                            Ok(content) => (Some(content), None),
                            Err(e) => (None, Some(e.to_string())),
                        };
                        send(
                            &writer,
                            AgentMessage::FileReadResult {
                                request_id,
                                content,
                                error,
                            },
                        )
                        .await?;
                    }
                    ServerMessage::FileWrite { request_id, path, content } => {
                        let error = write_file(&path, &content).await.err().map(|e| e.to_string());
                        send(
                            &writer,
                            AgentMessage::FileActionResult {
                                request_id,
                                success: error.is_none(),
                                error,
                            },
                        )
                        .await?;
                    }
                    ServerMessage::FileDelete { request_id, path } => {
                        let error = delete_file(&path).await.err().map(|e| e.to_string());
                        send(
                            &writer,
                            AgentMessage::FileActionResult {
                                request_id,
                                success: error.is_none(),
                                error,
                            },
                        )
                        .await?;
                    }
                    ServerMessage::FileMkdir { request_id, path } => {
                        let error = create_dir(&path).await.err().map(|e| e.to_string());
                        send(
                            &writer,
                            AgentMessage::FileActionResult {
                                request_id,
                                success: error.is_none(),
                                error,
                            },
                        )
                        .await?;
                    }
                    ServerMessage::FileRename { request_id, from, to } => {
                        let error = rename_file(&from, &to).await.err().map(|e| e.to_string());
                        send(
                            &writer,
                            AgentMessage::FileActionResult {
                                request_id,
                                success: error.is_none(),
                                error,
                            },
                        )
                        .await?;
                    }
                    ServerMessage::CommandKill { command_id } => {
                        if let Some(pid) = running_commands.lock().await.remove(&command_id) {
                            let _ = kill_process(pid).await;
                        }
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    Ok(())
}

async fn execute_command<W>(
    writer: Arc<Mutex<W>>,
    running_commands: Arc<Mutex<HashMap<String, u32>>>,
    command_id: String,
    command: String,
    working_dir: Option<String>,
    env: HashMap<String, String>,
    timeout: i64,
    script_content: Option<String>,
    script_name: Option<String>,
) -> Result<()>
where
    W: SinkExt<Message> + Unpin + Send + 'static,
    <W as futures::Sink<Message>>::Error: std::fmt::Debug,
{
    let started = std::time::Instant::now();
    send(
        &writer,
        AgentMessage::CommandStarted {
            command_id: command_id.clone(),
        },
    )
    .await?;

    let mut temp_script_path = None;
    let effective_command = if let Some(content) = script_content {
        let script_name = script_name.unwrap_or_else(|| "remote_script.sh".to_string());
        let extension = std::path::Path::new(&script_name)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if cfg!(windows)
            && matches!(extension.as_str(), "cmd" | "bat")
            && (command.trim().is_empty() || command.trim() == "{script}")
        {
            content
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join(" & ")
        } else {
            let temp_dir = std::env::current_dir()?;
            let path = temp_dir.join(format!("zhuque-{}-{}", command_id, sanitize_name(&script_name)));
            tokio::fs::write(&path, content).await?;
            temp_script_path = Some(path.clone());
            if command.trim().is_empty() || command.trim() == "{script}" {
                default_script_command(&path)
            } else {
                command.replace("{script}", &quote_path(&path))
            }
        }
    } else {
        command
    };

    let mut cmd = shell_command(&effective_command);
    send(
        &writer,
        AgentMessage::CommandOutput {
            command_id: command_id.clone(),
            stream: "system".to_string(),
            line: format!("[command] {}", effective_command),
        },
    )
    .await?;
    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }
    cmd.envs(env).stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            send(
                &writer,
                AgentMessage::CommandFinished {
                    command_id,
                    status: "failed".to_string(),
                    exit_code: None,
                    error: Some(e.to_string()),
                    duration_ms: Some(started.elapsed().as_millis() as i64),
                },
            )
            .await?;
            return Ok(());
        }
    };
    if let Some(pid) = child.id() {
        running_commands
            .lock()
            .await
            .insert(command_id.clone(), pid);
    }

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let mut io_tasks = Vec::new();

    if let Some(stdout) = stdout {
        let stdout_writer = writer.clone();
        let stdout_command_id = command_id.clone();
        io_tasks.push(tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = send(
                    &stdout_writer,
                    AgentMessage::CommandOutput {
                        command_id: stdout_command_id.clone(),
                        stream: "stdout".to_string(),
                        line,
                    },
                )
                .await;
            }
        }));
    }

    if let Some(stderr) = stderr {
        let stderr_writer = writer.clone();
        let stderr_command_id = command_id.clone();
        io_tasks.push(tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = send(
                    &stderr_writer,
                    AgentMessage::CommandOutput {
                        command_id: stderr_command_id.clone(),
                        stream: "stderr".to_string(),
                        line,
                    },
                )
                .await;
            }
        }));
    }

    let status = if timeout > 0 {
        match tokio::time::timeout(std::time::Duration::from_secs(timeout as u64), child.wait()).await {
            Ok(result) => result?,
            Err(_) => {
                let _ = child.kill().await;
                running_commands.lock().await.remove(&command_id);
                send(
                    &writer,
                    AgentMessage::CommandFinished {
                        command_id,
                        status: "timeout".to_string(),
                        exit_code: None,
                        error: Some(format!("timeout after {}s", timeout)),
                        duration_ms: Some(started.elapsed().as_millis() as i64),
                    },
                )
                .await?;
                return Ok(());
            }
        }
    } else {
        child.wait().await?
    };
    running_commands.lock().await.remove(&command_id);
    for task in io_tasks {
        let _ = task.await;
    }

    let success = status.success();
    send(
        &writer,
        AgentMessage::CommandFinished {
            command_id,
            status: if success { "success" } else { "failed" }.to_string(),
            exit_code: status.code().map(|c| c as i64),
            error: None,
            duration_ms: Some(started.elapsed().as_millis() as i64),
        },
    )
    .await?;

    if let Some(path) = temp_script_path {
        let _ = tokio::fs::remove_file(path).await;
    }

    Ok(())
}

async fn send<W>(writer: &Arc<Mutex<W>>, message: AgentMessage) -> Result<()>
where
    W: SinkExt<Message> + Unpin + Send,
    <W as futures::Sink<Message>>::Error: std::fmt::Debug,
{
    let text = serde_json::to_string(&message)?;
    writer
        .lock()
        .await
        .send(Message::Text(text))
        .await
        .map_err(|e| anyhow!("websocket send failed: {:?}", e))
}

fn shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("cmd");
        let trimmed = command.trim();
        let command = trimmed
            .strip_prefix("cmd /C ")
            .or_else(|| trimmed.strip_prefix("cmd /c "))
            .unwrap_or(trimmed);
        cmd.arg("/C").arg(command);
        cmd
    }

    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd
    }
}

async fn kill_process(pid: u32) -> Result<()> {
    #[cfg(windows)]
    {
        let status = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status()
            .await?;
        if status.success() {
            Ok(())
        } else {
            Err(anyhow!("taskkill failed for pid {}", pid))
        }
    }

    #[cfg(not(windows))]
    {
        let status = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .await?;
        if status.success() {
            Ok(())
        } else {
            Err(anyhow!("kill failed for pid {}", pid))
        }
    }
}

fn arg(args: &[String], name: &str) -> Result<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
        .ok_or_else(|| anyhow!("missing argument {}", name))
}

fn arg_optional(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}

async fn load_credentials(path: &str) -> Result<AgentCredentials> {
    let content = tokio::fs::read_to_string(path).await?;
    Ok(serde_json::from_str(&content)?)
}

async fn save_credentials(path: &str, credentials: &AgentCredentials) -> Result<()> {
    let content = serde_json::to_string_pretty(&credentials)?;
    tokio::fs::write(path, content).await?;
    Ok(())
}

fn parse_agent_id(value: &str) -> Result<i64> {
    value
        .parse::<i64>()
        .map_err(|_| anyhow!("invalid --agent-id '{}': create a machine in the web console and copy the generated command", value))
}

fn ws_url(server: &str, agent_id: i64, token: &str) -> String {
    let base = server.trim_end_matches('/');
    let base = if base.starts_with("https://") {
        base.replacen("https://", "wss://", 1)
    } else if base.starts_with("http://") {
        base.replacen("http://", "ws://", 1)
    } else {
        format!("ws://{}", base)
    };
    format!(
        "{}/api/remote/agent/connect?agent_id={}&token={}",
        base, agent_id, token
    )
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "zhuque-agent".to_string())
}

fn metrics() -> serde_json::Value {
    let mut sys = sysinfo::System::new_all();
    sys.refresh_all();
    serde_json::json!({
        "cpu_usage": sys.global_cpu_info().cpu_usage(),
        "memory_total": sys.total_memory(),
        "memory_available": sys.available_memory(),
        "uptime_seconds": sysinfo::System::uptime()
    })
}

fn default_script_command(path: &std::path::Path) -> String {
    let quoted = quote_path(path);
    match path.extension().and_then(|s| s.to_str()).unwrap_or_default() {
        "py" => format!("python {}", quoted),
        "js" => format!("node {}", quoted),
        "ts" => format!("bun {}", quoted),
        "ps1" => format!("powershell -ExecutionPolicy Bypass -File {}", quoted),
        _ => {
            if cfg!(windows) {
                format!("call {}", quoted)
            } else {
                format!("sh {}", quoted)
            }
        }
    }
}

fn quote_path(path: &std::path::Path) -> String {
    let raw = path.to_string_lossy();
    if cfg!(windows) {
        format!("\"{}\"", raw.replace('"', "\\\""))
    } else {
        format!("'{}'", raw.replace('\'', "'\\''"))
    }
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

async fn list_files(path: &str) -> Result<serde_json::Value> {
    let path = resolve_allowed_path(path)?;
    let mut entries = Vec::new();
    let mut dir = tokio::fs::read_dir(&path).await?;
    while let Some(entry) = dir.next_entry().await? {
        let metadata = entry.metadata().await?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs().to_string());
        entries.push(serde_json::json!({
            "name": entry.file_name().to_string_lossy(),
            "path": display_path(&entry.path()),
            "is_directory": metadata.is_dir(),
            "size": if metadata.is_file() { Some(metadata.len()) } else { None },
            "modified": modified
        }));
    }
    Ok(serde_json::json!(entries))
}

async fn read_file(path: &str) -> Result<String> {
    let path = resolve_allowed_path(path)?;
    let metadata = tokio::fs::metadata(&path).await?;
    if !metadata.is_file() {
        return Err(anyhow!("path is not a file"));
    }
    if metadata.len() > 10 * 1024 * 1024 {
        return Err(anyhow!("file is larger than 10MB"));
    }
    Ok(tokio::fs::read_to_string(path).await?)
}

async fn write_file(path: &str, content: &str) -> Result<()> {
    let path = resolve_allowed_target_path(path)?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(path, content).await?;
    Ok(())
}

async fn delete_file(path: &str) -> Result<()> {
    let path = resolve_allowed_path(path)?;
    let metadata = tokio::fs::metadata(&path).await?;
    if metadata.is_dir() {
        tokio::fs::remove_dir_all(path).await?;
    } else {
        tokio::fs::remove_file(path).await?;
    }
    Ok(())
}

async fn create_dir(path: &str) -> Result<()> {
    let path = resolve_allowed_target_path(path)?;
    tokio::fs::create_dir_all(path).await?;
    Ok(())
}

async fn rename_file(from: &str, to: &str) -> Result<()> {
    let from = resolve_allowed_path(from)?;
    let to = resolve_allowed_target_path(to)?;
    if let Some(parent) = to.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::rename(from, to).await?;
    Ok(())
}

fn resolve_allowed_path(path: &str) -> Result<std::path::PathBuf> {
    let requested = std::path::PathBuf::from(path);
    let full_path = if requested.is_absolute() {
        requested
    } else {
        std::env::current_dir()?.join(requested)
    };
    let full_path = full_path.canonicalize()?;

    let roots = allowed_roots()?;
    if roots.iter().any(|root| full_path.starts_with(root)) {
        Ok(full_path)
    } else {
        let allowed = roots
            .iter()
            .map(|root| display_path(root))
            .collect::<Vec<_>>()
            .join("; ");
        Err(anyhow!(
            "path is outside allowed roots: {}. allowed roots: {}",
            display_path(&full_path),
            allowed
        ))
    }
}

fn resolve_allowed_target_path(path: &str) -> Result<std::path::PathBuf> {
    let requested = std::path::PathBuf::from(path);
    let full_path = if requested.is_absolute() {
        requested
    } else {
        std::env::current_dir()?.join(requested)
    };
    let parent = full_path
        .parent()
        .ok_or_else(|| anyhow!("invalid target path"))?
        .canonicalize()?;
    let file_name = full_path
        .file_name()
        .ok_or_else(|| anyhow!("invalid target path"))?;
    let target = parent.join(file_name);
    let roots = allowed_roots()?;
    if roots.iter().any(|root| target.starts_with(root)) {
        Ok(target)
    } else {
        let allowed = roots
            .iter()
            .map(|root| display_path(root))
            .collect::<Vec<_>>()
            .join("; ");
        Err(anyhow!(
            "path is outside allowed roots: {}. allowed roots: {}",
            display_path(&target),
            allowed
        ))
    }
}

fn allowed_roots() -> Result<Vec<std::path::PathBuf>> {
    let roots = std::env::var("ZHUQUE_AGENT_ALLOWED_ROOTS").unwrap_or_else(|_| ".".to_string());
    roots
        .split(';')
        .filter(|s| !s.trim().is_empty())
        .map(|root| {
            let path = std::path::PathBuf::from(root.trim());
            let path = if path.is_absolute() {
                path
            } else {
                std::env::current_dir()?.join(path)
            };
            Ok(path.canonicalize()?)
        })
        .collect()
}

fn apply_allowed_roots_arg(args: &[String]) {
    if let Some(roots) = arg_optional(args, "--allowed-roots") {
        std::env::set_var("ZHUQUE_AGENT_ALLOWED_ROOTS", roots);
    }
}

fn display_path(path: &std::path::Path) -> String {
    clean_windows_verbatim_path(&path.to_string_lossy())
}

fn clean_windows_verbatim_path(path: &str) -> String {
    path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
}
