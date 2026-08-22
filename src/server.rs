//! Web 服务：内嵌 SPA + REST API + SSE 流式聊天 / 实时事件。

use crate::agent::{app_name, user_id, AgentHub};
use crate::audit::AuditLog;
use crate::config::{Config, ConfigStore};
use crate::events::EventBus;
use crate::gate::ApprovalManager;
use crate::mcp::McpHub;
use crate::memory::MemoryStore;
use crate::skills::SkillRegistry;
use crate::tools::planner::TaskStore;
use adk_rust::{Content, Event, Part, SessionId};
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ConfigStore>,
    pub audit: Arc<AuditLog>,
    pub memory: Arc<MemoryStore>,
    pub bus: EventBus,
    pub approvals: Arc<ApprovalManager>,
    pub skills: Arc<SkillRegistry>,
    pub tasks: Arc<TaskStore>,
    pub mcp: Arc<McpHub>,
    pub hub: Arc<AgentHub>,
    /// 当前正在进行的对话运行：session_id -> 中止信号（用户点击 STOP 时触发）
    pub runs: Arc<std::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Notify>>>>,
}

#[derive(Serialize)]
struct ApiError {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(json!(self))).into_response()
    }
}

fn err(e: impl std::fmt::Display) -> ApiError {
    ApiError { error: e.to_string() }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/style.css", get(style_css))
        .route("/app.js", get(app_js))
        .route("/api/health", get(health))
        .route("/api/chat", post(chat))
        .route("/api/chat/stop", post(chat_stop))
        .route("/api/events", get(events_stream))
        .route("/api/settings", get(get_settings).put(put_settings))
        .route("/api/audit", get(get_audit))
        .route("/api/sessions", get(list_sessions).post(create_session))
        .route("/api/sessions/{id}/history", get(session_history))
        .route("/api/approvals", get(list_approvals))
        .route("/api/approvals/{id}/resolve", post(resolve_approval))
        .route("/api/skills", get(list_skills).post(rescan_skills))
        .route("/api/tasks", get(list_tasks))
        .route("/api/memory", get(list_memory))
        .route("/api/mcp/status", get(mcp_status))
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../assets/index.html"))
}

async fn style_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../assets/style.css"),
    )
}

async fn app_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        include_str!("../assets/app.js"),
    )
}

async fn health(State(state): State<AppState>) -> Json<Value> {
    let cfg = state.config.get().await;
    Json(json!({
        "ok": true,
        "app": app_name(),
        "provider": cfg.model.provider,
        "model": cfg.model.model,
        "agent_ready": state.hub.runner().await.is_some(),
    }))
}

// ---------------------------------------------------------------------------
// 聊天（SSE 流式）
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ChatRequest {
    message: String,
    session_id: Option<String>,
    /// 前端界面语言（zh / en），缺省视为 zh；用户消息内显式指定语言时优先于它
    #[serde(default)]
    lang: Option<String>,
}

