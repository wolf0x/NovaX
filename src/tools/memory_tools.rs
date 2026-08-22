//! 记忆工具：供智能体自主存取本地长期记忆。

use crate::audit::AuditLog;
use crate::memory::MemoryStore;
use adk_rust::prelude::*;
use adk_rust::tool::FunctionTool;
use serde_json::json;
use std::sync::Arc;

pub fn build_memory_tools(memory: Arc<MemoryStore>, audit: Arc<AuditLog>) -> Vec<Arc<dyn Tool>> {
    let m = memory.clone();
    let a = audit.clone();
    let remember = FunctionTool::new(
        "memory_remember",
        "把重要信息存入长期记忆（用户偏好、事实、约定等）。参数: content (string, 记忆内容); \
         category (string, 可选: user|preference|fact|task|other, 默认 other); \
         tags (array of string, 可选, 便于召回的关键词标签)",
        move |_ctx, args| {
            let m = m.clone();
            let a = a.clone();
            async move {
                let content = args.get("content").and_then(|v| v.as_str()).unwrap_or_default();
                if content.is_empty() {
                    return Ok(json!({ "error": "缺少参数 content" }));
                }
                let category = args.get("category").and_then(|v| v.as_str()).unwrap_or("other");
                let tags: Vec<String> = args
                    .get("tags")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|t| t.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                match m.add(category, content, &tags) {
                    Ok(entry) => {
                        a.log("tool_call", "memory_remember", &args.to_string(), "ok", "ok", "", 0);
                        Ok(json!({ "ok": true, "id": entry.id }))
                    }
                    Err(e) => Ok(json!({ "error": e.to_string() })),
                }
            }
        },
    );

    let m = memory.clone();
    let a = audit.clone();
    let recall = FunctionTool::new(
        "memory_recall",
        "从长期记忆中召回与查询相关的记忆。在回答涉及用户偏好、历史约定或之前讨论过的事实时先调用。 \
         参数: query (string, 查询关键词); category (string, 可选); limit (integer, 可选, 默认 5)",
        move |_ctx, args| {
            let m = m.clone();
            let a = a.clone();
            async move {
                let query = args.get("query").and_then(|v| v.as_str()).unwrap_or_default();
                let category = args.get("category").and_then(|v| v.as_str()).unwrap_or("");
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
                let hits = m.recall(query, category, limit.max(1));
                a.log("tool_call", "memory_recall", &args.to_string(), &format!("{} 条命中", hits.len()), "ok", "", 0);
                Ok(json!({ "hits": hits }))
            }
        },
    );

    let m = memory;
    let a = audit;
    let forget = FunctionTool::new(
        "memory_forget",
        "删除一条长期记忆。参数: id (string, 记忆 id)",
        move |_ctx, args| {
            let m = m.clone();
            let a = a.clone();
            async move {
                let id = args.get("id").and_then(|v| v.as_str()).unwrap_or_default();
                if id.is_empty() {
                    return Ok(json!({ "error": "缺少参数 id" }));
                }
                match m.forget(id) {
                    Ok(true) => {
                        a.log("tool_call", "memory_forget", &args.to_string(), "deleted", "ok", "", 0);
                        Ok(json!({ "ok": true }))
                    }
                    Ok(false) => Ok(json!({ "error": "未找到该记忆" })),
                    Err(e) => Ok(json!({ "error": e.to_string() })),
                }
            }
        },
    );

    vec![Arc::new(remember), Arc::new(recall), Arc::new(forget)]
}
