use crate::api::AppState;
use crate::models::{
    AiConfig, Claims, CreateEnvVar, CreateTask, CronInput, UpdateEnvVar, UpdateTask,
};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use futures::{
    sink::SinkExt,
    stream::{Stream, StreamExt},
};
use once_cell::sync::Lazy;
use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, convert::Infallible, process::Stdio, sync::Arc, time::Duration};
use tokio::process::Command;
use tokio::sync::{broadcast, RwLock};
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Deserialize)]
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
    #[serde(default)]
    pub allow_changes: bool,
    #[serde(default)]
    pub retry: bool,
    pub session_id: Option<String>,
    #[serde(default)]
    pub force_compress: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiHistoryMessage {
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub metadata: Option<String>,
}

const MASKED_API_KEY: &str = "********";

#[derive(Debug, Serialize)]
struct AiConfigResponse {
    enabled: bool,
    provider: String,
    base_url: String,
    api_key: String,
    model: String,
    context_window_tokens: u32,
    compression_ratio: u8,
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
            context_window_tokens: config.context_window_tokens,
            compression_ratio: config.compression_ratio,
        }
    }
}

const DEFAULT_CONTEXT_WINDOW_TOKENS: usize = 131_072;
const MIN_CONTEXT_WINDOW_TOKENS: usize = 8_192;
const COMPRESSED_CONTEXT_TARGET_PERCENT: usize = 60;
const AI_REQUEST_MAX_ATTEMPTS: usize = 3;

fn should_retry_status(status: reqwest::StatusCode) -> bool {
    status.is_server_error()
        || matches!(
            status,
            reqwest::StatusCode::REQUEST_TIMEOUT
                | reqwest::StatusCode::TOO_EARLY
                | reqwest::StatusCode::TOO_MANY_REQUESTS
        )
}

async fn send_json_with_retries(
    client: &Client,
    endpoint: &str,
    api_key: &str,
    payload: &Value,
    operation: &str,
    validate: fn(&Value) -> Result<(), String>,
) -> Result<Value, String> {
    let mut last_error = String::new();
    for attempt in 1..=AI_REQUEST_MAX_ATTEMPTS {
        match client
            .post(endpoint)
            .bearer_auth(api_key)
            .json(payload)
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                if !status.is_success() {
                    last_error = format!(
                        "{operation}返回 {status}: {}",
                        body.chars().take(500).collect::<String>()
                    );
                    if !should_retry_status(status) {
                        return Err(last_error);
                    }
                } else {
                    match serde_json::from_str::<Value>(&body) {
                        Ok(value) => match validate(&value) {
                            Ok(()) => return Ok(value),
                            Err(error) => last_error = format!("{operation}响应无效: {error}"),
                        },
                        Err(error) => {
                            let preview = body.chars().take(240).collect::<String>();
                            last_error = format!(
                                "{operation}响应不是合法 JSON: {error}，响应片段: {preview}"
                            );
                        }
                    }
                }
            }
            Err(error) => {
                last_error = format!("{operation}请求失败: {error}");
            }
        }
        if attempt < AI_REQUEST_MAX_ATTEMPTS {
            tracing::warn!("{last_error}，将在第 {} 次尝试后重试", attempt);
            sleep(Duration::from_millis(300 * attempt as u64)).await;
        }
    }
    Err(format!("{operation}失败，已重试 2 次: {last_error}"))
}

fn validate_compression_response(value: &Value) -> Result<(), String> {
    value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| {
            message
                .get("content")
                .and_then(Value::as_str)
                .filter(|content| !content.trim().is_empty())
                .or_else(|| message.get("reasoning_content").and_then(Value::as_str))
        })
        .filter(|content| !content.trim().is_empty())
        .map(|_| ())
        .ok_or_else(|| "响应缺少摘要内容".to_string())
}

fn validate_agent_response(value: &Value) -> Result<(), String> {
    value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .map(|_| ())
        .ok_or_else(|| "响应缺少 choices[0].message".to_string())
}
#[derive(Debug, Clone, Copy, Default)]
struct ProviderUsage {
    prompt_tokens: Option<usize>,
    completion_tokens: Option<usize>,
    total_tokens: Option<usize>,
    cache_hit_tokens: Option<usize>,
    cache_miss_tokens: Option<usize>,
}

#[derive(Debug, Clone)]
struct ProviderCallResult {
    message: Value,
    usage: Option<ProviderUsage>,
    text_chunks: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct CompressionOutcome {
    changed: bool,
    provider_usage: Option<ProviderUsage>,
    request_estimate: usize,
    source_estimate: usize,
}

fn provider_usage(value: &Value) -> Option<ProviderUsage> {
    let usage = value.get("usage")?.as_object()?;
    let read = |key: &str| usage.get(key).and_then(Value::as_u64).map(|v| v as usize);
    let prompt_tokens = read("prompt_tokens").or_else(|| read("input_tokens"));
    let completion_tokens = read("completion_tokens").or_else(|| read("output_tokens"));
    let total_tokens = read("total_tokens").or_else(|| {
        prompt_tokens
            .zip(completion_tokens)
            .map(|(prompt, completion)| prompt + completion)
    });
    let cache_hit_tokens = read("prompt_cache_hit_tokens").or_else(|| {
        usage
            .get("prompt_tokens_details")
            .and_then(|details| details.get("cached_tokens"))
            .and_then(Value::as_u64)
            .map(|value| value as usize)
    });
    let cache_miss_tokens = read("prompt_cache_miss_tokens").or_else(|| {
        prompt_tokens.zip(cache_hit_tokens).map(|(prompt, hit)| prompt.saturating_sub(hit))
    });
    let result = ProviderUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cache_hit_tokens,
        cache_miss_tokens,
    };
    (result.prompt_tokens.is_some()
        || result.completion_tokens.is_some()
        || result.total_tokens.is_some())
    .then_some(result)
}

fn provider_context_tokens(usage: Option<ProviderUsage>) -> Option<usize> {
    usage.and_then(|value| value.prompt_tokens)
}

fn provider_context_tokens_without_prompt(
    usage: Option<ProviderUsage>,
    prompt: &str,
) -> Option<usize> {
    provider_context_tokens(usage).map(|tokens| tokens.saturating_sub(estimate_tokens(prompt)))
}

fn provider_cache_tokens(usage: Option<ProviderUsage>) -> Option<(usize, usize)> {
    usage.and_then(|value| {
        value
            .cache_hit_tokens
            .zip(value.cache_miss_tokens)
            .or_else(|| value.cache_hit_tokens.map(|hit| (hit, 0)))
    })
}

fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

fn stored_message_tokens(content: &str) -> usize {
    estimate_tokens(content) + 4
}

fn value_tokens(value: &Value) -> usize {
    value
        .get("content")
        .and_then(Value::as_str)
        .map(estimate_tokens)
        .unwrap_or_else(|| estimate_tokens(&value.to_string()))
        + 4
}

fn trim_to_tokens(text: &str, max_tokens: usize) -> String {
    let max_chars = max_tokens.saturating_mul(4);
    let mut value: String = text.chars().take(max_chars).collect();
    if value.len() < text.len() {
        value.push_str("\n[内容已压缩]");
    }
    value
}

fn context_limit(config: &AiConfig) -> usize {
    (config.context_window_tokens as usize)
        .clamp(MIN_CONTEXT_WINDOW_TOKENS, DEFAULT_CONTEXT_WINDOW_TOKENS)
}

fn compression_trigger(config: &AiConfig) -> usize {
    context_limit(config)
        .saturating_mul(config.compression_ratio.clamp(10, 95) as usize)
        .div_ceil(100)
}

fn fallback_compact_json_messages(messages: &mut Vec<Value>, config: &AiConfig) {
    let target = context_limit(config)
        .saturating_mul(COMPRESSED_CONTEXT_TARGET_PERCENT)
        .div_ceil(100);
    if messages.len() <= 1 || messages.iter().map(value_tokens).sum::<usize>() <= target {
        return;
    }
    let system = messages
        .first()
        .cloned()
        .unwrap_or_else(|| json!({"role":"system","content":""}));
    let current = messages
        .last()
        .cloned()
        .unwrap_or_else(|| json!({"role":"user","content":""}));
    let mut result = vec![system];
    let recent_budget = target.saturating_mul(40).div_ceil(100);
    let mut recent = Vec::new();
    let mut used = 0;
    for value in messages[1..messages.len() - 1].iter().rev() {
        let cost = value_tokens(value);
        if used + cost > recent_budget {
            break;
        }
        recent.push(value.clone());
        used += cost;
    }
    recent.reverse();
    result.push(json!({
        "role": "system",
        "content": "[历史上下文压缩失败，以下仅保留最近对话。请以当前请求为准。]",
    }));
    result.extend(recent);
    result.push(current);
    *messages = result;
}

