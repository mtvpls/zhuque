use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use sysinfo::{System, Disks};
use once_cell::sync::Lazy;
use std::time::SystemTime;

use super::AppState;

static START_TIME: Lazy<SystemTime> = Lazy::new(|| SystemTime::now());

#[derive(Debug, Serialize, Deserialize)]
pub struct SystemInfo {
    pub cpu_usage: f32,
    pub memory_total: u64,
    pub memory_used: u64,
    pub memory_available: u64,
    pub memory_usage_percent: f32,
    pub disks: Vec<DiskInfo>,
    pub start_time: u64,
    pub uptime_seconds: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DiskInfo {
    pub name: String,
    pub mount_point: String,
    pub total_space: u64,
    pub available_space: u64,
    pub used_space: u64,
    pub usage_percent: f32,
}

pub async fn get_system_info(
    State(_state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let mut sys = System::new_all();
    sys.refresh_all();

    // 等待一小段时间以获取准确的CPU使用率
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    sys.refresh_cpu();

    let cpu_usage = sys.global_cpu_info().cpu_usage();
    let memory_total = sys.total_memory();
    let memory_used = sys.used_memory();
    let memory_available = sys.available_memory();
    let memory_usage_percent = if memory_total > 0 {
        (memory_used as f32 / memory_total as f32) * 100.0
    } else {
        0.0
    };

    let disks = Disks::new_with_refreshed_list();
    let disk_info: Vec<DiskInfo> = disks
        .iter()
        .map(|disk| {
            let total = disk.total_space();
            let available = disk.available_space();
            let used = total - available;
            let usage_percent = if total > 0 {
                (used as f32 / total as f32) * 100.0
            } else {
                0.0
            };

            DiskInfo {
                name: disk.name().to_string_lossy().to_string(),
                mount_point: disk.mount_point().to_string_lossy().to_string(),
                total_space: total,
                available_space: available,
                used_space: used,
                usage_percent,
            }
        })
        .collect();

    let info = SystemInfo {
        cpu_usage,
        memory_total,
        memory_used,
        memory_available,
        memory_usage_percent,
        disks: disk_info,
        start_time: START_TIME
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        uptime_seconds: SystemTime::now()
            .duration_since(*START_TIME)
            .unwrap()
            .as_secs(),
    };

    Ok(Json(info))
}

pub async fn get_webhook_config(
    State(_state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, StatusCode> {
    let webhook_token = std::env::var("WEBHOOK_TOKEN").ok();

    Ok(Json(json!({
        "configured": webhook_token.is_some(),
        "token": webhook_token
    })))
}
