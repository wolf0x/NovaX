/* Nova Agent 前端：聊天 / 审批 / 任务 / 审计 / 设置 / 中英切换 */
"use strict";

const $ = (sel) => document.querySelector(sel);
const state = {
  sessionId: null,
  sessions: [],
  streaming: false,
  settings: null,
  mcpServers: [],
};

/* ============================ 国际化（中/英） ============================ */

const I18N = {
  zh: {
    brand_sub: "ADK-Rust v2 自主智能体",
    tab_chat: "对话", tab_tasks: "任务", tab_audit: "审计日志", tab_settings: "设置",
    status_checking: "检测中", status_no_model: "模型未配置", status_offline: "服务离线",
    new_session: "＋ 新会话", session_hint: "会话",
    welcome_title: "你好，我是 Nova",
    welcome_p1: "我可以读取、编写、修改、删除文件并执行命令（均受门禁管控），还能加载技能、调用 MCP 工具、记忆你的偏好并为复杂任务制定计划。",
    welcome_p2: "先在「设置」中配置模型 API，然后直接向我下达指令。",
    input_placeholder: "输入指令，Enter 发送，Shift+Enter 换行",
    send: "发送",
    side_task_title: "当前任务", no_active_task: "暂无进行中的任务", side_activity_title: "工具活动",
    tasks_title: "任务计划", refresh: "刷新", no_tasks: "还没有任务计划。向智能体下达一个多步骤任务试试。",
    audit_title: "审计日志", all_types: "全部类型", all_status: "全部状态", tool_filter_ph: "工具名过滤", query: "查询",
    col_time: "时间", col_type: "类型", col_tool: "工具", col_args: "参数", col_result: "结果", col_status: "状态", col_duration: "耗时",
    settings_title: "设置", save_apply: "保存并应用",
    model_config: "模型配置", cfg_provider: "提供商", provider_openai_compat: "OpenAI 兼容端点", provider_ollama: "Ollama 本地",
    cfg_api_key: "API Key", cfg_model: "模型", cfg_base_url: "Base URL（OpenAI 兼容 / Ollama 可选）",
    cfg_max_iter: "最大推理轮次", cfg_extra: "附加系统指令", cfg_extra_ph: "追加给智能体的行为约束（可选）",
    gates_title: "系统能力门禁", gates_hint: "每项能力独立开关；关闭后智能体调用将被直接拒绝。",
    gate_read: "读取（文件/目录）", gate_write: "写入（创建/覆盖文件）", gate_edit: "修改（精确替换内容）",
    gate_delete: "删除（文件/目录）", gate_execute: "运行（bash 命令，执行前做意图判定）",
    approval_title: "高危操作人工审批", approval_hint: "勾选的能力执行前需在页面上人工批准。",
    appr_delete: "删除需审批", appr_execute: "运行需审批", appr_write: "写入需审批", appr_edit: "修改需审批",
    appr_timeout: "审批超时（秒）",
    mcp_title: "MCP 服务器", add_mcp: "＋ 添加 MCP 服务器", conn_status: "连接状态", no_mcp: "未配置 MCP 服务器",
    skills_title: "技能（SKILLS）",
    skills_hint: "将目录放入 skills/<技能名>/SKILL.md（frontmatter 含 name/description），点击刷新即可被智能体加载。",
    rescan: "重新扫描", no_skills: "未发现技能", no_desc: "（无描述）", memory_title: "记忆", no_memory: "暂无记忆条目",
    save_ok: "配置已保存并生效（已写入 config.toml）", save_agent_err: "配置已保存，但智能体未就绪：",
    approval_card: "高危操作等待审批", approval_cap: "能力", approval_tool: "工具", approve: "批准执行", deny: "拒绝",
    thinking: "思考过程", params: "参数", error: "错误", conn_err: "连接错误", req_failed: "请求失败",
    you: "你", events: "事件", empty_msg: "消息不能为空",
    mcp_name_ph: "名称", mcp_cmd_ph: "command（stdio）", mcp_args_ph: "args，空格分隔",
    mcp_url_ph: "url（http）", mcp_env_ph: "env：K=V;K2=V2", del: "删除",
  },
  en: {
    brand_sub: "Autonomous agent on ADK-Rust v2",
    tab_chat: "Chat", tab_tasks: "Tasks", tab_audit: "Audit Log", tab_settings: "Settings",
    status_checking: "Checking", status_no_model: "Model not configured", status_offline: "Service offline",
    new_session: "+ New session", session_hint: "Session",
    welcome_title: "Hi, I'm Nova",
    welcome_p1: "I can read, write, edit, delete files and run commands (all gate-controlled), load skills, call MCP tools, remember your preferences and plan complex tasks.",
    welcome_p2: "Configure a model API in Settings first, then give me instructions.",
    input_placeholder: "Type a command. Enter to send, Shift+Enter for newline",
    send: "Send",
    side_task_title: "Current task", no_active_task: "No active task", side_activity_title: "Tool activity",
    tasks_title: "Task plans", refresh: "Refresh", no_tasks: "No task plans yet. Give the agent a multi-step task to try.",
    audit_title: "Audit log", all_types: "All types", all_status: "All statuses", tool_filter_ph: "Filter by tool", query: "Query",
    col_time: "Time", col_type: "Type", col_tool: "Tool", col_args: "Args", col_result: "Result", col_status: "Status", col_duration: "Duration",
    settings_title: "Settings", save_apply: "Save & Apply",
    model_config: "Model", cfg_provider: "Provider", provider_openai_compat: "OpenAI-compatible", provider_ollama: "Ollama (local)",
    cfg_api_key: "API Key", cfg_model: "Model", cfg_base_url: "Base URL (optional for OpenAI-compatible / Ollama)",
    cfg_max_iter: "Max reasoning iterations", cfg_extra: "Extra system instruction", cfg_extra_ph: "Additional behavior constraints (optional)",
    gates_title: "System capability gates", gates_hint: "Each capability has its own switch; disabled calls are rejected immediately.",
    gate_read: "Read (files/dirs)", gate_write: "Write (create/overwrite)", gate_edit: "Edit (exact replace)",
    gate_delete: "Delete (files/dirs)", gate_execute: "Execute (bash, intent-judged before run)",
    approval_title: "Human approval for risky ops", approval_hint: "Checked capabilities require on-page approval before execution.",
    appr_delete: "Delete needs approval", appr_execute: "Execute needs approval", appr_write: "Write needs approval", appr_edit: "Edit needs approval",
    appr_timeout: "Approval timeout (seconds)",
    mcp_title: "MCP servers", add_mcp: "+ Add MCP server", conn_status: "Connection status", no_mcp: "No MCP servers configured",
    skills_title: "Skills",
    skills_hint: "Place skills/<name>/SKILL.md (frontmatter with name/description) under skills/, then rescan to load.",
    rescan: "Rescan", no_skills: "No skills found", no_desc: "(no description)", memory_title: "Memory", no_memory: "No memory entries",
    save_ok: "Saved and applied (written to config.toml)", save_agent_err: "Saved, but agent not ready: ",
    approval_card: "Risky operation awaiting approval", approval_cap: "capability", approval_tool: "tool", approve: "Approve", deny: "Deny",
    thinking: "Thinking", params: "parameters", error: "Error", conn_err: "Connection error", req_failed: "Request failed",
    you: "You", events: "events", empty_msg: "Message cannot be empty",
    mcp_name_ph: "name", mcp_cmd_ph: "command (stdio)", mcp_args_ph: "args, space separated",
    mcp_url_ph: "url (http)", mcp_env_ph: "env: K=V;K2=V2", del: "Delete",
  },
};

