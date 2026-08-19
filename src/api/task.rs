use crate::api::AppState;
use crate::models::{CreateTask, UpdateTask};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use futures::stream::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct ListTasksQuery {
    /// 字段过滤：simple=只返回id和name，默认返回全部字段
    fields: Option<String>,
    /// 关键字搜索（按名称或命令模糊匹配）
    search: Option<String>,
}

#[derive(Debug, Serialize)]
struct SimpleTask {
    id: i64,
    name: String,
    enabled: bool,
}

pub async fn list_tasks(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListTasksQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    let tasks = state
        .task_service
        .list_with_search(query.search.as_deref())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // 根据 fields 参数决定返回格式
    match query.fields.as_deref() {
        Some("simple") => {
            let simple_tasks: Vec<SimpleTask> = tasks
                .into_iter()
                .map(|t| SimpleTask {
                    id: t.id,
                    name: t.name,
                    enabled: t.enabled,
                })
                .collect();
            Ok(Json(serde_json::json!(simple_tasks)))
        }
        _ => Ok(Json(serde_json::json!(tasks))),
    }
}

pub async fn get_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    let task = state
        .task_service
        .get(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(task))
}

pub async fn create_task(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateTask>,
) -> Result<impl IntoResponse, StatusCode> {
    let task = state
        .task_service
        .create(payload)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    state
        .scheduler
        .add_task_to_scheduler(task.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let task = state
        .task_service
        .get(task.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok((StatusCode::CREATED, Json(task)))
}

pub async fn update_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateTask>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .task_service
        .update(id, payload)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // 只更新当前任务，避免重载过程中返回旧的 next_run_at
    state
        .scheduler
        .update_task_in_scheduler(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let task = state
        .task_service
        .get(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(task))
}

pub async fn delete_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    let deleted = state
        .task_service
        .delete(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !deleted {
        return Err(StatusCode::NOT_FOUND);
    }

    state
        .scheduler
        .remove_task_from_scheduler(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn run_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .scheduler
        .run_task_now(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::ACCEPTED)
}
pub async fn kill_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .scheduler
        .kill_task(id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(StatusCode::NO_CONTENT)
}

/// 列出正在执行的任务
pub async fn list_running_tasks(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, StatusCode> {
    let running = state.scheduler.list_running().await;
    Ok(Json(running))
}

/// 订阅正在执行的任务（SSE）
pub async fn subscribe_running_tasks(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.scheduler.subscribe_running_tasks_with_data().await;

    // 立即发送当前状态
    let initial_running = state.scheduler.list_running().await;
    let initial_update = serde_json::json!({
        "running_ids": initial_running,
        "changed_task_id": 0,
        "change_type": "initial"
    });
    let initial_json = serde_json::to_string(&initial_update).unwrap_or_else(|_| "{}".to_string());

    let stream = async_stream::stream! {
        // 首次推送当前状态
        yield Ok(Event::default().data(initial_json));

        // 订阅后续变化
        loop {
            match rx.recv().await {
                Ok(update) => {
                    let json = serde_json::to_string(&update).unwrap_or_else(|_| "{}".to_string());
                    yield Ok(Event::default().data(json));
                }
                Err(_) => {
                    // channel 关闭，结束流
                    break;
                }
            }
        }
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// 列出所有活跃的执行
pub async fn list_executions(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, StatusCode> {
    let executions = state.scheduler.list_executions().await;
    Ok(Json(executions))
}

/// 订阅执行日志（SSE）
pub async fn subscribe_execution_logs(
    State(state): State<Arc<AppState>>,
    Path(execution_id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    // 获取执行信息（包含开始时间）
    let execution_info = state.scheduler.get_execution(&execution_id).await;

    // 获取历史日志
    let history = state.scheduler.get_log_history(&execution_id).await;

    // 订阅新日志
    let mut rx = state
        .scheduler
        .subscribe_logs(&execution_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    let sse_stream = async_stream::stream! {
        // 先发送任务开始时间
        if let Some(info) = execution_info {
            let local_time = info.started_at.with_timezone(&chrono::Local);
            let start_time = local_time.format("%Y-%m-%d %H:%M:%S").to_string();
            yield Ok(Event::default().data(format!("[任务开始时间: {}]", start_time)));
        }

        // 发送历史日志
        for line in history {
            yield Ok(Event::default().data(line));
        }

        // 发送实时日志
        while let Ok(line) = rx.recv().await {
            yield Ok(Event::default().data(line));
        }
    };

    Ok(Sse::new(sse_stream).keep_alive(KeepAlive::default()))
}

/// Webhook触发任务
pub async fn webhook_trigger_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .scheduler
        .run_task_now(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "message": "Task triggered successfully",
            "task_id": id
        }))
    ))
}
