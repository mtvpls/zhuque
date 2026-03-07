use crate::services::AuthService;
use crate::models::Claims;
use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::sync::Arc;

pub async fn auth_middleware(
    State(auth_service): State<Arc<AuthService>>,
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Response {
    // 先尝试从 Authorization header 获取 token
    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .or_else(|| {
            // 如果header中没有，尝试从查询参数获取（用于SSE）
            request
                .uri()
                .query()
                .and_then(|q| {
                    q.split('&')
                        .find(|p| p.starts_with("token="))
                        .and_then(|p| p.strip_prefix("token="))
                })
        });

    let token = match token {
        Some(t) => t,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": "Unauthorized",
                    "message": "Missing or invalid Authorization header or token parameter"
                })),
            )
                .into_response();
        }
    };

    // 验证 token
    let claims = match auth_service.verify_token(token) {
        Ok(claims) => {
            // 拒绝session token访问受保护资源
            if claims.sub.starts_with("session:") {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({
                        "error": "Unauthorized",
                        "message": "Session token cannot be used to access protected resources"
                    })),
                )
                    .into_response();
            }
            claims
        }
        Err(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": "Unauthorized",
                    "message": "Invalid or expired token"
                })),
            )
                .into_response();
        }
    };

    // 将 claims 注入到请求扩展中
    request.extensions_mut().insert(claims);

    next.run(request).await
}