let lang = localStorage.getItem("nova-lang") === "en" ? "en" : "zh";

function t(key) {
  return (I18N[lang] && I18N[lang][key]) ?? I18N.zh[key] ?? key;
}

function applyI18n() {
  document.querySelectorAll("[data-i18n]").forEach((el) => {
    el.textContent = t(el.dataset.i18n);
  });
  document.querySelectorAll("[data-i18n-ph]").forEach((el) => {
    el.placeholder = t(el.dataset.i18nPh);
  });
  const btn = $("#btn-lang");
  if (btn) btn.textContent = lang === "zh" ? "EN" : "中文";
}

function setLang(next) {
  lang = next;
  localStorage.setItem("nova-lang", lang);
  applyI18n();
  checkHealth();
  // 重渲染动态区域，使其使用新语言
  loadTasks().catch(() => {});
  if (state.settings) loadSettings().catch(() => {});
  loadPendingApprovals();
}

/* ============================ Markdown 渲染 ============================ */

function escapeHtml(s) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;").replace(/'/g, "&#39;");
}

function inlineMd(s) {
  let out = escapeHtml(s);
  out = out.replace(/`([^`]+)`/g, (_, c) => `<code>${c}</code>`);
  out = out.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  out = out.replace(/(^|[^*])\*([^*\n]+)\*/g, "$1<em>$2</em>");
  out = out.replace(/\[([^\]]+)\]\((https?:[^)\s]+)\)/g, '<a href="$2" target="_blank" rel="noopener">$1</a>');
  return out;
}

function renderMarkdown(src) {
  const lines = src.replace(/\r\n/g, "\n").split("\n");
  const html = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];

    // 代码块
    const fence = line.match(/^```(\w*)/);
    if (fence) {
      const lang = fence[1] || "";
      const buf = [];
      i++;
      while (i < lines.length && !lines[i].startsWith("```")) { buf.push(lines[i]); i++; }
      i++; // 跳过结束 ```
      html.push(`<pre>${lang ? `<span class="lang">${escapeHtml(lang)}</span>` : ""}<code>${escapeHtml(buf.join("\n"))}</code></pre>`);
      continue;
    }
    // 标题
    const h = line.match(/^(#{1,4})\s+(.*)/);
    if (h) { html.push(`<h${h[1].length}>${inlineMd(h[2])}</h${h[1].length}>`); i++; continue; }
    // 分割线
    if (/^(-{3,}|\*{3,})\s*$/.test(line)) { html.push("<hr>"); i++; continue; }
    // 表格
    if (line.includes("|") && i + 1 < lines.length && /^\s*\|?[\s:|-]+\|?\s*$/.test(lines[i + 1]) && lines[i + 1].includes("-")) {
      const rows = [];
      const parseRow = (l) => l.trim().replace(/^\||\|$/g, "").split("|").map((c) => inlineMd(c.trim()));
      rows.push(parseRow(line)); i += 2;
      while (i < lines.length && lines[i].includes("|")) { rows.push(parseRow(lines[i])); i++; }
      const head = rows[0].map((c) => `<th>${c}</th>`).join("");
      const body = rows.slice(1).map((r) => `<tr>${r.map((c) => `<td>${c}</td>`).join("")}</tr>`).join("");
      html.push(`<table><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table>`);
      continue;
    }
    // 引用
    if (line.startsWith(">")) {
      const buf = [];
      while (i < lines.length && lines[i].startsWith(">")) { buf.push(lines[i].replace(/^>\s?/, "")); i++; }
      html.push(`<blockquote>${renderMarkdown(buf.join("\n"))}</blockquote>`);
      continue;
    }
    // 无序列表
    if (/^\s*[-*+]\s+/.test(line)) {
      const buf = [];
      while (i < lines.length && /^\s*[-*+]\s+/.test(lines[i])) { buf.push(`<li>${inlineMd(lines[i].replace(/^\s*[-*+]\s+/, ""))}</li>`); i++; }
      html.push(`<ul>${buf.join("")}</ul>`);
      continue;
    }
    // 有序列表
    if (/^\s*\d+[.)]\s+/.test(line)) {
      const buf = [];
      while (i < lines.length && /^\s*\d+[.)]\s+/.test(lines[i])) { buf.push(`<li>${inlineMd(lines[i].replace(/^\s*\d+[.)]\s+/, ""))}</li>`); i++; }
      html.push(`<ol>${buf.join("")}</ol>`);
      continue;
    }
    // 空行
    if (line.trim() === "") { i++; continue; }
    // 段落：收集到空行为止
    const buf = [line];
    i++;
    while (i < lines.length && lines[i].trim() !== "" && !/^(```|#{1,4}\s|>|\s*[-*+]\s|\s*\d+[.)]\s)/.test(lines[i])) {
      buf.push(lines[i]); i++;
    }
    html.push(`<p>${inlineMd(buf.join(" "))}</p>`);
  }
  return html.join("\n");
}

/* ============================ 视图切换 ============================ */

document.querySelectorAll(".tab").forEach((tab) => {
  tab.addEventListener("click", () => {
    document.querySelectorAll(".tab").forEach((t) => t.classList.remove("active"));
    tab.classList.add("active");
    document.querySelectorAll(".view").forEach((v) => v.classList.add("hidden"));
    $(`#view-${tab.dataset.view}`).classList.remove("hidden");
    if (tab.dataset.view === "audit") loadAudit();
    if (tab.dataset.view === "tasks") loadTasks();
    if (tab.dataset.view === "settings") loadSettings();
  });
});