fn force_local_compact_json_messages(messages: &mut Vec<Value>, config: &AiConfig) -> bool {
    if messages.len() <= 1 {
        return false;
    }
    let total = messages.iter().map(value_tokens).sum::<usize>();
    let target = context_limit(config)
        .saturating_mul(COMPRESSED_CONTEXT_TARGET_PERCENT)
        .div_ceil(100)
        .min(
            total
                .saturating_mul(COMPRESSED_CONTEXT_TARGET_PERCENT)
                .div_ceil(100)
                .max(64),
        );
    let source = messages[1..]
        .iter()
        .map(|value| {
            let role = value
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("message");
            let content = value.get("content").and_then(Value::as_str).unwrap_or("");
            format!("{role}: {content}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let compacted = vec![
        messages
            .first()
            .cloned()
            .unwrap_or_else(|| json!({"role":"system","content":""})),
        json!({
            "role": "system",
            "content": format!("[历史上下文压缩摘要]\n{}", trim_to_tokens(&source, target)),
        }),
    ];
    let changed = compacted.iter().map(value_tokens).sum::<usize>() < total;
    if changed {
        *messages = compacted;
    }
    changed
}

async fn compress_json_messages(
    client: &Client,
    endpoint: &str,
    api_key: &str,
    model: &str,
    messages: &mut Vec<Value>,
    config: &AiConfig,
    force: bool,
) -> Result<CompressionOutcome, String> {
    let total = messages.iter().map(value_tokens).sum::<usize>();
    if (!force && total <= compression_trigger(config)) || messages.len() <= 1 {
        return Ok(CompressionOutcome {
            changed: false,
            provider_usage: None,
            request_estimate: 0,
            source_estimate: 0,
        });
    }

    let configured_target = context_limit(config)
        .saturating_mul(COMPRESSED_CONTEXT_TARGET_PERCENT)
        .div_ceil(100);
    let target = if force {
        configured_target.min(
            total
                .saturating_mul(COMPRESSED_CONTEXT_TARGET_PERCENT)
                .div_ceil(100)
                .max(64),
        )
    } else {
        configured_target
    };
    let summary_budget = target.saturating_mul(45).div_ceil(100).max(64);
    let recent_budget = if force {
        0
    } else {
        target.saturating_mul(35).div_ceil(100)
    };
    let mut recent = Vec::new();
    let mut recent_used = 0;
    for value in messages[1..].iter().rev() {
        let cost = value_tokens(value);
        if recent_used + cost > recent_budget {
            break;
        }
        recent.push(value.clone());
        recent_used += cost;
    }
    recent.reverse();
    let recent_start = messages.len().saturating_sub(recent.len());
    let old = &messages[1..recent_start];
    if old.is_empty() {
        fallback_compact_json_messages(messages, config);
        return Ok(CompressionOutcome {
            changed: messages.iter().map(value_tokens).sum::<usize>() < total,
            provider_usage: None,
            request_estimate: 0,
            source_estimate: 0,
        });
    }

    let source_budget = context_limit(config).saturating_mul(70).div_ceil(100);
    let mut source = String::new();
    for value in old {
        let line = match value.get("content").and_then(Value::as_str) {
            Some(content) => content.to_string(),
            None => value.to_string(),
        };
        let role = value
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("message");
        let remaining = source_budget.saturating_sub(estimate_tokens(&source));
        if remaining == 0 {
            break;
        }
        source.push_str(&format!("{role}: {}\n", trim_to_tokens(&line, remaining)));
    }

    let source_estimate = estimate_tokens(&source);
    let request_estimate = source_estimate
        + estimate_tokens("你是上下文压缩器。请压缩以下历史上下文，摘要控制在约 tokens 内：")
        + 8;

    let summary_messages = vec![
        json!({
            "role": "system",
            "content": "你是上下文压缩器。把给定的历史对话压缩成可供另一个 AI 继续工作的事实摘要。保留用户目标、约束、已做决定、关键文件路径、代码/API 细节、错误和未完成事项；删除寒暄和重复内容。不要执行任务，不要回答用户，不要编造信息。只输出简洁摘要。",
        }),
        json!({
            "role": "user",
            "content": format!("请压缩以下历史上下文，摘要控制在约 {summary_budget} tokens 内：\n\n{source}"),
        }),
    ];
    let response = send_json_with_retries(
        client,
        endpoint,
        api_key,
        &json!({
            "model": model,
            "stream": false,
            "messages": summary_messages,
            "temperature": 0.1,
            "max_tokens": summary_budget,
        }),
        "上下文压缩请求",
        validate_compression_response,
    )
    .await?;
    let usage = provider_usage(&response);
    let summary = response
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| {
            message
                .get("content")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .or_else(|| message.get("reasoning_content").and_then(Value::as_str))
        })
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "上下文压缩响应缺少摘要".to_string())?;

    let mut compacted = vec![messages
        .first()
        .cloned()
        .unwrap_or_else(|| json!({"role":"system","content":""}))];
    compacted.push(json!({
        "role": "system",
        "content": format!("[历史上下文压缩摘要]\n{}", trim_to_tokens(summary, summary_budget)),
    }));
    compacted.extend(recent);
    *messages = compacted;
    Ok(CompressionOutcome {
        changed: true,
        provider_usage: usage,
        request_estimate,
        source_estimate,
    })
}

fn agent_system_prompt() -> &'static str {
    r#"你是 Zhuque 的工作区 Agent。你可以浏览脚本目录、读取文件、搜索代码，管理定时任务、环境变量和依赖，查看执行日志，联网搜索公开网页，可以直接调用对应工具执行脚本和命令，直接调用对应工具。需要 Python、Node.js 或 Linux 依赖时，必须使用依赖管理工具安装，不得直接通过 shell、pip、npm、apt 等命令安装。脚本执行期间可通过内置 helper 主动发送通知到已配置的通知渠道：Shell 使用 `notify "标题" "内容"`；Python 使用 `from notify import send` 后调用 `send("标题", "内容")`；Node.js 使用 `const { sendNotify } = require('sendNotify')` 后调用 `sendNotify('标题', '内容')`；TypeScript（Bun）使用 `import { sendNotify } from 'sendNotify'` 后调用 `sendNotify('标题', '内容')`。需要主动通知时优先使用对应脚本语言的 helper，不要安装额外通知依赖；标题和内容均必填。先使用工具获取必要上下文，不要猜测文件内容。用户要求修改文件时，优先使用 `edit_file` 做精确字符串替换以节省输出 token；只有新建文件或无法精确替换时才使用 `write_file`，删除文件使用 `delete_file`。必须调用这些工具，不能通过 Markdown、JSON 或最终回复伪造文件修改，也不要输出文件完整内容作为修改提案。修改完成后用简短文字说明结果。所有路径必须是工作区相对路径。附加目录存在时，所有工具路径必须限制在该目录内；工具 path 为空表示附加目录根目录，禁止回到工作区根目录。运行权限、模式、当前文件和本轮用户请求会放在最后一条 user 消息中。保持本系统提示词不变，优先利用稳定前缀缓存。"#
}

