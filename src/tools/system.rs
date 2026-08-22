//! 系统五能力工具：读 / 写 / 改 / 删 / 运行。
//!
//! 每次调用经过三级门禁流水线（能力开关 → 人工审批 → 执行），并写入审计日志。

use crate::audit::AuditLog;
use crate::events::EventBus;
use crate::gate::GateKeeper;
use crate::intent::IntentClassifier;
use adk_rust::prelude::*;
use adk_rust::tool::FunctionTool;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;

const MAX_READ_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_CHARS: usize = 32 * 1024;

pub struct SystemToolDeps {
    pub gate: Arc<GateKeeper>,
    pub audit: Arc<AuditLog>,
    pub bus: EventBus,
    /// 命令意图分类器（可插拔，见 intent 模块）
    pub judge: Arc<dyn IntentClassifier>,
}

/// 构建全部五个系统能力工具
pub fn build_system_tools(deps: &SystemToolDeps) -> Vec<Arc<dyn Tool>> {
    vec![
        read_tool(deps),
        write_tool(deps),
        edit_tool(deps),
        delete_tool(deps),
        execute_tool(deps),
    ]
}

/// 工具执行的统一包装：门禁检查 → 计时 → 审计 → 广播活动事件
async fn run_gated<F, Fut>(
    deps: &SystemToolDeps,
    capability: &str,
    tool_name: &str,
    summary: &str,
    args: &Value,
    action: F,
) -> Value
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<Value>>,
{
    if let Err(e) = deps
        .gate
        .authorize(capability, tool_name, summary, args, "")
        .await
    {
        deps.bus.publish(json!({
            "type": "tool_activity",
            "tool": tool_name,
            "summary": summary,
            "status": "denied",
        }));
        return json!({ "error": e.message(), "denied": true });
    }

    deps.bus.publish(json!({
        "type": "tool_activity",
        "tool": tool_name,
        "summary": summary,
        "status": "running",
    }));

    let started = Instant::now();
    let outcome = action().await;
    let duration_ms = started.elapsed().as_millis() as i64;

    match outcome {
        Ok(result) => {
            deps.audit.log(
                "tool_call",
                tool_name,
                &args.to_string(),
                &summarize_result(&result),
                "ok",
                "",
                duration_ms,
            );
            deps.bus.publish(json!({
                "type": "tool_activity",
                "tool": tool_name,
                "summary": summary,
                "status": "ok",
            }));
            result
        }
        Err(err) => {
            deps.audit.log(
                "tool_call",
                tool_name,
                &args.to_string(),
                &err.to_string(),
                "error",
                "",
                duration_ms,
            );
            deps.bus.publish(json!({
                "type": "tool_activity",
                "tool": tool_name,
                "summary": summary,
                "status": "error",
            }));
            json!({ "error": err.to_string() })
        }
    }
}

fn summarize_result(result: &Value) -> String {
    let s = result.to_string();
    crate::audit::truncate(&s, 400)
}

