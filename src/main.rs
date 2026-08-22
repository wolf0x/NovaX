//! Nova Agent：基于 ADK-Rust v2 的自主智能体，单一二进制，默认 8899 端口 Web 服务（config.toml 可改）。

mod agent;
mod audit;
mod config;
mod events;
mod gate;
mod intent;
mod mcp;
mod memory;
mod server;
mod skills;
mod tools;

use config::ConfigStore;
use events::EventBus;
use gate::{ApprovalManager, GateKeeper};
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    // 1. 配置（相对于启动目录解析，启动日志会打印实际路径）
    let config_path = std::env::current_dir()?.join("config.toml");
    let store = ConfigStore::load(&config_path)?;
    let cfg = store.get().await;
    println!("配置文件: {}", config_path.display());
    let data_dir = cfg.paths.data_dir.clone();
    let skills_dir = cfg.paths.skills_dir.clone();
    std::fs::create_dir_all(&data_dir).ok();
    std::fs::create_dir_all(&skills_dir).ok();

    // 2. 基础服务
    let audit = Arc::new(audit::AuditLog::open(format!("{data_dir}/audit.db"))?);
    let memory = Arc::new(memory::MemoryStore::load(format!("{data_dir}/memory.jsonl"))?);
    let bus = EventBus::new();
    let approvals = ApprovalManager::new(bus.clone(), audit.clone());
    let gate = GateKeeper::new(store.clone(), approvals.clone(), audit.clone());
    let skills = skills::SkillRegistry::new(&skills_dir).await;
    let tasks = tools::planner::TaskStore::load(format!("{data_dir}/tasks.json"), bus.clone())?;

    // 3. MCP 连接
    let mcp = mcp::McpHub::new();
    mcp.rebuild(&cfg.mcp_servers).await;

    // 4. 智能体（模型未配置时仍启动服务，可在 Settings 中配置后热生效）
    let hub = agent::AgentHub::new(
        store.clone(),
        skills.clone(),
        mcp.clone(),
        gate,
        audit.clone(),
        memory.clone(),
        tasks.clone(),
        bus.clone(),
    );
    match hub.rebuild().await {
        Ok(()) => tracing::info!("agent ready ({} / {})", cfg.model.provider, cfg.model.model),
        Err(e) => tracing::warn!(error = %e, "agent not ready; configure model in Settings"),
    }
    audit.log(
        "system",
        "startup",
        "",
        &format!("provider={}, model={}", cfg.model.provider, cfg.model.model),
        "ok",
        "",
        0,
    );

    // 5. Web 服务
    let state = server::AppState {
        config: store,
        audit,
        memory,
        bus,
        approvals,
        skills,
        tasks,
        mcp,
        hub,
    };
    server::serve(state).await?;
    Ok(())
}