/* ============================ 状态指示 ============================ */

async function checkHealth() {
  try {
    const r = await fetch("/api/health");
    const d = await r.json();
    const dot = $("#status-dot");
    if (d.agent_ready) {
      dot.className = "dot ok";
      $("#status-text").textContent = `${d.provider} / ${d.model}`;
    } else {
      dot.className = "dot err";
      $("#status-text").textContent = t("status_no_model");
    }
  } catch {
    $("#status-dot").className = "dot err";
    $("#status-text").textContent = t("status_offline");
  }
}
checkHealth();
setInterval(checkHealth, 15000);

/* ============================ 会话管理 ============================ */

async function loadSessions(selectId) {
  const r = await fetch("/api/sessions");
  const d = await r.json();
  state.sessions = d.sessions || [];
  const sel = $("#session-select");
  sel.innerHTML = "";
  for (const s of state.sessions) {
    const opt = document.createElement("option");
    opt.value = s.session_id;
    opt.textContent = `${s.session_id.slice(0, 8)}…（${s.events} ${t("events")}）`;
    sel.appendChild(opt);
  }
  if (selectId && state.sessions.some((s) => s.session_id === selectId)) {
    sel.value = selectId;
  }
  if (!state.sessionId && state.sessions.length) state.sessionId = sel.value;
  sel.value = state.sessionId || "";
}

