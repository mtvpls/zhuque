use crate::api::AppState;
use crate::models::AiConfig;
use axum::{
    extract::State,
    http::StatusCode,
    response::{sse::{Event, KeepAlive, Sse}, IntoResponse},
    Json,
};
use futures::{stream::Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{convert::Infallible, process::Stdio, sync::Arc, time::Duration};
use tokio::process::Command;
use tokio::time::timeout;

#[derive(Debug, Deserialize)]
pub struct AiChatRequest {
    pub mode: String,
    pub prompt: String,
    pub file_name: Option<String>,
    pub file_path: Option<String>,
    pub file_content: Option<String>,
    pub execution_output: Option<String>,
    pub directory_path: Option<String>,
    #[serde(default)]
    pub history: Vec<AiHistoryMessage>,
    #[serde(default)]
    pub allow_commands: bool,
}

#[derive(Debug, Deserialize)]
pub struct AiHistoryMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    stream: bool,
    messages: Vec<ChatMessage<'a>>,
}

const MASKED_API_KEY: &str = "********";

#[derive(Debug, Serialize)]
struct AiConfigResponse {
    enabled: bool,
    provider: String,
    base_url: String,
    api_key: String,
    model: String,
}

impl From<AiConfig> for AiConfigResponse {
    fn from(config: AiConfig) -> Self {
        Self {
            enabled: config.enabled,
            provider: config.provider,
            base_url: config.base_url,
            api_key: if config.api_key.is_empty() {
                String::new()
            } else {
                MASKED_API_KEY.to_string()
            },
            model: config.model,
        }
    }
}

#[derive(Debug, Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: String,
}

