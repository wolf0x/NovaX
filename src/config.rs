//! 配置管理：所有配置项持久化于 config.toml，Web Settings 页全量覆盖。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

pub const DEFAULT_PORT: u16 = 8899;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: DEFAULT_PORT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelConfig {
    /// gemini | openai_compatible | deepseek | ollama | anthropic
    pub provider: String,
    pub api_key: String,
    pub model: String,
    /// OpenAI 兼容端点 / Ollama host 使用
    pub base_url: String,
    pub max_iterations: u32,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            provider: "deepseek".into(),
            api_key: String::new(),
            model: "deepseek-chat".into(),
            base_url: String::new(),
            max_iterations: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GatesConfig {
    pub read: bool,
    pub write: bool,
    pub edit: bool,
    pub delete: bool,
    pub execute: bool,
}

impl Default for GatesConfig {
    fn default() -> Self {
        Self {
            read: true,
            write: true,
            edit: true,
            delete: false,
            execute: false,
        }
    }
}

impl GatesConfig {
    pub fn is_enabled(&self, capability: &str) -> bool {
        match capability {
            "read" => self.read,
            "write" => self.write,
            "edit" => self.edit,
            "delete" => self.delete,
            "execute" => self.execute,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ApprovalConfig {
    /// 需要人工审批的能力列表，例如 ["delete", "execute"]
    pub require_for: Vec<String>,
    pub timeout_secs: u64,
}

impl Default for ApprovalConfig {
    fn default() -> Self {
        Self {
            require_for: vec!["delete".into(), "execute".into()],
            timeout_secs: 300,
        }
    }
}

impl ApprovalConfig {
    pub fn requires(&self, capability: &str) -> bool {
        self.require_for.iter().any(|c| c == capability)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IntentConfig {
    /// 是否对命令执行做意图判定；关闭后回退到关键词启发式（删除类命令仍受 delete 门禁）
    pub enabled: bool,
    /// 单次判定的超时（秒）
    pub timeout_secs: u64,
}

impl Default for IntentConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout_secs: 45,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PathsConfig {
    pub data_dir: String,
    pub skills_dir: String,
}

impl Default for PathsConfig {
    fn default() -> Self {
        Self {
            data_dir: "data".into(),
            skills_dir: "skills".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct McpServerConfig {
    pub name: String,
    /// stdio | http
    pub transport: String,
    /// stdio: 可执行文件路径；http: 留空
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    /// http: MCP 服务地址
    pub url: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentBehavior {
    /// 附加的系统指令
    pub extra_instruction: String,
}

impl Default for AgentBehavior {
    fn default() -> Self {
        Self {
            extra_instruction: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub model: ModelConfig,
    pub gates: GatesConfig,
    pub approval: ApprovalConfig,
    pub intent: IntentConfig,
    pub paths: PathsConfig,
    pub agent: AgentBehavior,
    pub mcp_servers: Vec<McpServerConfig>,
}

/// 配置存储：读写 config.toml，内存中提供热更新。
pub struct ConfigStore {
    path: PathBuf,
    inner: RwLock<Config>,
}

impl ConfigStore {
    /// 从文件加载；不存在则写入默认配置。
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Arc<Self>> {
        let path = path.as_ref().to_path_buf();
        let config = if path.exists() {
            let raw = std::fs::read_to_string(&path)?;
            let mut cfg: Config = toml::from_str(&raw)?;
            // 兜底：确保危险能力默认关闭的状态不被旧文件意外打开
            cfg.mcp_servers.retain(|s| !s.name.is_empty());
            cfg
        } else {
            let cfg = Config::default();
            let raw = toml::to_string_pretty(&cfg)?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::write(&path, raw)?;
            cfg
        };
        Ok(Arc::new(Self {
            path,
            inner: RwLock::new(config),
        }))
    }

    pub async fn get(&self) -> Config {
        self.inner.read().await.clone()
    }

    /// 在配置读锁下执行闭包（供需要原子读取快照的场景使用）。
    #[allow(dead_code)]
    pub async fn read_with<R>(&self, f: impl FnOnce(&Config) -> R) -> R {
        let guard = self.inner.read().await;
        f(&guard)
    }

    /// 全量替换配置并原子写回磁盘。
    pub async fn replace(&self, new_config: Config) -> anyhow::Result<()> {
        let raw = toml::to_string_pretty(&new_config)?;
        let tmp = self.path.with_extension("toml.tmp");
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(&tmp, raw)?;
        std::fs::rename(&tmp, &self.path)?;
        *self.inner.write().await = new_config;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn roundtrip_config() {
        let dir = std::env::temp_dir().join(format!("nova_cfg_{}", uuid_like()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let store = ConfigStore::load(&path).unwrap();
        let mut cfg = store.get().await;
        cfg.server.port = 9999;
        cfg.gates.delete = true;
        cfg.model.provider = "ollama".into();
        cfg.mcp_servers.push(McpServerConfig {
            name: "fs".into(),
            transport: "stdio".into(),
            command: "npx".into(),
            args: vec!["-y".into()],
            enabled: true,
            ..Default::default()
        });
        store.replace(cfg.clone()).await.unwrap();

        let store2 = ConfigStore::load(&path).unwrap();
        let cfg2 = store2.get().await;
        assert_eq!(cfg2.server.port, 9999);
        assert!(cfg2.gates.delete);
        assert_eq!(cfg2.mcp_servers.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    fn uuid_like() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string()
    }

    #[test]
    fn gate_matrix() {
        let gates = GatesConfig::default();
        assert!(gates.is_enabled("read"));
        assert!(!gates.is_enabled("delete"));
        assert!(!gates.is_enabled("execute"));
        let approval = ApprovalConfig::default();
        assert!(approval.requires("delete"));
        assert!(approval.requires("execute"));
        assert!(!approval.requires("read"));
    }
}