$("#session-select").addEventListener("change", async (e) => {
  state.sessionId = e.target.value;
  await renderHistory();
});

$("#btn-new-session").addEventListener("click", async () => {
  const r = await fetch("/api/sessions", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" });
  const d = await r.json();
  state.sessionId = d.session_id;
  $("#messages").innerHTML = "";
  await loadSessions(state.sessionId);
});

async function renderHistory() {
  if (!state.sessionId) return;
  const r = await fetch(`/api/sessions/${state.sessionId}/history`);
  const d = await r.json();
  const box = $("#messages");
  box.innerHTML = "";
  for (const m of d.messages || []) {
    appendMessage(m.role === "user" ? "user" : "model", m.text, false);
  }
  box.scrollTop = box.scrollHeight;
}

/* ============================ 聊天 ============================ */

function appendMessage(role, text, streaming) {
  const box = $("#messages");
  const welcome = box.querySelector(".welcome");
  if (welcome) welcome.remove();
  const div = document.createElement("div");
  div.className = `msg ${role}`;
  div.innerHTML = `<div class="avatar">${role === "user" ? t("you").slice(0, 1) : "N"}</div>
    <div class="body"><div class="who">${role === "user" ? t("you") : "Nova"}</div><div class="md content"></div></div>`;
  const content = div.querySelector(".content");
  content.innerHTML = renderMarkdown(text);
  if (streaming) content.classList.add("cursor");
  box.appendChild(div);
  box.scrollTop = box.scrollHeight;
  return content;
}

function appendToolChip(name, args) {
  const box = $("#messages");
  const div = document.createElement("div");
  div.className = "msg model";
  const argText = args && typeof args === "object" ? JSON.stringify(args, null, 2) : String(args);
  div.innerHTML = `<div class="avatar">N</div><div class="body">
    <span class="tool-chip"><span class="t-icon">⚙</span>${escapeHtml(name)}
      <details><summary>${t("params")}</summary><pre>${escapeHtml(argText)}</pre></details></span></div>`;
  box.appendChild(div);
  box.scrollTop = box.scrollHeight;
  return div;
}

async function sendChat() {
  if (state.streaming) return;
  const input = $("#chat-input");
  const message = input.value.trim();
  if (!message) return;
  input.value = "";
  appendMessage("user", message, false);
  state.streaming = true;
  $("#btn-send").disabled = true;

  let assistantEl = null;
  let buffer = "";

  function handleChatEvent(evt) {
    switch (evt.type) {
      case "session":
        state.sessionId = evt.session_id;
        $("#session-hint").textContent = `${t("session_hint")} ${evt.session_id.slice(0, 8)}…`;
        break;
      case "text":
        buffer += evt.text;
        if (!assistantEl) assistantEl = appendMessage("model", buffer, true);
        else assistantEl.innerHTML = renderMarkdown(buffer);
        assistantEl.classList.add("cursor");
        $("#messages").scrollTop = $("#messages").scrollHeight;
        break;
      case "thinking": {
        const box = $("#messages");
        if (!box.querySelector(".thinking-live")) {
          const d = document.createElement("div");
          d.className = "msg model";
          d.innerHTML = `<div class="avatar">N</div><div class="body"><details class="thinking-live"><summary>${t("thinking")}</summary><div class="md muted"></div></details></div>`;
          box.appendChild(d);
        }
        const thinkingEl = box.querySelector(".thinking-live .md");
        thinkingEl.textContent += evt.text;
        break;
      }
      case "tool_call":
        if (assistantEl) { assistantEl.classList.remove("cursor"); assistantEl = null; buffer = ""; }
        appendToolChip(evt.name, evt.args);
        break;
      case "tool_result": {
        const chips = document.querySelectorAll(".tool-chip");
        const chip = chips[chips.length - 1];
        if (chip && !evt.ok) chip.classList.add("err");
        break;
      }
      case "error":
        appendMessage("model", `**${t("error")}**：${evt.message}`, false);
        break;
      case "done":
        if (assistantEl) assistantEl.classList.remove("cursor");
        break;
    }
  }

  try {
    const resp = await fetch("/api/chat", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ message, session_id: state.sessionId }),
    });
    if (!resp.ok || !resp.body) {
      let msg = `${t("req_failed")} (${resp.status})`;
      try { msg = (await resp.json()).error || msg; } catch {}
      appendMessage("model", `**${t("error")}**：${msg}`, false);
      return;
    }
    const reader = resp.body.getReader();
    const decoder = new TextDecoder();
    let sseBuf = "";
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      sseBuf += decoder.decode(value, { stream: true });
      let idx;
      while ((idx = sseBuf.indexOf("\n\n")) >= 0) {
        const block = sseBuf.slice(0, idx);
        sseBuf = sseBuf.slice(idx + 2);
        const dataLine = block.split("\n").find((l) => l.startsWith("data:"));
        if (!dataLine) continue;
        let evt;
        try { evt = JSON.parse(dataLine.slice(5).trim()); } catch { continue; }
        handleChatEvent(evt);
      }
    }
  } catch (e) {
    appendMessage("model", `**${t("conn_err")}**：${e.message || e}`, false);
  } finally {
    state.streaming = false;
    $("#btn-send").disabled = false;
    loadSessions(state.sessionId);
  }
}