fn agent_tools() -> Value {
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
                "description": "读取脚本工作区内一个文本文件。可选 start_line/end_line 按 1 开始的行号读取区间；省略时读取完整文件。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "start_line": { "type": "integer", "minimum": 1 },
                        "end_line": { "type": "integer", "minimum": 1 }
                    },
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
        json!({
            "type": "function",
            "function": {
                "name": "list_tasks",
                "description": "列出定时任务，可按名称或命令搜索。不要返回任务环境变量内容。",
                "parameters": { "type": "object", "properties": { "search": { "type": "string" } } }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "create_task",
                "description": "创建定时任务。只有用户明确要求创建时调用；必须提供名称、命令和 cron。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" }, "command": { "type": "string" },
                        "cron": { "type": "string", "description": "cron 表达式，也可传字符串数组" },
                        "type": { "type": "string", "enum": ["cron", "manual", "startup"] },
                        "enabled": { "type": "boolean" }, "env": { "type": "string" },
                        "working_dir": { "type": "string" }, "timeout": { "type": "integer", "minimum": 0 }
                    },
                    "required": ["name", "command", "cron"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "update_task",
                "description": "编辑已有定时任务。先确认任务 id，只修改用户明确要求的字段。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer" }, "name": { "type": "string" }, "command": { "type": "string" },
                        "cron": { "type": "string" }, "type": { "type": "string", "enum": ["cron", "manual", "startup"] },
                        "enabled": { "type": "boolean" }, "env": { "type": "string" }, "working_dir": { "type": "string" },
                        "timeout": { "type": "integer", "minimum": 0 }
                    },
                    "required": ["id"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "delete_task",
                "description": "删除已有定时任务。仅在用户明确要求删除并确认 id 后调用。",
                "parameters": { "type": "object", "properties": { "id": { "type": "integer" } }, "required": ["id"] }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "list_logs",
                "description": "查看任务执行日志列表，可按 task_id、页码和每页数量过滤；列表不含正文。",
                "parameters": {
                    "type": "object", "properties": {
                        "task_id": { "type": "integer" }, "page": { "type": "integer", "minimum": 1 },
                        "page_size": { "type": "integer", "minimum": 1, "maximum": 100 }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_log",
                "description": "读取一条任务执行日志的完整输出。",
                "parameters": { "type": "object", "properties": { "id": { "type": "integer" } }, "required": ["id"] }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "list_env_vars",
                "description": "列出环境变量名称、备注和启用状态；出于安全原因不返回变量值。",
                "parameters": { "type": "object", "properties": { "search": { "type": "string" } } }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "create_env_var",
                "description": "创建环境变量。value 只用于写入，结果中不回显。",
                "parameters": {
                    "type": "object", "properties": {
                        "key": { "type": "string" }, "value": { "type": "string" },
                        "remark": { "type": "string" }, "enabled": { "type": "boolean" }
                    }, "required": ["key", "value"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "update_env_var",
                "description": "编辑环境变量。value 只用于写入，结果中不回显。",
                "parameters": {
                    "type": "object", "properties": {
                        "id": { "type": "integer" }, "key": { "type": "string" }, "value": { "type": "string" },
                        "remark": { "type": "string" }, "enabled": { "type": "boolean" }
                    }, "required": ["id"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "delete_env_var",
                "description": "删除环境变量。仅在用户明确要求删除并确认 id 后调用。",
                "parameters": { "type": "object", "properties": { "id": { "type": "integer" } }, "required": ["id"] }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "web_search",
                "description": "联网搜索公开网页，返回标题、摘要和链接；需要实时信息时使用。",
                "parameters": {
                    "type": "object", "properties": {
                        "query": { "type": "string" }, "limit": { "type": "integer", "minimum": 1, "maximum": 10 }
                    }, "required": ["query"]
                }
            }
        }),
    ];

    tools.push(json!({
        "type": "function",
        "function": {
            "name": "write_file",
            "description": "写入工作区内一个文本文件。用户明确要求修改或创建文件时使用；content 必须是完整文件内容。",
            "parameters": {
                "type": "object",
                "properties": { "path": { "type": "string" }, "content": { "type": "string" } },
                "required": ["path", "content"]
            }
        }
    }));
    tools.push(json!({
        "type": "function",
        "function": {
            "name": "edit_file",
            "description": "编辑工作区内文本文件：将 old_string 替换为 new_string。默认替换全部匹配；仅在用户明确要求修改时调用。",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old_string": { "type": "string" },
                    "new_string": { "type": "string" },
                    "replace_all": { "type": "boolean" }
                },
                "required": ["path", "old_string", "new_string"]
            }
        }
    }));
    tools.push(json!({
        "type": "function",
        "function": {
            "name": "delete_file",
            "description": "删除工作区内一个文件。仅在用户明确要求删除时使用。",
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
    if requested == base
        || requested.starts_with(&base_prefix)
        || requested.starts_with(&format!("{base}\\"))
    {
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
        let entries = service
            .list_dir(&directory)
            .await
            .map_err(|e| e.to_string())?;
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
    cancel: Option<&CancellationToken>,
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
        None => match cancel {
            Some(token) => tokio::select! {
                _ = token.cancelled() => return Err("命令已取消".to_string()),
                result = process.output() => result.map_err(|e| format!("命令执行失败: {e}"))?,
            },
            None => process
                .output()
                .await
                .map_err(|e| format!("命令执行失败: {e}"))?,
        },
    };

    let stdout = String::from_utf8_lossy(&output.stdout)
        .chars()
        .take(120000)
        .collect::<String>();
    let stderr = String::from_utf8_lossy(&output.stderr)
        .chars()
        .take(120000)
        .collect::<String>();
    Ok(json!({
        "success": output.status.success(),
        "code": output.status.code(),
        "stdout": stdout,
        "stderr": stderr,
    }))
}

#[derive(Debug, Deserialize)]
struct CreateTaskToolArgs {
    name: String,
    command: String,
    cron: CronInput,
    #[serde(rename = "type")]
    task_type: Option<String>,
    enabled: Option<bool>,
    env: Option<String>,
    pre_command: Option<String>,
    post_command: Option<String>,
    group_id: Option<i64>,
    working_dir: Option<String>,
    timeout: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct UpdateTaskToolArgs {
    id: i64,
    name: Option<String>,
    command: Option<String>,
    cron: Option<CronInput>,
    #[serde(rename = "type")]
    task_type: Option<String>,
    enabled: Option<bool>,
    env: Option<String>,
    pre_command: Option<String>,
    post_command: Option<String>,
    group_id: Option<i64>,
    working_dir: Option<String>,
    timeout: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct EnvToolArgs {
    id: Option<i64>,
    key: Option<String>,
    value: Option<String>,
    remark: Option<String>,
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct IdToolArgs {
    id: i64,
}

fn parse_tool_args<T: DeserializeOwned>(arguments: &Value) -> Result<T, String> {
    serde_json::from_value(arguments.clone()).map_err(|error| format!("工具参数无效: {error}"))
}

fn task_summary(task: &crate::models::Task) -> Value {
    json!({
        "id": task.id,
        "name": task.name,
        "command": task.command,
        "cron": task.cron,
        "type": task.task_type,
        "enabled": task.enabled,
        "working_dir": task.working_dir,
        "timeout": task.timeout,
        "last_run_at": task.last_run_at,
        "next_run_at": task.next_run_at,
    })
}

fn env_summary(env: &crate::models::EnvVar) -> Value {
    json!({
        "id": env.id,
        "key": env.key,
        "remark": env.remark,
        "enabled": env.enabled,
        "value": "[已隐藏]",
    })
}

fn redact_tool_arguments(name: &str, arguments: &Value) -> Value {
    if !matches!(name, "create_env_var" | "update_env_var") {
        return arguments.clone();
    }
    let mut value = arguments.clone();
    if let Some(object) = value.as_object_mut() {
        if object.contains_key("value") {
            object.insert("value".to_string(), Value::String("[已隐藏]".to_string()));
        }
    }
    value
}

fn decode_xml(value: &str) -> String {
    value.replace("<![CDATA[", "").replace("]]>", "")
        .replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">")
        .replace("&quot;", "\"").replace("&apos;", "'")
}

fn xml_tag(item: &str, tag: &str) -> String {
    let start_tag = format!("<{tag}>");
    let end_tag = format!("</{tag}>");
    item.find(&start_tag).and_then(|start| {
        let content_start = start + start_tag.len();
        item[content_start..].find(&end_tag)
            .map(|end| decode_xml(&item[content_start..content_start + end]))
    }).unwrap_or_default()
}

async fn web_search(query: &str, limit: usize) -> Result<Value, String> {
    if query.trim().is_empty() {
        return Err("搜索关键词不能为空".to_string());
    }
    let limit = limit.clamp(1, 10);
    let url = format!(
        "https://www.bing.com/search?format=rss&count={limit}&q={}",
        urlencoding::encode(query.trim())
    );
    let client = Client::builder()
        .user_agent("Zhuque/AI-Workbench")
        .build()
        .map_err(|error| format!("创建搜索客户端失败: {error}"))?;
    let response = client.get(url).send().await.map_err(|error| format!("联网搜索请求失败: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("搜索服务返回 {}", response.status()));
    }
    let body = response.text().await.map_err(|error| format!("读取搜索结果失败: {error}"))?;
    let results = body.split("<item>").skip(1).take(limit).map(|item| json!({
        "title": xml_tag(item, "title"),
        "url": xml_tag(item, "link"),
        "snippet": xml_tag(item, "description"),
        "published_at": xml_tag(item, "pubDate"),
    })).collect::<Vec<_>>();
    Ok(json!({ "query": query.trim(), "source": "Bing RSS", "results": results }))
}

async fn execute_agent_tool(
    state: &AppState,
    name: &str,
    arguments: &Value,
    allow_commands: bool,
    allow_changes: bool,
    attached_directory: Option<&str>,
    cancel: Option<&CancellationToken>,
) -> Result<Value, String> {
    if !allow_changes && matches!(name, "write_file" | "edit_file" | "delete_file") {
        return Err("操作未执行：系统拦截".to_string());
    }

    if !allow_commands && matches!(
        name,
        "create_task" | "update_task" | "delete_task"
            | "create_env_var" | "update_env_var" | "delete_env_var"
    ) {
        return Err("操作未执行：系统拦截".to_string());
    }

    match name {
        "write_file" => {
            let path = scope_agent_path(attached_directory, &argument_string(arguments, "path")?);
            let content = argument_string(arguments, "content")?;
            state.script_service.write(&path, &content).await.map_err(|e| e.to_string())?;
            Ok(json!({ "written": true, "path": path, "bytes": content.len() }))
        }
        "delete_file" => {
            let path = scope_agent_path(attached_directory, &argument_string(arguments, "path")?);
            state.script_service.delete(&path).await.map_err(|e| e.to_string())?;
            Ok(json!({ "deleted": true, "path": path }))
        }
        "list_dir" => {
            let path = arguments.get("path").and_then(Value::as_str).unwrap_or("");
            let path = scope_agent_path(attached_directory, path);
            let entries = state
                .script_service
                .list_dir(&path)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_value(entries).map_err(|e| e.to_string())
        }
        "read_file" => {
            let path = argument_string(arguments, "path")?;
            let path = scope_agent_path(attached_directory, &path);
            let content = state
                .script_service
                .read(&path)
                .await
                .map_err(|e| e.to_string())?;
            let all_lines: Vec<&str> = content.lines().collect();
            let start_line = arguments.get("start_line").and_then(Value::as_u64).unwrap_or(1) as usize;
            let end_line = arguments
                .get("end_line")
                .and_then(Value::as_u64)
                .map(|value| value as usize)
                .unwrap_or(all_lines.len().max(1));
            if start_line == 0 || end_line < start_line {
                return Err("读取行区间无效：start_line 必须不大于 end_line，且从 1 开始".to_string());
            }
            let selected = if arguments.get("start_line").is_some() || arguments.get("end_line").is_some() {
                all_lines
                    .get(start_line.saturating_sub(1)..end_line.min(all_lines.len()))
                    .unwrap_or(&[])
                    .join("\n")
            } else {
                content.chars().take(160000).collect::<String>()
            };
            Ok(json!({
                "path": path,
                "start_line": if arguments.get("start_line").is_some() || arguments.get("end_line").is_some() { start_line } else { 1 },
                "end_line": if arguments.get("start_line").is_some() || arguments.get("end_line").is_some() { end_line.min(all_lines.len()) } else { all_lines.len() },
                "content": selected,
                "truncated": if arguments.get("start_line").is_some() || arguments.get("end_line").is_some() { end_line > all_lines.len() } else { content.chars().count() > 160000 },
            }))
        }
        "edit_file" => {
            let path = scope_agent_path(attached_directory, &argument_string(arguments, "path")?);
            let old_string = argument_string(arguments, "old_string")?;
            let new_string = argument_string(arguments, "new_string")?;
            if old_string.is_empty() {
                return Err("old_string 不能为空".to_string());
            }
            let content = state.script_service.read(&path).await.map_err(|e| e.to_string())?;
            let replace_all = arguments.get("replace_all").and_then(Value::as_bool).unwrap_or(true);
            let occurrences = content.matches(&old_string).count();
            if occurrences == 0 {
                return Err("未找到要替换的文本，请先使用 read_file 获取准确内容".to_string());
            }
            let updated = if replace_all {
                content.replace(&old_string, &new_string)
            } else {
                content.replacen(&old_string, &new_string, 1)
            };
            state.script_service.write(&path, &updated).await.map_err(|e| e.to_string())?;
            Ok(json!({ "edited": true, "path": path, "replacements": if replace_all { occurrences } else { 1 } }))
        }
        "search_files" => {
            let query = argument_string(arguments, "query")?;
            let path = arguments.get("path").and_then(Value::as_str).unwrap_or("");
            let path = scope_agent_path(attached_directory, path);
            search_workspace(&state.script_service, &path, &query).await
        }
        "list_tasks" => {
            let search = arguments.get("search").and_then(Value::as_str);
            let tasks = state.task_service.list_with_search(search).await.map_err(|e| e.to_string())?;
            Ok(json!({ "tasks": tasks.iter().map(task_summary).collect::<Vec<_>>() }))
        }
        "create_task" => {
            let args: CreateTaskToolArgs = parse_tool_args(arguments)?;
            let task = state.task_service.create(CreateTask {
                name: args.name,
                command: args.command,
                cron: args.cron,
                task_type: args.task_type.unwrap_or_else(|| "cron".to_string()),
                enabled: args.enabled.unwrap_or(true),
                env: args.env,
                pre_command: args.pre_command,
                post_command: args.post_command,
                group_id: args.group_id,
                working_dir: args.working_dir,
                notification: None,
                timeout: args.timeout.unwrap_or(0).max(0),
            }).await.map_err(|e| e.to_string())?;
            state.scheduler.add_task_to_scheduler(task.id).await.map_err(|e| e.to_string())?;
            Ok(json!({ "task": task_summary(&task) }))
        }
        "update_task" => {
            let args: UpdateTaskToolArgs = parse_tool_args(arguments)?;
            let task = state.task_service.update(args.id, UpdateTask {
                name: args.name,
                command: args.command,
                cron: args.cron,
                task_type: args.task_type,
                enabled: args.enabled,
                env: args.env,
                pre_command: args.pre_command,
                post_command: args.post_command,
                group_id: args.group_id,
                working_dir: args.working_dir,
                notification: None,
                timeout: args.timeout.map(|value| value.max(0)),
            }).await.map_err(|e| e.to_string())?.ok_or_else(|| "任务不存在".to_string())?;
            state.scheduler.update_task_in_scheduler(args.id).await.map_err(|e| e.to_string())?;
            Ok(json!({ "task": task_summary(&task) }))
        }
        "delete_task" => {
            let args: IdToolArgs = parse_tool_args(arguments)?;
            let deleted = state.task_service.delete(args.id).await.map_err(|e| e.to_string())?;
            if !deleted { return Err("任务不存在".to_string()); }
            state.scheduler.remove_task_from_scheduler(args.id).await.map_err(|e| e.to_string())?;
            Ok(json!({ "deleted": true, "id": args.id }))
        }
        "list_logs" => {
            let task_id = arguments.get("task_id").and_then(Value::as_i64);
            let page = arguments.get("page").and_then(Value::as_i64).unwrap_or(1).max(1);
            let page_size = arguments.get("page_size").and_then(Value::as_i64).unwrap_or(10).clamp(1, 100);
            let logs = state.log_service.list(task_id, page, page_size).await.map_err(|e| e.to_string())?;
            serde_json::to_value(logs).map_err(|e| e.to_string())
        }
        "get_log" => {
            let args: IdToolArgs = parse_tool_args(arguments)?;
            let log = state.log_service.get(args.id).await.map_err(|e| e.to_string())?.ok_or_else(|| "日志不存在".to_string())?;
            serde_json::to_value(log).map_err(|e| e.to_string())
        }
        "list_env_vars" => {
            let search = arguments.get("search").and_then(Value::as_str);
            let vars = state.env_service.list_with_search(search).await.map_err(|e| e.to_string())?;
            Ok(json!({ "variables": vars.iter().map(env_summary).collect::<Vec<_>>() }))
        }
        "create_env_var" => {
            let args: EnvToolArgs = parse_tool_args(arguments)?;
            let key = args.key.ok_or_else(|| "缺少环境变量 key".to_string())?;
            let value = args.value.ok_or_else(|| "缺少环境变量 value".to_string())?;
            let env = state.env_service.create(CreateEnvVar { key, value, remark: args.remark, enabled: args.enabled }).await.map_err(|e| e.to_string())?;
            Ok(json!({ "created": true, "variable": env_summary(&env) }))
        }
        "update_env_var" => {
            let args: EnvToolArgs = parse_tool_args(arguments)?;
            let id = args.id.ok_or_else(|| "缺少环境变量 id".to_string())?;
            let env = state.env_service.update(id, UpdateEnvVar { key: args.key, value: args.value, remark: args.remark, enabled: args.enabled }).await.map_err(|e| e.to_string())?.ok_or_else(|| "环境变量不存在".to_string())?;
            Ok(json!({ "updated": true, "variable": env_summary(&env) }))
        }
        "delete_env_var" => {
            let args: IdToolArgs = parse_tool_args(arguments)?;
            let deleted = state.env_service.delete(args.id).await.map_err(|e| e.to_string())?;
            if !deleted { return Err("环境变量不存在".to_string()); }
            Ok(json!({ "deleted": true, "id": args.id }))
        }
        "web_search" => {
            let query = argument_string(arguments, "query")?;
            let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(5) as usize;
            web_search(&query, limit).await
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
            run_workspace_command(
                &state.script_service,
                &command,
                timeout_secs,
                attached_directory,
                cancel,
            )
            .await
        }
        "run_script" | "run_command" => Err("操作未执行：系统拦截".to_string()),
        _ => Err(format!("未知工具: {name}")),
    }
}

#[derive(Debug)]
struct AiJob {
    session_id: Option<String>,
    user_key: String,
    sender: broadcast::Sender<String>,
    events: RwLock<Vec<String>>,
    cancel: CancellationToken,
}

static AI_JOBS: Lazy<RwLock<HashMap<String, Arc<AiJob>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

#[derive(Debug, Deserialize)]
pub struct AgentWsQuery {
    job_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum AgentWsMessage {
    #[serde(rename = "start")]
    Start { request: AiChatRequest },
    #[serde(rename = "cancel")]
    Cancel,
}

async fn publish_job(job: &Arc<AiJob>, payload: Value) {
    let sequence = {
        let mut events = job.events.write().await;
        let sequence = events.len() as u64 + 1;
        let message = json!({ "seq": sequence, "event": payload }).to_string();
        events.push(message.clone());
        let _ = job.sender.send(message);
        sequence
    };
    let _ = sequence;
}

async fn run_agent_job(state: Arc<AppState>, request: AiChatRequest, job: Arc<AiJob>) {
    if !request.retry {
        if let Some(title) = store_session_message(
            &state,
            request.session_id.as_deref(),
            &job.user_key,
            "user",
            &request.prompt,
        )
        .await
        {
            publish_job(&job, json!({"type":"session_title","title":title})).await;
        }
    }
    let config = match state.config_service.get_ai_config().await {
        Ok(config)
            if config.enabled
                && !config.api_key.trim().is_empty()
                && !config.model.trim().is_empty() =>
        {
            config
        }
        Ok(_) => {
            publish_job(&job, json!({"type":"error","message":"AI 尚未配置，请先在系统配置中填写 Provider、API Key 和模型"})).await;
            set_session_job(&state, request.session_id.as_deref(), &job.user_key, None).await;
            publish_job(&job, json!({"type":"done"})).await;
            return;
        }
        Err(error) => {
            publish_job(&job, json!({"type":"error","message":error.to_string()})).await;
            set_session_job(&state, request.session_id.as_deref(), &job.user_key, None).await;
            publish_job(&job, json!({"type":"done"})).await;
            return;
        }
    };
    let base_url = config.base_url.trim_end_matches('/');
    let endpoint = if base_url.ends_with("/chat/completions") {
        base_url.to_string()
    } else {
        format!("{base_url}/chat/completions")
    };
    let client = match Client::builder().build() {
        Ok(client) => client,
        Err(error) => {
            publish_job(&job, json!({"type":"error","message":error.to_string()})).await;
            set_session_job(&state, request.session_id.as_deref(), &job.user_key, None).await;
            publish_job(&job, json!({"type":"done"})).await;
            return;
        }
    };
    let system = agent_system_prompt();
    let stable_context = format!(
        "执行工具状态: 由系统处理\n模式: {}\n当前文件名: {}\n当前路径: {}\n当前附加目录: {}\n当前文件内容:\n{}\n\n最近执行输出:\n{}",
        request.mode,
        request.file_name.as_deref().unwrap_or("未选择文件"),
        request.file_path.as_deref().unwrap_or(""),
        request.directory_path.as_deref().unwrap_or("未附加目录"),
        request.file_content.as_deref().unwrap_or("未提供"),
        request.execution_output.as_deref().unwrap_or("未提供"),
    );
    let mut messages = vec![json!({"role":"system","content":system})];
    let history_start = request.history.len().saturating_sub(40);
    let retry_user_index = request.retry.then(|| {
        request.history.iter().rposition(|item| item.role == "user" && item.content.trim() == request.prompt.trim())
    }).flatten();
    for (history_index, item) in request.history.iter().enumerate().skip(history_start) {
        if Some(history_index) == retry_user_index { continue; }
        if item.content.trim().is_empty() && item.metadata.is_none() { continue; }
        if !matches!(item.role.as_str(), "user" | "assistant" | "tool") { continue; }
        let metadata = item.metadata.as_deref().and_then(|raw| serde_json::from_str::<Value>(raw).ok());
        let is_runtime_context = metadata.as_ref().and_then(|value| value.get("runtime_context")).and_then(Value::as_bool) == Some(true)
            || (item.role == "user" && item.content.starts_with("运行权限:") && item.content.contains("当前请求:"));
        if is_runtime_context { continue; }
        let mut message = json!({"role": item.role, "content": item.content.chars().take(12000).collect::<String>()});
        if let Some(metadata) = metadata {
            if let Some(value) = metadata.get("tool_calls") { message["tool_calls"] = value.clone(); }
            if let Some(value) = metadata.get("tool_call_id") { message["tool_call_id"] = value.clone(); }
            if let Some(value) = metadata.get("name") { message["name"] = value.clone(); }
        }
        messages.push(message);
    }
    messages.push(json!({
        "role": "user",
        "content": format!("{}\n\n当前请求:\n{}", stable_context, request.prompt),
        "metadata": {"runtime_context": true},
    }));
    let tools = agent_tools();
    'agent_loop: loop {
        if let Err(error) = compress_json_messages(
            &client,
            &endpoint,
            &config.api_key,
            &config.model,
            &mut messages,
            &config,
            false,
        )
        .await
        {
            tracing::warn!("{error}; falling back to local context compaction");
            fallback_compact_json_messages(&mut messages, &config);
        }
        if job.cancel.is_cancelled() {
            publish_job(&job, json!({"type":"cancelled","message":"任务已取消"})).await;
            set_session_job(&state, request.session_id.as_deref(), &job.user_key, None).await;
            publish_job(&job, json!({"type":"done"})).await;
            return;
        }
        let response = match call_agent_provider(
            &client,
            &endpoint,
            &config.api_key,
            &config.model,
            &messages,
            &tools,
            request.session_id.as_deref().map(|id| id),
            Some(&job),
        )
        .await
        {
            Ok(value) => value,
            Err(error) => {
                publish_job(&job, json!({"type":"error","message":error})).await;
                set_session_job(&state, request.session_id.as_deref(), &job.user_key, None).await;
                publish_job(&job, json!({"type":"done"})).await;
                return;
            }
        };
        let provider_usage = response.usage;
        if let Some(tokens) = provider_context_tokens_without_prompt(provider_usage, &request.prompt) {
            set_session_context_tokens(
                &state,
                request.session_id.as_deref(),
                &job.user_key,
                tokens,
            )
            .await;
            publish_job(&job, json!({
                "type": "context_usage",
                "tokens": tokens,
                "source": "provider",
            }))
            .await;
        }
        if let Some((hit, miss)) = provider_cache_tokens(provider_usage) {
            publish_job(&job, json!({
                "type": "cache_usage",
                "hit_tokens": hit,
                "miss_tokens": miss,
                "source": "provider",
            }))
            .await;
        }
        let response = response.message;
        let tool_calls = response
            .get("tool_calls")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        messages.push(response.clone());
        if tool_calls.is_empty() {
            break;
        }
        for call in tool_calls {
            let call_id = call
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("tool-call");
            let function = call.get("function").cloned().unwrap_or(Value::Null);
            let name = function.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = match function.get("arguments") {
                Some(Value::String(raw)) => {
                    serde_json::from_str::<Value>(raw).unwrap_or(Value::Object(Default::default()))
                }
                Some(value @ Value::Object(_)) => value.clone(),
                _ => Value::Object(Default::default()),
            };
            publish_job(
                &job,
                json!({"type":"tool_call","tool":name,"arguments":redact_tool_arguments(name, &arguments)}),
            )
            .await;
            let result = execute_agent_tool(
                &state,
                name,
                &arguments,
                request.allow_commands,
                request.allow_changes,
                request.directory_path.as_deref(),
                Some(&job.cancel),
            )
            .await;
            let (content, success) = match result {
                Ok(value) => (value.to_string(), true),
                Err(error) => (json!({"error":error}).to_string(), false),
            };
            publish_job(&job, json!({"type":"tool_result","tool":name,"success":success,"result":content.chars().take(6000).collect::<String>()})).await;
            messages.push(json!({"role":"tool","tool_call_id":call_id,"content":content}));
            if !success && content.contains("系统拦截") {
                break 'agent_loop;
            }
        }
    }
    let _ = store_agent_protocol_messages(
        &state,
        request.session_id.as_deref(),
        &job.user_key,
        &messages,
    )
    .await;
    publish_job(&job, json!({
        "type": "conversation_sync",
        "messages": agent_history_for_client(&messages),
    })).await;
    set_session_job(&state, request.session_id.as_deref(), &job.user_key, None).await;
    publish_job(&job, json!({"type":"done"})).await;
}

pub async fn agent_ws(
    ws: WebSocketUpgrade,
    Query(query): Query<AgentWsQuery>,
    State(state): State<Arc<AppState>>,
    Claims { sub, .. }: Claims,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_agent_ws(socket, query.job_id, state, sub))
}

async fn handle_agent_ws(
    socket: WebSocket,
    requested_job_id: Option<String>,
    state: Arc<AppState>,
    user_key: String,
) {
    let (mut sender, mut receiver) = socket.split();
    let job = if let Some(job_id) = requested_job_id {
        match AI_JOBS
            .read()
            .await
            .get(&job_id)
            .filter(|job| job.user_key == user_key)
            .cloned()
        {
            Some(job) => Some(job),
            None => {
                let _ = sender
                    .send(Message::Text(
                        json!({"type":"error","message":"后台任务不存在或已过期"})
                            .to_string()
                            .into(),
                    ))
                    .await;
                return;
            }
        }
    } else {
        None
    };
    let job = match job {
        Some(job) => job,
        None => {
            let Some(Ok(Message::Text(message))) = receiver.next().await else {
                return;
            };
            let Ok(AgentWsMessage::Start { request }) =
                serde_json::from_str::<AgentWsMessage>(&message)
            else {
                let _ = sender
                    .send(Message::Text(
                        json!({"type":"error","message":"首条 WebSocket 消息必须是 start"})
                            .to_string()
                            .into(),
                    ))
                    .await;
                return;
            };
            let job_id = uuid::Uuid::new_v4().to_string();
            let (job_sender, _) = broadcast::channel(256);
            let job = Arc::new(AiJob {
                session_id: request.session_id.clone(),
                user_key: user_key.clone(),
                sender: job_sender,
                events: RwLock::new(Vec::new()),
                cancel: CancellationToken::new(),
            });
            set_session_job(
                &state,
                request.session_id.as_deref(),
                &user_key,
                Some(&job_id),
            )
            .await;
            AI_JOBS.write().await.insert(job_id.clone(), job.clone());
            publish_job(&job, json!({"type":"job_started","job_id":job_id})).await;
            let job_for_task = job.clone();
            tokio::spawn(run_agent_job(state, request, job_for_task));
            job
        }
    };
    let mut events = job.sender.subscribe();
    let replay = job.events.read().await.clone();
    let mut last_sequence = 0u64;
    for message in replay {
        if let Ok(value) = serde_json::from_str::<Value>(&message) {
            last_sequence = value
                .get("seq")
                .and_then(Value::as_u64)
                .unwrap_or(last_sequence);
        }
        if sender.send(Message::Text(message.into())).await.is_err() {
            return;
        }
    }
    loop {
        tokio::select! {
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(message))) => {
                        if matches!(serde_json::from_str::<AgentWsMessage>(&message), Ok(AgentWsMessage::Cancel)) {
                            job.cancel.cancel();
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => return,
                    _ => {}
                }
            }
            event = events.recv() => {
                match event {
                    Ok(message) => {
                        let sequence = serde_json::from_str::<Value>(&message).ok().and_then(|v| v.get("seq").and_then(Value::as_u64)).unwrap_or(0);
                        if sequence <= last_sequence { continue; }
                        last_sequence = sequence;
                        if sender.send(Message::Text(message.into())).await.is_err() { return; }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        }
    }
}

pub async fn agent(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AiChatRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)> {
    let config = state
        .config_service
        .get_ai_config()
        .await
        .map_err(internal_error)?;
    if !config.enabled || config.api_key.trim().is_empty() || config.model.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "AI 尚未配置，请先在系统配置中填写 Provider、API Key 和模型".into(),
        ));
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
    let client = Client::builder().build().map_err(internal_error)?;
    let state = state.clone();

    let stream = async_stream::stream! {
        let system = agent_system_prompt();
        let stable_context = format!(
            "执行工具状态: 由系统处理\n模式: {}\n当前文件名: {}\n当前路径: {}\n当前附加目录: {}\n当前文件内容:\n{}\n\n最近执行输出:\n{}",
                    request.mode,
            request.file_name.as_deref().unwrap_or("未选择文件"),
            request.file_path.as_deref().unwrap_or(""),
            request.directory_path.as_deref().unwrap_or("未附加目录"),
            request.file_content.as_deref().unwrap_or("未提供"),
            request.execution_output.as_deref().unwrap_or("未提供"),
        );
        let mut messages = vec![
            json!({"role": "system", "content": system}),
            json!({"role": "user", "content": stable_context, "metadata": {"runtime_context": true}}),
        ];
        let history_start = request.history.len().saturating_sub(40);
        for item in request.history.iter().skip(history_start) {
            if !item.content.trim().is_empty() && matches!(item.role.as_str(), "user" | "assistant" | "tool") {
                let mut message = json!({"role": item.role, "content": item.content.chars().take(12000).collect::<String>()});
                if let Some(metadata) = item.metadata.as_deref().and_then(|raw| serde_json::from_str::<Value>(raw).ok()) {
                    if let Some(value) = metadata.get("tool_calls") { message["tool_calls"] = value.clone(); }
                    if let Some(value) = metadata.get("tool_call_id") { message["tool_call_id"] = value.clone(); }
                    if let Some(value) = metadata.get("name") { message["name"] = value.clone(); }
                }
                messages.push(message);
            }
        }
        messages.push(json!({"role": "user", "content": request.prompt}));
        let tools = agent_tools();

        'agent_stream_loop: loop {
            let response = match call_agent_provider(
                &client,
                &endpoint,
                &config.api_key,
                &config.model,
                &messages,
                &tools,
                request.session_id.as_deref().map(|id| id),
                None,
            ).await {
                Ok(value) => value,
                Err(error) => {
                    yield Ok(Event::default().event("agent").data(json!({"type":"error","message":error}).to_string()));
                    return;
                }
            };
            for chunk in &response.text_chunks {
                yield Ok(Event::default().event("agent").data(json!({"type":"text","content":chunk}).to_string()));
            }
            let provider_usage = response.usage;
            if let Some(tokens) = provider_context_tokens_without_prompt(provider_usage, &request.prompt) {
                yield Ok(Event::default().event("agent").data(json!({
                    "type": "context_usage",
                    "tokens": tokens,
                    "source": "provider",
                }).to_string()));
            }
            if let Some((hit, miss)) = provider_cache_tokens(provider_usage) {
                yield Ok(Event::default().event("agent").data(json!({
                    "type": "cache_usage",
                    "hit_tokens": hit,
                    "miss_tokens": miss,
                    "source": "provider",
                }).to_string()));
            }
            let response = response.message;
            let tool_calls = response.get("tool_calls").and_then(Value::as_array).cloned().unwrap_or_default();
            messages.push(response.clone());
            if tool_calls.is_empty() {
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
                    "arguments": redact_tool_arguments(name, &arguments),
                }).to_string()));

                let result = execute_agent_tool(&state, name, &arguments, request.allow_commands, request.allow_changes, request.directory_path.as_deref(), None).await;
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
                if !success && content.contains("系统拦截") {
                    break 'agent_stream_loop;
                }
            }
        }

        yield Ok(Event::default().event("agent").data(json!({"type":"done"}).to_string()));
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

fn apply_agent_stream_chunk(
    data: &str,
    content: &mut String,
    text_chunks: &mut Vec<String>,
    tool_calls: &mut Vec<Value>,
    usage: &mut Option<ProviderUsage>,
) {
    let data = data.trim();
    if data.is_empty() || data == "[DONE]" {
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return;
    };
    if let Some(parsed_usage) = provider_usage(&value) {
        *usage = Some(parsed_usage);
    }
    let Some(choice) = value.get("choices").and_then(Value::as_array).and_then(|items| items.first()) else {
        return;
    };
    let Some(delta) = choice.get("delta") else {
        return;
    };
    if let Some(text) = delta.get("content").and_then(Value::as_str) {
        content.push_str(text);
        text_chunks.push(text.to_string());
    }
    let Some(deltas) = delta.get("tool_calls").and_then(Value::as_array) else {
        return;
    };
    for delta_call in deltas {
        let index = delta_call.get("index").and_then(Value::as_u64).unwrap_or(tool_calls.len() as u64) as usize;
        while tool_calls.len() <= index {
            tool_calls.push(json!({"id":"","type":"function","function":{"name":"","arguments":""}}));
        }
        let current = &mut tool_calls[index];
        if let Some(id) = delta_call.get("id").and_then(Value::as_str) {
            current["id"] = Value::String(id.to_string());
        }
        if let Some(name) = delta_call.get("function").and_then(|value| value.get("name")).and_then(Value::as_str) {
            let existing = current["function"]["name"].as_str().unwrap_or("").to_string();
            current["function"]["name"] = Value::String(existing + name);
        }
        if let Some(arguments) = delta_call.get("function").and_then(|value| value.get("arguments")).and_then(Value::as_str) {
            let existing = current["function"]["arguments"].as_str().unwrap_or("").to_string();
            current["function"]["arguments"] = Value::String(existing + arguments);
        }
    }
}

async fn publish_stream_text(
    job: Option<&Arc<AiJob>>,
    chunks: &[String],
    published: &mut usize,
) {
    let Some(job) = job else {
        *published = chunks.len();
        return;
    };
    while *published < chunks.len() {
        publish_job(job, json!({"type":"text","content":chunks[*published]})).await;
        *published += 1;
    }
}

async fn call_agent_provider(
    client: &Client,
    endpoint: &str,
    api_key: &str,
    model: &str,
    messages: &[Value],
    tools: &Value,
    prompt_cache_key: Option<&str>,
    stream_job: Option<&Arc<AiJob>>,
) -> Result<ProviderCallResult, String> {
    let mut payload = json!({
        "model": model,
        "stream": true,
        "messages": messages.iter().map(|message| {
            let mut message = message.clone();
            if let Value::Object(object) = &mut message {
                object.remove("metadata");
            }
            message
        }).collect::<Vec<_>>(),
        "tools": tools,
        "tool_choice": "auto",
    });
    if let Some(key) = prompt_cache_key {
        payload["prompt_cache_key"] = Value::String(key.to_string());
    }
    let mut last_error = String::new();
    for attempt in 1..=AI_REQUEST_MAX_ATTEMPTS {
        let response = match client.post(endpoint).bearer_auth(api_key).json(&payload).send().await {
            Ok(response) => response,
            Err(error) => {
                last_error = format!("AI 请求失败: {error}");
                if attempt < AI_REQUEST_MAX_ATTEMPTS {
                    sleep(Duration::from_millis(400 * attempt as u64)).await;
                    continue;
                }
                return Err(last_error);
            }
        };
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            last_error = format!("AI 请求返回 {status}: {}", body.chars().take(500).collect::<String>());
            if attempt < AI_REQUEST_MAX_ATTEMPTS && should_retry_status(status) {
                sleep(Duration::from_millis(400 * attempt as u64)).await;
                continue;
            }
            return Err(last_error);
        }
        let mut body_stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut raw_body = String::new();
        let mut content = String::new();
        let mut text_chunks = Vec::new();
        let mut tool_calls = Vec::new();
        let mut usage = None;
        let mut published_text_chunks = 0usize;
        while let Some(chunk) = body_stream.next().await {
            let bytes = chunk.map_err(|error| format!("读取 AI 流失败: {error}"))?;
            let text = String::from_utf8_lossy(&bytes);
            raw_body.push_str(&text);
            buffer.push_str(&text);
            while let Some(index) = buffer.find('\n') {
                let line = buffer[..index].trim_end_matches('\r').to_string();
                buffer.drain(..=index);
                if let Some(data) = line.strip_prefix("data:") {
                    apply_agent_stream_chunk(data, &mut content, &mut text_chunks, &mut tool_calls, &mut usage);
                    publish_stream_text(stream_job, &text_chunks, &mut published_text_chunks).await;
                }
            }
        }
        if !buffer.trim().is_empty() {
            if let Some(data) = buffer.trim().strip_prefix("data:") {
                apply_agent_stream_chunk(data, &mut content, &mut text_chunks, &mut tool_calls, &mut usage);
                publish_stream_text(stream_job, &text_chunks, &mut published_text_chunks).await;
            }
        }
        if text_chunks.is_empty() && tool_calls.is_empty() {
            if let Ok(value) = serde_json::from_str::<Value>(&raw_body) {
                usage = provider_usage(&value);
                if let Some(message) = value.get("choices").and_then(Value::as_array).and_then(|items| items.first()).and_then(|item| item.get("message")).cloned() {
                    content = message.get("content").and_then(Value::as_str).unwrap_or("").to_string();
                    if !content.is_empty() {
                        text_chunks.push(content.clone());
                    }
                    tool_calls = message.get("tool_calls").and_then(Value::as_array).cloned().unwrap_or_default();
                }
            }
        }
        let mut message = json!({"role":"assistant","content":content});
        if !tool_calls.is_empty() {
            message["tool_calls"] = Value::Array(tool_calls);
        }
        if content.is_empty() && message.get("tool_calls").is_none() {
            return Err("AI 流响应缺少内容".to_string());
        }
        return Ok(ProviderCallResult { message, usage, text_chunks });
    }
    Err(last_error)
}

fn internal_error(error: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

#[derive(Debug, Serialize)]
pub struct AiSessionSummary {
    id: String,
    title: String,
    directory_path: Option<String>,
    file_path: Option<String>,
    active_job_id: Option<String>,
    current_context_tokens: Option<i64>,
    updated_at: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AiStoredMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<String>,
}

pub async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Claims { sub, .. }: Claims,
) -> Result<Json<Vec<AiSessionSummary>>, (StatusCode, String)> {
    let pool = state.db_pool.read().await;
    let rows = sqlx::query_as::<_, AiSessionSummaryRow>(
        "SELECT id, title, directory_path, file_path, active_job_id, context_tokens AS current_context_tokens, CAST(updated_at AS TEXT) AS updated_at FROM ai_sessions WHERE user_key = ? ORDER BY updated_at DESC"
    ).bind(sub).fetch_all(&*pool).await.map_err(internal_error)?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

#[derive(Debug, sqlx::FromRow)]
struct AiSessionSummaryRow {
    id: String,
    title: String,
    directory_path: Option<String>,
    file_path: Option<String>,
    active_job_id: Option<String>,
    current_context_tokens: Option<i64>,
    updated_at: String,
}
impl From<AiSessionSummaryRow> for AiSessionSummary {
    fn from(row: AiSessionSummaryRow) -> Self {
        Self {
            id: row.id,
            title: row.title,
            directory_path: row.directory_path,
            file_path: row.file_path,
            active_job_id: row.active_job_id,
            current_context_tokens: row.current_context_tokens,
            updated_at: row.updated_at,
        }
    }
}

pub async fn create_session(
    State(state): State<Arc<AppState>>,
    Claims { sub, .. }: Claims,
) -> Result<Json<AiSessionSummary>, (StatusCode, String)> {
    let id = uuid::Uuid::new_v4().to_string();
    let pool = state.db_pool.read().await;
    sqlx::query("INSERT INTO ai_sessions (id, user_key, title) VALUES (?, ?, '新会话')")
        .bind(&id)
        .bind(sub)
        .execute(&*pool)
        .await
        .map_err(internal_error)?;
    let row = sqlx::query_as::<_, AiSessionSummaryRow>("SELECT id, title, directory_path, file_path, active_job_id, context_tokens AS current_context_tokens, CAST(updated_at AS TEXT) AS updated_at FROM ai_sessions WHERE id = ?").bind(&id).fetch_one(&*pool).await.map_err(internal_error)?;
    Ok(Json(row.into()))
}

pub async fn get_session_messages(
    State(state): State<Arc<AppState>>,
    Claims { sub, .. }: Claims,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<AiStoredMessage>>, (StatusCode, String)> {
    let pool = state.db_pool.read().await;
    let owns: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM ai_sessions WHERE id = ? AND user_key = ?",
    )
    .bind(&session_id)
    .bind(sub)
    .fetch_optional(&*pool)
    .await
    .map_err(internal_error)?;
    if owns.is_none() {
        return Err((StatusCode::NOT_FOUND, "AI 会话不存在".into()));
    }
    let rows = sqlx::query_as::<_, AiStoredMessage>(
        "SELECT role, content, metadata FROM ai_messages WHERE session_id = ? ORDER BY id",
    )
    .bind(session_id)
    .fetch_all(&*pool)
    .await
    .map_err(internal_error)?;
    Ok(Json(rows))
}

#[derive(Debug, Serialize)]
pub struct AiCompressionResponse {
    messages: Vec<AiStoredMessage>,
    before_tokens: usize,
    after_tokens: usize,
    before_messages: usize,
    after_messages: usize,
    compressed: bool,
    token_source: String,
}

pub async fn compress_session(
    State(state): State<Arc<AppState>>,
    Claims { sub, .. }: Claims,
    Path(session_id): Path<String>,
) -> Result<Json<AiCompressionResponse>, (StatusCode, String)> {
    let rows = {
        let pool = state.db_pool.read().await;
        let owns: Option<(String,)> =
            sqlx::query_as("SELECT id FROM ai_sessions WHERE id = ? AND user_key = ?")
                .bind(&session_id)
                .bind(&sub)
                .fetch_optional(&*pool)
                .await
                .map_err(internal_error)?;
        if owns.is_none() {
            return Err((StatusCode::NOT_FOUND, "AI 会话不存在".into()));
        }
        sqlx::query_as::<_, AiStoredMessage>(
            "SELECT role, content, metadata FROM ai_messages WHERE session_id = ? ORDER BY id",
        )
        .bind(&session_id)
        .fetch_all(&*pool)
        .await
        .map_err(internal_error)?
    };

    if rows.len() < 2 {
        let before_tokens = rows
            .iter()
            .map(|row| stored_message_tokens(&row.content))
            .sum();
        let before_messages = rows.len();
        return Ok(Json(AiCompressionResponse {
            messages: rows,
            before_tokens,
            after_tokens: before_tokens,
            before_messages,
            after_messages: before_messages,
            compressed: false,
            token_source: "estimate".to_string(),
        }));
    }

    let config = state
        .config_service
        .get_ai_config()
        .await
        .map_err(internal_error)?;
    if !config.enabled || config.api_key.trim().is_empty() || config.model.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "AI 尚未配置，请先在系统配置中填写 Provider、API Key 和模型".into(),
        ));
    }
    let base_url = config.base_url.trim_end_matches('/');
    let endpoint = if base_url.ends_with("/chat/completions") {
        base_url.to_string()
    } else {
        format!("{base_url}/chat/completions")
    };
    let client = Client::builder().build().map_err(internal_error)?;
    let mut messages = vec![json!({"role":"system","content":""})];
    messages.extend(
        rows.iter()
            .map(|message| json!({"role": message.role, "content": message.content})),
    );
    let before_tokens_estimate = rows
        .iter()
        .map(|row| stored_message_tokens(&row.content))
        .sum::<usize>();
    let compression_outcome = match compress_json_messages(
        &client,
        &endpoint,
        &config.api_key,
        &config.model,
        &mut messages,
        &config,
        true,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!("手动上下文压缩调用 AI 失败，使用本地降级压缩: {error}");
            force_local_compact_json_messages(&mut messages, &config);
            CompressionOutcome {
                changed: true,
                provider_usage: None,
                request_estimate: 0,
                source_estimate: 0,
            }
        }
    };

    let compressed: Vec<AiStoredMessage> = messages
        .into_iter()
        .filter_map(|message| {
            let role = message.get("role").and_then(Value::as_str)?;
            let content = message.get("content").and_then(Value::as_str)?.to_string();
            if content.trim().is_empty()
                || (role == "system" && !content.starts_with("[历史上下文压缩摘要]"))
            {
                return None;
            }
            let role = if role == "system" {
                "assistant".to_string()
            } else {
                role.to_string()
            };
            Some(AiStoredMessage { role, content, metadata: None })
        })
        .collect();

    let pool = state.db_pool.read().await;
    sqlx::query("DELETE FROM ai_messages WHERE session_id = ?")
        .bind(&session_id)
        .execute(&*pool)
        .await
        .map_err(internal_error)?;
    for message in &compressed {
        sqlx::query("INSERT INTO ai_messages (session_id, role, content, metadata) VALUES (?, ?, ?, ?)")
            .bind(&session_id)
            .bind(&message.role)
            .bind(&message.content)
            .bind(&message.metadata)
            .execute(&*pool)
            .await
            .map_err(internal_error)?;
    }
    sqlx::query(
        "UPDATE ai_sessions SET updated_at = CURRENT_TIMESTAMP WHERE id = ? AND user_key = ?",
    )
    .bind(&session_id)
    .bind(&sub)
    .execute(&*pool)
    .await
    .map_err(internal_error)?;

    let after_tokens_estimate = compressed
        .iter()
        .map(|row| stored_message_tokens(&row.content))
        .sum::<usize>();
    let (before_tokens, after_tokens, token_source) = match compression_outcome.provider_usage {
        Some(usage) if usage.prompt_tokens.is_some() || usage.completion_tokens.is_some() => {
            let scale = usage
                .prompt_tokens
                .filter(|_| compression_outcome.request_estimate > 0)
                .map(|actual| {
                    (actual as f64 / compression_outcome.request_estimate as f64).clamp(0.5, 3.0)
                });
            let calibrated_before = scale
                .map(|factor| (before_tokens_estimate as f64 * factor).round() as usize)
                .unwrap_or(before_tokens_estimate);
            let calibrated_after = usage
                .completion_tokens
                .map(|tokens| tokens + 4)
                .unwrap_or_else(|| {
                    scale
                        .map(|factor| (after_tokens_estimate as f64 * factor).round() as usize)
                        .unwrap_or(after_tokens_estimate)
                });
            (
                calibrated_before,
                calibrated_after,
                "provider+estimate".to_string(),
            )
        }
        _ => (
            before_tokens_estimate,
            after_tokens_estimate,
            "estimate".to_string(),
        ),
    };
    let before_messages = rows.len();
    let after_messages = compressed.len();
    Ok(Json(AiCompressionResponse {
        messages: compressed,
        before_tokens,
        after_tokens,
        before_messages,
        after_messages,
        compressed: compression_outcome.changed && after_tokens < before_tokens,
        token_source,
    }))
}

