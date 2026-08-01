//! 上游模型列表条目类型。
//!
//! 历史上这里包含从上游 `/v1/models` 端点拉取模型列表的逻辑；独立网关已改为
//! 显式配置上游模型（gateway domain），拉取逻辑与 model_cache 已删除。本模块
//! 仅保留 `FetchedModel` 数据结构，供 v17 provenance 迁移和 provider_models DAO 复用。

use serde::{Deserialize, Serialize};

/// 获取到的模型信息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchedModel {
    pub id: String,
    pub owned_by: Option<String>,
}
