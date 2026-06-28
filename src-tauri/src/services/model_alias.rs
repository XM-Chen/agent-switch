use rusqlite::Connection;
use serde::Serialize;
use std::sync::Mutex;

use crate::db::dao::endpoint_models;
use crate::db::dao::model_aliases;

/// 别名解析上下文。
#[derive(Debug, Clone, Default)]
pub struct ResolutionContext {
    pub tool: Option<String>,
    pub route_id: Option<String>,
    pub endpoint_id: Option<String>,
}

/// 解析结果。
#[derive(Debug, Serialize)]
pub struct ResolvedAlias {
    pub alias_name: String,
    pub matched_scope: String,
    pub candidates: Vec<AliasCandidate>,
}

#[derive(Debug, Serialize)]
pub struct AliasCandidate {
    pub endpoint_id: Option<String>,
    pub model_name: String,
    pub priority: i64,
    pub is_valid: bool,
    pub invalid_reason: Option<String>,
}

/// 解析别名，按优先级返回候选链。
///
/// 优先级（从高到低）：
/// 1. tool 级
/// 2. route 级
/// 3. endpoint 级
/// 4. global 级
/// 5. 原名匹配
pub fn resolve(db: &Mutex<Connection>, alias: &str, ctx: &ResolutionContext) -> ResolvedAlias {
    // 1. tool 级
    if let Some(tool) = &ctx.tool {
        if let Some(cands) = query_aliases(db, "tool", Some(tool), alias) {
            if !cands.is_empty() {
                return ResolvedAlias {
                    alias_name: alias.to_string(),
                    matched_scope: "tool".to_string(),
                    candidates: cands,
                };
            }
        }
    }

    // 2. route 级
    if let Some(rid) = &ctx.route_id {
        if let Some(cands) = query_aliases(db, "route", Some(rid), alias) {
            if !cands.is_empty() {
                return ResolvedAlias {
                    alias_name: alias.to_string(),
                    matched_scope: "route".to_string(),
                    candidates: cands,
                };
            }
        }
    }

    // 3. endpoint 级
    if let Some(eid) = &ctx.endpoint_id {
        if let Some(cands) = query_aliases(db, "endpoint", Some(eid), alias) {
            if !cands.is_empty() {
                return ResolvedAlias {
                    alias_name: alias.to_string(),
                    matched_scope: "endpoint".to_string(),
                    candidates: cands,
                };
            }
        }
    }

    // 4. global 级
    if let Some(cands) = query_aliases(db, "global", None, alias) {
        if !cands.is_empty() {
            return ResolvedAlias {
                alias_name: alias.to_string(),
                matched_scope: "global".to_string(),
                candidates: cands,
            };
        }
    }

    // 5. 原名匹配：在所有可用端点模型中查找同名模型。
    let cands = match endpoint_models::list(db, None, None, None) {
        Ok(models) => models
            .into_iter()
            .filter(|m| m.is_available && m.model_name == alias)
            .map(|m| AliasCandidate {
                endpoint_id: Some(m.endpoint_id),
                model_name: m.model_name,
                priority: 0,
                is_valid: true,
                invalid_reason: None,
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    if !cands.is_empty() {
        return ResolvedAlias {
            alias_name: alias.to_string(),
            matched_scope: "name_match".to_string(),
            candidates: cands,
        };
    }

    // 6. 未找到。
    ResolvedAlias {
        alias_name: alias.to_string(),
        matched_scope: "not_found".to_string(),
        candidates: Vec::new(),
    }
}

fn query_aliases(
    db: &Mutex<Connection>,
    scope_type: &str,
    scope_id: Option<&str>,
    alias_name: &str,
) -> Option<Vec<AliasCandidate>> {
    let rows = model_aliases::list(db, Some(scope_type), scope_id).ok()?;
    let matched: Vec<AliasCandidate> = rows
        .into_iter()
        .filter(|r| r.alias_name == alias_name && r.enabled)
        .map(|r| AliasCandidate {
            endpoint_id: r.target_endpoint_id,
            model_name: r.target_model_name,
            priority: r.priority,
            is_valid: r.invalid_reason.is_none(),
            invalid_reason: r.invalid_reason,
        })
        .collect();
    Some(matched)
}