pub async fn delete_session(
    State(state): State<Arc<AppState>>,
    Claims { sub, .. }: Claims,
    Path(session_id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let pool = state.db_pool.read().await;
    let active_job: Option<(Option<String>,)> =
        sqlx::query_as("SELECT active_job_id FROM ai_sessions WHERE id = ? AND user_key = ?")
            .bind(&session_id)
            .bind(&sub)
            .fetch_optional(&*pool)
            .await
            .map_err(internal_error)?;

    let Some((active_job_id,)) = active_job else {
        return Err((StatusCode::NOT_FOUND, "AI 会话不存在".into()));
    };

    if let Some(job_id) = active_job_id {
        if let Some(job) = AI_JOBS.read().await.get(&job_id).cloned() {
            job.cancel.cancel();
        }
    }

    sqlx::query("DELETE FROM ai_messages WHERE session_id = ?")
        .bind(&session_id)
        .execute(&*pool)
        .await
        .map_err(internal_error)?;
    sqlx::query("DELETE FROM ai_sessions WHERE id = ? AND user_key = ?")
        .bind(&session_id)
        .bind(&sub)
        .execute(&*pool)
        .await
        .map_err(internal_error)?;

    Ok(StatusCode::NO_CONTENT)
}

async fn set_session_context_tokens(
    state: &AppState,
    session_id: Option<&str>,
    user_key: &str,
    tokens: usize,
) {
    let Some(session_id) = session_id.filter(|value| !value.is_empty()) else {
        return;
    };
    let pool = state.db_pool.read().await;
    let _ = sqlx::query("UPDATE ai_sessions SET context_tokens = ? WHERE id = ? AND user_key = ?")
        .bind(tokens as i64)
        .bind(session_id)
        .bind(user_key)
        .execute(&*pool)
        .await;
}

async fn set_session_job(
    state: &AppState,
    session_id: Option<&str>,
    user_key: &str,
    job_id: Option<&str>,
) {
    let Some(session_id) = session_id.filter(|value| !value.is_empty()) else {
        return;
    };
    let pool = state.db_pool.read().await;
    let _ = sqlx::query("UPDATE ai_sessions SET active_job_id = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ? AND user_key = ?")
        .bind(job_id).bind(session_id).bind(user_key).execute(&*pool).await;
}

fn agent_history_for_client(messages: &[Value]) -> Vec<Value> {
    messages.iter().filter_map(|message| {
        let role = message.get("role").and_then(Value::as_str)?;
        if !matches!(role, "user" | "assistant" | "tool") { return None; }
        let metadata = message.get("metadata");
        let runtime = metadata.and_then(|value| value.get("runtime_context")).and_then(Value::as_bool) == Some(true);
        let raw_content = message.get("content").and_then(Value::as_str).unwrap_or("");
        let content = if runtime {
            raw_content.split("\n\n当前请求:\n").nth(1).unwrap_or("")
        } else {
            raw_content
        };
        let mut output = json!({"role": role, "content": content});
        let protocol = json!({
            "tool_calls": message.get("tool_calls"),
            "tool_call_id": message.get("tool_call_id"),
            "name": message.get("name"),
        });
        if protocol.get("tool_calls").is_some() || protocol.get("tool_call_id").is_some() || protocol.get("name").is_some() {
            output["metadata"] = Value::String(protocol.to_string());
        }
        if output["content"].as_str().unwrap_or("").trim().is_empty() && output.get("metadata").is_none() { return None; }
        Some(output)
    }).collect()
}

async fn store_agent_protocol_messages(
    state: &AppState,
    session_id: Option<&str>,
    user_key: &str,
    messages: &[Value],
) -> bool {
    let Some(session_id) = session_id.filter(|value| !value.is_empty()) else {
        return false;
    };
    let pool = state.db_pool.read().await;
    let _ = sqlx::query("DELETE FROM ai_messages WHERE session_id = ?")
        .bind(session_id)
        .execute(&*pool)
        .await;
    for message in messages {
        let Some(role) = message.get("role").and_then(Value::as_str) else { continue; };
        if !matches!(role, "user" | "assistant" | "tool") { continue; }
        let content = message.get("content").and_then(Value::as_str).unwrap_or("");
        let is_runtime_context = message.get("metadata").and_then(|value| value.get("runtime_context")).and_then(Value::as_bool) == Some(true);
        let persisted_content = if is_runtime_context {
            message.get("content").and_then(Value::as_str).and_then(|value| value.split("\n\n当前请求:\n").nth(1)).unwrap_or("")
        } else {
            content
        };
        let has_protocol_metadata = message.get("tool_calls").is_some()
            || message.get("tool_call_id").is_some()
            || message.get("name").is_some();
        if persisted_content.trim().is_empty() && !has_protocol_metadata { continue; }
        let metadata = serde_json::to_string(&json!({
            "tool_calls": message.get("tool_calls"),
            "tool_call_id": message.get("tool_call_id"),
            "name": message.get("name"),
        })).ok();
        let _ = sqlx::query("INSERT INTO ai_messages (session_id, role, content, metadata) SELECT ?, ?, ?, ? WHERE EXISTS (SELECT 1 FROM ai_sessions WHERE id = ? AND user_key = ?)")
            .bind(session_id).bind(role).bind(persisted_content).bind(metadata).bind(session_id).bind(user_key)
            .execute(&*pool).await;
    }
    let _ = sqlx::query("UPDATE ai_sessions SET updated_at = CURRENT_TIMESTAMP WHERE id = ? AND user_key = ?")
        .bind(session_id).bind(user_key).execute(&*pool).await;
    true
}

async fn store_session_message(
    state: &AppState,
    session_id: Option<&str>,
    user_key: &str,
    role: &str,
    content: &str,
) -> Option<String> {
    let Some(session_id) = session_id.filter(|value| !value.is_empty()) else {
        return None;
    };
    let pool = state.db_pool.read().await;
    let _ = sqlx::query("INSERT INTO ai_messages (session_id, role, content) SELECT ?, ?, ? WHERE EXISTS (SELECT 1 FROM ai_sessions WHERE id = ? AND user_key = ?)")
        .bind(session_id).bind(role).bind(content).bind(session_id).bind(user_key).execute(&*pool).await;
    if role == "user" {
        let trimmed = content.trim();
        let first_line = trimmed.split(['\n', '\r']).next().unwrap_or("").trim();
        let end = first_line
            .char_indices()
            .find(|(_, character)| matches!(character, '。' | '！' | '？' | '.' | '!' | '?'))
            .map(|(index, character)| index + character.len_utf8())
            .unwrap_or(first_line.len());
        let title: String = first_line[..end].chars().take(40).collect();
        let title = if title.trim().is_empty() {
            "未命名会话".to_string()
        } else {
            title
        };
        let changed = sqlx::query(
            "UPDATE ai_sessions SET title = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ? AND user_key = ? AND title = '新会话' AND NOT EXISTS (SELECT 1 FROM ai_messages WHERE session_id = ? AND role = 'user' AND id < (SELECT MAX(id) FROM ai_messages WHERE session_id = ? AND role = 'user'))",
        )
        .bind(&title).bind(session_id).bind(user_key).bind(session_id).bind(session_id)
        .execute(&*pool).await
        .map(|result| result.rows_affected() > 0)
        .unwrap_or(false);
        if changed {
            return Some(title);
        }
    } else {
        let _ = sqlx::query(
            "UPDATE ai_sessions SET updated_at = CURRENT_TIMESTAMP WHERE id = ? AND user_key = ?",
        )
        .bind(session_id)
        .bind(user_key)
        .execute(&*pool)
        .await;
    }
    None
}

pub async fn config(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let value = state
        .config_service
        .get_ai_config()
        .await
        .map_err(internal_error)?;
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
    let value = value.normalized();
    state
        .config_service
        .update_ai_config(&value)
        .await
        .map_err(internal_error)?;
    Ok(Json(AiConfigResponse::from(value)))
}
