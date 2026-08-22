//! 命令意图判定模块（可插拔）。
//!
//! 对外暴露 [`IntentClassifier`] trait 与统一的 [`IntentVerdict`] 结果；
//! 当前内置 [`LlmIntentClassifier`]（由主模型穿透 python/管道/base64 等表面形式
//! 判定最终意图）。未来可新增实现（规则引擎、专用小模型等），在装配处替换即可，
//! 门禁流水线（`tools/system.rs`）无需改动。
//! 行为开关与超时见 config.toml 的 `[intent]` 段。

use crate::agent::build_model;
use crate::config::ConfigStore;
use adk_rust::{Content, GenerateContentConfig, LlmRequest, Part};
use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// 意图判定结果（所有分类器实现统一输出）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentVerdict {
    /// 涉及的系统能力：read / write / edit / delete 的子集；为空表示未触及文件类能力
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// 是否具有破坏性/不可逆（删除、格式化、杀进程、改系统配置等）
    #[serde(default)]
    pub dangerous: bool,
    /// 简短理由（展示给审批用户）
    #[serde(default)]
    pub reason: String,
}

/// 命令意图分类器：门禁流水线的可插拔扩展点。
///
/// 返回 `Err` 表示分类器不可用（未配置模型、调用失败、输出不可解析等），
/// 调用方必须走保守兑底路径，不得直接放行。
#[async_trait]
pub trait IntentClassifier: Send + Sync {
    async fn classify(&self, command: &str) -> anyhow::Result<IntentVerdict>;
}

/// 基于主模型的 LLM 意图分类器（当前默认实现）。
pub struct LlmIntentClassifier {
    config: Arc<ConfigStore>,
}

impl LlmIntentClassifier {
    pub fn new(config: Arc<ConfigStore>) -> Arc<Self> {
        Arc::new(Self { config })
    }
}

const JUDGE_PROMPT: &str = "你是安全门禁的「命令意图判定器」。分析给定的 shell 命令，判定它的最终意图。\
命令可能包含管道、链式命令、内嵌 python/node/perl/ruby 等脚本、heredoc、base64 解码后执行等形式，\
你必须穿透表面形式，判定它实际执行的操作。\
判定涉及的系统能力（可多选，只填确实会发生的）：\
- read：读取文件/目录内容（如 cat/ls/grep/find 输出、python 打开文件读取）\
- write：新建文件或整体覆盖已有文件内容（如重定向写入/tee/cp 目标为文件、python 写文件）\
- edit：修改已有文件的部分内容（如 sed -i/patch/脚本读改写）\
- delete：删除、清空或截断文件/目录（如 rm/unlink、python os.remove/shutil.rmtree、truncate、向磁盘分区写裸数据、drop table 等）\
若命令不触及上述能力（纯计算、查看系统状态、网络请求、打印输出等），则 capabilities 为空数组。\
同时判定 dangerous：命令是否不可逆或有破坏性（删除数据、格式化、强制杀进程、修改系统关键配置、提权操作等）。\
只输出一行 JSON，不要输出任何其他文字，格式：\
{\"capabilities\":[\"delete\"],\"dangerous\":true,\"reason\":\"简短理由\"}";

#[async_trait]
impl IntentClassifier for LlmIntentClassifier {
    async fn classify(&self, command: &str) -> anyhow::Result<IntentVerdict> {
        let cfg = self.config.get().await;
        let model = build_model(&cfg.model)?;

        let request = LlmRequest {
            model: cfg.model.model.clone(),
            contents: vec![Content::new("user")
                .with_text(&format!("{JUDGE_PROMPT}\n\n待判定的命令：\n{command}"))],
            config: Some(GenerateContentConfig {
                temperature: Some(0.0),
                ..Default::default()
            }),
            tools: HashMap::new(),
            previous_response_id: None,
        };

        let timeout = Duration::from_secs(cfg.intent.timeout_secs.max(5));
        let stream = model.generate_content(request, false).await?;
        let raw = tokio::time::timeout(timeout, collect_text(stream))
            .await
            .map_err(|_| anyhow::anyhow!("意图判定超时"))??;

        parse_verdict(&raw)
    }
}