fn read_tool(deps: &SystemToolDeps) -> Arc<dyn Tool> {
    let gate = deps.gate.clone();
    let audit = deps.audit.clone();
    let bus = deps.bus.clone();
    let judge = deps.judge.clone();
    Arc::new(
        FunctionTool::new(
            "sys_read",
            "读取文件内容或列出目录。参数: path (string, 文件或目录的绝对路径); \
             max_bytes (integer, 可选, 最大读取字节数, 默认 65536)",
            move |_ctx, args| {
                let gate = gate.clone();
                let audit = audit.clone();
                let bus = bus.clone();
                let judge = judge.clone();
                async move {
                    let path = args
                        .get("path")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let max_bytes = args
                        .get("max_bytes")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(MAX_READ_BYTES as u64) as usize;
                    if path.is_empty() {
                        return Ok(json!({ "error": "缺少参数 path" }));
                    }
                    let deps = SystemToolDeps { gate, audit, bus, judge };
                    let summary = format!("读取 {path}");
                    Ok(run_gated(&deps, "read", "sys_read", &summary, &args, move || async move {
                        let meta = tokio::fs::metadata(&path).await?;
                        if meta.is_dir() {
                            let mut entries_out = Vec::new();
                            let mut rd = tokio::fs::read_dir(&path).await?;
                            while let Some(entry) = rd.next_entry().await? {
                                let name = entry.file_name().to_string_lossy().to_string();
                                let ft = entry.file_type().await?;
                                let kind = if ft.is_dir() { "dir" } else { "file" };
                                let size = entry.metadata().await.map(|m| m.len()).unwrap_or(0);
                                entries_out.push(json!({ "name": name, "kind": kind, "size": size }));
                            }
                            entries_out.sort_by(|a, b| {
                                a["kind"].as_str().cmp(&b["kind"].as_str())
                                    .then(a["name"].as_str().cmp(&b["name"].as_str()))
                            });
                            Ok(json!({ "kind": "directory", "path": path, "entries": entries_out }))
                        } else {
                            // 按上限流式读取，避免大文件全量载入内存
                            use tokio::io::AsyncReadExt;
                            let cap = max_bytes.min(MAX_READ_BYTES);
                            let mut f = tokio::fs::File::open(&path).await?;
                            let mut buf = vec![0u8; cap];
                            let n = f.read(&mut buf).await?;
                            let truncated = meta.len() as usize > n;
                            let text = String::from_utf8_lossy(&buf[..n]).to_string();
                            Ok(json!({
                                "kind": "file",
                                "path": path,
                                "size": meta.len(),
                                "truncated": truncated,
                                "content": text,
                            }))
                        }
                    })
                    .await)
                }
            },
        )
        .with_read_only(true),
    )
}

fn write_tool(deps: &SystemToolDeps) -> Arc<dyn Tool> {
    let gate = deps.gate.clone();
    let audit = deps.audit.clone();
    let bus = deps.bus.clone();
    let judge = deps.judge.clone();
    Arc::new(FunctionTool::new(
        "sys_write",
        "写入（创建或覆盖）文件。参数: path (string, 绝对路径); content (string, 文件完整内容)",
        move |_ctx, args| {
            let gate = gate.clone();
            let audit = audit.clone();
            let bus = bus.clone();
            let judge = judge.clone();
            async move {
                let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let content = args.get("content").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                if path.is_empty() {
                    return Ok(json!({ "error": "缺少参数 path" }));
                }
                let deps = SystemToolDeps { gate, audit, bus, judge };
                let summary = format!("写入文件 {path} ({} 字节)", content.len());
                Ok(run_gated(&deps, "write", "sys_write", &summary, &args, move || async move {
                    if let Some(parent) = std::path::Path::new(&path).parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                    tokio::fs::write(&path, &content).await?;
                    Ok(json!({ "ok": true, "path": path, "bytes_written": content.len() }))
                })
                .await)
            }
        },
    ))
}

