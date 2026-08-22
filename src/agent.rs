//! 智能体核心：多提供商模型工厂 + 工具装配 + Runner 管理（配置变更热重建）。

use crate::config::{Config, ConfigStore, ModelConfig};
use crate::events::EventBus;
use crate::gate::GateKeeper;
use crate::mcp::McpHub;
use crate::memory::MemoryStore;
use crate::skills::SkillRegistry;
use crate::tools::{self, planner::TaskStore};
use crate::audit::AuditLog;
use adk_rust::prelude::*;
use adk_rust::runner::Runner;
use adk_rust::session::{CreateRequest, GetRequest, InMemorySessionService, ListRequest, SessionService};
use adk_rust::{SessionId, UserId};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

const APP_NAME: &str = "nova_agent";
const USER_ID: &str = "local_user";

pub fn user_id() -> UserId {
    UserId::new(USER_ID).expect("static user id")
}

pub fn app_name() -> &'static str {
    APP_NAME
}

/// 根据配置构建 LLM 客户端
pub fn build_model(cfg: &ModelConfig) -> anyhow::Result<Arc<dyn Llm>> {
    match cfg.provider.as_str() {
        "gemini" => {
            anyhow::ensure!(!cfg.api_key.is_empty(), "Gemini 需要配置 api_key");
            Ok(Arc::new(GeminiModel::new(cfg.api_key.clone(), cfg.model.clone())?))
        }
        "openai_compatible" => {
            anyhow::ensure!(!cfg.api_key.is_empty(), "OpenAI 兼容端点需要配置 api_key");
            let c = if cfg.base_url.is_empty() {
                OpenAIConfig::new(cfg.api_key.clone(), cfg.model.clone())
            } else {
                OpenAIConfig::compatible(cfg.api_key.clone(), cfg.base_url.clone(), cfg.model.clone())
            };
            Ok(Arc::new(OpenAIClient::new(c)?))
        }
        "deepseek" => {
            anyhow::ensure!(!cfg.api_key.is_empty(), "DeepSeek 需要配置 api_key");
            let mut c = DeepSeekConfig::new(cfg.api_key.clone(), cfg.model.clone());
            if !cfg.base_url.is_empty() {
                c = c.with_base_url(cfg.base_url.clone());
            }
            Ok(Arc::new(DeepSeekClient::new(c)?))
        }
        "ollama" => {
            let c = if cfg.base_url.is_empty() {
                OllamaConfig::new(cfg.model.clone())
            } else {
                OllamaConfig::with_host(cfg.base_url.clone(), cfg.model.clone())
            };
            Ok(Arc::new(OllamaModel::new(c)?))
        }
        "anthropic" => {
            anyhow::ensure!(!cfg.api_key.is_empty(), "Anthropic 需要配置 api_key");
            let c = AnthropicConfig::new(cfg.api_key.clone(), cfg.model.clone());
            Ok(Arc::new(AnthropicClient::new(c)?))
        }
        other => anyhow::bail!("不支持的 provider: {other}"),
    }
}

/// AgentHub：持有 Runner 与会话服务；配置 / 技能 / MCP 变更时重建。
pub struct AgentHub {
    runner: tokio::sync::RwLock<Option<Arc<Runner>>>,
    session_service: Arc<InMemorySessionService>,
    config: Arc<ConfigStore>,
    skills: Arc<SkillRegistry>,
    mcp: Arc<McpHub>,
    gate: Arc<GateKeeper>,
    audit: Arc<AuditLog>,
    memory: Arc<MemoryStore>,
    tasks: Arc<TaskStore>,
    bus: EventBus,
    /// 工作区根目录（与二进制同级）
    workspace: PathBuf,
    /// 制品输出目录（workspace/output）
    output_dir: PathBuf,
}

