use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing_subscriber::Layer;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemLogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: String,
    pub target: String,
    pub message: String,
}

#[derive(Clone)]
pub struct SystemLogCollector {
    logs: Arc<Mutex<VecDeque<SystemLogEntry>>>,
    max_size: usize,
}

impl SystemLogCollector {
    pub fn new(max_size: usize) -> Self {
        Self {
            logs: Arc::new(Mutex::new(VecDeque::with_capacity(max_size))),
            max_size,
        }
    }

    pub fn add_log(&self, entry: SystemLogEntry) {
        let mut logs = self.logs.lock().unwrap();
        if logs.len() >= self.max_size {
            logs.pop_front();
        }
        logs.push_back(entry);
    }

    pub fn get_logs(&self) -> Vec<SystemLogEntry> {
        let logs = self.logs.lock().unwrap();
        // 倒序返回，最新的在前面
        logs.iter().rev().cloned().collect()
    }
}

pub struct SystemLogLayer {
    collector: SystemLogCollector,
}

impl SystemLogLayer {
    pub fn new(collector: SystemLogCollector) -> Self {
        Self { collector }
    }
}

impl<S> Layer<S> for SystemLogLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let metadata = event.metadata();
        let mut visitor = LogVisitor::default();
        event.record(&mut visitor);

        let entry = SystemLogEntry {
            timestamp: Utc::now(),
            level: metadata.level().to_string(),
            target: metadata.target().to_string(),
            message: visitor.message,
        };

        self.collector.add_log(entry);
    }
}

#[derive(Default)]
struct LogVisitor {
    message: String,
}

impl tracing::field::Visit for LogVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
            // Remove surrounding quotes
            if self.message.starts_with('"') && self.message.ends_with('"') {
                self.message = self.message[1..self.message.len() - 1].to_string();
            }
        }
    }
}