fn edit_tool(deps: &SystemToolDeps) -> Arc<dyn Tool> {
    let gate = deps.gate.clone();
    let audit = deps.audit.clone();
    let bus = deps.bus.clone();
    let judge = deps.judge.clone();
    Arc::new(FunctionTool::new(
        "sys_edit",
        "精确修改文件：将文件中的 old_text 替换为 new_text。参数: path (string); \
         old_text (string, 必须与文件中现有内容完全一致); new_text (string); \
         replace_all (boolean, 可选, 默认只替换第一处)",
        move |_ctx, args| {
            let gate = gate.clone();
            let audit = audit.clone();
            let bus = bus.clone();
            let judge = judge.clone();
            async move {
                let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let old_text = args.get("old_text").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let new_text = args.get("new_text").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let replace_all = args.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false);
                if path.is_empty() || old_text.is_empty() {
                    return Ok(json!({ "error": "缺少参数 path 或 old_text" }));
                }
                let deps = SystemToolDeps { gate, audit, bus, judge };
                let summary = format!("修改文件 {path}");
                Ok(run_gated(&deps, "edit", "sys_edit", &summary, &args, move || async move {
                    let original = tokio::fs::read_to_string(&path)
                        .await
                        .map_err(|e| anyhow::anyhow!("读取文件失败: {e}"))?;
                    let count = original.matches(&old_text).count();
                    if count == 0 {
                        anyhow::bail!("old_text 在文件中不存在（必须精确匹配，包含空白与缩进）");
                    }
                    if count > 1 && !replace_all {
                        anyhow::bail!("old_text 在文件中出现 {count} 次，不唯一。请提供更多上下文使其唯一，或设置 replace_all = true");
                    }
                    let updated = if replace_all {
                        original.replace(&old_text, &new_text)
                    } else {
                        original.replacen(&old_text, &new_text, 1)
                    };
                    tokio::fs::write(&path, &updated).await?;
                    Ok(json!({ "ok": true, "path": path, "replacements": if replace_all { count } else { 1 } }))
                })
                .await)
            }
        },
    ))
}

fn delete_tool(deps: &SystemToolDeps) -> Arc<dyn Tool> {
    let gate = deps.gate.clone();
    let audit = deps.audit.clone();
    let bus = deps.bus.clone();
    let judge = deps.judge.clone();
    Arc::new(FunctionTool::new(
        "sys_delete",
        "删除文件或目录。高危操作，需要用户审批。参数: path (string, 绝对路径); \
         recursive (boolean, 可选, 删除非空目录时必须为 true)",
        move |_ctx, args| {
            let gate = gate.clone();
            let audit = audit.clone();
            let bus = bus.clone();
            let judge = judge.clone();
            async move {
                let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let recursive = args.get("recursive").and_then(|v| v.as_bool()).unwrap_or(false);
                if path.is_empty() {
                    return Ok(json!({ "error": "缺少参数 path" }));
                }
                let deps = SystemToolDeps { gate, audit, bus, judge };
                let summary = format!("删除 {path}{}", if recursive { "（递归）" } else { "" });
                Ok(run_gated(&deps, "delete", "sys_delete", &summary, &args, move || async move {
                    let meta = tokio::fs::metadata(&path).await?;
                    if meta.is_dir() {
                        if recursive {
                            tokio::fs::remove_dir_all(&path).await?;
                        } else {
                            tokio::fs::remove_dir(&path).await
                                .map_err(|e| anyhow::anyhow!("目录非空或无法删除（如需递归删除请设置 recursive=true）: {e}"))?;
                        }
                    } else {
                        tokio::fs::remove_file(&path).await?;
                    }
                    Ok(json!({ "ok": true, "deleted": path }))
                })
                .await)
            }
        },
    ))
}