$("#btn-send").addEventListener("click", sendChat);
$("#chat-input").addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); sendChat(); }
});

/* ============================ 实时事件（审批/任务/活动） ============================ */

function connectEvents() {
  const es = new EventSource("/api/events");
  es.onmessage = (e) => {
    let evt;
    try { evt = JSON.parse(e.data); } catch { return; }
    if (evt.type === "approval_request") renderApproval(evt.approval);
    if (evt.type === "approval_resolved") removeApproval(evt.id);
    if (evt.type === "approval_expired") removeApproval(evt.id);
    if (evt.type === "task_update") { renderSideTask(evt.task); if (!$("#view-tasks").classList.contains("hidden")) loadTasks(); }
    if (evt.type === "tool_activity") pushActivity(evt);
  };
  es.onerror = () => { es.close(); setTimeout(connectEvents, 3000); };
}
connectEvents();

function renderApproval(a) {
  const box = $("#approvals");
  if (box.querySelector(`[data-id="${a.id}"]`)) return;
  const card = document.createElement("div");
  card.className = "approval-card";
  card.dataset.id = a.id;
  card.innerHTML = `
    <div class="a-title">⚠ ${t("approval_card")} · ${t("approval_cap")}: ${escapeHtml(a.capability)} · ${t("approval_tool")}: ${escapeHtml(a.tool)}</div>
    <div class="a-summary">${escapeHtml(a.summary)}</div>
    <pre>${escapeHtml(JSON.stringify(a.args, null, 2))}</pre>
    <div class="a-actions">
      <button class="btn success small" data-act="approve">${t("approve")}</button>
      <button class="btn danger small" data-act="deny">${t("deny")}</button>
    </div>`;
  card.querySelectorAll("button").forEach((btn) =>
    btn.addEventListener("click", async () => {
      const approved = btn.dataset.act === "approve";
      btn.disabled = true;
      const r = await fetch(`/api/approvals/${a.id}/resolve`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ approved }),
      });
      let ok = false;
      try { ok = (await r.json()).ok === true; } catch {}
      if (ok) {
        removeApproval(a.id);
      } else {
        // 审批已超时/失效：移除卡片并提示，避免用户误以为已批准
        removeApproval(a.id);
        appendMessage("model", `**${t("error")}**：${approved ? t("approve") : t("deny")} — approval expired / 审批已失效`, false);
      }
    })
  );
  box.appendChild(card);
}