async fn chat(
    State(state): State<AppState>,
    Json(body): Json<ChatRequest>,
) -> Result<Sse<impl futures::Stream<Item = Result<SseEvent, Infallible>>>, ApiError> {
    let runner = state
        .hub
        .runner()
        .await
        .ok_or_else(|| err("智能体尚未就绪，请先在 Settings 中配置模型并保存"))?;

    if body.message.trim().is_empty() {
        return Err(err("消息不能为空"));
    }

    // 会话：复用或新建
    let session_id = match &body.session_id {
        Some(id) if !id.is_empty() => {
            if session_exists(&state, id).await {
                id.clone()
            } else {
                state.hub.create_session().await.map_err(err)?
            }
        }
        _ => state.hub.create_session().await.map_err(err)?,
    };

    state.audit.log(
        "chat",
        "user",
        &crate::audit::truncate(&body.message, 2000),
        "",
        "ok",
        &session_id,
        0,
    );

    let session = SessionId::new(&session_id).map_err(err)?;
    // 界面语言联动：默认按前端当前语言回复，用户在消息内显式指定语言时优先用户指定
    let lang_hint = if body.lang.as_deref() == Some("en") {
        "(System hint: current UI language is English. Reply in English unless the user explicitly requests another language in this message.)"
    } else {
        "(系统提示：当前界面语言为中文。除非用户在本条消息中明确要求其他语言，否则请用中文回复。)"
    };
    let content = Content::new("user").with_text(&format!("{lang_hint}\n{}", body.message));
    let mut stream = runner.run(user_id(), session, content).await.map_err(err)?;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<SseEvent, Infallible>>(128);
    let audit = state.audit.clone();
    let sid = session_id.clone();

    // 注册本次运行，供 /api/chat/stop 中止
    let stop_notify = Arc::new(tokio::sync::Notify::new());
    state
        .runs
        .lock()
        .unwrap()
        .insert(session_id.clone(), stop_notify.clone());
    let runs = state.runs.clone();

    // 先发会话标识
    if let Ok(sse) = SseEvent::default().json_data(json!({ "type": "session", "session_id": sid })) {
        let _ = tx.send(Ok(sse)).await;
    }

    tokio::spawn(async move {
        let mut final_text = String::new();
        let mut turn_buf = String::new();
        let mut flusher = SentenceFlusher::default();
        let mut sent_text = String::new();
        let mut stopped = false;
        let mut disconnected = false;

        loop {
            // 用户中止信号 与 模型事件流 竞争；中止分支置位后退出循环，
            // 丢弃未处理的后续事件（已启动的子命令因 kill_on_drop 一并终止）
            enum Pump {
                Stopped,
                End,
                Item(adk_rust::Result<Event>),
            }
            let pump = tokio::select! {
                biased;
                _ = stop_notify.notified() => Pump::Stopped,
                item = stream.next() => match item {
                    Some(result) => Pump::Item(result),
                    None => Pump::End,
                },
            };
            let result = match pump {
                Pump::Stopped => {
                    stopped = true;
                    break;
                }
                Pump::End => break,
                Pump::Item(result) => result,
            };

            let events: Vec<Value> = match result {
                Ok(event) => convert_event(&event, &mut final_text, &mut turn_buf),
                Err(e) => vec![json!({ "type": "error", "message": e.to_string() })],
            };
            if events.iter().any(|e| e["type"] == "error") {
                if let Some(e) = events.first() {
                    audit.log("chat", "agent", "", e["message"].as_str().unwrap_or(""), "error", &sid, 0);
                }
            }
            for data in events {
                if data["type"] == "text" {
                    // 整句缓冲：token 级增量攒成整句再下发，避免逐词蹦字；
                    // 同时清理 DeepSeek 等模型中文 token 间的前导空格。
                    let delta = data["text"].as_str().unwrap_or_default();
                    for chunk in flusher.push(delta) {
                        sent_text.push_str(&chunk);
                        if let Ok(sse) = SseEvent::default()
                            .json_data(json!({ "type": "text", "text": chunk, "partial": true }))
                        {
                            if tx.send(Ok(sse)).await.is_err() {
                                disconnected = true;
                            }
                        }
                    }
                    if disconnected {
                        break;
                    }
                    continue;
                }
                // 非文本事件（工具调用等）前先冲刷挂起文本，保证展示顺序
                if let Some(rest) = flusher.take_pending() {
                    sent_text.push_str(&rest);
                    if let Ok(sse) = SseEvent::default()
                        .json_data(json!({ "type": "text", "text": rest, "partial": true }))
                    {
                        if tx.send(Ok(sse)).await.is_err() {
                            disconnected = true;
                            break;
                        }
                    }
                }
                let Ok(sse) = SseEvent::default().json_data(data) else {
                    continue;
                };
                if tx.send(Ok(sse)).await.is_err() {
                    disconnected = true; // 客户端断开
                    break;
                }
            }
            if disconnected {
                break;
            }
        }

        // 中止：通知前端，审计留痕（已发送的部分内容仍记录）
        if stopped {
            audit.log("chat", "agent", "", "用户中止了本次运行", "denied", &sid, 0);
            if let Ok(sse) = SseEvent::default().json_data(json!({ "type": "stopped" })) {
                let _ = tx.send(Ok(sse)).await;
            }
        }

        // 冲刷最后一段挂起文本（客户端断开时跳过）
        if !disconnected {
            if let Some(rest) = flusher.take_pending() {
                sent_text.push_str(&rest);
                if let Ok(sse) = SseEvent::default()
                    .json_data(json!({ "type": "text", "text": rest, "partial": true }))
                {
                    let _ = tx.send(Ok(sse)).await;
                }
            }
        }
        if !sent_text.is_empty() {
            audit.log(
                "chat",
                "agent",
                "",
                &crate::audit::truncate(&sent_text, 2000),
                "ok",
                &sid,
                0,
            );
        }
        if !disconnected {
            if let Ok(done) = SseEvent::default().json_data(json!({ "type": "done" })) {
                let _ = tx.send(Ok(done)).await;
            }
        }

        // 清理运行注册表：仅当登记的仍是本次运行的信号时才移除，
        // 避免误删用户中止后立刻发起的新运行
        let mut r = runs.lock().unwrap();
        if r.get(&sid).map(|n| Arc::ptr_eq(n, &stop_notify)).unwrap_or(false) {
            r.remove(&sid);
        }
    });

    Ok(Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx)).keep_alive(KeepAlive::default()))
}