fn execute_tool(deps: &SystemToolDeps) -> Arc<dyn Tool> {
    let gate = deps.gate.clone();
    let audit = deps.audit.clone();
    let bus = deps.bus.clone();
    // 意图判定与二级门禁使用的独立克隆，供命令执行闭包内部使用
    let gate_inner = deps.gate.clone();
    let judge = deps.judge.clone();
    let audit_inner = deps.audit.clone();
    Arc::new(FunctionTool::new(
        "sys_execute",
        "在 bash 中执行命令。执行前会对命令做意图判定（含 python 等内嵌脚本）： \
         意图不触及需审批能力时直接放行；触及被管控能力或具有破坏性时转入用户审批。 \
         参数: command (string, 完整命令); cwd (string, 可选, 工作目录); \
         timeout_secs (integer, 可选, 默认 120, 最大 600)",
        move |_ctx, args| {
            let gate = gate.clone();
            let audit = audit.clone();
            let bus = bus.clone();
            let gate_inner = gate_inner.clone();
            let judge = judge.clone();
            let audit_inner = audit_inner.clone();
            async move {
                let command = args.get("command").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let cwd = args.get("cwd").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let timeout_secs = args.get("timeout_secs").and_then(|v| v.as_u64()).unwrap_or(120).min(600);
                if command.is_empty() {
                    return Ok(json!({ "error": "缺少参数 command" }));
                }
                let deps = SystemToolDeps { gate, audit, bus, judge: judge.clone() };
                let summary = format!("执行命令: {command}");
                let args_for_gate = args.clone();
                Ok(run_gated(&deps, "execute", "sys_execute", &summary, &args, move || async move {
                    // 意图判定流水线：以分类器对命令最终意图的判定为准（穿透 python 等脚本形式）。
                    // 三种路径：判定开启且成功→按意图路由；开启但不可用→保守兑底；
                    // 主动关闭→关键词启发式（删除类受 delete 门禁，其余直接放行）。
                    let intent_cfg = gate_inner.config.get().await.intent.clone();
                    if intent_cfg.enabled {
                        match judge.classify(&command).await {
                        Ok(verdict) => {
                            audit_inner.log(
                                "gate_decision",
                                "intent_judge",
                                &command,
                                &serde_json::to_string(&verdict).unwrap_or_default(),
                                "ok",
                                "",
                                0,
                            );
                            // 破坏性意图：无论审批策略如何，一律人工审批（安全优先）
                            if verdict.dangerous {
                                let timeout = gate_inner.config.get().await.approval.timeout_secs;
                                let approved = gate_inner
                                    .approvals
                                    .request(
                                        "execute",
                                        "sys_execute",
                                        &format!("破坏性意图需审批：{}\n命令：{}", verdict.reason, command),
                                        &args_for_gate,
                                        timeout,
                                        "",
                                    )
                                    .await
                                    .unwrap_or(false);
                                if !approved {
                                    return Ok(json!({ "error": "操作被拒绝：用户未批准该破坏性命令", "denied": true }));
                                }
                            }
                            // 意图触及的文件类能力：逐项走对应门禁（开关 + 审批策略）
                            for cap in &verdict.capabilities {
                                if let Err(e) = gate_inner
                                    .authorize(
                                        cap,
                                        "sys_execute",
                                        &format!("意图判定触及 {} 能力（{}）：{}", cap, verdict.reason, command),
                                        &args_for_gate,
                                        "",
                                    )
                                    .await
                                {
                                    return Ok(json!({ "error": e.message(), "denied": true }));
                                }
                            }
                        }
                        Err(e) => {
                            // 判定开启但不可用（未配置模型/调用失败/超时）：保守处理。
                            audit_inner.log(
                                "gate_decision",
                                "intent_judge",
                                &command,
                                &format!("意图判定不可用：{e}"),
                                "error",
                                "",
                                0,
                            );
                            if command_contains_delete(&command) {
                                if let Err(e2) = gate_inner
                                    .authorize("delete", "sys_execute", &format!("命令含删除操作: {command}"), &args_for_gate, "")
                                    .await
                                {
                                    return Ok(json!({ "error": e2.message(), "denied": true }));
                                }
                            } else {
                                let timeout = gate_inner.config.get().await.approval.timeout_secs;
                                let approved = gate_inner
                                    .approvals
                                    .request(
                                        "execute",
                                        "sys_execute",
                                        &format!("意图判定不可用（{e}），请人工确认命令：{command}"),
                                        &args_for_gate,
                                        timeout,
                                        "",
                                    )
                                    .await
                                    .unwrap_or(false);
                                if !approved {
                                    return Ok(json!({ "error": "操作被拒绝：用户未批准该命令", "denied": true }));
                                }
                            }
                        }
                    }
                    } else {
                        // 意图判定已在 [intent] 中主动关闭：回退关键词启发式，
                        // 删除类命令仍受 delete 门禁（开关+审批策略），其余命令直接放行。
                        audit_inner.log(
                            "gate_decision",
                            "intent_judge",
                            &command,
                            "意图判定已关闭，使用关键词启发式",
                            "ok",
                            "",
                            0,
                        );
                        if command_contains_delete(&command) {
                            if let Err(e2) = gate_inner
                                .authorize("delete", "sys_execute", &format!("命令含删除操作: {command}"), &args_for_gate, "")
                                .await
                            {
                                return Ok(json!({ "error": e2.message(), "denied": true }));
                            }
                        }
                    }

                    let mut cmd = tokio::process::Command::new("bash");
                    cmd.arg("-c").arg(&command);
                    if !cwd.is_empty() {
                        cmd.current_dir(&cwd);
                    }
                    cmd.stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        // 安全语义：超时或任务取消时必须终止命令（含其派生的子进程）
                        .kill_on_drop(true)
                        .process_group(0);
                    let child = cmd.spawn()?;
                    let pid = child.id();
                    let output = tokio::time::timeout(
                        std::time::Duration::from_secs(timeout_secs),
                        child.wait_with_output(),
                    )
                    .await
                    .map_err(|_| {
                        // 超时：杀死整个进程组（bash 派生的子命令是独立子进程）
                        if let Some(pid) = pid {
                            let _ = std::process::Command::new("kill")
                                .args(["-9", &format!("-{pid}")])
                                .output();
                        }
                        anyhow::anyhow!("命令执行超时（{timeout_secs}s），已终止")
                    })??;

                    let stdout = clip_output(&String::from_utf8_lossy(&output.stdout));
                    let stderr = clip_output(&String::from_utf8_lossy(&output.stderr));
                    Ok(json!({
                        "exit_code": output.status.code(),
                        "stdout": stdout,
                        "stderr": stderr,
                    }))
                })
                .await)
            }
        },
    ))
}

