//! 门禁控制：每项系统能力独立开关；高危操作（可配置）执行前需用户在 Web 端人工审批。

use crate::audit::AuditLog;
use crate::config::ConfigStore;
use crate::events::EventBus;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

/// 门禁决策结果
#[derive(Debug)]
pub enum GateError {
    /// 能力开关已关闭
    Disabled(String),
    /// 用户拒绝或审批超时
    Refused(String),
}

impl GateError {
    pub fn message(&self) -> String {
        match self {
            GateError::Disabled(cap) => {
                format!("能力 `{cap}` 已被管理员在设置中禁用，无法执行此操作。请告知用户并建议其在 Settings 中开启。")
            }
            GateError::Refused(reason) => {
                format!("操作被拒绝：{reason}")
            }
        }
    }
}

struct ApprovalHandle {
    tx: Option<oneshot::Sender<bool>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApproval {
    pub id: String,
    pub capability: String,
    pub tool: String,
    pub summary: String,
    pub args: Value,
    pub created_ts: String,
    pub status: String, // pending | approved | denied | expired
}

pub struct ApprovalManager {
    pending: Mutex<HashMap<String, ApprovalHandle>>,
    records: Mutex<Vec<PendingApproval>>,
    bus: EventBus,
    audit: Arc<AuditLog>,
}

impl ApprovalManager {
    pub fn new(bus: EventBus, audit: Arc<AuditLog>) -> Arc<Self> {
        Arc::new(Self {
            pending: Mutex::new(HashMap::new()),
            records: Mutex::new(Vec::new()),
            bus,
            audit,
        })
    }

    /// 创建一条审批请求并等待用户决定。返回 true 表示批准。
    pub async fn request(
        self: &Arc<Self>,
        capability: &str,
        tool: &str,
        summary: &str,
        args: &Value,
        timeout_secs: u64,
        session_id: &str,
    ) -> Result<bool, String> {
        let id = crate::memory::uuid_v4();
        let (tx, rx) = oneshot::channel::<bool>();
        self.pending
            .lock()
            .unwrap()
            .insert(id.clone(), ApprovalHandle { tx: Some(tx) });

        let record = PendingApproval {
            id: id.clone(),
            capability: capability.to_string(),
            tool: tool.to_string(),
            summary: summary.to_string(),
            args: args.clone(),
            created_ts: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            status: "pending".into(),
        };
        self.records.lock().unwrap().push(record.clone());
        self.audit.log(
            "approval",
            tool,
            &args.to_string(),
            &format!("等待人工审批: {summary}"),
            "pending",
            session_id,
            0,
        );
        self.bus.publish(json!({ "type": "approval_request", "approval": record }));

        let result = tokio::time::timeout(Duration::from_secs(timeout_secs.max(5)), rx).await;
        let (approved, timed_out) = match result {
            Ok(Ok(v)) => (v, false),
            // 通道被丢弃或超时：视为拒绝；超时额外通知前端移除审批卡片
            Ok(Err(_)) => (false, false),
            Err(_) => (false, true),
        };
        if timed_out {
            self.bus.publish(json!({ "type": "approval_expired", "id": id }));
        }
        // 若尚未被 resolve 端点处理（例如超时），更新状态
        self.finish_record(&id, if approved { "approved" } else { if timed_out { "expired" } else { "denied" } });
        self.pending.lock().unwrap().remove(&id);
        Ok(approved)
    }

    /// Web 端批准 / 拒绝。
    pub fn resolve(&self, id: &str, approved: bool) -> bool {
        let mut pending = self.pending.lock().unwrap();
        let handle = pending.get_mut(id);
        let resolved = match handle {
            Some(h) => h.tx.take().map(|tx| tx.send(approved).is_ok()).unwrap_or(false),
            None => false,
        };
        drop(pending);
        if resolved {
            let status = if approved { "approved" } else { "denied" };
            self.finish_record(id, status);
            self.bus.publish(json!({ "type": "approval_resolved", "id": id, "approved": approved }));
        }
        resolved
    }

    fn finish_record(&self, id: &str, status: &str) {
        let mut records = self.records.lock().unwrap();
        if let Some(r) = records.iter_mut().find(|r| r.id == id) {
            if r.status == "pending" {
                r.status = status.to_string();
            }
        }
        // 防止长期运行内存无界增长：只保留最近 500 条记录（完整历史在审计库）
        let excess = records.len().saturating_sub(500);
        if excess > 0 {
            records.drain(0..excess);
        }
    }

    /// 当前待处理审批
    pub fn list_pending(&self) -> Vec<PendingApproval> {
        self.records
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.status == "pending")
            .cloned()
            .collect()
    }

    /// 最近审批记录（倒序）
    pub fn list_recent(&self, limit: usize) -> Vec<PendingApproval> {
        let records = self.records.lock().unwrap();
        records.iter().rev().take(limit).cloned().collect()
    }
}

/// 门禁管理器：三级流水线（能力开关 → 审批 → 放行），全程落审计。
pub struct GateKeeper {
    pub config: Arc<ConfigStore>,
    pub approvals: Arc<ApprovalManager>,
    pub audit: Arc<AuditLog>,
}

impl GateKeeper {
    pub fn new(
        config: Arc<ConfigStore>,
        approvals: Arc<ApprovalManager>,
        audit: Arc<AuditLog>,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            approvals,
            audit,
        })
    }

    /// 对某项能力执行门禁检查；通过返回 Ok(())，否则返回带说明的 GateError。
    pub async fn authorize(
        &self,
        capability: &str,
        tool: &str,
        summary: &str,
        args: &Value,
        session_id: &str,
    ) -> Result<(), GateError> {
        let cfg = self.config.get().await;

        // 1. 能力开关
        if !cfg.gates.is_enabled(capability) {
            self.audit.log(
                "gate_decision",
                tool,
                &args.to_string(),
                &format!("能力 {capability} 已禁用"),
                "denied",
                session_id,
                0,
            );
            return Err(GateError::Disabled(capability.to_string()));
        }

        // 2. 高危人工审批
        if cfg.approval.requires(capability) {
            let timeout = cfg.approval.timeout_secs;
            let approved = self
                .approvals
                .request(capability, tool, summary, args, timeout, session_id)
                .await
                .unwrap_or(false);
            self.audit.log(
                "gate_decision",
                tool,
                &args.to_string(),
                if approved { "人工审批通过" } else { "人工审批拒绝/超时" },
                if approved { "ok" } else { "denied" },
                session_id,
                0,
            );
            if !approved {
                return Err(GateError::Refused("用户未批准该操作（拒绝或超时）".into()));
            }
            return Ok(());
        }

        // 3. 放行
        self.audit.log(
            "gate_decision",
            tool,
            &args.to_string(),
            &format!("能力 {capability} 已启用，无需审批"),
            "ok",
            session_id,
            0,
        );
        Ok(())
    }
}