impl AgentHub {
    pub fn new(
        config: Arc<ConfigStore>,
        skills: Arc<SkillRegistry>,
        mcp: Arc<McpHub>,
        gate: Arc<GateKeeper>,
        audit: Arc<AuditLog>,
        memory: Arc<MemoryStore>,
        tasks: Arc<TaskStore>,
        bus: EventBus,
        workspace: PathBuf,
        output_dir: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            runner: tokio::sync::RwLock::new(None),
            session_service: Arc::new(InMemorySessionService::new()),
            config,
            skills,
            mcp,
            gate,
            audit,
            memory,
            tasks,
            bus,
            workspace,
            output_dir,
        })
    }

    /// 构建（或重建）智能体与 Runner。返回错误说明（如模型配置缺失）。
    pub async fn rebuild(&self) -> anyhow::Result<()> {
        let cfg = self.config.get().await;
        let model = build_model(&cfg.model)?;
        let agent = self.build_agent(&cfg, model).await?;

        let runner = Runner::builder()
            .app_name(APP_NAME)
            .agent(Arc::new(agent))
            .session_service(self.session_service.clone() as Arc<dyn adk_rust::session::SessionService>)
            .build()?;
        *self.runner.write().await = Some(Arc::new(runner));
        Ok(())
    }

    async fn build_agent(&self, cfg: &Config, model: Arc<dyn Llm>) -> anyhow::Result<LlmAgent> {
        let instruction = self.compose_instruction(cfg).await;

        let sys_deps = tools::system::SystemToolDeps {
            gate: self.gate.clone(),
            audit: self.audit.clone(),
            bus: self.bus.clone(),
            judge: crate::intent::LlmIntentClassifier::new(self.config.clone()),
        };
        let mut tools: Vec<Arc<dyn Tool>> = tools::system::build_system_tools(&sys_deps);
        tools.extend(tools::memory_tools::build_memory_tools(
            self.memory.clone(),
            self.audit.clone(),
        ));
        tools.extend(tools::planner::build_planner_tools(self.tasks.clone()));
        tools.push(crate::skills::build_load_skill_tool(
            self.skills.clone(),
            self.audit.clone(),
        ));

        let mut builder = LlmAgentBuilder::new(APP_NAME)
            .description("运行于本地系统的自主智能体，具备受门禁管控的读/写/改/删/运行能力")
            .instruction(instruction)
            .model(model)
            .max_iterations(cfg.model.max_iterations.max(5));

        for t in tools {
            builder = builder.tool(t);
        }
        for ts in self.mcp.toolsets().await {
            builder = builder.toolset(ts);
        }
        Ok(builder.build()?)
    }

    async fn compose_instruction(&self, cfg: &Config) -> String {
        let skills_index = self.skills.index_for_instruction().await;
        let gates = &cfg.gates;
        let capability_line = format!(
            "当前系统能力开关：读(read)={}，写(write)={}，改(edit)={}，删(delete)={}，运行(execute)={}。",
            on_off(gates.read),
            on_off(gates.write),
            on_off(gates.edit),
            on_off(gates.delete),
            on_off(gates.execute),
        );
        let approval_line = format!(
            "需要人工审批的能力：{}。调用这些能力时工具会挂起等待用户在页面上批准，请耐心等待，不要重复调用。",
            if cfg.approval.require_for.is_empty() {
                "无".to_string()
            } else {
                cfg.approval.require_for.join("、")
            }
        );

        let workspace_line = format!(
            "工作区根目录：{}\n制品输出目录：{}（所有生成的文件、报告、制品必须写入此目录下）\n",
            self.workspace.display(),
            self.output_dir.display(),
        );

        format!(
            "你是 Nova，一个运行在本地 Linux 系统上的自主智能体（基于 ADK-Rust 构建）。\n\n\
## 能力\n\
你通过工具与系统交互：sys_read（读）、sys_write（写）、sys_edit（改）、sys_delete（删）、sys_execute（运行命令）。{capability_line}\n\
{approval_line}\n\
sys_execute 执行前会先对命令做意图判定（含 python 等内嵌脚本）：意图不触及需审批能力时放行，触及被管控能力或具有破坏性时转入人工审批。\n\
严禁绕过门禁：不得用其他工具完成被禁用能力对应的操作（例如删除能力被禁用时，绝对不允许通过任何形式的命令或脚本删除文件，也不得用写文件方式变相删除）。若某操作被拒绝或禁用，向用户解释原因并给出替代建议，不要尝试同参数的变体重试。\n\n\
外部 MCP 工具若可用，可直接调用；但 MCP 工具同样受门禁约束：绝对不得用 MCP 工具完成被禁用能力对应的操作（如用文件系统类 MCP 工具删除/写入被禁的能力）。\n\n\
## 工作区\n\
{workspace_line}\
用 sys_write 创建文件时，除非用户明确指定了其他路径，否则一律使用制品输出目录的绝对路径。\n\n\
## 记忆与规划\n\
- 用 memory_remember 保存用户偏好、约定与重要事实；回答前先 memory_recall 检索相关记忆。\n\
- 多步骤任务（≥3 步）必须先用 task_plan 制定计划，每完成一步用 task_update 更新进度。\n\n\
## 输出规范\n\
- 始终使用清晰、结构化的 Markdown 回复：合理使用标题、列表、代码块（注明语言）、表格与加粗。\n\
- 展示文件内容或命令输出时使用代码块。\n\
- 输出语言：默认跟随每条用户消息附带的界面语言提示（与前端语言切换联动）；用户在消息中明确指定语言时，严格按用户指定回复；两者都缺失时默认使用中文。\n\n\
{skills_index}\n\
{extra}",
            extra = cfg.agent.extra_instruction
        )
    }

    pub async fn runner(&self) -> Option<Arc<Runner>> {
        self.runner.read().await.clone()
    }

    pub fn session_service(&self) -> Arc<InMemorySessionService> {
        self.session_service.clone()
    }

    /// 创建新会话
    pub async fn create_session(&self) -> anyhow::Result<String> {
        let session_id = SessionId::generate();
        self.session_service
            .create(CreateRequest {
                app_name: APP_NAME.to_string(),
                user_id: USER_ID.to_string(),
                session_id: Some(session_id.to_string()),
                state: HashMap::new(),
            })
            .await?;
        Ok(session_id.to_string())
    }

    /// 获取某会话的历史事件（用于前端重放）
    pub async fn session_history(&self, session_id: &str) -> Vec<serde_json::Value> {
        let Ok(session) = self
            .session_service
            .get(GetRequest {
                app_name: APP_NAME.to_string(),
                user_id: USER_ID.to_string(),
                session_id: session_id.to_string(),
                num_recent_events: None,
                after: None,
            })
            .await
        else {
            return Vec::new();
        };
        let events = session.events().all();
        history_to_messages(&events)
    }

    /// 列出所有会话的 id 与事件数
    pub async fn list_sessions(&self) -> Vec<serde_json::Value> {
        let Ok(sessions) = self
            .session_service
            .list(ListRequest {
                app_name: APP_NAME.to_string(),
                user_id: USER_ID.to_string(),
                limit: None,
                offset: None,
            })
            .await
        else {
            return Vec::new();
        };
        sessions
            .iter()
            .map(|s| {
                serde_json::json!({
                    "session_id": s.id(),
                    "events": s.events().len(),
                })
            })
            .collect()
    }
}

fn on_off(b: bool) -> &'static str {
    if b { "开启" } else { "关闭" }
}

/// 将会话事件转换为前端可渲染的消息列表
pub fn history_to_messages(events: &[Event]) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for event in events {
        let Some(content) = &event.llm_response.content else {
            continue;
        };
        let role = content.role.as_str();
        if role != "user" && role != "model" {
            continue;
        }
        let mut text = String::new();
        for part in &content.parts {
            if let Part::Text { text: t } = part {
                text.push_str(t);
            }
        }
        // 剥离服务端注入的界面语言提示行，历史回放只展示用户原文
        if let Some(rest) = text.strip_prefix("(系统提示：") {
            if let Some(pos) = rest.find(")\n") {
                text = rest[pos + 2..].to_string();
            }
        } else if let Some(rest) = text.strip_prefix("(System hint:") {
            if let Some(pos) = rest.find(")\n") {
                text = rest[pos + 2..].to_string();
            }
        }
        if text.trim().is_empty() {
            continue;
        }
        out.push(serde_json::json!({ "role": role, "text": text }));
    }
    out
}
