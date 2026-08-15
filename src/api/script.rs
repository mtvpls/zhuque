use crate::api::AppState;
use axum::{
    body::Bytes,
    extract::{Multipart, Path, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use futures::stream::{Stream, StreamExt};
use serde::Deserialize;
use std::convert::Infallible;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct ListQuery {
    pub path: Option<String>,
}

#[derive(Deserialize)]
pub struct UploadQuery {
    pub path: Option<String>,
}

#[derive(Deserialize)]
pub struct RenameRequest {
    pub new_path: String,
}

#[derive(Deserialize)]
pub struct CopyRequest {
    pub target_path: String,
}

#[derive(Deserialize)]
pub struct ExecuteContentRequest {
    pub content: String,
    pub script_type: String, // sh, py, js
    pub env: Option<String>, // JSON格式的环境变量，如 {"KEY":"value"}
    pub file_path: Option<String>, // 当前文件路径，用于设置工作目录
}

pub async fn list_scripts(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(query): axum::extract::Query<ListQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    let path = query.path.as_deref().unwrap_or("");
    let files = state.script_service
        .list_dir(path)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(files))
}

pub async fn get_script(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    let content = state.script_service
        .read(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(content)
}

pub async fn update_script(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
    body: Bytes,
) -> Result<impl IntoResponse, StatusCode> {
    let content = String::from_utf8(body.to_vec()).map_err(|_| StatusCode::BAD_REQUEST)?;

    state.script_service
        .write(&path, &content)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_script(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    state.script_service
        .delete(&path)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn upload_script(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, StatusCode> {
    let mut target_path: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| {
            tracing::error!("Failed to get next field: {:?}", e);
            StatusCode::BAD_REQUEST
        })?
    {
        let field_name = field.name().map(|s| s.to_string());

        // 如果是path字段，保存目标路径
        if field_name.as_deref() == Some("path") {
            target_path = Some(
                field
                    .text()
                    .await
                    .map_err(|e| {
                        tracing::error!("Failed to read path field: {:?}", e);
                        StatusCode::BAD_REQUEST
                    })?,
            );
            continue;
        }

        // 处理文件字段
        let file_name = field
            .file_name()
            .map(|s| s.to_string())
            .ok_or_else(|| {
                tracing::error!("No file name in upload");
                StatusCode::BAD_REQUEST
            })?;

        tracing::info!("Uploading file: {}", file_name);

        let content = field
            .bytes()
            .await
            .map_err(|e| {
                tracing::error!("Failed to read file content: {:?}", e);
                StatusCode::BAD_REQUEST
            })?;

        // 确定最终路径
        let final_path = if let Some(ref path) = target_path {
            // 如果指定了路径，使用指定路径
            if path.is_empty() || path.ends_with('/') {
                format!("{}{}", path, file_name)
            } else {
                format!("{}/{}", path, file_name)
            }
        } else {
            // 否则直接使用文件名
            file_name
        };

        tracing::info!("Writing to path: {}", final_path);

        state
            .script_service
            .write_bytes(&final_path, &content)
            .await
            .map_err(|e| {
                tracing::error!("Failed to write file: {:?}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
    }

    Ok(StatusCode::CREATED)
}

/// 上传并解压压缩包（支持 .zip, .tar.gz, .tar）
pub async fn upload_archive(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, StatusCode> {
    let mut target_path: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
    {
        let field_name = field.name().map(|s| s.to_string());

        // 如果是path字段，保存目标路径
        if field_name.as_deref() == Some("path") {
            target_path = Some(
                field
                    .text()
                    .await
                    .map_err(|_| StatusCode::BAD_REQUEST)?,
            );
            continue;
        }

        // 处理文件字段
        let file_name = field
            .file_name()
            .map(|s| s.to_string())
            .ok_or(StatusCode::BAD_REQUEST)?;

        // 获取文件数据（二进制）
        let data = field
            .bytes()
            .await
            .map_err(|_| StatusCode::BAD_REQUEST)?;

        // 检查文件类型
        let is_zip = file_name.ends_with(".zip");
        let is_tar_gz = file_name.ends_with(".tar.gz") || file_name.ends_with(".tgz");
        let is_tar = file_name.ends_with(".tar");

        if !is_zip && !is_tar_gz && !is_tar {
            return Err(StatusCode::BAD_REQUEST);
        }

        // 解压到目标目录
        let extract_path = target_path.as_deref().unwrap_or("");

        if is_zip {
            state
                .script_service
                .extract_zip(&data, extract_path)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        } else if is_tar_gz {
            state
                .script_service
                .extract_tar_gz(&data, extract_path)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        } else if is_tar {
            state
                .script_service
                .extract_tar(&data, extract_path)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
    }

    Ok(StatusCode::CREATED)
}

pub async fn create_directory(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .script_service
        .create_directory(&path)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::CREATED)
}

pub async fn delete_directory(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .script_service
        .delete_directory(&path)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn rename_script(
    State(state): State<Arc<AppState>>,
    Path(old_path): Path<String>,
    Json(payload): Json<RenameRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .script_service
        .rename(&old_path, &payload.new_path)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn copy_script(
    State(state): State<Arc<AppState>>,
    Path(source_path): Path<String>,
    Json(payload): Json<CopyRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .script_service
        .copy(&source_path, &payload.target_path)
        .await
        .map_err(|e| {
            tracing::error!("Copy failed: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(StatusCode::NO_CONTENT)
}

/// 执行已保存的脚本（SSE流式返回）
pub async fn execute_script(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    let started_at = std::time::Instant::now();
    let (execution_id, stream) = state
        .script_service
        .execute_script(&path, None)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // 首先发送execution_id
    let sse_stream = async_stream::stream! {
        yield Ok(Event::default().event("execution_id").data(execution_id));

        let mut s = Box::pin(stream);
        let mut status = "success";
        while let Some(result) = s.next().await {
            match result {
                Ok(line) => {
                    if line.starts_with("[EXIT] Process exited with code ") && !line.ends_with("code 0") {
                        status = "failed";
                    }
                    yield Ok(Event::default().data(line));
                }
                Err(e) => {
                    status = "failed";
                    yield Ok(Event::default().data(format!("[ERROR] {}", e)));
                }
            }
        }
        yield Ok(Event::default().data(format!(
            "[NOTIFY] 模拟发送通知: 脚本运行完成，结果={}，耗时={}ms",
            status,
            started_at.elapsed().as_millis()
        )));
    };

    Ok(Sse::new(sse_stream).keep_alive(KeepAlive::default()))
}

/// 执行临时脚本内容（用于调试，SSE流式返回）
pub async fn execute_content(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ExecuteContentRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    let started_at = std::time::Instant::now();
    let (execution_id, stream) = state
        .script_service
        .execute_content(&payload.content, &payload.script_type, payload.env.as_deref(), payload.file_path.as_deref())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // 首先发送execution_id
    let sse_stream = async_stream::stream! {
        yield Ok(Event::default().event("execution_id").data(execution_id));

        let mut s = Box::pin(stream);
        let mut status = "success";
        while let Some(result) = s.next().await {
            match result {
                Ok(line) => {
                    if line.starts_with("[EXIT] Process exited with code ") && !line.ends_with("code 0") {
                        status = "failed";
                    }
                    yield Ok(Event::default().data(line));
                }
                Err(e) => {
                    status = "failed";
                    yield Ok(Event::default().data(format!("[ERROR] {}", e)));
                }
            }
        }
        yield Ok(Event::default().data(format!(
            "[NOTIFY] 模拟发送通知: 脚本调试完成，结果={}，耗时={}ms",
            status,
            started_at.elapsed().as_millis()
        )));
    };

    Ok(Sse::new(sse_stream).keep_alive(KeepAlive::default()))
}

/// 中止正在执行的脚本
pub async fn kill_execution(
    State(state): State<Arc<AppState>>,
    Path(execution_id): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .script_service
        .kill_execution(&execution_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(StatusCode::NO_CONTENT)
}

/// 列出正在执行的脚本
pub async fn list_running(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, StatusCode> {
    let running = state.script_service.list_running().await;
    Ok(Json(running))
}