pub async fn chat(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AiChatRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)> {
    let config = state
        .config_service
        .get_ai_config()
        .await
        .map_err(internal_error)?;

    if !config.enabled || config.api_key.trim().is_empty() || config.model.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "AI 尚未配置，请先在系统配置中填写 Provider、API Key 和模型".into()));
    }

    if request.prompt.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "请求内容不能为空".into()));
    }

    let base_url = config.base_url.trim_end_matches('/');
    let endpoint = if base_url.ends_with("/chat/completions") {
        base_url.to_string()
    } else {
        format!("{base_url}/chat/completions")
    };

    let system = if request.mode == "implement" {
        "你是 Zhuque 的脚本修改 Agent。只能修改用户提供的当前文件，不能猜测其他文件内容。\n\
必须只返回一个合法 JSON 对象，不要 Markdown、代码围栏或额外文字：\n\
{\"summary\":\"简短说明\",\"files\":[{\"path\":\"当前文件路径\",\"operation\":\"update\",\"content\":\"完整的新文件内容\"}]}\n\
如果不需要修改，files 返回空数组。不要输出 API Key、环境变量值或其他凭据。"
    } else {
        "你是 Zhuque 的脚本开发 Agent。你只能基于用户提供的工作区上下文回答。\n\
模式：ask 只分析；plan 只给出计划。\n\
不要猜测未提供的文件内容，不要输出 API Key、环境变量值或其他凭据。"
    };
    let context = format!(
        "模式: {}\n文件名: {}\n路径: {}\n脚本内容:\n{}\n\n最近执行输出:\n{}\n\n用户请求:\n{}",
        request.mode,
        request.file_name.as_deref().unwrap_or("未选择文件"),
        request.file_path.as_deref().unwrap_or(""),
        request.file_content.as_deref().unwrap_or("未提供"),
        request.execution_output.as_deref().unwrap_or("未提供"),
        request.prompt,
    );
    let mut messages = vec![ChatMessage {
        role: "system",
        content: system.to_string(),
    }];

    // 保留最近几轮对话，支持连续追问，同时限制上下文体积。
    for item in request.history.iter().rev().take(12).collect::<Vec<_>>().into_iter().rev() {
        if (item.role == "user" || item.role == "assistant") && !item.content.trim().is_empty() {
            messages.push(ChatMessage {
                role: if item.role == "user" { "user" } else { "assistant" },
                content: item.content.chars().take(12000).collect(),
            });
        }
    }
    messages.push(ChatMessage {
        role: "user",
        content: context,
    });

    let payload = ChatRequest {
        model: &config.model,
        stream: true,
        messages,
    };

    let client = Client::builder()
        .build()
        .map_err(internal_error)?;
    let response = client
        .post(endpoint)
        .bearer_auth(config.api_key)
        .json(&payload)
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("AI 请求失败: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err((StatusCode::BAD_GATEWAY, format!("AI 服务返回 {status}: {body}")));
    }

    let stream = async_stream::stream! {
        let mut bytes = response.bytes_stream();
        let mut buffer = String::new();
        while let Some(chunk) = bytes.next().await {
            match chunk {
                Ok(chunk) => {
                    buffer.push_str(&String::from_utf8_lossy(&chunk));
                    while let Some(index) = buffer.find('\n') {
                        let line = buffer[..index].trim_end_matches('\r').to_string();
                        buffer.drain(..=index);
                        if let Some(data) = line.strip_prefix("data:") {
                            let data = data.trim();
                            if !data.is_empty() && data != "[DONE]" {
                                yield Ok(Event::default().data(data));
                            }
                        }
                    }
                }
                Err(error) => {
                    yield Ok(Event::default().event("error").data(error.to_string()));
                    break;
                }
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}


#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentFileChange {
    path: String,
    operation: String,
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentResult {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    files: Vec<AgentFileChange>,
}

fn agent_tools(allow_commands: bool) -> Value {
    let mut tools = vec![
        json!({
            "type": "function",
            "function": {
                "name": "list_dir",
                "description": "列出脚本工作区内指定目录的直接子项。path 为空表示工作区根目录。",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string" } }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "读取脚本工作区内一个文本文件。",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "search_files",
                "description": "在脚本工作区内递归搜索文本，返回匹配文件和行号。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "path": { "type": "string" }
                    },
                    "required": ["query"]
                }
            }
        }),
    ];

    if allow_commands {
        tools.push(json!({
            "type": "function",
            "function": {
                "name": "run_script",
                "description": "执行工作区内的一个脚本并返回输出。",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }
            }
        }));
        tools.push(json!({
            "type": "function",
            "function": {
                "name": "run_command",
                "description": "在脚本工作区根目录执行一条 shell 命令。默认不设超时；如需限制本次命令，可传 timeout_secs（秒），传 0 或省略表示不限制。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string" },
                        "timeout_secs": { "type": "integer", "minimum": 0, "description": "本次命令的超时时间（秒），0 或省略表示不限制。" }
                    },
                    "required": ["command"]
                }
            }
        }));
    }

    Value::Array(tools)
}

fn argument_string(arguments: &Value, name: &str) -> Result<String, String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("缺少工具参数: {name}"))
}

fn scope_agent_path(directory: Option<&str>, path: &str) -> String {
    let base = directory.unwrap_or("").trim_matches(['/', '\\']);
    let requested = path.trim().trim_matches(['/', '\\']);
    if base.is_empty() {
        return requested.to_string();
    }
    if requested.is_empty() {
        return base.to_string();
    }
    let base_prefix = format!("{base}/");
    if requested == base || requested.starts_with(&base_prefix) || requested.starts_with(&format!("{base}\\")) {
        requested.replace('\\', "/")
    } else {
        format!("{base}/{requested}")
    }
}


