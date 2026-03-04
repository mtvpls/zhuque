// 非 Android 平台：完整的终端功能
#[cfg(not(target_os = "android"))]
pub use terminal_impl::*;

#[cfg(not(target_os = "android"))]
mod terminal_impl {
    use crate::api::AppState;
    use axum::{
        extract::{
            ws::{Message, WebSocket},
            Query, State, WebSocketUpgrade,
        },
        response::IntoResponse,
    };
    use futures::{sink::SinkExt, stream::StreamExt};
    use serde::{Deserialize, Serialize};
    use std::io::{Read, Write};
    use std::sync::Arc;

    #[derive(Debug, Deserialize)]
    pub struct ConnectQuery {
        pub token: Option<String>,
    }

    #[derive(Debug, Deserialize, Serialize)]
    #[serde(tag = "type")]
    enum TerminalMessage {
        #[serde(rename = "input")]
        Input { data: String },
        #[serde(rename = "resize")]
        Resize { rows: u16, cols: u16 },
    }

    pub async fn connect_terminal(
        ws: WebSocketUpgrade,
        Query(_query): Query<ConnectQuery>,
        State(state): State<Arc<AppState>>,
    ) -> impl IntoResponse {
        ws.on_upgrade(move |socket| handle_socket(socket, state))
    }

    async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
        let (mut sender, mut receiver) = socket.split();

        // 创建终端会话
        let (session_id, mut reader, mut writer, master) = match state
            .terminal_service
            .create_session(std::collections::HashMap::new(), 24, 80)
            .await
        {
            Ok(session) => session,
            Err(e) => {
                tracing::error!("Failed to create terminal session: {}", e);
                let _ = sender
                    .send(Message::Text(format!("Error: {}", e)))
                    .await;
                return;
            }
        };

        tracing::info!("Terminal session created: {}", session_id);

        // 发送会话 ID
        if let Err(e) = sender
            .send(Message::Text(format!(
                r#"{{"type":"session","id":"{}"}}"#,
                session_id
            )))
            .await
        {
            tracing::error!("Failed to send session ID: {}", e);
            return;
        }

        // 从 PTY 读取并发送到 WebSocket
        let session_id_clone = session_id.clone();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(100);

        // 在 blocking 线程中读取 PTY
        tokio::task::spawn_blocking(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(n) if n > 0 => {
                        let data = buffer[..n].to_vec();
                        if tx.blocking_send(data).is_err() {
                            tracing::error!("Failed to send data through channel");
                            break;
                        }
                    }
                    Ok(_) => {
                        tracing::info!("PTY reader closed for session: {}", session_id_clone);
                        break;
                    }
                    Err(e) => {
                        tracing::error!("Failed to read from PTY: {}", e);
                        break;
                    }
                }
            }
        });

        // 从 channel 接收数据并发送到 WebSocket
        let session_id_clone2 = session_id.clone();
        let read_task = tokio::spawn(async move {
            while let Some(data) = rx.recv().await {
                if let Err(e) = sender.send(Message::Binary(data)).await {
                    tracing::error!("Failed to send to websocket: {}", e);
                    break;
                }
            }
            tracing::info!("Read task finished for session: {}", session_id_clone2);
        });

        // 从 WebSocket 读取并写入到 PTY
        let session_id_clone = session_id.clone();
        let state_clone = state.clone();
        let write_task = tokio::spawn(async move {
            while let Some(msg) = receiver.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        // 尝试解析为 JSON 消息
                        if let Ok(terminal_msg) = serde_json::from_str::<TerminalMessage>(&text) {
                            match terminal_msg {
                                TerminalMessage::Input { data } => {
                                    if let Err(e) = writer.write_all(data.as_bytes()) {
                                        tracing::error!("Failed to write to PTY: {}", e);
                                        break;
                                    }
                                    if let Err(e) = writer.flush() {
                                        tracing::error!("Failed to flush PTY: {}", e);
                                        break;
                                    }
                                }
                                TerminalMessage::Resize { rows, cols } => {
                                    let master_lock = master.lock().await;
                                    if let Err(e) = master_lock.resize(portable_pty::PtySize {
                                        rows,
                                        cols,
                                        pixel_width: 0,
                                        pixel_height: 0,
                                    }) {
                                        tracing::error!("Failed to resize PTY: {}", e);
                                    }
                                }
                            }
                        } else {
                            // 如果不是 JSON，直接作为输入
                            if let Err(e) = writer.write_all(text.as_bytes()) {
                                tracing::error!("Failed to write to PTY: {}", e);
                                break;
                            }
                            if let Err(e) = writer.flush() {
                                tracing::error!("Failed to flush PTY: {}", e);
                                break;
                            }
                        }
                    }
                    Ok(Message::Binary(data)) => {
                        if let Err(e) = writer.write_all(&data) {
                            tracing::error!("Failed to write to PTY: {}", e);
                            break;
                        }
                        if let Err(e) = writer.flush() {
                            tracing::error!("Failed to flush PTY: {}", e);
                            break;
                        }
                    }
                    Ok(Message::Close(_)) => {
                        tracing::info!("WebSocket closed for session: {}", session_id_clone);
                        break;
                    }
                    Err(e) => {
                        tracing::error!("WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }

            // 清理会话
            if let Err(e) = state_clone
                .terminal_service
                .remove_session(&session_id_clone)
                .await
            {
                tracing::error!("Failed to remove session: {}", e);
            }
        });

        // 等待任务完成
        tokio::select! {
            _ = read_task => {
                tracing::info!("Read task finished for session: {}", session_id);
            }
            _ = write_task => {
                tracing::info!("Write task finished for session: {}", session_id);
            }
        }
    }
}

// Android 平台：返回不支持的错误
#[cfg(target_os = "android")]
pub use terminal_stub::*;

#[cfg(target_os = "android")]
mod terminal_stub {
    use crate::api::AppState;
    use axum::{
        extract::{Query, State, WebSocketUpgrade},
        http::StatusCode,
        response::IntoResponse,
        Json,
    };
    use serde::Deserialize;
    use std::sync::Arc;

    #[derive(Debug, Deserialize)]
    pub struct ConnectQuery {
        pub token: Option<String>,
    }

    pub async fn connect_terminal(
        _ws: WebSocketUpgrade,
        Query(_query): Query<ConnectQuery>,
        State(_state): State<Arc<AppState>>,
    ) -> impl IntoResponse {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(serde_json::json!({
                "error": "Terminal is not supported on Android platform"
            })),
        )
    }
}
