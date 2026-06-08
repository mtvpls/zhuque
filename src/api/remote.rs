use crate::api::AppState;
use crate::models::{
    CreateRemoteAgentRequest, CreateRemoteCommandRequest, RegisterAgentRequest, RemoteAgentMessage,
    RemoteMoveFileRequest, RemotePathRequest, RemoteWriteFileRequest, RunRemoteScriptRequest,
};
use axum::{
    extract::{
        ws::{Message, WebSocket},
        Path, Query, State, WebSocketUpgrade,
    },
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use futures::{SinkExt, Stream, StreamExt};
use serde::Deserialize;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Deserialize)]
pub struct AgentConnectQuery {
    pub agent_id: i64,
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct ListCommandsQuery {
    pub agent_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct FileQuery {
    pub path: Option<String>,
}

pub async fn register_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<RegisterAgentRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let token = headers
        .get("x-register-token")
        .and_then(|v| v.to_str().ok());

    state
        .remote_service
        .register_agent(token, payload)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        })
}

pub async fn connect_agent(
    ws: WebSocketUpgrade,
    Query(query): Query<AgentConnectQuery>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_agent_socket(socket, state, query))
}

async fn handle_agent_socket(socket: WebSocket, state: Arc<AppState>, query: AgentConnectQuery) {
    if let Err(e) = state
        .remote_service
        .authenticate_agent(query.agent_id, &query.token)
        .await
    {
        tracing::warn!("Remote agent auth failed: {}", e);
        return;
    }

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let session_id = match state
        .remote_service
        .attach_session(query.agent_id, tx, None)
        .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("Failed to attach remote session: {}", e);
            return;
        }
    };

    let write_task = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            match serde_json::to_string(&message) {
                Ok(text) => {
                    if ws_sender.send(Message::Text(text)).await.is_err() {
                        break;
                    }
                }
                Err(e) => tracing::error!("Failed to encode remote server message: {}", e),
            }
        }
    });

    let read_state = state.clone();
    let read_session_id = session_id.clone();
    let read_task = tokio::spawn(async move {
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    match serde_json::from_str::<RemoteAgentMessage>(&text) {
                        Ok(message) => {
                            if let Err(e) = read_state
                                .remote_service
                                .handle_agent_message(query.agent_id, message)
                                .await
                            {
                                tracing::error!("Failed to handle agent message: {}", e);
                            }
                        }
                        Err(e) => tracing::warn!("Invalid agent message: {}", e),
                    }
                }
                Ok(Message::Close(_)) => break,
                Err(e) => {
                    tracing::warn!("Remote agent websocket error: {}", e);
                    break;
                }
                _ => {}
            }
        }

        if let Err(e) = read_state
            .remote_service
            .detach_session(query.agent_id, &read_session_id)
            .await
        {
            tracing::error!("Failed to detach remote session: {}", e);
        }
    });

    tokio::select! {
        _ = write_task => {}
        _ = read_task => {}
    }
}

pub async fn list_agents(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .remote_service
        .list_agents()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn create_agent(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateRemoteAgentRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .remote_service
        .create_agent(payload)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to create remote agent: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

pub async fn get_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    let agent = state
        .remote_service
        .get_agent(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(agent))
}

pub async fn create_command(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<i64>,
    Json(payload): Json<CreateRemoteCommandRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .remote_service
        .create_command(agent_id, payload, None)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to create remote command: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

pub async fn run_script(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<i64>,
    Json(payload): Json<RunRemoteScriptRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let content = state
        .script_service
        .read(&payload.path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let script_name = std::path::Path::new(&payload.path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("remote_script.sh")
        .to_string();
    let command = payload.command.unwrap_or_default();

    state
        .remote_service
        .create_script_command(
            agent_id,
            script_name,
            content,
            command,
            payload.working_dir,
            payload.env,
            payload.timeout,
        )
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to run remote script: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

pub async fn get_agent_status(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .remote_service
        .request_status(agent_id)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to get remote status: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

pub async fn list_files(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<i64>,
    Query(query): Query<FileQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .remote_service
        .list_files(agent_id, query.path.unwrap_or_else(|| ".".to_string()))
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to list remote files: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

pub async fn read_file(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<i64>,
    Query(query): Query<FileQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    let path = query.path.ok_or(StatusCode::BAD_REQUEST)?;
    state
        .remote_service
        .read_file(agent_id, path)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to read remote file: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

pub async fn write_file(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<i64>,
    Json(payload): Json<RemoteWriteFileRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .remote_service
        .write_file(agent_id, payload.path, payload.content)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to write remote file: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

pub async fn delete_file(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<i64>,
    Json(payload): Json<RemotePathRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .remote_service
        .delete_file(agent_id, payload.path)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to delete remote file: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

pub async fn create_dir(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<i64>,
    Json(payload): Json<RemotePathRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .remote_service
        .create_dir(agent_id, payload.path)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to create remote directory: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

pub async fn rename_file(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<i64>,
    Json(payload): Json<RemoteMoveFileRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .remote_service
        .rename_file(agent_id, payload.from, payload.to)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to rename remote file: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

pub async fn list_commands(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListCommandsQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .remote_service
        .list_commands(query.agent_id)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn get_command(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    let command = state
        .remote_service
        .get_command(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(command))
}

pub async fn kill_command(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .remote_service
        .kill_command(&id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_command_logs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .remote_service
        .list_command_logs(&id)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn stream_command_logs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    let history = state
        .remote_service
        .list_command_logs(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut rx = state.remote_service.subscribe_logs(&id).await;

    let stream = async_stream::stream! {
        for log in history {
            yield Ok(Event::default().data(format!("[{}] {}", log.stream, log.line)));
        }

        while let Ok(line) = rx.recv().await {
            yield Ok(Event::default().data(line));
        }
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
