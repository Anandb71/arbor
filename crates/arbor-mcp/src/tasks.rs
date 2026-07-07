//! Task manager for MCP Tasks extension (`io.modelcontextprotocol/tasks`).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

static TASK_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct TaskRecord {
    pub id: String,
    pub tool: String,
    pub status: TaskStatus,
    pub progress: u8,
    pub message: String,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

pub struct TaskManager {
    tasks: RwLock<HashMap<String, TaskRecord>>,
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            tasks: RwLock::new(HashMap::new()),
        }
    }

    pub async fn create(&self, tool: &str, message: &str) -> String {
        let id = format!("task-{}", TASK_COUNTER.fetch_add(1, Ordering::Relaxed));
        let now = now_ms();
        let record = TaskRecord {
            id: id.clone(),
            tool: tool.to_string(),
            status: TaskStatus::Pending,
            progress: 0,
            message: message.to_string(),
            result: None,
            error: None,
            created_at: now,
            updated_at: now,
        };
        self.tasks.write().await.insert(id.clone(), record);
        id
    }

    pub async fn set_running(&self, id: &str, message: &str, progress: u8) {
        let mut tasks = self.tasks.write().await;
        if let Some(t) = tasks.get_mut(id) {
            t.status = TaskStatus::Running;
            t.message = message.to_string();
            t.progress = progress;
            t.updated_at = now_ms();
        }
    }

    pub async fn complete(&self, id: &str, result: Value) {
        let mut tasks = self.tasks.write().await;
        if let Some(t) = tasks.get_mut(id) {
            t.status = TaskStatus::Completed;
            t.progress = 100;
            t.message = "Completed".to_string();
            t.result = Some(result);
            t.updated_at = now_ms();
        }
    }

    pub async fn fail(&self, id: &str, error: &str) {
        let mut tasks = self.tasks.write().await;
        if let Some(t) = tasks.get_mut(id) {
            t.status = TaskStatus::Failed;
            t.error = Some(error.to_string());
            t.message = error.to_string();
            t.updated_at = now_ms();
        }
    }

    pub async fn cancel(&self, id: &str) -> bool {
        let mut tasks = self.tasks.write().await;
        if let Some(t) = tasks.get_mut(id) {
            if matches!(
                t.status,
                TaskStatus::Pending | TaskStatus::Running
            ) {
                t.status = TaskStatus::Cancelled;
                t.message = "Cancelled by client".to_string();
                t.updated_at = now_ms();
                return true;
            }
        }
        false
    }

    pub async fn get(&self, id: &str) -> Option<TaskRecord> {
        self.tasks.read().await.get(id).cloned()
    }

    /// JSON response for `tasks/get`.
    pub async fn get_response(&self, id: &str) -> Option<Value> {
        let t = self.get(id).await?;
        Some(serde_json::json!({
            "taskId": t.id,
            "status": t.status,
            "progress": t.progress,
            "message": t.message,
            "tool": t.tool,
            "result": t.result,
            "error": t.error,
            "createdAt": t.created_at,
            "updatedAt": t.updated_at
        }))
    }

    /// JSON response for `tasks/update` (progress poll).
    pub async fn update_response(&self, id: &str) -> Option<Value> {
        self.get_response(id).await
    }

    /// Wrap a tool result as a task handle response for long-running ops.
    pub fn task_handle_response(task_id: &str) -> Value {
        serde_json::json!({
            "isTask": true,
            "task": {
                "taskId": task_id,
                "status": "running"
            }
        })
    }
}

impl Default for TaskManager {
    fn default() -> Self {
        Self::new()
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn task_lifecycle() {
        let mgr = TaskManager::new();
        let id = mgr.create("test_tool", "starting").await;
        mgr.set_running(&id, "working", 50).await;
        mgr.complete(&id, serde_json::json!({ "ok": true })).await;

        let resp = mgr.get_response(&id).await.unwrap();
        assert_eq!(resp["status"], "completed");
        assert_eq!(resp["progress"], 100);
    }

    #[tokio::test]
    async fn task_cancel() {
        let mgr = TaskManager::new();
        let id = mgr.create("test_tool", "starting").await;
        assert!(mgr.cancel(&id).await);
        let t = mgr.get(&id).await.unwrap();
        assert_eq!(t.status, TaskStatus::Cancelled);
    }
}