/// 中止指定会话当前正在进行的运行；用户随后可补充上下文再次发送。
#[derive(Deserialize)]
struct StopRequest {
    session_id: String,
}

async fn chat_stop(State(state): State<AppState>, Json(body): Json<StopRequest>) -> Json<Value> {
    let notify = state.runs.lock().unwrap().get(&body.session_id).cloned();
    match notify {
        Some(n) => {
            n.notify_one();
            state.audit.log("chat", "user", "stop", "用户请求中止当前运行", "ok", &body.session_id, 0);
            Json(json!({ "ok": true }))
        }
        None => Json(json!({ "ok": false, "error": "无正在进行的运行" })),
    }
}

async fn session_exists(state: &AppState, session_id: &str) -> bool {
    use adk_rust::session::{GetRequest, SessionService};
    state
        .hub
        .session_service()
        .get(GetRequest {
            app_name: app_name().to_string(),
            user_id: "local_user".to_string(),
            session_id: session_id.to_string(),
            num_recent_events: None,
            after: None,
        })
        .await
        .is_ok()
}

/// 把 ADK Event 转换成前端 SSE 事件。
///
/// 去重规则：LLM 回合结束时，提供商通常会再发一条 `partial = false` 的完整事件，
/// 其内容与之前流式增量（`partial = true`）累计的文本重复。`turn_buf` 记录当前回合
/// 已下发的增量文本：完整事件若与其重复则跳过，仅下发差异部分，避免前端显示两遍。
fn convert_event(event: &Event, final_text: &mut String, turn_buf: &mut String) -> Vec<Value> {
    let mut out = Vec::new();

    if let Some(msg) = &event.llm_response.error_message {
        out.push(json!({ "type": "error", "message": msg }));
        return out;
    }

    let Some(content) = &event.llm_response.content else {
        return out;
    };
    let role = content.role.as_str();

    for part in &content.parts {
        match part {
            Part::Text { text } if role == "model" => {
                if event.llm_response.partial {
                    // 流式增量：原样下发并计入本回合累计
                    turn_buf.push_str(text);
                    final_text.push_str(text);
                    out.push(json!({
                        "type": "text",
                        "text": text,
                        "partial": true,
                    }));
                } else if text == turn_buf {
                    // 完整事件与已流式下发的内容完全重复：丢弃，回合结束
                    turn_buf.clear();
                } else if let Some(rest) = text.strip_prefix(turn_buf.as_str()) {
                    // 完整事件比已下发内容多出尾部：只补发差异
                    if !rest.is_empty() {
                        final_text.push_str(rest);
                        out.push(json!({ "type": "text", "text": rest, "partial": false }));
                    }
                    turn_buf.clear();
                } else {
                    // 非流式提供商的单次完整响应（或新内容）：整段下发，开启新回合
                    final_text.push_str(text);
                    out.push(json!({ "type": "text", "text": text, "partial": false }));
                    turn_buf.clear();
                }
            }
            Part::Thinking { thinking, .. } => {
                out.push(json!({ "type": "thinking", "text": thinking }));
            }
            Part::FunctionCall { name, args, .. } => {
                turn_buf.clear();
                out.push(json!({
                    "type": "tool_call",
                    "name": name,
                    "args": args,
                }));
            }
            Part::FunctionResponse { function_response, .. } => {
                turn_buf.clear();
                let name = &function_response.name;
                let resp = &function_response.response;
                let summary = crate::audit::truncate(&resp.to_string(), 600);
                let ok = resp.get("error").is_none() && resp.get("denied").is_none();
                out.push(json!({
                    "type": "tool_result",
                    "name": name,
                    "ok": ok,
                    "summary": summary,
                }));
            }
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 整句缓冲：把 token 级流式增量攒成整句下发，并清理中文间多余空格
// ---------------------------------------------------------------------------

/// 单块最大长度（字符数），超过则强制下发，避免长段落无终止符时缓冲过大。
const FLUSH_MAX_CHARS: usize = 80;

#[derive(Default)]
struct SentenceFlusher {
    pending: String,
}

impl SentenceFlusher {
    /// 推入一段增量，返回可立即下发的完整句子块（可能为空）。
    fn push(&mut self, mut delta: &str) -> Vec<String> {
        // 中文间多余空格清理：前文以中文（含中文标点）结尾且增量以空格开头、
        // 空格后仍是中文时，丢弃该前导空格（DeepSeek 等模型的中文 token 常带前导空格）。
        if delta.starts_with(' ') {
            if let Some(last) = self.pending.chars().next_back() {
                if is_cjk(last) {
                    let rest = delta.trim_start_matches(' ');
                    if rest.chars().next().is_some_and(is_cjk) {
                        delta = rest;
                    }
                }
            }
        }
        self.pending.push_str(delta);

        let mut out = Vec::new();
        loop {
            if self.pending.is_empty() {
                break;
            }
            // 在句子终止符处切分；无终止符但超长时按阈值切分。
            let cut = self
                .pending
                .char_indices()
                .find(|(i, c)| {
                    if !is_terminator(*c) {
                        return false;
                    }
                    // 数字间的点（版本号 / IP / 小数）不作为句末，避免切碎 "1.2"、"127.0.0.1"
                    if *c == '.' {
                        let prev_num = self.pending[..*i]
                            .chars()
                            .next_back()
                            .is_some_and(|p| p.is_ascii_digit());
                        let next_num = self.pending[*i + 1..]
                            .chars()
                            .next()
                            .is_some_and(|n| n.is_ascii_digit());
                        if prev_num && next_num {
                            return false;
                        }
                    }
                    true
                })
                .map(|(i, c)| i + c.len_utf8());
            match cut {
                Some(pos) => {
                    out.push(self.pending[..pos].to_string());
                    self.pending.drain(..pos);
                }
                None if self.pending.chars().count() >= FLUSH_MAX_CHARS => {
                    out.push(std::mem::take(&mut self.pending));
                }
                None => break,
            }
        }
        out
    }

    /// 取走全部挂起文本（流结束或工具事件前调用）。
    fn take_pending(&mut self) -> Option<String> {
        if self.pending.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.pending))
        }
    }
}