async fn collect_text(
    mut stream: adk_rust::LlmResponseStream,
) -> adk_rust::Result<String> {
    let mut text = String::new();
    while let Some(item) = stream.next().await {
        let resp = item?;
        if let Some(err) = &resp.error_message {
            return Err(adk_rust::AdkError::model(err.clone()));
        }
        if let Some(content) = &resp.content {
            for part in &content.parts {
                if let Part::Text { text: t } = part {
                    text.push_str(t);
                }
            }
        }
    }
    Ok(text)
}

/// 从模型输出中提取并校验 JSON 判定（容忍代码块包裹、前后杂散文字、
/// 以及模型在 JSON 后追加额外内容/多输出一个 JSON 的情况——只取第一个完整对象）。
fn parse_verdict(raw: &str) -> anyhow::Result<IntentVerdict> {
    let start = raw.find('{').ok_or_else(|| anyhow::anyhow!("模型未输出 JSON"))?;
    let slice = &raw[start..];
    // 流式解析：只读取第一个完整 JSON 对象，忽略其后的任何多余字符（避免 "trailing characters"）
    let deserializer = serde_json::Deserializer::from_str(slice);
    let mut verdict = match deserializer.into_iter::<IntentVerdict>().next() {
        Some(Ok(v)) => v,
        Some(Err(e)) => return Err(anyhow::anyhow!("判定 JSON 解析失败: {e}")),
        None => anyhow::bail!("模型输出的 JSON 不完整"),
    };
    // 只保留合法能力名，防止提示注入引入未知能力绕过门禁
    verdict.capabilities.retain(|c| matches!(c.as_str(), "read" | "write" | "edit" | "delete"));
    verdict.capabilities.sort();
    verdict.capabilities.dedup();
    Ok(verdict)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_verdict_tolerant() {
        let raw = "好的，判定如下：\n```json\n{\"capabilities\": [\"delete\", \"delete\", \"hack\"], \"dangerous\": true, \"reason\": \"删除文件\"}\n```";
        let v = parse_verdict(raw).unwrap();
        assert_eq!(v.capabilities, vec!["delete"]);
        assert!(v.dangerous);
        assert_eq!(v.reason, "删除文件");
    }

    #[test]
    fn parse_verdict_benign() {
        let raw = "{\"capabilities\": [], \"dangerous\": false, \"reason\": \"仅查看目录\"}";
        let v = parse_verdict(raw).unwrap();
        assert!(v.capabilities.is_empty());
        assert!(!v.dangerous);
    }

    #[test]
    fn parse_verdict_invalid() {
        assert!(parse_verdict("没有任何 JSON").is_err());
        assert!(parse_verdict("{\"capabilities\": ").is_err());
    }

    /// 模型在 JSON 后追加杂散文字或重复输出一个 JSON：只取第一个对象，不报 trailing characters。
    #[test]
    fn parse_verdict_trailing_content() {
        let raw = "{\"capabilities\":[\"write\"],\"dangerous\":false,\"reason\":\"下载并写入 /tmp 文件\"} 补充说明：该命令还会读取网络内容";
        let v = parse_verdict(raw).unwrap();
        assert_eq!(v.capabilities, vec!["write"]);
        assert!(!v.dangerous);

        let raw2 = "{\"capabilities\":[],\"dangerous\":false,\"reason\":\"仅查看\"}\n{\"capabilities\":[\"delete\"],\"dangerous\":true,\"reason\":\"第二个 JSON\"}";
        let v2 = parse_verdict(raw2).unwrap();
        assert!(v2.capabilities.is_empty()); // 取第一个
    }
}
