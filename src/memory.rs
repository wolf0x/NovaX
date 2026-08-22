//! 本地记忆：JSONL 存储 + 关键词匹配与时间衰减召回。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    /// RFC3339 时间戳
    pub ts: String,
    /// user | preference | fact | task | other
    pub category: String,
    pub content: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryHit {
    #[serde(flatten)]
    pub entry: MemoryEntry,
    pub score: f64,
}

pub struct MemoryStore {
    path: PathBuf,
    entries: Mutex<Vec<MemoryEntry>>,
}

impl MemoryStore {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut entries = Vec::new();
        if path.exists() {
            let raw = std::fs::read_to_string(&path)?;
            for line in raw.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(e) = serde_json::from_str::<MemoryEntry>(line) {
                    entries.push(e);
                }
            }
        }
        Ok(Self {
            path,
            entries: Mutex::new(entries),
        })
    }

    pub fn add(&self, category: &str, content: &str, tags: &[String]) -> anyhow::Result<MemoryEntry> {
        let entry = MemoryEntry {
            id: uuid_v4(),
            ts: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            category: if category.is_empty() { "other" } else { category }.to_string(),
            content: content.to_string(),
            tags: tags.to_vec(),
        };
        self.append_to_file(&entry)?;
        self.entries.lock().unwrap().push(entry.clone());
        Ok(entry)
    }

    /// 关键词匹配 + 时间衰减打分，返回前 limit 条。
    pub fn recall(&self, query: &str, category: &str, limit: usize) -> Vec<MemoryHit> {
        let keywords: Vec<String> = query
            .split(|c: char| !(c.is_alphanumeric() || c == '_' || ('\u{4e00}'..='\u{9fff}').contains(&c)))
            .filter(|w| !w.is_empty())
            .map(|w| w.to_lowercase())
            .collect();
        let now = chrono::Utc::now();
        let entries = self.entries.lock().unwrap();
        let mut hits: Vec<MemoryHit> = entries
            .iter()
            .filter(|e| category.is_empty() || e.category == category)
            .filter_map(|e| {
                let text = format!("{} {}", e.content, e.tags.join(" ")).to_lowercase();
                let matched = keywords.iter().filter(|k| text.contains(k.as_str())).count();
                if keywords.is_empty() || matched == 0 {
                    // 无关键词时返回全部（按时间衰减排序）；无匹配则丢弃
                    if keywords.is_empty() {
                        let age_days = age_days(&e.ts, &now);
                        return Some(MemoryHit {
                            entry: e.clone(),
                            score: 1.0 / (1.0 + age_days),
                        });
                    }
                    return None;
                }
                let base = matched as f64 / keywords.len() as f64;
                let age_days = age_days(&e.ts, &now);
                let decay = 1.0 / (1.0 + age_days / 30.0);
                Some(MemoryHit {
                    entry: e.clone(),
                    score: base * 0.7 + decay * 0.3,
                })
            })
            .collect();
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(limit);
        hits
    }

    pub fn forget(&self, id: &str) -> anyhow::Result<bool> {
        let mut entries = self.entries.lock().unwrap();
        let before = entries.len();
        entries.retain(|e| e.id != id);
        let removed = entries.len() < before;
        if removed {
            self.rewrite_file(&entries)?;
        }
        Ok(removed)
    }

    pub fn list(&self, limit: usize) -> Vec<MemoryEntry> {
        let entries = self.entries.lock().unwrap();
        entries
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }

    fn append_to_file(&self, entry: &MemoryEntry) -> anyhow::Result<()> {
        use std::io::Write;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{}", serde_json::to_string(entry)?)?;
        Ok(())
    }

    fn rewrite_file(&self, entries: &[MemoryEntry]) -> anyhow::Result<()> {
        let raw: Vec<String> = entries
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<_, _>>()?;
        std::fs::write(&self.path, raw.join("\n") + "\n")?;
        Ok(())
    }
}

fn age_days(ts: &str, now: &chrono::DateTime<chrono::Utc>) -> f64 {
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|t| (now.timestamp() - t.timestamp()).max(0) as f64 / 86400.0)
        .unwrap_or(0.0)
}

pub fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_recall_forget() {
        let dir = std::env::temp_dir().join(format!(
            "nova_mem_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = MemoryStore::load(dir.join("memory.jsonl")).unwrap();
        let e1 = store.add("preference", "用户喜欢深色主题", &["theme".into()]).unwrap();
        store.add("fact", "项目部署在 Linux 服务器上", &["deploy".into()]).unwrap();

        let hits = store.recall("深色主题", "", 5);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].entry.id, e1.id);

        assert!(store.forget(&e1.id).unwrap());
        let hits2 = store.recall("深色主题", "", 5);
        assert!(hits2.is_empty());

        // 重新加载后依然持久
        let store2 = MemoryStore::load(dir.join("memory.jsonl")).unwrap();
        assert_eq!(store2.list(10).len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