/// 中文汉字与常用中文标点（空格清理的判定范围）。
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x4E00..=0x9FFF      // 汉字
        | 0x3400..=0x4DBF    // 扩展 A
        | 0x3000..=0x303F    // 中文标点（。、《》等）
        | 0xFF00..=0xFFEF    // 全角字符（！？：等）
    )
}

/// 句子终止符（含换行）。
fn is_terminator(c: char) -> bool {
    matches!(c, '。' | '！' | '？' | '；' | '\n' | '.' | '!' | '?' | ';')
}

// ---------------------------------------------------------------------------
// 实时事件流（审批请求 / 任务更新 / 工具活动）
// ---------------------------------------------------------------------------

async fn events_stream(
    State(state): State<AppState>,
) -> Sse<impl futures::Stream<Item = Result<SseEvent, Infallible>>> {
    let mut rx = state.bus.subscribe();
    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(value) => {
                    if let Ok(ev) = SseEvent::default().json_data(value) {
                        yield Ok(ev);
                    }
                }
                // 接收过慢导致丢帧：可恢复，继续接收而非断流（避免审批请求丢失）
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

async fn get_settings(State(state): State<AppState>) -> Json<Value> {
    let cfg = state.config.get().await;
    Json(json!({
        "config": cfg,
        "config_path": state.config.path().to_string_lossy(),
        "providers": ["deepseek", "gemini", "openai_compatible", "ollama", "anthropic"],
        "mcp_statuses": state.mcp.statuses().await,
        "agent_ready": state.hub.runner().await.is_some(),
    }))
}

#[derive(Deserialize)]
struct PutSettingsRequest {
    config: Config,
    #[serde(default)]
    rebuild_mcp: bool,
}

async fn put_settings(
    State(state): State<AppState>,
    Json(body): Json<PutSettingsRequest>,
) -> Result<Json<Value>, ApiError> {
    let old = state.config.get().await;
    state.config.replace(body.config.clone()).await.map_err(err)?;
    state.audit.log(
        "config_change",
        "settings",
        "",
        &format!(
            "provider: {} -> {}, gates: {:?} -> {:?}",
            old.model.provider,
            body.config.model.provider,
            old.gates,
            body.config.gates
        ),
        "ok",
        "",
        0,
    );

    // MCP 重连（仅在请求方要求或配置变化时）
    let mcp_changed = body.rebuild_mcp || old.mcp_servers != body.config.mcp_servers;
    if mcp_changed {
        state.mcp.rebuild(&body.config.mcp_servers).await;
    }

    // 模型 / 行为 / 工具面变化时重建智能体
    let agent_changed = mcp_changed
        || old.model != body.config.model
        || old.agent.extra_instruction != body.config.agent.extra_instruction;
    let mut agent_error: Option<String> = None;
    if agent_changed {
        if let Err(e) = state.hub.rebuild().await {
            agent_error = Some(e.to_string());
        }
    }

    Ok(Json(json!({
        "ok": true,
        "agent_ready": state.hub.runner().await.is_some(),
        "agent_error": agent_error,
        "mcp_statuses": state.mcp.statuses().await,
    })))
}

// ---------------------------------------------------------------------------
// 审计 / 会话 / 审批 / 技能 / 任务 / 记忆 / MCP
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct AuditQuery {
    #[serde(rename = "type")]
    event_type: Option<String>,
    tool: Option<String>,
    status: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn get_audit(State(state): State<AppState>, Query(q): Query<AuditQuery>) -> Result<Json<Value>, ApiError> {
    let events = state
        .audit
        .query(
            q.event_type.as_deref().unwrap_or(""),
            q.tool.as_deref().unwrap_or(""),
            q.status.as_deref().unwrap_or(""),
            q.limit.unwrap_or(100).clamp(1, 1000),
            q.offset.unwrap_or(0).max(0),
        )
        .map_err(err)?;
    Ok(Json(json!({ "events": events })))
}

async fn list_sessions(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "sessions": state.hub.list_sessions().await }))
}

