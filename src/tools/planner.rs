//! 任务规划：智能体将复杂任务拆解为步骤清单，持久化并实时推送进度。

use crate::events::EventBus;
use adk_rust::prelude::*;
use adk_rust::tool::FunctionTool;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStep {
    pub title: String,
    /// pending | in_progress | done | failed
    pub status: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPlan {
    pub id: String,
    pub title: String,
    pub created_ts: String,
    /// active | completed | abandoned
    pub status: String,
    pub steps: Vec<TaskStep>,
}

pub struct TaskStore {
    path: PathBuf,
    bus: EventBus,
    tasks: RwLock<Vec<TaskPlan>>,
}

impl TaskStore {
    pub fn load(path: impl Into<PathBuf>, bus: EventBus) -> anyhow::Result<Arc<Self>> {
        let path = path.into();
        let mut tasks = Vec::new();
        if path.exists() {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                tasks = serde_json::from_str(&raw).unwrap_or_default();
            }
        }
        Ok(Arc::new(Self {
            path,
            bus,
            tasks: RwLock::new(tasks),
        }))
    }

    async fn persist(&self, tasks: &[TaskPlan]) {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if let Ok(raw) = serde_json::to_string_pretty(tasks) {
            if let Err(e) = std::fs::write(&self.path, raw) {
                tracing::warn!(error = %e, "tasks persist failed");
            }
        }
    }

    pub async fn create(&self, title: &str, steps: Vec<String>) -> TaskPlan {
        let plan = TaskPlan {
            id: crate::memory::uuid_v4(),
            title: title.to_string(),
            created_ts: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            status: "active".into(),
            steps: steps
                .into_iter()
                .map(|title| TaskStep {
                    title,
                    status: "pending".into(),
                    note: String::new(),
                })
                .collect(),
        };
        let mut tasks = self.tasks.write().await;
        // 新计划开始时，将之前仍为 active 的计划标记为 abandoned
        for t in tasks.iter_mut() {
            if t.status == "active" {
                t.status = "abandoned".into();
            }
        }
        tasks.push(plan.clone());
        self.persist(&tasks).await;
        drop(tasks);
        self.bus.publish(json!({ "type": "task_update", "task": plan }));
        plan
    }

    pub async fn update_step(
        &self,
        task_id: &str,
        step_index: usize,
        status: &str,
        note: &str,
    ) -> Option<TaskPlan> {
        let mut tasks = self.tasks.write().await;
        let plan = tasks.iter_mut().find(|t| t.id == task_id)?;
        let step = plan.steps.get_mut(step_index)?;
        if !matches!(status, "pending" | "in_progress" | "done" | "failed") {
            return None;
        }
        step.status = status.to_string();
        if !note.is_empty() {
            step.note = note.to_string();
        }
        if plan.steps.iter().all(|s| s.status == "done" || s.status == "failed") {
            plan.status = "completed".into();
        }
        let snapshot = plan.clone();
        self.persist(&tasks).await;
        drop(tasks);
        self.bus.publish(json!({ "type": "task_update", "task": snapshot }));
        Some(snapshot)
    }

    pub async fn list(&self, limit: usize) -> Vec<TaskPlan> {
        self.tasks.read().await.iter().rev().take(limit).cloned().collect()
    }
}

pub fn build_planner_tools(store: Arc<TaskStore>) -> Vec<Arc<dyn Tool>> {
    let s = store.clone();
    let plan_tool = FunctionTool::new(
        "task_plan",
        "为复杂的多步骤任务创建执行计划。当用户的请求包含多个步骤（≥3）或需要先调研再执行时，必须先调用本工具制定计划。 \
         参数: title (string, 任务标题); steps (array of string, 按顺序执行的步骤描述)",
        move |_ctx, args| {
            let s = s.clone();
            async move {
                let title = args.get("title").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let steps: Vec<String> = args
                    .get("steps")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                if title.is_empty() || steps.is_empty() {
                    return Ok(json!({ "error": "缺少参数 title 或 steps" }));
                }
                let plan = s.create(&title, steps).await;
                Ok(json!({ "ok": true, "task_id": plan.id, "steps": plan.steps.len() }))
            }
        },
    );

    let s = store;
    let update_tool = FunctionTool::new(
        "task_update",
        "更新任务计划中某个步骤的状态。每完成一个步骤就立即更新。 \
         参数: task_id (string); step_index (integer, 从 0 开始); \
         status (string: pending|in_progress|done|failed); note (string, 可选, 步骤结果备注)",
        move |_ctx, args| {
            let s = s.clone();
            async move {
                let task_id = args.get("task_id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let step_index = args.get("step_index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let status = args.get("status").and_then(|v| v.as_str()).unwrap_or_default();
                let note = args.get("note").and_then(|v| v.as_str()).unwrap_or_default();
                if task_id.is_empty() || status.is_empty() {
                    return Ok(json!({ "error": "缺少参数 task_id 或 status" }));
                }
                match s.update_step(&task_id, step_index, status, note).await {
                    Some(plan) => Ok(json!({ "ok": true, "task": plan })),
                    None => Ok(json!({ "error": "未找到任务或步骤，或 status 非法" })),
                }
            }
        },
    );

    vec![Arc::new(plan_tool), Arc::new(update_tool)]
}