fn clip_output(s: &str) -> String {
    if s.chars().count() <= MAX_OUTPUT_CHARS {
        s.to_string()
    } else {
        let head: String = s.chars().take(MAX_OUTPUT_CHARS / 2).collect();
        let tail: String = s.chars().rev().take(MAX_OUTPUT_CHARS / 2).collect::<Vec<_>>().into_iter().rev().collect();
        format!("{head}\n...[输出过长已截断]...\n{tail}")
    }
}

/// 识别命令中的删除类操作（防止通过 sys_execute 绕过 delete 门禁）。
/// 匹配独立 token：rm / rmdir / unlink / shred（含绝对路径形式）及 find -delete。
fn command_contains_delete(command: &str) -> bool {
    if command.contains("-delete") {
        return true;
    }
    command
        .split(|c: char| {
            c.is_whitespace()
                || matches!(c, '|' | '&' | ';' | '(' | ')' | '`' | '<' | '>' | '"' | '\'' | '$')
        })
        .any(|token| {
            let t = token.to_lowercase();
            let base = t.rsplit('/').next().unwrap_or(&t);
            matches!(base, "rm" | "rmdir" | "unlink" | "shred")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_delete_commands() {
        assert!(command_contains_delete("rm -rf /tmp/x"));
        assert!(command_contains_delete("cd /tmp && rm a.txt"));
        assert!(command_contains_delete("/bin/rm file"));
        assert!(command_contains_delete("find . -name '*.log' -delete"));
        assert!(command_contains_delete("ls | xargs rmdir"));
        // 非删除命令不应误报（避免误拦截正常命令）
        assert!(!command_contains_delete("ls -la"));
        assert!(!command_contains_delete("cat alarm.txt"));
        // "perform"/"format" 含 rm 子串，但不是独立的 rm token，不应误报
        assert!(!command_contains_delete("echo perform format"));
        assert!(!command_contains_delete("grep -r 'format' src/"));
    }
}
