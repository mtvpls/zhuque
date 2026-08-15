use crate::api::AppState;
use crate::models::{CreateSubscription, UpdateSubscription};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

pub async fn list_subscriptions(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, StatusCode> {
    let subs = state
        .subscription_service
        .list()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(subs))
}

pub async fn get_subscription(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    let sub = state
        .subscription_service
        .get(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(sub))
}

pub async fn create_subscription(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateSubscription>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let sub = state
        .subscription_service
        .create(payload)
        .await
        .map_err(|e| {
            let message = e.to_string();
            let status = if message.contains("已存在") || message.contains("不支持") || message.contains("必须") || message.contains("保存路径") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, Json(serde_json::json!({ "error": message })))
        })?;

    // 刷新订阅调度器
    if let Err(e) = state.subscription_scheduler.reload_subscriptions().await {
        tracing::error!("Failed to reload subscription scheduler: {}", e);
    }

    Ok((StatusCode::CREATED, Json(sub)))
}

pub async fn update_subscription(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateSubscription>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let sub = state
        .subscription_service
        .update(id, payload)
        .await
        .map_err(|e| {
            let message = e.to_string();
            let status = if message.contains("不支持") || message.contains("必须") || message.contains("保存路径") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, Json(serde_json::json!({ "error": message })))
        })?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "订阅不存在" })) ))?;

    // 刷新订阅调度器
    if let Err(e) = state.subscription_scheduler.reload_subscriptions().await {
        tracing::error!("Failed to reload subscription scheduler: {}", e);
    }

    Ok(Json(sub))
}

pub async fn delete_subscription(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    let deleted = state
        .subscription_service
        .delete(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if deleted {
        // 刷新订阅调度器
        if let Err(e) = state.subscription_scheduler.reload_subscriptions().await {
            tracing::error!("Failed to reload subscription scheduler: {}", e);
        }
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

pub async fn run_subscription(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    state
        .subscription_service
        .run(id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() }))
            )
        })?;

    Ok(Json(serde_json::json!({ "message": "Subscription task started" })))
}
