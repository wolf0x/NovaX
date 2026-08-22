//! 审计日志：rusqlite 本地库，记录工具执行、门禁决策、审批、对话与配置变更。

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: i64,
    pub ts: String,
    /// tool_call | gate_decision | approval | chat | config_change | system
    #[serde(rename = "type")]
    pub event_type: String,
    pub tool: String,
    pub args: String,
    pub result_summary: String,
    /// ok | denied | error | pending
    pub status: String,
    pub session_id: String,
    pub duration_ms: i64,
}

pub struct AuditLog {
    conn: Mutex<Connection>,
}

impl AuditLog {
    pub fn open(db_path: impl AsRef<Path>) -> anyhow::Result<Self> {
        if let Some(parent) = db_path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts TEXT NOT NULL,
                type TEXT NOT NULL,
                tool TEXT NOT NULL DEFAULT '',
                args TEXT NOT NULL DEFAULT '',
                result_summary TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT '',
                session_id TEXT NOT NULL DEFAULT '',
                duration_ms INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_events_ts ON events(ts DESC);
            CREATE INDEX IF NOT EXISTS idx_events_type ON events(type);",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 写入一条审计事件，返回事件 id。失败仅记录日志，不影响主流程。
    pub fn log(
        &self,
        event_type: &str,
        tool: &str,
        args: &str,
        result_summary: &str,
        status: &str,
        session_id: &str,
        duration_ms: i64,
    ) {
        let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let args = truncate(args, 4000);
        let result_summary = truncate(result_summary, 4000);
        let res = self.conn.lock().unwrap().execute(
            "INSERT INTO events (ts, type, tool, args, result_summary, status, session_id, duration_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![ts, event_type, tool, args, result_summary, status, session_id, duration_ms],
        );
        if let Err(e) = res {
            tracing::warn!(error = %e, "audit insert failed");
        }
    }

    /// 分页查询，支持按 type / tool / status 过滤（空串表示不过滤）。
    pub fn query(
        &self,
        filter_type: &str,
        filter_tool: &str,
        filter_status: &str,
        limit: i64,
        offset: i64,
    ) -> anyhow::Result<Vec<AuditEvent>> {
        let mut sql = String::from("SELECT id, ts, type, tool, args, result_summary, status, session_id, duration_ms FROM events WHERE 1=1");
        let mut binds: Vec<String> = Vec::new();
        if !filter_type.is_empty() {
            sql.push_str(" AND type = ?");
            binds.push(filter_type.to_string());
        }
        if !filter_tool.is_empty() {
            sql.push_str(" AND tool LIKE ?");
            binds.push(format!("%{filter_tool}%"));
        }
        if !filter_status.is_empty() {
            sql.push_str(" AND status = ?");
            binds.push(filter_status.to_string());
        }
        sql.push_str(" ORDER BY id DESC LIMIT ? OFFSET ?");

        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&sql)?;
        let bind_refs: Vec<&dyn rusqlite::ToSql> = binds
            .iter()
            .map(|s| s as &dyn rusqlite::ToSql)
            .chain([&limit as &dyn rusqlite::ToSql, &offset as &dyn rusqlite::ToSql])
            .collect();
        let rows = stmt.query_map(bind_refs.as_slice(), |row| {
            Ok(AuditEvent {
                id: row.get(0)?,
                ts: row.get(1)?,
                event_type: row.get(2)?,
                tool: row.get(3)?,
                args: row.get(4)?,
                result_summary: row.get(5)?,
                status: row.get(6)?,
                session_id: row.get(7)?,
                duration_ms: row.get(8)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_and_query() {
        let dir = std::env::temp_dir().join(format!(
            "nova_audit_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let audit = AuditLog::open(dir.join("audit.db")).unwrap();
        audit.log("tool_call", "sys_read", "{\"path\":\"/tmp\"}", "ok", "ok", "s1", 12);
        audit.log("gate_decision", "sys_delete", "{}", "disabled", "denied", "s1", 0);
        let all = audit.query("", "", "", 10, 0).unwrap();
        assert_eq!(all.len(), 2);
        let denied = audit.query("", "", "denied", 10, 0).unwrap();
        assert_eq!(denied.len(), 1);
        assert_eq!(denied[0].tool, "sys_delete");
        std::fs::remove_dir_all(&dir).ok();
    }
}