function removeApproval(id) {
  const card = document.querySelector(`.approval-card[data-id="${id}"]`);
  if (card) card.remove();
}

async function loadPendingApprovals() {
  try {
    const r = await fetch("/api/approvals");
    const d = await r.json();
    $("#approvals").innerHTML = "";
    (d.pending || []).forEach(renderApproval);
  } catch {}
}
loadPendingApprovals();

/* ============================ 任务 ============================ */

function taskStepsHtml(task) {
  const marks = { done: "✓", in_progress: "▶", failed: "✗", pending: "○" };
  return task.steps
    .map((s, i) => `<div class="task-step ${s.status}"><span class="mark">${marks[s.status] || "○"}</span>
      <span class="s-title">${i + 1}. ${escapeHtml(s.title)}</span>
      ${s.note ? `<span class="note">— ${escapeHtml(s.note)}</span>` : ""}</div>`)
    .join("");
}

function renderSideTask(task) {
  const el = $("#side-task");
  if (!task) { el.className = "side-card empty"; el.textContent = t("no_active_task"); return; }
  el.className = "side-card";
  el.innerHTML = `<strong>${escapeHtml(task.title)}</strong>
    <span class="badge ${task.status}">${task.status}</span>
    <div style="margin-top:8px">${taskStepsHtml(task)}</div>`;
}

async function loadTasks() {
  const r = await fetch("/api/tasks");
  const d = await r.json();
  const list = $("#tasks-list");
  const tasks = d.tasks || [];
  if (!tasks.length) { list.innerHTML = `<p class="muted">${t("no_tasks")}</p>`; return; }
  list.innerHTML = tasks
    .map((task) => `<div class="task-card">
      <h4>${escapeHtml(task.title)}<span class="badge ${task.status}">${task.status}</span></h4>
      <div class="t-meta">${task.created_ts} · ${task.id.slice(0, 8)}…</div>
      ${taskStepsHtml(task)}</div>`)
    .join("");
  const active = tasks.find((t) => t.status === "active");
  if (active) renderSideTask(active);
}
$("#btn-refresh-tasks").addEventListener("click", loadTasks);

function pushActivity(evt) {
  const box = $("#side-activity");
  const item = document.createElement("div");
  item.className = "activity-item";
  const time = new Date().toLocaleTimeString();
  item.innerHTML = `<span class="a-tool">${escapeHtml(evt.tool)}</span>
    <span class="st-${evt.status}">${evt.status}</span>
    <div class="muted">${escapeHtml(evt.summary || "")} · ${time}</div>`;
  box.prepend(item);
  while (box.children.length > 30) box.lastChild.remove();
}

/* ============================ 审计 ============================ */

