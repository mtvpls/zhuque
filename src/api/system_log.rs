use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, sse::Event, Sse},
    Json,
};
use futures::stream::{self, Stream};
use serde_json::json;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::StreamExt as _;

use crate::api::AppState;

pub async fn get_system_logs(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let logs = state.system_log_collector.get_logs();

    Ok(Json(json!({
        "logs": logs
    })))
}

pub async fn stream_system_logs(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let collector = state.system_log_collector.clone();

    let stream = stream::unfold((), move |_| {
        let collector = collector.clone();
        async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let logs = collector.get_logs();
            let data = serde_json::to_string(&logs).unwrap_or_default();
            Some((Ok(Event::default().data(data)), ()))
        }
    });

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}
