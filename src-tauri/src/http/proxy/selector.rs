/// 端点选择器。
///
/// 从 DB 中按协议类型加载候选端点，提供 Fill-First 和 Round-Robin 两种选择策略。
/// 自动跳过已禁用、冷却中、已失败的端点。
/// 支持模型锁检查回调和能力过滤。
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
    /// 协议类型（查询条件）。
    protocol_type: String,
    /// 已加载的候选端点（按 priority ASC 排序）。
    candidates: Vec<EndpointRow>,
    /// 当前位置游标（用于 Round-Robin）。
    cursor: u64,
    /// 选择策略：fill-first 或 round-robin。
    strategy: String,
    /// 必需的模型能力（None 表示不限制）。
    required_capability: Option<String>,
}

impl EndpointSelector {
    /// 创建选择器，指定协议类型。
    pub fn new(protocol_type: &str) -> Self {
        Self {
            protocol_type: protocol_type.to_string(),
            candidates: Vec::new(),
            cursor: 0,
            strategy: constants::FILL_FIRST.to_string(),
            required_capability: None,
        }
    }

    /// 从 DB 加载候选端点（已启用 + 匹配协议类型，按 priority ASC）。
    pub async fn load_candidates(&mut self, db: &Mutex<Connection>) -> Result<(), String> {
        let rows = endpoints::list_by_protocol(db, &self.protocol_type)?;
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
    /// - `model_lock_check`：回调函数，接收 endpoint_id 返回该端点模型是否可用（true=可用）。
    /// - 返回 `(EndpointRow, index_in_candidates)` 或 `None`（无可用端点）。
    pub fn next(
        &mut self,
        failed_ids: &HashSet<String>,
        model_lock_check: impl Fn(&str) -> bool,
    ) -> Option<(EndpointRow, usize)> {
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

            // 跳过冷却中的
            if Self::is_on_cooldown(candidate) {
                continue;
            }

            // 跳过已失败的
            if failed_ids.contains(&candidate.id) {
                continue;
            }

            // 跳过模型锁定的
            if !model_lock_check(&candidate.id) {
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
