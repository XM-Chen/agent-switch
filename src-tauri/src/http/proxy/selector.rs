/// 端点选择器。
///
/// 从 DB 中按协议类型加载候选端点，提供 Fill-First 和 Round-Robin 两种选择策略。
/// 自动跳过已禁用、冷却中、已失败的端点。
/// 支持能力过滤。
use std::collections::HashSet;
use std::sync::Mutex;

use rusqlite::Connection;
use time::format_description::well_known::Iso8601;
use time::OffsetDateTime;

use crate::db::dao::endpoint_models;
use crate::db::dao::endpoints::{self, EndpointRow};
use crate::http::proxy::constants;

/// 端点选择器。
pub struct EndpointSelector {
    /// 已加载的候选端点（按 priority ASC 排序）。
    candidates: Vec<EndpointRow>,
    /// 当前位置游标（用于 Round-Robin）。
    cursor: u64,
    /// 选择策略：fill-first 或 round-robin。
    strategy: String,
    /// 必需的模型能力（None 表示不限制）。
    required_capability: Option<String>,
    /// 是否跳过冷却检查（测试模式）。
    skip_cooldown: bool,
}

impl EndpointSelector {
    /// 创建选择器。`protocol_type` 参数保留以兼容调用点签名；候选加载不按协议过滤
    /// （跨协议翻译由 translate 层处理）。
    pub fn new(_protocol_type: &str) -> Self {
        Self {
            candidates: Vec::new(),
            cursor: 0,
            strategy: constants::FILL_FIRST.to_string(),
            required_capability: None,
            skip_cooldown: false,
        }
    }

    /// 设置是否跳过冷却检查（测试模式使用）。
    pub fn set_skip_cooldown(&mut self, val: bool) {
        self.skip_cooldown = val;
    }

    /// 从 DB 加载候选端点（已启用 + 匹配协议类型，按 priority ASC）。
    /// 加载所有已启用端点作为候选（不按协议过滤）。
    ///
    /// 参考 9router / ccs / cpa / sub2api：选择器按 model/capability/priority 筛选，
    /// 而非按入站协议过滤端点；跨协议翻译由 translate 层在转发时按
    /// `protocol_from != protocol_to` 判断触发。
    pub async fn load_candidates(&mut self, db: &Mutex<Connection>) -> Result<(), String> {
        let rows = endpoints::list_enabled(db)?;
        self.candidates = rows;
        self.cursor = 0;
        Ok(())
    }

    /// 设置选择策略。
    pub fn set_strategy(&mut self, strategy: &str) {
        match strategy {
            constants::FILL_FIRST => self.strategy = constants::FILL_FIRST.to_string(),
            constants::ROUND_ROBIN => self.strategy = constants::ROUND_ROBIN.to_string(),
            _ => tracing::warn!("未知选择策略: {}，使用 fill-first", strategy),
        }
    }

    /// 设置必需的模型能力（无则不限制）。
    ///
    /// 设置后，`next()` 会在遍历候选时自动跳过没有任何模型具备该能力的端点。
    pub fn set_required_capability(&mut self, capability: &str) {
        self.required_capability = Some(capability.to_string());
    }

    /// 在加载候选后调用，过滤掉没有任何 capable 模型的端点。
    ///
    /// 复用 `has_capable_model` DAO 查询该端点是否有至少一个可用模型
    /// 的 `capabilities` 字段包含 required_capability。
    pub fn filter_by_capability(&mut self, db: &Mutex<Connection>) -> Result<(), String> {
        let Some(ref cap) = self.required_capability else {
            return Ok(());
        };
        let mut retained = Vec::new();
        for ep in self.candidates.drain(..) {
            match endpoint_models::has_capable_model(db, &ep.id, cap) {
                Ok(true) => retained.push(ep),
                Ok(false) => {
                    tracing::debug!("端点 '{}' 无 {} 能力模型，已从候选排除", ep.name, cap);
                }
                Err(e) => {
                    tracing::warn!("查询端点 '{}' 能力模型失败: {}，保留候选", ep.name, e);
                    retained.push(ep);
                }
            }
        }
        self.candidates = retained;
        Ok(())
    }

    /// 选择下一个可用端点。
    ///
    /// - `failed_ids`：已失败的端点 ID 集合（本轮跳过）。
    /// - 返回 `(EndpointRow, index_in_candidates)` 或 `None`（无可用端点）。
    ///
    /// 模型锁检查已移至模型映射之后（需 `upstream_model` 才能正确查锁），
    /// 由主循环在 `next` 返回后单独执行，故此处不再需要回调。
    pub fn next(&mut self, failed_ids: &HashSet<String>) -> Option<(EndpointRow, usize)> {
        if self.candidates.is_empty() {
            return None;
        }

        let len = self.candidates.len();
        let start = if self.strategy == constants::ROUND_ROBIN {
            (self.cursor % len as u64) as usize
        } else {
            0usize
        };

        for i in 0..len {
            let idx = (start + i) % len;
            let candidate = &self.candidates[idx];

            // 跳过已禁用的
            if !candidate.enabled {
                continue;
            }

            // 跳过冷却中的（测试模式跳过此检查）
            if !self.skip_cooldown && Self::is_on_cooldown(candidate) {
                continue;
            }

            // 跳过已失败的
            if failed_ids.contains(&candidate.id) {
                continue;
            }

            // 匹配成功
            self.cursor = (idx + 1) as u64;
            return Some((candidate.clone(), idx));
        }

        None
    }

    /// 返回当前候选列表（只读引用）。
    pub fn candidates(&self) -> &[EndpointRow] {
        &self.candidates
    }

    /// 检查端点是否在冷却中。
    fn is_on_cooldown(endpoint: &EndpointRow) -> bool {
        match &endpoint.cooldown_until {
            Some(cooldown_str) => {
                match OffsetDateTime::parse(cooldown_str, &Iso8601::DEFAULT) {
                    Ok(cooldown_time) => OffsetDateTime::now_utc() < cooldown_time,
                    Err(_) => false, // 无法解析视为不在冷却中
                }
            }
            None => false,
        }
    }
}