async function loadAudit() {
  const params = new URLSearchParams();
  if ($("#audit-type").value) params.set("type", $("#audit-type").value);
  if ($("#audit-status").value) params.set("status", $("#audit-status").value);
  if ($("#audit-tool").value) params.set("tool", $("#audit-tool").value);
  params.set("limit", "200");
  const r = await fetch(`/api/audit?${params}`);
  const d = await r.json();
  const tbody = $("#audit-table tbody");
  tbody.innerHTML = (d.events || [])
    .map((e) => `<tr>
      <td>${e.ts}</td>
      <td>${e.type}</td>
      <td>${escapeHtml(e.tool)}</td>
      <td class="args" title="${escapeHtml(e.args)}">${escapeHtml(e.args)}</td>
      <td class="args" title="${escapeHtml(e.result_summary)}">${escapeHtml(e.result_summary)}</td>
      <td><span class="status-pill ${e.status}">${e.status}</span></td>
      <td>${e.duration_ms}ms</td>
    </tr>`)
    .join("");
}
$("#btn-refresh-audit").addEventListener("click", loadAudit);

/* ============================ 设置 ============================ */

async function loadSettings() {
  const r = await fetch("/api/settings");
  const d = await r.json();
  state.settings = d;
  const c = d.config;
  $("#config-path").textContent = d.config_path;
  $("#cfg-provider").value = c.model.provider;
  $("#cfg-api-key").value = c.model.api_key;
  $("#cfg-model").value = c.model.model;
  $("#cfg-base-url").value = c.model.base_url;
  $("#cfg-max-iter").value = c.model.max_iterations;
  $("#cfg-extra").value = c.agent.extra_instruction;
  $("#gate-read").checked = c.gates.read;
  $("#gate-write").checked = c.gates.write;
  $("#gate-edit").checked = c.gates.edit;
  $("#gate-delete").checked = c.gates.delete;
  $("#gate-execute").checked = c.gates.execute;
  $("#appr-delete").checked = c.approval.require_for.includes("delete");
  $("#appr-execute").checked = c.approval.require_for.includes("execute");
  $("#appr-write").checked = c.approval.require_for.includes("write");
  $("#appr-edit").checked = c.approval.require_for.includes("edit");
  $("#cfg-appr-timeout").value = c.approval.timeout_secs;
  state.mcpServers = JSON.parse(JSON.stringify(c.mcp_servers || []));
  renderMcpList();
  renderMcpStatus(d.mcp_statuses || []);
  loadSkills();
  loadMemory();
}

function renderMcpStatus(statuses) {
  const el = $("#mcp-status");
  if (!statuses.length) { el.textContent = t("no_mcp"); return; }
  el.innerHTML = statuses
    .map((s) => `<div style="padding:3px 0">${s.connected ? "🟢" : "🔴"} <b>${escapeHtml(s.name)}</b>
      <span class="muted">[${s.transport}] ${escapeHtml(s.target)}</span>
      ${s.error ? `<div class="muted">${escapeHtml(s.error)}</div>` : ""}</div>`)
    .join("");
}

function renderMcpList() {
  const box = $("#mcp-list");
  box.innerHTML = "";
  state.mcpServers.forEach((s, idx) => {
    const item = document.createElement("div");
    item.className = "mcp-item";
    item.innerHTML = `
      <div class="row">
        <input style="flex:2" placeholder="${t("mcp_name_ph")}" value="${escapeHtml(s.name)}" data-f="name">
        <select style="flex:1" data-f="transport">
          <option value="stdio" ${s.transport !== "http" ? "selected" : ""}>stdio</option>
          <option value="http" ${s.transport === "http" ? "selected" : ""}>http</option>
        </select>
        <label class="switch" title="Enable"><input type="checkbox" data-f="enabled" ${s.enabled ? "checked" : ""}><span class="slider"></span></label>
        <button class="btn ghost small" data-act="del">${t("del")}</button>
      </div>
      <div class="row">
        <input style="flex:1" placeholder="${t("mcp_cmd_ph")}" value="${escapeHtml(s.command)}" data-f="command">
        <input style="flex:2" placeholder="${t("mcp_args_ph")}" value="${escapeHtml((s.args || []).join(" "))}" data-f="args">
      </div>
      <div class="row">
        <input style="flex:2" placeholder="${t("mcp_url_ph")}" value="${escapeHtml(s.url)}" data-f="url">
        <input style="flex:2" placeholder="${t("mcp_env_ph")}" value="${escapeHtml(Object.entries(s.env || {}).map(([k, v]) => `${k}=${v}`).join(";"))}" data-f="env">
      </div>`;
    item.querySelectorAll("[data-f]").forEach((el) =>
      el.addEventListener("change", () => {
        const f = el.dataset.f;
        if (f === "enabled") { s.enabled = el.checked; return; }
        if (f === "args") { s.args = el.value.split(/\s+/).filter(Boolean); return; }
        if (f === "env") {
          s.env = {};
          el.value.split(";").forEach((kv) => {
            const i = kv.indexOf("=");
            if (i > 0) s.env[kv.slice(0, i).trim()] = kv.slice(i + 1).trim();
          });
          return;
        }
        s[f] = el.value;
      })
    );
    item.querySelector('[data-act="del"]').addEventListener("click", () => {
      state.mcpServers.splice(idx, 1);
      renderMcpList();
    });
    box.appendChild(item);
  });
}