#[derive(Deserialize)]
struct CreateSessionRequest {
    #[serde(default)]
    _placeholder: Option<String>,
}

async fn create_session(State(state): State<AppState>, _body: Option<Json<CreateSessionRequest>>) -> Result<Json<Value>, ApiError> {
    let id = state.hub.create_session().await.map_err(err)?;
    Ok(Json(json!({ "session_id": id })))
}

async fn session_history(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    Json(json!({ "session_id": id, "messages": state.hub.session_history(&id).await }))
}

async fn list_approvals(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "pending": state.approvals.list_pending(),
        "recent": state.approvals.list_recent(20),
    }))
}

#[derive(Deserialize)]
struct ResolveApprovalRequest {
    approved: bool,
}

async fn resolve_approval(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ResolveApprovalRequest>,
) -> Json<Value> {
    let resolved = state.approvals.resolve(&id, body.approved);
    state.audit.log(
        "approval",
        "user_decision",
        &id,
        if body.approved { "approved" } else { "denied" },
        if resolved { "ok" } else { "expired" },
        "",
        0,
    );
    Json(json!({ "ok": resolved }))
}

async fn list_skills(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "dir": state.skills.dir().to_string_lossy(),
        "skills": state.skills.list().await,
    }))
}

async fn rescan_skills(State(state): State<AppState>) -> Json<Value> {
    state.skills.rescan().await;
    Json(json!({ "ok": true, "count": state.skills.list().await.len() }))
}

async fn list_tasks(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "tasks": state.tasks.list(20).await }))
}

async fn list_memory(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "entries": state.memory.list(100) }))
}

async fn mcp_status(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "servers": state.mcp.statuses().await }))
}

