/// 模型映射器。
///
/// 解析请求体中的 `model` 字段，通过 `model_alias::resolve` 查找别名映射。
/// 如果别名未匹配则执行角色名映射（ccs 风格），最后改写 body["model"] 为上游模型名。
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::services::model_alias::{self, ResolutionContext};

/// 模型映射结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMappingResult {
    /// 原始请求中的模型名。
    pub original_model: String,
    /// 改写后的上游模型名。
    pub upstream_model: String,
    /// 匹配到的别名名称（若未匹配则为空）。
    pub resolved_alias: String,
    /// 匹配作用域（tool/route/endpoint/global/name_match/not_found）。
    pub resolved_scope: String,
}

/// 模型映射器。
pub struct ModelMapper {
    db: Arc<Mutex<Connection>>,
    tool: String,
}

impl ModelMapper {
    /// 创建模型映射器。
    ///
    /// - `db`：数据库连接。
    /// - `tool`：工具标识（如 "claude-code"），用于作用域解析。
    pub fn new(db: Arc<Mutex<Connection>>, tool: &str) -> Self {
        Self {
            db,
            tool: tool.to_string(),
        }
    }

    /// 解析模型名并改写请求体。
    ///
    /// 执行步骤：
    /// 1. 从 body["model"] 读取原始模型名。
    /// 2. 调用 `model_alias::resolve` 查找别名。
    /// 3. 若别名未匹配，执行角色名映射。
    /// 4. 将 body["model"] 改写为上游模型名。
    /// 5. 返回 ModelMappingResult。
    pub fn resolve_and_rewrite(
        &self,
        body: &mut serde_json::Value,
    ) -> Result<ModelMappingResult, String> {
        let original_model = body
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "请求体缺少 model 字段".to_string())?;

        // 1. 调用 model_alias::resolve
        let ctx = ResolutionContext {
            tool: Some(self.tool.clone()),
            route_id: None,
            endpoint_id: None,
        };
        let resolved = model_alias::resolve(&self.db, &original_model, &ctx);

        let (upstream_model, resolved_alias, resolved_scope) = if !resolved.candidates.is_empty() {
            // 取最高优先级候选
            let first = &resolved.candidates[0];
            let upstream = first.model_name.clone();
            (upstream, resolved.alias_name, resolved.matched_scope)
        } else {
            // 2. 未匹配别名，执行角色名映射
            let upstream = self.role_mapping(&original_model);
            (upstream, "".to_string(), "role_mapping".to_string())
        };

        // 3. 改写 body["model"]
        body["model"] = serde_json::json!(upstream_model);

        Ok(ModelMappingResult {
            original_model,
            upstream_model,
            resolved_alias,
            resolved_scope,
        })
    }

    /// 角色名映射（ccs 风格）。
    ///
    /// 当别名词典未命中时，检查模型名是否包含 haiku/sonnet/opus/fable 等角色关键字。
    /// 功能级角色名映射到具体上游模型版本。
    fn role_mapping(&self, model: &str) -> String {
        // 去掉 [1M] 后缀
        let cleaned = model.trim_end_matches("[1M]").trim();

        // 角色关键字 → 上游模型映射
        match cleaned {
            "haiku" => "claude-sonnet-4-20250514",
            "sonnet" => "claude-sonnet-4-20250514",
            "opus" => "claude-sonnet-4-20250514",
            "fable" => "claude-sonnet-4-20250514",
            // 其他角色名称保持原样（不作隐式映射）
            other
                if other.contains("haiku")
                    || other.contains("sonnet")
                    || other.contains("opus")
                    || other.contains("fable") =>
            {
                cleaned
            }
            other => other,
        }
        .to_string()
    }
}
