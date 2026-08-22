# NovaX

基于 [ADK-Rust v2](https://github.com/zavora-ai/adk-rust) 构建的自主智能体。单一二进制文件，运行于 Linux，启动后提供 Web 服务（默认 `127.0.0.1:8899`），通过对话指导智能体完成系统操作任务。

## 特性

- **五项系统能力 + 门禁控制**：读 / 写 / 改 / 删 / 运行各自独立开关；高危操作执行前在页面上人工审批
- **命令意图判定**：`sys_execute` 执行前由 LLM 判定命令最终意图（穿透 python/管道/base64 等脚本形式），意图良性直接放行，触及管控能力或具有破坏性则转入审批；判定不可用时保守兜底（fail-closed）
- **多提供商可配置**：DeepSeek / Gemini / OpenAI 兼容端点 / Ollama 本地 / Anthropic，全部配置在 Web Settings 中修改并持久化到 `config.toml`
- **SKILLS 技能**：`skills/<名称>/SKILL.md`（frontmatter 含 name/description），自动注入系统指令，按需全文加载
- **MCP 集成**：支持 stdio 与 Streamable HTTP 两种传输，Settings 中增删并热重连
- **本地记忆**：JSONL 长期记忆，关键词 + 时间衰减召回，智能体自主存取
- **任务规划**：多步骤任务自动拆解为计划，实时推送进度卡片
- **审计日志**：工具调用 / 门禁决策 / 审批 / 对话 / 配置变更全部落 SQLite，Web 页可查询
- **中英双语界面**，流式 Markdown 输出

## 快速开始

```bash
# 从 Releases 下载二进制（或自行构建：cargo build --release）
tar -xzf nova-agent-v*-linux-x86_64.tar.gz
./nova-agent
# 访问 http://127.0.0.1:8899 ，在「设置」中填入模型 API Key 并保存
```

首次启动会在运行目录自动生成 `config.toml`（参见 [config.example.toml](config.example.toml)）。**请从项目根目录启动**，配置/数据/技能路径相对启动目录解析。

## 配置说明（config.toml）

| 段 | 说明 |
|---|---|
| `[server]` | 监听地址与端口（默认 127.0.0.1:8899） |
| `[model]` | provider / api_key / model / base_url / max_iterations |
| `[gates]` | read/write/edit/delete/execute 五项能力开关 |
| `[approval]` | 需人工审批的能力列表与超时 |
| `[[mcp_servers]]` | MCP 服务器（name/transport/command/args/env/url/enabled） |
| `[agent]` | 附加系统指令 |

## 构建

```bash
cargo build --release   # 产物为 target/release/nova-agent
cargo test              # 单元测试
```

## 安全须知

- 服务默认仅绑定 `127.0.0.1`，请勿直接暴露到公网（无多用户鉴权）
- `config.toml` 含 API Key，注意保管，勿提交到版本库
- 删除/运行等能力默认关闭或需审批，建议在熟悉行为前保持该默认