/// 启动 HTTP 服务
pub async fn serve(state: AppState) -> anyhow::Result<()> {
    let cfg = state.config.get().await;
    let addr = format!("{}:{}", cfg.server.host, cfg.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(addr = %addr, "web server listening");
    println!("Nova Agent 已启动: http://{addr}");
    axum::serve(listener, router(state)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_rust::{EventActions, LlmResponse};

    fn model_event(text: &str, partial: bool) -> Event {
        Event {
            id: "e1".into(),
            timestamp: chrono::Utc::now(),
            invocation_id: "inv".into(),
            branch: "main".into(),
            author: "nova_agent".into(),
            llm_response: LlmResponse {
                content: Some(Content::new("model").with_text(text)),
                partial,
                ..Default::default()
            },
            actions: EventActions::default(),
            long_running_tool_ids: Vec::new(),
            llm_request: None,
            provider_metadata: std::collections::HashMap::new(),
        }
    }

    /// 流式增量后紧跟重复的完整事件：前端只应收到一份内容。
    #[test]
    fn dedup_partial_then_full() {
        let mut final_text = String::new();
        let mut turn_buf = String::new();

        let mut emitted = String::new();
        for (chunk, partial) in [("各项指标", true), ("明细与总结", true), ("各项指标明细与总结", false)] {
            for evt in convert_event(&model_event(chunk, partial), &mut final_text, &mut turn_buf) {
                if evt["type"] == "text" {
                    emitted.push_str(evt["text"].as_str().unwrap());
                }
            }
        }
        assert_eq!(emitted, "各项指标明细与总结");
        assert_eq!(final_text, "各项指标明细与总结");
    }

    /// 非流式单次完整响应：整段下发一次，不丢失。
    #[test]
    fn single_full_response() {
        let mut final_text = String::new();
        let mut turn_buf = String::new();
        let evts = convert_event(&model_event("总结内容", false), &mut final_text, &mut turn_buf);
        assert_eq!(evts.len(), 1);
        assert_eq!(evts[0]["text"], "总结内容");
        assert_eq!(final_text, "总结内容");
    }

    /// 完整事件比已流式内容多出尾部：只补发差异。
    #[test]
    fn full_event_with_extra_tail() {
        let mut final_text = String::new();
        let mut turn_buf = String::new();
        convert_event(&model_event("前半段", true), &mut final_text, &mut turn_buf);
        let evts = convert_event(&model_event("前半段后半段", false), &mut final_text, &mut turn_buf);
        assert_eq!(evts.len(), 1);
        assert_eq!(evts[0]["text"], "后半段");
        assert_eq!(final_text, "前半段后半段");
    }

    /// 整句缓冲：token 级增量攒到句号才下发，且清理中文间前导空格。
    #[test]
    fn flusher_sentence_and_cjk_spaces() {
        let mut f = SentenceFlusher::default();
        // DeepSeek 风格的 token 流：中文 token 带前导空格，无终止符时不下发
        assert!(f.push("环境").is_empty());
        assert!(f.push(" 很").is_empty());
        assert!(f.push(" 干净").is_empty());
        let chunks = f.push("。");
        assert_eq!(chunks, vec!["环境很干净。"]);
    }

    /// 无终止符的长文本按阈值强制下发；结束时无残留。
    #[test]
    fn flusher_threshold_and_take_pending() {
        let mut f = SentenceFlusher::default();
        let long = "a".repeat(120);
        let chunks = f.push(&long);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), 120);
        assert!(f.take_pending().is_none());

        f.push("最后一句没有句号");
        assert_eq!(f.take_pending(), Some("最后一句没有句号".to_string()));
        assert!(f.take_pending().is_none());
    }

    /// 英文单词间空格、数字小数点不受空格清理影响；数字间的点不作句末。
    #[test]
    fn flusher_keeps_legit_spaces() {
        let mut f = SentenceFlusher::default();
        assert!(f.push("version 1.2").is_empty()); // 1.2 中的点不是句末
        let chunks = f.push(" is out!");
        assert_eq!(chunks, vec!["version 1.2 is out!"]);

        // IP 地址跨增量拼接后内容完整（即使切分点在 IP 中间，前端累加渲染不受影响）
        let mut joined = String::new();
        let mut f3 = SentenceFlusher::default();
        for d in ["服务地址 127.", "0.0.1:8899 可用。"] {
            for c in f3.push(d) {
                joined.push_str(&c);
            }
        }
        assert_eq!(joined, "服务地址 127.0.0.1:8899 可用。");
    }
}
