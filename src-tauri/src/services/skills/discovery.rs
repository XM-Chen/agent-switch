//! Skills 发现与未托管扫描（cc-skills 阶段 C）。
//!
//! - `search`：通过 GitHub Search API 按关键字发现候选 skill 仓库；网络仅在显式调用时发生。
//! - `scan_unmanaged`：扫描各 app live skills 目录中「非 agent-switch 托管」的同名目录，
//!   供前端提示用户是否导入纳管。扫描只读，不改动 live 目录，也不联网。
//!
//! skills.sh 发现入口在阶段 C 首版按 GitHub 搜索实现；skills.sh 契约核实后再接入其专用来源。

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db::dao::skills;

use super::download;
use super::{is_managed_projection, SkillApp, ENTRY_FILE};

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredSkill {
    pub repo: String,
    pub owner: String,
    pub name: String,
    pub description: Option<String>,
    pub stars: u64,
    pub default_branch: Option<String>,
    pub html_url: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchReport {
    pub query: String,
    pub results: Vec<DiscoveredSkill>,
}

/// GitHub Search API 响应的最小子集。
#[derive(Debug, Deserialize)]
struct GithubSearchResponse {
    items: Vec<GithubRepoItem>,
}

#[derive(Debug, Deserialize)]
struct GithubRepoItem {
    full_name: String,
    description: Option<String>,
    #[serde(default)]
    stargazers_count: u64,
    default_branch: Option<String>,
    html_url: String,
    owner: GithubOwner,
    name: String,
}

#[derive(Debug, Deserialize)]
struct GithubOwner {
    login: String,
}

/// 通过 GitHub Search API 搜索候选 skill 仓库。
///
/// 查询在用户关键字基础上追加 `skill` 语义修饰，尽量命中 skill 仓库。
/// token 可选（匿名限速更低）。
pub async fn search(query: &str, token: Option<String>) -> Result<SearchReport, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err("搜索关键字不能为空".to_string());
    }

    let client = download::http_client()?;
    // 语义：关键字 + topic/名称包含 skill 的仓库，按 star 排序。
    let q = format!("{} skill in:name,description,topics", trimmed);
    let mut req = client
        .get("https://api.github.com/search/repositories")
        .query(&[("q", q.as_str()), ("sort", "stars"), ("per_page", "20")])
        .header("Accept", "application/vnd.github+json");
    if let Some(token) = token.as_deref() {
        if !token.trim().is_empty() {
            req = req.header("Authorization", format!("Bearer {}", token.trim()));
        }
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("GitHub 搜索请求失败: {}", e))?;
    let status = resp.status();
    if !status.is_success() {
        if status.as_u16() == 403 {
            return Err("GitHub 搜索受限（可能触发匿名限速），稍后重试或配置 token".to_string());
        }
        return Err(format!("GitHub 搜索返回错误状态: {}", status));
    }

    let body: GithubSearchResponse = resp
        .json()
        .await
        .map_err(|e| format!("解析 GitHub 搜索响应失败: {}", e))?;

    let results = body
        .items
        .into_iter()
        .map(|item| DiscoveredSkill {
            repo: item.full_name,
            owner: item.owner.login,
            name: item.name,
            description: item.description,
            stars: item.stargazers_count,
            default_branch: item.default_branch,
            html_url: item.html_url,
            source: "github".to_string(),
        })
        .collect();

    Ok(SearchReport {
        query: trimmed.to_string(),
        results,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct UnmanagedSkill {
    pub app: String,
    pub label: String,
    pub directory: String,
    pub path: String,
    pub has_entry_file: bool,
    pub known_directory: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanUnmanagedReport {
    pub items: Vec<UnmanagedSkill>,
}

/// 扫描各 app live skills 目录，列出非托管（无 agent-switch 标记）的候选目录。
///
/// 只读扫描：不联网、不改动 live 目录。`known_directory` 表示该目录名已在 DB skills 表出现。
pub fn scan_unmanaged(db: &Mutex<Connection>, _data_dir: &Path) -> Result<ScanUnmanagedReport, String> {
    let known: std::collections::BTreeSet<String> = skills::list(db)?
        .into_iter()
        .map(|s| s.directory)
        .collect();

    let mut items = Vec::new();
    for app in SkillApp::all() {
        let (config_root, target_root) = app.target_dirs()?;
        if !config_root.exists() || !target_root.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&target_root)
            .map_err(|e| format!("读取 {} skills 目录失败: {}", app.as_str(), e))?
        {
            let entry = entry.map_err(|e| format!("读取目录项失败: {}", e))?;
            let path = entry.path();
            if !path.is_dir() || is_managed_projection(&path) {
                continue;
            }
            let Some(dir) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            items.push(UnmanagedSkill {
                app: app.as_str().to_string(),
                label: app.label().to_string(),
                directory: dir.to_string(),
                path: path.to_string_lossy().to_string(),
                has_entry_file: path.join(ENTRY_FILE).is_file(),
                known_directory: known.contains(dir),
            });
        }
    }

    Ok(ScanUnmanagedReport { items })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn search_rejects_empty_query() {
        assert!(search("  ", None).await.is_err());
    }
}
