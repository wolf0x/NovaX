//! SKILLS 技能：扫描 skills/ 目录下的 SKILL.md，解析 frontmatter 建索引，
//! 摘要注入系统指令，全文经 load_skill 工具按需加载。

use crate::audit::AuditLog;
use adk_rust::prelude::*;
use adk_rust::tool::FunctionTool;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: String,
}

pub struct SkillRegistry {
    dir: PathBuf,
    skills: RwLock<Vec<Skill>>,
}

impl SkillRegistry {
    pub async fn new(dir: impl Into<PathBuf>) -> Arc<Self> {
        let dir = dir.into();
        let registry = Arc::new(Self {
            dir,
            skills: RwLock::new(Vec::new()),
        });
        registry.rescan().await;
        registry
    }

    /// 递归查找所有 SKILL.md 并重建索引。
    pub async fn rescan(&self) {
        let mut found = Vec::new();
        collect_skill_files(&self.dir, &mut found, 0);
        let mut skills = Vec::new();
        for path in found {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                let (name, description) = parse_frontmatter(&raw, &path);
                skills.push(Skill {
                    name,
                    description,
                    path: path.to_string_lossy().to_string(),
                });
            }
        }
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        *self.skills.write().await = skills;
    }

    pub async fn list(&self) -> Vec<Skill> {
        self.skills.read().await.clone()
    }

    /// 生成注入系统指令的技能索引文本；无技能时为空串。
    pub async fn index_for_instruction(&self) -> String {
        let skills = self.skills.read().await;
        if skills.is_empty() {
            return String::new();
        }
        let mut out = String::from("可用技能（SKILLS）：当用户需求与某技能相关时，先用 load_skill 加载其完整内容并严格遵循。\n");
        for s in skills.iter() {
            out.push_str(&format!("- {}: {}\n", s.name, s.description));
        }
        out
    }

    pub async fn load_content(&self, name: &str) -> Option<String> {
        let skills = self.skills.read().await;
        let skill = skills.iter().find(|s| s.name == name)?;
        std::fs::read_to_string(&skill.path).ok()
    }

    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }
}

fn collect_skill_files(dir: &std::path::Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 4 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_skill_files(&path, out, depth + 1);
        } else if path.file_name().and_then(|n| n.to_str()) == Some("SKILL.md") {
            out.push(path);
        }
    }
}

/// 解析 YAML frontmatter 中的 name / description；缺失时回退到目录名与首行。
fn parse_frontmatter(raw: &str, path: &std::path::Path) -> (String, String) {
    let fallback_name = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unnamed")
        .to_string();

    let trimmed = raw.trim_start();
    if let Some(rest) = trimmed.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let fm = &rest[..end];
            let name = fm
                .lines()
                .find_map(|l| strip_field(l, "name"))
                .unwrap_or_else(|| fallback_name.clone());
            let description = fm
                .lines()
                .find_map(|l| strip_field(l, "description"))
                .unwrap_or_default();
            return (name, description);
        }
    }
    let first_line = raw.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim_start_matches('#').trim();
    (fallback_name, first_line.to_string())
}

fn strip_field(line: &str, field: &str) -> Option<String> {
    let line = line.trim();
    let prefix = format!("{field}:");
    if let Some(v) = line.strip_prefix(&prefix) {
        let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    None
}

pub fn build_load_skill_tool(registry: Arc<SkillRegistry>, audit: Arc<AuditLog>) -> Arc<dyn Tool> {
    Arc::new(FunctionTool::new(
        "load_skill",
        "按名称加载一个技能的完整内容（SKILL.md 全文），然后按其中的指引执行。参数: name (string, 技能名)",
        move |_ctx, args| {
            let registry = registry.clone();
            let audit = audit.clone();
            async move {
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or_default();
                if name.is_empty() {
                    return Ok(json!({ "error": "缺少参数 name" }));
                }
                match registry.load_content(name).await {
                    Some(content) => {
                        audit.log("tool_call", "load_skill", &args.to_string(), name, "ok", "", 0);
                        Ok(json!({ "name": name, "content": content }))
                    }
                    None => Ok(json!({ "error": format!("未找到技能 {name}") })),
                }
            }
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_parsing() {
        let raw = "---\nname: code-review\ndescription: 代码审查专家\n---\n# 正文";
        let (name, desc) = parse_frontmatter(raw, std::path::Path::new("/tmp/x/SKILL.md"));
        assert_eq!(name, "code-review");
        assert_eq!(desc, "代码审查专家");

        // 无 frontmatter 回退
        let (name2, _) = parse_frontmatter("# 标题\n内容", std::path::Path::new("/tmp/mydir/SKILL.md"));
        assert_eq!(name2, "mydir");
    }
}
