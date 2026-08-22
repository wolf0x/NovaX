//! MCP 集成：按 config.toml 中 [[mcp_servers]] 声明连接外部 MCP 服务器，
//! 聚合其工具为 Toolset 提供给智能体；Settings 变更后支持重建连接。

use crate::config::McpServerConfig;
use adk_rust::prelude::*;
use adk_rust::tool::mcp::rmcp::{transport::TokioChildProcess, ServiceExt};
use adk_rust::tool::{McpHttpClientBuilder, McpToolset};
use serde::Serialize;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize)]
pub struct McpStatus {
    pub name: String,
    pub transport: String,
    pub target: String,
    pub connected: bool,
    pub error: String,
}

struct ConnectedServer {
    status: McpStatus,
    toolset: Option<Arc<dyn Toolset>>,
}

pub struct McpHub {
    servers: RwLock<Vec<ConnectedServer>>,
}

impl McpHub {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            servers: RwLock::new(Vec::new()),
        })
    }

    /// 按配置重建全部 MCP 连接（先断开旧的）。
    pub async fn rebuild(self: &Arc<Self>, configs: &[McpServerConfig]) {
        // 断开旧连接：drop toolset 即释放客户端
        {
            let mut servers = self.servers.write().await;
            servers.clear();
        }

        let mut results = Vec::new();
        for cfg in configs {
            if cfg.name.is_empty() || !cfg.enabled {
                continue;
            }
            let result = match cfg.transport.as_str() {
                "http" => connect_http(cfg).await,
                _ => connect_stdio(cfg).await,
            };
            match result {
                Ok(toolset) => {
                    results.push(ConnectedServer {
                        status: McpStatus {
                            name: cfg.name.clone(),
                            transport: cfg.transport.clone(),
                            target: if cfg.transport == "http" {
                                cfg.url.clone()
                            } else {
                                format!("{} {}", cfg.command, cfg.args.join(" "))
                            },
                            connected: true,
                            error: String::new(),
                        },
                        toolset: Some(toolset),
                    });
                    tracing::info!(name = %cfg.name, "MCP server connected");
                }
                Err(e) => {
                    tracing::warn!(name = %cfg.name, error = %e, "MCP server connect failed");
                    results.push(ConnectedServer {
                        status: McpStatus {
                            name: cfg.name.clone(),
                            transport: cfg.transport.clone(),
                            target: if cfg.transport == "http" {
                                cfg.url.clone()
                            } else {
                                format!("{} {}", cfg.command, cfg.args.join(" "))
                            },
                            connected: false,
                            error: e.to_string(),
                        },
                        toolset: None,
                    });
                }
            }
        }
        *self.servers.write().await = results;
    }

    /// 当前已连接的 MCP Toolset 列表
    pub async fn toolsets(&self) -> Vec<Arc<dyn Toolset>> {
        self.servers
            .read()
            .await
            .iter()
            .filter_map(|s| s.toolset.clone())
            .collect()
    }

    pub async fn statuses(&self) -> Vec<McpStatus> {
        self.servers.read().await.iter().map(|s| s.status.clone()).collect()
    }
}

async fn connect_stdio(cfg: &McpServerConfig) -> anyhow::Result<Arc<dyn Toolset>> {
    anyhow::ensure!(!cfg.command.is_empty(), "stdio 服务器缺少 command");
    let mut command = Command::new(&cfg.command);
    command.args(&cfg.args);
    for (k, v) in &cfg.env {
        command.env(k, v);
    }
    let client = ()
        .serve(TokioChildProcess::new(command)?)
        .await
        .map_err(|e| anyhow::anyhow!("MCP 握手失败: {e}"))?;
    let toolset = McpToolset::new(client).with_name(&cfg.name);
    Ok(Arc::new(toolset))
}

async fn connect_http(cfg: &McpServerConfig) -> anyhow::Result<Arc<dyn Toolset>> {
    anyhow::ensure!(!cfg.url.is_empty(), "http 服务器缺少 url");
    let toolset = McpHttpClientBuilder::new(&cfg.url)
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("MCP HTTP 连接失败: {e}"))?
        .with_name(&cfg.name);
    Ok(Arc::new(toolset))
}