async fn search_workspace(
    service: &crate::services::ScriptService,
    root: &str,
    query: &str,
) -> Result<Value, String> {
    if query.trim().is_empty() {
        return Err("搜索内容不能为空".to_string());
    }

    let mut pending = vec![(root.to_string(), 0usize)];
    let mut matches = Vec::new();
    let mut visited = 0usize;

    while let Some((directory, depth)) = pending.pop() {
        if depth > 8 || visited >= 1000 {
            break;
        }
        visited += 1;
        let entries = service.list_dir(&directory).await.map_err(|e| e.to_string())?;
        for entry in entries {
            if entry.is_directory {
                pending.push((entry.path, depth + 1));
                continue;
            }
            if matches.len() >= 200 {
                break;
            }
            let content = match service.read(&entry.path).await {
                Ok(value) => value,
                Err(_) => continue,
            };
            let mut lines = Vec::new();
            for (line_number, line) in content.lines().enumerate() {
                if line.contains(query) {
                    lines.push(json!({
                        "line": line_number + 1,
                        "text": line.chars().take(300).collect::<String>(),
                    }));
                    if lines.len() >= 5 {
                        break;
                    }
                }
            }
            if !lines.is_empty() {
                matches.push(json!({ "path": entry.path, "matches": lines }));
            }
        }
    }

    Ok(json!({
        "matches": matches,
        "truncated": matches.len() >= 200 || visited >= 1000,
    }))
}

async fn run_workspace_command(
    service: &crate::services::ScriptService,
    command: &str,
    timeout_secs: Option<u64>,
    working_directory: Option<&str>,
) -> Result<Value, String> {
    if command.trim().is_empty() {
        return Err("命令不能为空".to_string());
    }

    let working_directory = working_directory.unwrap_or("");
    service
        .list_dir(working_directory)
        .await
        .map_err(|error| format!("附加目录无效: {error}"))?;

    let mut process = Command::new(if cfg!(windows) { "cmd" } else { "sh" });
    if cfg!(windows) {
        process.args(["/C", command]);
    } else {
        process.args(["-lc", command]);
    }
    process
        .current_dir(service.get_full_path(working_directory))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let output = match timeout_secs.filter(|seconds| *seconds > 0) {
        Some(seconds) => timeout(Duration::from_secs(seconds), process.output())
            .await
            .map_err(|_| "命令执行超时".to_string())?
            .map_err(|e| format!("命令执行失败: {e}"))?,
        None => process
            .output()
            .await
            .map_err(|e| format!("命令执行失败: {e}"))?,
    };

    let stdout = String::from_utf8_lossy(&output.stdout).chars().take(120000).collect::<String>();
    let stderr = String::from_utf8_lossy(&output.stderr).chars().take(120000).collect::<String>();
    Ok(json!({
        "success": output.status.success(),
        "code": output.status.code(),
        "stdout": stdout,
        "stderr": stderr,
    }))
}

async fn execute_agent_tool(
    state: &AppState,
    name: &str,
    arguments: &Value,
    allow_commands: bool,
    attached_directory: Option<&str>,
) -> Result<Value, String> {
    match name {
        "list_dir" => {
            let path = arguments.get("path").and_then(Value::as_str).unwrap_or("");
            let path = scope_agent_path(attached_directory, path);
            let entries = state.script_service.list_dir(&path).await.map_err(|e| e.to_string())?;
            serde_json::to_value(entries).map_err(|e| e.to_string())
        }
        "read_file" => {
            let path = argument_string(arguments, "path")?;
            let path = scope_agent_path(attached_directory, &path);
            let content = state.script_service.read(&path).await.map_err(|e| e.to_string())?;
            Ok(json!({
                "path": path,
                "content": content.chars().take(160000).collect::<String>(),
                "truncated": content.chars().count() > 160000,
            }))
        }
        "search_files" => {
            let query = argument_string(arguments, "query")?;
            let path = arguments.get("path").and_then(Value::as_str).unwrap_or("");
            let path = scope_agent_path(attached_directory, path);
            search_workspace(&state.script_service, &path, &query).await
        }
        "run_script" if allow_commands => {
            let path = argument_string(arguments, "path")?;
            let path = scope_agent_path(attached_directory, &path);
            let (execution_id, stream) = state
                .script_service
                .execute_script(&path, None)
                .await
                .map_err(|e| e.to_string())?;
            let mut stream = Box::pin(stream);
            let mut output = String::new();
            while let Some(line) = stream.next().await {
                match line {
                    Ok(value) => {
                        output.push_str(&value);
                        output.push('\n');
                        if output.chars().count() > 120000 {
                            break;
                        }
                    }
                    Err(error) => {
                        output.push_str("[ERROR] ");
                        output.push_str(&error.to_string());
                        output.push('\n');
                        break;
                    }
                }
            }
            Ok(json!({
                "execution_id": execution_id,
                "output": output.chars().take(120000).collect::<String>(),
            }))
        }
        "run_command" if allow_commands => {
            let command = argument_string(arguments, "command")?;
            let timeout_secs = arguments.get("timeout_secs").and_then(Value::as_u64);
            run_workspace_command(&state.script_service, &command, timeout_secs, attached_directory).await
        }
        "run_script" | "run_command" => Err("用户未允许执行命令或脚本".to_string()),
        _ => Err(format!("未知工具: {name}")),
    }
}