$("#btn-add-mcp").addEventListener("click", () => {
  state.mcpServers.push({ name: "", transport: "stdio", command: "", args: [], env: {}, url: "", enabled: true });
  renderMcpList();
});

$("#btn-save-settings").addEventListener("click", async () => {
  const requireFor = [];
  if ($("#appr-delete").checked) requireFor.push("delete");
  if ($("#appr-execute").checked) requireFor.push("execute");
  if ($("#appr-write").checked) requireFor.push("write");
  if ($("#appr-edit").checked) requireFor.push("edit");
  const config = {
    server: state.settings.config.server,
    paths: state.settings.config.paths,
    model: {
      provider: $("#cfg-provider").value,
      api_key: $("#cfg-api-key").value.trim(),
      model: $("#cfg-model").value.trim(),
      base_url: $("#cfg-base-url").value.trim(),
      max_iterations: parseInt($("#cfg-max-iter").value || "30", 10),
    },
    gates: {
      read: $("#gate-read").checked,
      write: $("#gate-write").checked,
      edit: $("#gate-edit").checked,
      delete: $("#gate-delete").checked,
      execute: $("#gate-execute").checked,
    },
    approval: {
      require_for: requireFor,
      timeout_secs: parseInt($("#cfg-appr-timeout").value || "300", 10),
    },
    agent: { extra_instruction: $("#cfg-extra").value },
    mcp_servers: state.mcpServers,
  };
  const errBox = $("#settings-error");
  try {
    const r = await fetch("/api/settings", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ config, rebuild_mcp: true }),
    });
    const d = await r.json();
    if (!r.ok) throw new Error(d.error || t("req_failed"));
    if (d.agent_error) {
      errBox.className = "alert";
      errBox.textContent = `${t("save_agent_err")}${d.agent_error}`;
    } else {
      errBox.className = "alert info";
      errBox.textContent = t("save_ok");
    }
    renderMcpStatus(d.mcp_statuses || []);
    checkHealth();
  } catch (e) {
    errBox.className = "alert";
    errBox.textContent = e.message;
  }
});

async function loadSkills() {
  const r = await fetch("/api/skills");
  const d = await r.json();
  const list = $("#skills-list");
  if (!(d.skills || []).length) {
    list.innerHTML = `<li class="muted">${t("no_skills")}（${escapeHtml(d.dir)}）</li>`;
    return;
  }
  list.innerHTML = d.skills
    .map((s) => `<li><b>${escapeHtml(s.name)}</b><div class="s-desc">${escapeHtml(s.description || t("no_desc"))}</div></li>`)
    .join("");
}

$("#btn-rescan-skills").addEventListener("click", async () => {
  await fetch("/api/skills", { method: "POST" });
  loadSkills();
});

async function loadMemory() {
  const r = await fetch("/api/memory");
  const d = await r.json();
  const list = $("#memory-list");
  if (!(d.entries || []).length) { list.innerHTML = `<li class="muted">${t("no_memory")}</li>`; return; }
  list.innerHTML = d.entries
    .map((e) => `<li>[${escapeHtml(e.category)}] ${escapeHtml(e.content)}<div class="m-ts">${e.ts} · ${escapeHtml((e.tags || []).join(", "))}</div></li>`)
    .join("");
}

/* ============================ 初始化 ============================ */

$("#btn-lang").addEventListener("click", () => setLang(lang === "zh" ? "en" : "zh"));
applyI18n();

(async function init() {
  await loadSessions();
  if (state.sessionId) await renderHistory();
  loadTasks();
})();