fn parse_agent_result(content: &str) -> Option<AgentResult> {
    let trimmed = content.trim();
    let candidate = if let Some(start) = trimmed.find("```") {
        let after_start = &trimmed[start + 3..];
        let body = after_start
            .strip_prefix("json")
            .or_else(|| after_start.strip_prefix("JSON"))
            .unwrap_or(after_start);
        body.split("```").next().unwrap_or(body).trim().to_string()
    } else if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if start <= end {
            trimmed[start..=end].to_string()
        } else {
            trimmed.to_string()
        }
    } else {
        trimmed.to_string()
    };
    serde_json::from_str(&candidate).ok()
}


pub async fn agent(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AiChatRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)> {
    let config = state.config_service.get_ai_config().await.map_err(internal_error)?;
    if !config.enabled || config.api_key.trim().is_empty() || config.model.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "AI 尚未配置，请先在系统配置中填写 Provider、API Key 和模型".into()));
    }
    if request.prompt.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "请求内容不能为空".into()));
    }

    let base_url = config.base_url.trim_end_matches('/');
    let endpoint = if base_url.ends_with("/chat/completions") {
        base_url.to_string()
    } else {
        format!("{base_url}/chat/completions")
    };
    let client = Client::builder()
        .build()
        .map_err(internal_error)?;
    let state = state.clone();

    let stream = async_stream::stream! {
        let system = format!(
            "你是 Zhuque 的脚本工作区 Agent。你可以浏览目录、读取文件、搜索代码{}。先使用工具获取必要上下文，不要猜测文件内容。当用户要求修改时，最后必须只返回合法 JSON，不要 Markdown 或额外文字。JSON 格式：{{\"summary\":\"简短说明\",\"files\":[{{\"path\":\"工作区相对路径\",\"operation\":\"update|create|delete\",\"content\":\"文件完整内容\"}}]}}。修改多个文件时全部放入 files。所有路径必须是工作区相对路径。附加目录存在时，所有工具路径都必须限制在该目录内；工具 path 为空表示附加目录根目录，禁止回到工作区根目录。{}",
            if request.allow_commands { "，并可执行脚本和命令" } else { "" },
            if request.allow_commands { "执行命令前确认命令不会破坏用户文件。" } else { "当前未授权执行命令或脚本。" },
        );
        let context = format!(
            "模式: {}\n当前文件名: {}\n当前路径: {}\n当前附加目录: {}\n当前文件内容:\n{}\n\n最近执行输出:\n{}\n\n用户请求:\n{}",
            request.mode,
            request.file_name.as_deref().unwrap_or("未选择文件"),
            request.file_path.as_deref().unwrap_or(""),
            request.directory_path.as_deref().unwrap_or("未附加目录"),
            request.file_content.as_deref().unwrap_or("未提供"),
            request.execution_output.as_deref().unwrap_or("未提供"),
            request.prompt,
        );
        let mut messages = vec![json!({"role": "system", "content": system})];
        for item in request.history.iter().rev().take(12).collect::<Vec<_>>().into_iter().rev() {
            if (item.role == "user" || item.role == "assistant") && !item.content.trim().is_empty() {
                messages.push(json!({
                    "role": item.role,
                    "content": item.content.chars().take(12000).collect::<String>(),
                }));
            }
        }
        messages.push(json!({"role": "user", "content": context}));
        let tools = agent_tools(request.allow_commands);
        let mut final_content = String::new();

        loop {
            let response = match call_agent_provider(
                &client,
                &endpoint,
                &config.api_key,
                &config.model,
                &messages,
                &tools,
            ).await {
                Ok(value) => value,
                Err(error) => {
                    yield Ok(Event::default().event("agent").data(json!({"type":"error","message":error}).to_string()));
                    return;
                }
            };

            let tool_calls = response.get("tool_calls").and_then(Value::as_array).cloned().unwrap_or_default();
            messages.push(response.clone());
            if tool_calls.is_empty() {
                final_content = response.get("content").and_then(Value::as_str).unwrap_or("").to_string();
                break;
            }

            for call in tool_calls {
                let call_id = call.get("id").and_then(Value::as_str).unwrap_or("tool-call");
                let function = call.get("function").cloned().unwrap_or(Value::Null);
                let name = function.get("name").and_then(Value::as_str).unwrap_or("");
                let arguments = match function.get("arguments") {
                    Some(Value::String(raw)) => serde_json::from_str::<Value>(raw)
                        .unwrap_or(Value::Object(Default::default())),
                    Some(value @ Value::Object(_)) => value.clone(),
                    _ => Value::Object(Default::default()),
                };

                yield Ok(Event::default().event("agent").data(json!({
                    "type": "tool_call",
                    "tool": name,
                    "arguments": arguments,
                }).to_string()));

                let result = execute_agent_tool(&state, name, &arguments, request.allow_commands, request.directory_path.as_deref()).await;
                let (content, success) = match result {
                    Ok(value) => (value.to_string(), true),
                    Err(error) => (json!({"error": error}).to_string(), false),
                };
                yield Ok(Event::default().event("agent").data(json!({
                    "type": "tool_result",
                    "tool": name,
                    "success": success,
                    "result": content.chars().take(6000).collect::<String>(),
                }).to_string()));
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": content,
                }));
            }
        }

        if let Some(result) = parse_agent_result(&final_content) {
            yield Ok(Event::default().event("agent").data(json!({
                "type": "changes",
                "summary": result.summary,
                "files": result.files,
            }).to_string()));
        } else {
            yield Ok(Event::default().event("agent").data(json!({
                "type": "text",
                "content": final_content,
            }).to_string()));
        }
        yield Ok(Event::default().event("agent").data(json!({"type":"done"}).to_string()));
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn call_agent_provider(
    client: &Client,
    endpoint: &str,
    api_key: &str,
    model: &str,
    messages: &[Value],
    tools: &Value,
) -> Result<Value, String> {
    let payload = json!({
        "model": model,
        "stream": false,
        "messages": messages,
        "tools": tools,
        "tool_choice": "auto",
    });
    let response = client
        .post(endpoint)
        .bearer_auth(api_key)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("AI 请求失败: {e}"))?;
    let status = response.status();
    let body = response.text().await.map_err(|e| format!("读取 AI 响应失败: {e}"))?;
    if !status.is_success() {
        return Err(format!("AI 服务返回 {status}: {body}"));
    }
    let value: Value = serde_json::from_str(&body).map_err(|e| format!("AI 响应不是合法 JSON: {e}"))?;
    value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .cloned()
        .ok_or_else(|| "AI 响应缺少 choices[0].message".to_string())
}

fn internal_error(error: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

pub async fn config(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let value = state.config_service.get_ai_config().await.map_err(internal_error)?;
    Ok(Json(AiConfigResponse::from(value)))
}

pub async fn update_config(
    State(state): State<Arc<AppState>>,
    Json(value): Json<AiConfig>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let mut value = value;
    if value.api_key == MASKED_API_KEY {
        value.api_key = state
            .config_service
            .get_ai_config()
            .await
            .map_err(internal_error)?
            .api_key;
    }
    state.config_service.update_ai_config(&value).await.map_err(internal_error)?;
    Ok(Json(AiConfigResponse::from(value)))
}
