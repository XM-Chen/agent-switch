//! 独立网关控制面的数据库写模型。
//!
//! 本模块只操作 Agent Switch 自有的 gateway domain 表，不读取或写入任何客户端配置。
//! 所有敏感上游配置只作为写入参数进入数据库；IPC 输出使用显式 DTO，绝不返回
//! `config_json` 或凭据密文。

use super::gateway_domain::{
    GatewayModelRecord, ModelAliasRecord, RouteTargetRecord, UpstreamRecord,
};
use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::gateway::credential;
use crate::services::credential_protector::{CredentialProtector, PlatformCredentialProtector};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

const MAX_NAME_LEN: usize = 200;
const MAX_ID_LEN: usize = 512;
const MAX_NOTES_LEN: usize = 8_000;
const MAX_CREDENTIAL_KIND_LEN: usize = 64;
const MAX_CREDENTIAL_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GatewayUpstreamDto {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub base_url: Option<String>,
    pub protocol: String,
    pub adapter_type: String,
    pub notes: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl TryFrom<UpstreamRecord> for GatewayUpstreamDto {
    type Error = AppError;

    fn try_from(record: UpstreamRecord) -> Result<Self, Self::Error> {
        Ok(Self {
            id: record.id,
            name: record.name,
            enabled: record.enabled,
            base_url: record.base_url,
            protocol: record.protocol,
            adapter_type: record.adapter_type,
            notes: record.notes,
            created_at: record.created_at,
            updated_at: record.updated_at,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGatewayUpstreamInput {
    pub name: String,
    pub enabled: bool,
    pub base_url: String,
    pub protocol: String,
    pub adapter_type: String,
    #[serde(default = "empty_json_object")]
    pub config_json: Value,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateGatewayUpstreamInput {
    pub name: String,
    pub base_url: String,
    pub protocol: String,
    pub adapter_type: String,
    #[serde(default = "empty_json_object")]
    pub config_json: Value,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamCredentialHintDto {
    pub id: String,
    pub upstream_id: String,
    pub credential_kind: String,
    pub encryption_scheme: String,
    pub key_hint: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGatewayModelInput {
    pub model_id: String,
    pub display_name: String,
    #[serde(default = "empty_json_object")]
    pub metadata_json: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateGatewayModelInput {
    pub model_id: String,
    pub display_name: String,
    #[serde(default = "empty_json_object")]
    pub metadata_json: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRouteTargetInput {
    pub gateway_model_id: String,
    pub upstream_id: String,
    pub target_model: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRouteTargetInput {
    pub upstream_id: String,
    pub target_model: String,
}

#[derive(Debug)]
struct ValidatedUpstream {
    name: String,
    base_url: String,
    protocol: String,
    adapter_type: String,
    config_json: String,
    notes: Option<String>,
}

fn empty_json_object() -> Value {
    Value::Object(serde_json::Map::new())
}

fn required_text(value: &str, field: &str, max_len: usize) -> Result<String, AppError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::InvalidInput(format!("{field}不能为空")));
    }
    if trimmed.chars().count() > max_len {
        return Err(AppError::InvalidInput(format!(
            "{field}长度不能超过 {max_len} 个字符"
        )));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(AppError::InvalidInput(format!("{field}不能包含控制字符")));
    }
    Ok(trimmed.to_string())
}

fn optional_text(
    value: Option<&str>,
    field: &str,
    max_len: usize,
) -> Result<Option<String>, AppError> {
    value
        .map(|value| required_text(value, field, max_len))
        .transpose()
}

fn normalize_base_url(value: &str) -> Result<String, AppError> {
    let value = required_text(value, "base URL", 2_048)?;
    let parsed = Url::parse(&value)
        .map_err(|error| AppError::InvalidInput(format!("base URL 无效: {error}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AppError::InvalidInput(
            "base URL 仅支持 http 或 https".to_string(),
        ));
    }
    if parsed.host_str().is_none() || parsed.cannot_be_a_base() {
        return Err(AppError::InvalidInput(
            "base URL 必须包含有效主机名".to_string(),
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AppError::InvalidInput(
            "base URL 不得内嵌用户名或密码".to_string(),
        ));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(AppError::InvalidInput(
            "base URL 不得包含 query 或 fragment".to_string(),
        ));
    }
    Ok(parsed.to_string())
}

fn normalize_protocol(value: &str) -> Result<String, AppError> {
    let value = required_text(value, "上游协议", 64)?.to_ascii_lowercase();
    match value.as_str() {
        "anthropic" | "anthropic_messages" => Ok("anthropic".to_string()),
        "openai" | "openai_chat" | "openai_chat_completions" => Ok("openai_chat".to_string()),
        "openai_responses" | "responses" => Ok("openai_responses".to_string()),
        "gemini" | "gemini_native" => Ok("gemini".to_string()),
        _ => Err(AppError::InvalidInput(format!("不支持的上游协议: {value}"))),
    }
}

fn normalize_adapter(value: &str) -> Result<String, AppError> {
    let value = required_text(value, "adapter", 64)?.to_ascii_lowercase();
    if matches!(
        value.as_str(),
        "claude" | "module_anthropic" | "codex" | "module_openai" | "gemini"
    ) {
        Ok(value)
    } else {
        Err(AppError::InvalidInput(format!(
            "不支持的上游 adapter: {value}"
        )))
    }
}

fn validate_protocol_adapter(protocol: &str, adapter: &str) -> Result<(), AppError> {
    let compatible = matches!(
        (protocol, adapter),
        ("anthropic", "claude" | "module_anthropic")
            | (
                "openai_chat" | "openai_responses",
                "codex" | "module_openai"
            )
            | ("gemini", "gemini")
    );
    if compatible {
        Ok(())
    } else {
        Err(AppError::InvalidInput(format!(
            "协议 {protocol} 与 adapter {adapter} 不兼容"
        )))
    }
}

fn normalized_json_key(key: &str) -> String {
    key.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn find_sensitive_json_path(value: &Value, path: &str) -> Option<String> {
    match value {
        Value::Object(object) => object.iter().find_map(|(key, child)| {
            let normalized = normalized_json_key(key);
            let child_path = format!("{path}.{key}");
            if matches!(
                normalized.as_str(),
                "apikey"
                    | "accesstoken"
                    | "refreshtoken"
                    | "authtoken"
                    | "authorization"
                    | "secretaccesskey"
                    | "clientsecret"
                    | "clientkey"
                    | "privatekey"
                    | "password"
                    | "secret"
                    | "token"
                    | "cookie"
            ) {
                Some(child_path)
            } else {
                find_sensitive_json_path(child, &child_path)
            }
        }),
        Value::Array(items) => items.iter().enumerate().find_map(|(index, child)| {
            find_sensitive_json_path(child, &format!("{path}[{index}]"))
        }),
        _ => None,
    }
}

fn validate_json_object(
    value: &Value,
    field: &str,
    reject_sensitive: bool,
) -> Result<String, AppError> {
    if !value.is_object() {
        return Err(AppError::InvalidInput(format!("{field}必须是 JSON 对象")));
    }
    if reject_sensitive {
        if let Some(path) = find_sensitive_json_path(value, field) {
            return Err(AppError::InvalidInput(format!(
                "{path} 属于敏感字段，请改用 DPAPI 凭据接口"
            )));
        }
    }
    serde_json::to_string(value)
        .map_err(|error| AppError::InvalidInput(format!("{field} 无法序列化: {error}")))
}

fn validate_upstream(
    name: &str,
    base_url: &str,
    protocol: &str,
    adapter_type: &str,
    config_json: &Value,
    notes: Option<&str>,
) -> Result<ValidatedUpstream, AppError> {
    let name = required_text(name, "上游名称", MAX_NAME_LEN)?;
    let base_url = normalize_base_url(base_url)?;
    let protocol = normalize_protocol(protocol)?;
    let adapter_type = normalize_adapter(adapter_type)?;
    validate_protocol_adapter(&protocol, &adapter_type)?;
    let config_json = validate_json_object(config_json, "configJson", true)?;
    let notes = optional_text(notes, "备注", MAX_NOTES_LEN)?;
    Ok(ValidatedUpstream {
        name,
        base_url,
        protocol,
        adapter_type,
        config_json,
        notes,
    })
}

fn upstream_exists(tx: &Transaction<'_>, upstream_id: &str) -> Result<bool, AppError> {
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM upstreams WHERE id = ?1)",
        [upstream_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| value != 0)
    .map_err(|error| AppError::Database(format!("检查上游引用失败: {error}")))
}

fn gateway_model_exists(tx: &Transaction<'_>, model_id: &str) -> Result<bool, AppError> {
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM gateway_models WHERE id = ?1)",
        [model_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| value != 0)
    .map_err(|error| AppError::Database(format!("检查网关模型引用失败: {error}")))
}

fn duplicate_upstream_exists(
    tx: &Transaction<'_>,
    exclude_id: Option<&str>,
    upstream: &ValidatedUpstream,
) -> Result<bool, AppError> {
    tx.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM upstreams
             WHERE name = ?1 COLLATE NOCASE AND base_url = ?2 AND protocol = ?3
               AND adapter_type = ?4 AND (?5 IS NULL OR id <> ?5)
         )",
        params![
            upstream.name,
            upstream.base_url,
            upstream.protocol,
            upstream.adapter_type,
            exclude_id
        ],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| value != 0)
    .map_err(|error| AppError::Database(format!("检查重复上游失败: {error}")))
}

fn available_route_exists(
    tx: &Transaction<'_>,
    gateway_model_id: &str,
    exclude_route_id: Option<&str>,
) -> Result<bool, AppError> {
    tx.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM route_targets rt
             JOIN upstreams u ON u.id = rt.upstream_id
             WHERE rt.gateway_model_id = ?1 AND rt.enabled = 1 AND u.enabled = 1
               AND (?2 IS NULL OR rt.id <> ?2)
         )",
        params![gateway_model_id, exclude_route_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| value != 0)
    .map_err(|error| AppError::Database(format!("检查可用路由候选失败: {error}")))
}

fn model_losing_routes_when_upstream_disabled(
    tx: &Transaction<'_>,
    upstream_id: &str,
) -> Result<Option<String>, AppError> {
    tx.query_row(
        "SELECT gm.id
         FROM gateway_models gm
         WHERE gm.enabled = 1
           AND EXISTS(
               SELECT 1 FROM route_targets affected
               WHERE affected.gateway_model_id = gm.id
                 AND affected.upstream_id = ?1 AND affected.enabled = 1
           )
           AND NOT EXISTS(
               SELECT 1 FROM route_targets rt
               JOIN upstreams u ON u.id = rt.upstream_id
               WHERE rt.gateway_model_id = gm.id AND rt.enabled = 1 AND u.enabled = 1
                 AND rt.upstream_id <> ?1
           )
         ORDER BY gm.id LIMIT 1",
        [upstream_id],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|error| AppError::Database(format!("检查上游停用引用失败: {error}")))
}

fn validate_model_identity(
    tx: &Transaction<'_>,
    model_id: &str,
    exclude_id: Option<&str>,
) -> Result<(), AppError> {
    let duplicate_model = tx
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM gateway_models
                 WHERE model_id = ?1 AND (?2 IS NULL OR id <> ?2)
             )",
            params![model_id, exclude_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| AppError::Database(format!("检查重复网关模型失败: {error}")))?
        != 0;
    if duplicate_model {
        return Err(AppError::InvalidInput(format!(
            "网关模型 ID 已存在: {model_id}"
        )));
    }
    let alias_conflict = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM model_aliases WHERE alias = ?1)",
            [model_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| AppError::Database(format!("检查模型别名冲突失败: {error}")))?
        != 0;
    if alias_conflict {
        return Err(AppError::InvalidInput(format!(
            "网关模型 ID 与现有别名冲突: {model_id}"
        )));
    }
    Ok(())
}

fn normalize_credential_kind(value: &str) -> Result<String, AppError> {
    let value = required_text(value, "凭据类型", MAX_CREDENTIAL_KIND_LEN)?.to_ascii_lowercase();
    if !credential::is_ready_kind(&value) {
        return Err(AppError::InvalidInput(format!(
            "不支持的凭据类型: {value}；请选择 bearer_token、x_api_key、google_api_key 或 google_oauth"
        )));
    }
    Ok(value)
}

fn credential_hint(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 8 {
        "***".to_string()
    } else {
        format!(
            "{}...{}",
            chars[..4].iter().collect::<String>(),
            chars[chars.len() - 4..].iter().collect::<String>()
        )
    }
}

fn load_upstream_dto(
    tx: &Transaction<'_>,
    id: &str,
) -> Result<Option<GatewayUpstreamDto>, AppError> {
    tx.query_row(
        "SELECT id, name, enabled, base_url, protocol, adapter_type, notes,
                created_at, updated_at
         FROM upstreams WHERE id = ?1",
        [id],
        |row| {
            Ok(GatewayUpstreamDto {
                id: row.get(0)?,
                name: row.get(1)?,
                enabled: row.get::<_, i64>(2)? != 0,
                base_url: row.get(3)?,
                protocol: row.get(4)?,
                adapter_type: row.get(5)?,
                notes: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        },
    )
    .optional()
    .map_err(|error| AppError::Database(format!("读取上游 DTO 失败: {error}")))
}

fn load_gateway_model(
    tx: &Transaction<'_>,
    id: &str,
) -> Result<Option<GatewayModelRecord>, AppError> {
    tx.query_row(
        "SELECT id, model_id, display_name, enabled, source, migration_status,
                legacy_app_type, legacy_source_id, metadata_json, created_at, updated_at
         FROM gateway_models WHERE id = ?1",
        [id],
        |row| {
            let raw: String = row.get(8)?;
            let metadata_json = serde_json::from_str(&raw).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    8,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            Ok(GatewayModelRecord {
                id: row.get(0)?,
                model_id: row.get(1)?,
                display_name: row.get(2)?,
                enabled: row.get::<_, i64>(3)? != 0,
                source: row.get(4)?,
                migration_status: row.get(5)?,
                legacy_app_type: row.get(6)?,
                legacy_source_id: row.get(7)?,
                metadata_json,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        },
    )
    .optional()
    .map_err(|error| AppError::Database(format!("读取网关模型失败: {error}")))
}

fn load_route_target(
    tx: &Transaction<'_>,
    id: &str,
) -> Result<Option<RouteTargetRecord>, AppError> {
    tx.query_row(
        "SELECT id, gateway_model_id, upstream_id, target_model, position, enabled,
                legacy_app_type, legacy_aggregate_id, created_at, updated_at
         FROM route_targets WHERE id = ?1",
        [id],
        |row| {
            Ok(RouteTargetRecord {
                id: row.get(0)?,
                gateway_model_id: row.get(1)?,
                upstream_id: row.get(2)?,
                target_model: row.get(3)?,
                position: row.get(4)?,
                enabled: row.get::<_, i64>(5)? != 0,
                legacy_app_type: row.get(6)?,
                legacy_aggregate_id: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
            })
        },
    )
    .optional()
    .map_err(|error| AppError::Database(format!("读取路由候选失败: {error}")))
}

fn write_route_order(
    tx: &Transaction<'_>,
    gateway_model_id: &str,
    ordered_ids: &[String],
    now: i64,
) -> Result<(), AppError> {
    if ordered_ids.is_empty() {
        return Ok(());
    }
    let max_position = tx
        .query_row(
            "SELECT MAX(position) FROM route_targets WHERE gateway_model_id = ?1",
            [gateway_model_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .map_err(|error| AppError::Database(format!("读取路由候选位置失败: {error}")))?;
    let temporary_base = max_position
        .unwrap_or(-1)
        .checked_add(1)
        .ok_or_else(|| AppError::InvalidInput("路由候选位置超出可排序范围".to_string()))?;
    let maximum_offset = i64::try_from(ordered_ids.len() - 1)
        .map_err(|_| AppError::InvalidInput("路由候选数量过大".to_string()))?;
    temporary_base
        .checked_add(maximum_offset)
        .ok_or_else(|| AppError::InvalidInput("路由候选位置超出可排序范围".to_string()))?;

    for (position, id) in ordered_ids.iter().enumerate() {
        let temporary_position = temporary_base
            + i64::try_from(position)
                .map_err(|_| AppError::InvalidInput("路由候选数量过大".to_string()))?;
        let updated = tx
            .execute(
                "UPDATE route_targets SET position = ?1, updated_at = ?2
                 WHERE id = ?3 AND gateway_model_id = ?4",
                params![temporary_position, now, id, gateway_model_id],
            )
            .map_err(|error| AppError::Database(format!("暂存路由候选排序失败: {error}")))?;
        if updated != 1 {
            return Err(AppError::InvalidInput(format!(
                "路由候选不属于模型或不存在: {id}"
            )));
        }
    }
    for (position, id) in ordered_ids.iter().enumerate() {
        let position = i64::try_from(position)
            .map_err(|_| AppError::InvalidInput("路由候选数量过大".to_string()))?;
        tx.execute(
            "UPDATE route_targets SET position = ?1, updated_at = ?2
             WHERE id = ?3 AND gateway_model_id = ?4",
            params![position, now, id, gateway_model_id],
        )
        .map_err(|error| AppError::Database(format!("提交路由候选排序失败: {error}")))?;
    }
    Ok(())
}

fn compact_route_order(
    tx: &Transaction<'_>,
    gateway_model_id: &str,
    now: i64,
) -> Result<(), AppError> {
    let ordered_ids = {
        let mut stmt = tx
            .prepare(
                "SELECT id FROM route_targets
                 WHERE gateway_model_id = ?1 ORDER BY position ASC, id ASC",
            )
            .map_err(|error| AppError::Database(format!("准备压缩路由排序失败: {error}")))?;
        let rows = stmt
            .query_map([gateway_model_id], |row| row.get::<_, String>(0))
            .map_err(|error| AppError::Database(format!("读取路由排序失败: {error}")))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| AppError::Database(format!("解析路由排序失败: {error}")))?
    };
    write_route_order(tx, gateway_model_id, &ordered_ids, now)
}

impl Database {
    pub fn list_gateway_upstream_dtos(&self) -> Result<Vec<GatewayUpstreamDto>, AppError> {
        self.list_upstreams()?
            .into_iter()
            .map(GatewayUpstreamDto::try_from)
            .collect()
    }

    pub fn get_gateway_upstream_dto(
        &self,
        upstream_id: &str,
    ) -> Result<Option<GatewayUpstreamDto>, AppError> {
        self.get_upstream(upstream_id)?
            .map(GatewayUpstreamDto::try_from)
            .transpose()
    }

    pub fn create_gateway_upstream(
        &self,
        input: &CreateGatewayUpstreamInput,
    ) -> Result<GatewayUpstreamDto, AppError> {
        let validated = validate_upstream(
            &input.name,
            &input.base_url,
            &input.protocol,
            &input.adapter_type,
            &input.config_json,
            input.notes.as_deref(),
        )?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp_millis();
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|error| AppError::Database(format!("开始创建上游事务失败: {error}")))?;
        if duplicate_upstream_exists(&tx, None, &validated)? {
            return Err(AppError::InvalidInput(
                "相同名称、URL、协议和 adapter 的上游已存在".to_string(),
            ));
        }
        tx.execute(
            "INSERT INTO upstreams
                (id, name, enabled, base_url, protocol, adapter_type, config_json, notes,
                 created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![
                id,
                validated.name,
                i64::from(input.enabled),
                validated.base_url,
                validated.protocol,
                validated.adapter_type,
                validated.config_json,
                validated.notes,
                now,
            ],
        )
        .map_err(|error| AppError::Database(format!("创建上游失败: {error}")))?;
        tx.commit()
            .map_err(|error| AppError::Database(format!("提交创建上游事务失败: {error}")))?;
        Ok(GatewayUpstreamDto {
            id,
            name: validated.name,
            enabled: input.enabled,
            base_url: Some(validated.base_url),
            protocol: validated.protocol,
            adapter_type: validated.adapter_type,
            notes: validated.notes,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn update_gateway_upstream(
        &self,
        upstream_id: &str,
        input: &UpdateGatewayUpstreamInput,
    ) -> Result<GatewayUpstreamDto, AppError> {
        let validated = validate_upstream(
            &input.name,
            &input.base_url,
            &input.protocol,
            &input.adapter_type,
            &input.config_json,
            input.notes.as_deref(),
        )?;
        let now = chrono::Utc::now().timestamp_millis();
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|error| AppError::Database(format!("开始更新上游事务失败: {error}")))?;
        let existing = tx
            .query_row(
                "SELECT enabled, created_at FROM upstreams WHERE id = ?1",
                [upstream_id],
                |row| Ok((row.get::<_, i64>(0)? != 0, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(|error| AppError::Database(format!("读取待更新上游失败: {error}")))?;
        let Some((enabled, created_at)) = existing else {
            return Err(AppError::InvalidInput(format!("上游不存在: {upstream_id}")));
        };
        if duplicate_upstream_exists(&tx, Some(upstream_id), &validated)? {
            return Err(AppError::InvalidInput(
                "相同名称、URL、协议和 adapter 的上游已存在".to_string(),
            ));
        }
        tx.execute(
            "UPDATE upstreams
             SET name = ?1, base_url = ?2, protocol = ?3, adapter_type = ?4,
                 config_json = ?5, notes = ?6, updated_at = ?7
             WHERE id = ?8",
            params![
                validated.name,
                validated.base_url,
                validated.protocol,
                validated.adapter_type,
                validated.config_json,
                validated.notes,
                now,
                upstream_id,
            ],
        )
        .map_err(|error| AppError::Database(format!("更新上游失败: {error}")))?;
        tx.commit()
            .map_err(|error| AppError::Database(format!("提交更新上游事务失败: {error}")))?;
        Ok(GatewayUpstreamDto {
            id: upstream_id.to_string(),
            name: validated.name,
            enabled,
            base_url: Some(validated.base_url),
            protocol: validated.protocol,
            adapter_type: validated.adapter_type,
            notes: validated.notes,
            created_at,
            updated_at: now,
        })
    }

    pub fn delete_gateway_upstream(&self, upstream_id: &str) -> Result<bool, AppError> {
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|error| AppError::Database(format!("开始删除上游事务失败: {error}")))?;
        if !upstream_exists(&tx, upstream_id)? {
            return Ok(false);
        }
        let route_references = tx
            .query_row(
                "SELECT COUNT(*) FROM route_targets WHERE upstream_id = ?1",
                [upstream_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| AppError::Database(format!("检查上游路由引用失败: {error}")))?;
        if route_references > 0 {
            return Err(AppError::InvalidInput(format!(
                "上游仍被 {route_references} 个路由候选引用，不能删除"
            )));
        }
        tx.execute("DELETE FROM upstreams WHERE id = ?1", [upstream_id])
            .map_err(|error| AppError::Database(format!("删除上游失败: {error}")))?;
        tx.commit()
            .map_err(|error| AppError::Database(format!("提交删除上游事务失败: {error}")))?;
        Ok(true)
    }

    pub fn set_gateway_upstream_enabled(
        &self,
        upstream_id: &str,
        enabled: bool,
    ) -> Result<GatewayUpstreamDto, AppError> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|error| AppError::Database(format!("开始切换上游状态事务失败: {error}")))?;
        let current = tx
            .query_row(
                "SELECT enabled FROM upstreams WHERE id = ?1",
                [upstream_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| AppError::Database(format!("读取上游状态失败: {error}")))?;
        let Some(current) = current else {
            return Err(AppError::InvalidInput(format!("上游不存在: {upstream_id}")));
        };
        if current != 0 && !enabled {
            if let Some(model_id) = model_losing_routes_when_upstream_disabled(&tx, upstream_id)? {
                return Err(AppError::InvalidInput(format!(
                    "停用该上游会使已启用网关模型 {model_id} 失去全部可用路由"
                )));
            }
        }
        tx.execute(
            "UPDATE upstreams SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
            params![i64::from(enabled), now, upstream_id],
        )
        .map_err(|error| AppError::Database(format!("切换上游状态失败: {error}")))?;
        let dto = load_upstream_dto(&tx, upstream_id)?
            .ok_or_else(|| AppError::Database("上游状态更新后记录消失".to_string()))?;
        tx.commit()
            .map_err(|error| AppError::Database(format!("提交切换上游状态事务失败: {error}")))?;
        Ok(dto)
    }

    pub fn list_upstream_credential_hints(
        &self,
        upstream_id: &str,
    ) -> Result<Vec<UpstreamCredentialHintDto>, AppError> {
        let conn = lock_conn!(self.conn);
        let exists = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM upstreams WHERE id = ?1)",
                [upstream_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| AppError::Database(format!("检查上游凭据引用失败: {error}")))?
            != 0;
        if !exists {
            return Err(AppError::InvalidInput(format!("上游不存在: {upstream_id}")));
        }
        let mut stmt = conn
            .prepare(
                "SELECT id, upstream_id, credential_kind, encryption_scheme, key_hint,
                        created_at, updated_at
                 FROM upstream_credentials WHERE upstream_id = ?1
                 ORDER BY credential_kind ASC, id ASC",
            )
            .map_err(|error| AppError::Database(format!("准备读取凭据提示失败: {error}")))?;
        let rows = stmt
            .query_map([upstream_id], |row| {
                Ok(UpstreamCredentialHintDto {
                    id: row.get(0)?,
                    upstream_id: row.get(1)?,
                    credential_kind: row.get(2)?,
                    encryption_scheme: row.get(3)?,
                    key_hint: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .map_err(|error| AppError::Database(format!("读取凭据提示失败: {error}")))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| AppError::Database(format!("解析凭据提示失败: {error}")))
    }

    pub fn replace_upstream_credential(
        &self,
        upstream_id: &str,
        credential_kind: &str,
        secret: &str,
    ) -> Result<UpstreamCredentialHintDto, AppError> {
        let protector = PlatformCredentialProtector;
        self.replace_upstream_credential_with_protector(
            upstream_id,
            credential_kind,
            secret,
            &protector,
        )
    }

    pub(crate) fn replace_upstream_credential_with_protector(
        &self,
        upstream_id: &str,
        credential_kind: &str,
        secret: &str,
        protector: &dyn CredentialProtector,
    ) -> Result<UpstreamCredentialHintDto, AppError> {
        let credential_kind = normalize_credential_kind(credential_kind)?;
        if secret.is_empty() || secret.trim().is_empty() {
            return Err(AppError::InvalidInput("凭据内容不能为空".to_string()));
        }
        if secret != secret.trim() {
            return Err(AppError::InvalidInput(
                "凭据内容不能包含首尾空白字符".to_string(),
            ));
        }
        if secret.len() > MAX_CREDENTIAL_BYTES {
            return Err(AppError::InvalidInput(format!(
                "凭据内容不能超过 {MAX_CREDENTIAL_BYTES} 字节"
            )));
        }
        if !credential::validate_payload(&credential_kind, secret.as_bytes()) {
            return Err(AppError::InvalidInput(format!(
                "凭据内容不符合 {credential_kind} 语义；google_oauth 必须包含当前可用的 access_token"
            )));
        }
        let upstream_identity = {
            let conn = lock_conn!(self.conn);
            conn.query_row(
                "SELECT protocol, adapter_type FROM upstreams WHERE id = ?1",
                [upstream_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| AppError::Database(format!("读取上游凭据适配信息失败: {error}")))?
        };
        let Some((protocol, adapter_type)) = upstream_identity else {
            return Err(AppError::InvalidInput(format!("上游不存在: {upstream_id}")));
        };
        if !credential::kind_can_serve(&credential_kind, &protocol, &adapter_type) {
            return Err(AppError::InvalidInput(format!(
                "凭据类型 {credential_kind} 与上游协议 {protocol}/adapter {adapter_type} 不兼容"
            )));
        }
        let encrypted_payload = protector.protect(secret.as_bytes())?;
        if encrypted_payload.is_empty() {
            return Err(AppError::Config("凭据保护器返回了空密文".to_string()));
        }
        let scheme = required_text(protector.scheme(), "凭据加密方案", 128)?;
        let hint = credential_hint(secret);
        let now = chrono::Utc::now().timestamp_millis();
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|error| AppError::Database(format!("开始替换凭据事务失败: {error}")))?;
        if !upstream_exists(&tx, upstream_id)? {
            return Err(AppError::InvalidInput(format!("上游不存在: {upstream_id}")));
        }
        let existing = tx
            .query_row(
                "SELECT id, created_at FROM upstream_credentials
                 WHERE upstream_id = ?1 AND credential_kind = ?2",
                params![upstream_id, credential_kind],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(|error| AppError::Database(format!("读取现有凭据失败: {error}")))?;
        let (id, created_at) = existing.unwrap_or_else(|| (uuid::Uuid::new_v4().to_string(), now));
        tx.execute(
            "INSERT INTO upstream_credentials
                (id, upstream_id, credential_kind, encrypted_payload, encryption_scheme,
                 key_hint, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(upstream_id, credential_kind) DO UPDATE SET
                encrypted_payload = excluded.encrypted_payload,
                encryption_scheme = excluded.encryption_scheme,
                key_hint = excluded.key_hint,
                updated_at = excluded.updated_at",
            params![
                id,
                upstream_id,
                credential_kind,
                encrypted_payload,
                scheme,
                hint,
                created_at,
                now,
            ],
        )
        .map_err(|error| AppError::Database(format!("保存加密凭据失败: {error}")))?;
        tx.commit()
            .map_err(|error| AppError::Database(format!("提交替换凭据事务失败: {error}")))?;
        Ok(UpstreamCredentialHintDto {
            id,
            upstream_id: upstream_id.to_string(),
            credential_kind,
            encryption_scheme: scheme,
            key_hint: Some(hint),
            created_at,
            updated_at: now,
        })
    }

    pub fn delete_upstream_credential(
        &self,
        upstream_id: &str,
        credential_kind: &str,
    ) -> Result<bool, AppError> {
        let credential_kind = normalize_credential_kind(credential_kind)?;
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|error| AppError::Database(format!("开始删除凭据事务失败: {error}")))?;
        if !upstream_exists(&tx, upstream_id)? {
            return Err(AppError::InvalidInput(format!("上游不存在: {upstream_id}")));
        }
        let deleted = tx
            .execute(
                "DELETE FROM upstream_credentials
                 WHERE upstream_id = ?1 AND credential_kind = ?2",
                params![upstream_id, credential_kind],
            )
            .map_err(|error| AppError::Database(format!("删除凭据失败: {error}")))?;
        tx.commit()
            .map_err(|error| AppError::Database(format!("提交删除凭据事务失败: {error}")))?;
        Ok(deleted == 1)
    }

    pub fn create_gateway_model(
        &self,
        input: &CreateGatewayModelInput,
    ) -> Result<GatewayModelRecord, AppError> {
        let model_id = required_text(&input.model_id, "网关模型 ID", MAX_ID_LEN)?;
        let display_name = required_text(&input.display_name, "网关模型显示名称", MAX_NAME_LEN)?;
        let metadata_json = validate_json_object(&input.metadata_json, "metadataJson", true)?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp_millis();
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|error| AppError::Database(format!("开始创建网关模型事务失败: {error}")))?;
        validate_model_identity(&tx, &model_id, None)?;
        tx.execute(
            "INSERT INTO gateway_models
                (id, model_id, display_name, enabled, source, migration_status,
                 metadata_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, 0, 'manual', 'draft', ?4, ?5, ?5)",
            params![id, model_id, display_name, metadata_json, now],
        )
        .map_err(|error| AppError::Database(format!("创建网关模型失败: {error}")))?;
        let record = load_gateway_model(&tx, &id)?
            .ok_or_else(|| AppError::Database("网关模型创建后记录消失".to_string()))?;
        tx.commit()
            .map_err(|error| AppError::Database(format!("提交创建网关模型事务失败: {error}")))?;
        Ok(record)
    }

    pub fn update_gateway_model(
        &self,
        gateway_model_id: &str,
        input: &UpdateGatewayModelInput,
    ) -> Result<GatewayModelRecord, AppError> {
        let model_id = required_text(&input.model_id, "网关模型 ID", MAX_ID_LEN)?;
        let display_name = required_text(&input.display_name, "网关模型显示名称", MAX_NAME_LEN)?;
        let metadata_json = validate_json_object(&input.metadata_json, "metadataJson", true)?;
        let now = chrono::Utc::now().timestamp_millis();
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|error| AppError::Database(format!("开始更新网关模型事务失败: {error}")))?;
        if !gateway_model_exists(&tx, gateway_model_id)? {
            return Err(AppError::InvalidInput(format!(
                "网关模型不存在: {gateway_model_id}"
            )));
        }
        validate_model_identity(&tx, &model_id, Some(gateway_model_id))?;
        tx.execute(
            "UPDATE gateway_models
             SET model_id = ?1, display_name = ?2, metadata_json = ?3, updated_at = ?4
             WHERE id = ?5",
            params![model_id, display_name, metadata_json, now, gateway_model_id],
        )
        .map_err(|error| AppError::Database(format!("更新网关模型失败: {error}")))?;
        let record = load_gateway_model(&tx, gateway_model_id)?
            .ok_or_else(|| AppError::Database("网关模型更新后记录消失".to_string()))?;
        tx.commit()
            .map_err(|error| AppError::Database(format!("提交更新网关模型事务失败: {error}")))?;
        Ok(record)
    }

    pub fn delete_gateway_model(&self, gateway_model_id: &str) -> Result<bool, AppError> {
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|error| AppError::Database(format!("开始删除网关模型事务失败: {error}")))?;
        if !gateway_model_exists(&tx, gateway_model_id)? {
            return Ok(false);
        }
        let alias_count = tx
            .query_row(
                "SELECT COUNT(*) FROM model_aliases WHERE gateway_model_id = ?1",
                [gateway_model_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| AppError::Database(format!("检查模型别名引用失败: {error}")))?;
        let route_count = tx
            .query_row(
                "SELECT COUNT(*) FROM route_targets WHERE gateway_model_id = ?1",
                [gateway_model_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| AppError::Database(format!("检查模型路由引用失败: {error}")))?;
        if alias_count > 0 || route_count > 0 {
            return Err(AppError::InvalidInput(format!(
                "网关模型仍被 {alias_count} 个别名和 {route_count} 个路由候选引用，不能删除"
            )));
        }
        tx.execute(
            "DELETE FROM gateway_models WHERE id = ?1",
            [gateway_model_id],
        )
        .map_err(|error| AppError::Database(format!("删除网关模型失败: {error}")))?;
        tx.commit()
            .map_err(|error| AppError::Database(format!("提交删除网关模型事务失败: {error}")))?;
        Ok(true)
    }

    pub fn set_gateway_model_enabled_strict(
        &self,
        gateway_model_id: &str,
        enabled: bool,
    ) -> Result<bool, AppError> {
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|error| AppError::Database(format!("开始更新模型状态事务失败: {error}")))?;
        let status = tx
            .query_row(
                "SELECT migration_status FROM gateway_models WHERE id = ?1",
                [gateway_model_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| AppError::Database(format!("读取网关模型状态失败: {error}")))?;
        let Some(status) = status else {
            return Ok(false);
        };
        if enabled && status != "active" {
            return Err(AppError::InvalidInput(
                "只有 active 网关模型才能启用".to_string(),
            ));
        }
        if enabled && !available_route_exists(&tx, gateway_model_id, None)? {
            return Err(AppError::InvalidInput(
                "网关模型至少需要一个已启用且上游可用的路由候选才能启用".to_string(),
            ));
        }
        tx.execute(
            "UPDATE gateway_models SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
            params![
                i64::from(enabled),
                chrono::Utc::now().timestamp_millis(),
                gateway_model_id
            ],
        )
        .map_err(|error| AppError::Database(format!("更新网关模型启用状态失败: {error}")))?;
        tx.commit()
            .map_err(|error| AppError::Database(format!("提交模型状态事务失败: {error}")))?;
        Ok(true)
    }

    pub fn set_gateway_model_state_strict(
        &self,
        gateway_model_id: &str,
        enabled: bool,
        migration_status: &str,
    ) -> Result<bool, AppError> {
        let migration_status =
            required_text(migration_status, "网关模型状态", 32)?.to_ascii_lowercase();
        if !matches!(migration_status.as_str(), "active" | "draft" | "conflict") {
            return Err(AppError::InvalidInput(format!(
                "不支持的网关模型状态: {migration_status}"
            )));
        }
        if enabled && migration_status != "active" {
            return Err(AppError::InvalidInput(
                "draft/conflict 网关模型不能处于启用状态".to_string(),
            ));
        }
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|error| AppError::Database(format!("开始更新模型状态事务失败: {error}")))?;
        if !gateway_model_exists(&tx, gateway_model_id)? {
            return Ok(false);
        }
        if enabled && !available_route_exists(&tx, gateway_model_id, None)? {
            return Err(AppError::InvalidInput(
                "网关模型至少需要一个已启用且上游可用的路由候选才能启用".to_string(),
            ));
        }
        tx.execute(
            "UPDATE gateway_models
             SET enabled = ?1, migration_status = ?2, updated_at = ?3 WHERE id = ?4",
            params![
                i64::from(enabled),
                migration_status,
                chrono::Utc::now().timestamp_millis(),
                gateway_model_id
            ],
        )
        .map_err(|error| AppError::Database(format!("更新网关模型状态失败: {error}")))?;
        tx.commit()
            .map_err(|error| AppError::Database(format!("提交模型状态事务失败: {error}")))?;
        Ok(true)
    }

    pub fn upsert_gateway_model_alias(
        &self,
        alias: &str,
        gateway_model_id: &str,
    ) -> Result<ModelAliasRecord, AppError> {
        let alias = required_text(alias, "模型别名", MAX_ID_LEN)?;
        let now = chrono::Utc::now().timestamp_millis();
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|error| AppError::Database(format!("开始保存模型别名事务失败: {error}")))?;
        if !gateway_model_exists(&tx, gateway_model_id)? {
            return Err(AppError::InvalidInput(format!(
                "网关模型不存在: {gateway_model_id}"
            )));
        }
        let model_conflict = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM gateway_models WHERE model_id = ?1)",
                [alias.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| AppError::Database(format!("检查模型别名冲突失败: {error}")))?
            != 0;
        if model_conflict {
            return Err(AppError::InvalidInput(format!(
                "模型别名与网关模型 ID 冲突: {alias}"
            )));
        }
        let created_at = tx
            .query_row(
                "SELECT created_at FROM model_aliases WHERE alias = ?1",
                [alias.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| AppError::Database(format!("读取现有模型别名失败: {error}")))?
            .unwrap_or(now);
        tx.execute(
            "INSERT INTO model_aliases (alias, gateway_model_id, created_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(alias) DO UPDATE SET gateway_model_id = excluded.gateway_model_id",
            params![alias, gateway_model_id, created_at],
        )
        .map_err(|error| AppError::Database(format!("保存模型别名失败: {error}")))?;
        tx.commit()
            .map_err(|error| AppError::Database(format!("提交模型别名事务失败: {error}")))?;
        Ok(ModelAliasRecord {
            alias,
            gateway_model_id: gateway_model_id.to_string(),
            created_at,
        })
    }

    pub fn delete_gateway_model_alias(&self, alias: &str) -> Result<bool, AppError> {
        let alias = required_text(alias, "模型别名", MAX_ID_LEN)?;
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|error| AppError::Database(format!("开始删除模型别名事务失败: {error}")))?;
        let deleted = tx
            .execute("DELETE FROM model_aliases WHERE alias = ?1", [alias])
            .map_err(|error| AppError::Database(format!("删除模型别名失败: {error}")))?;
        tx.commit()
            .map_err(|error| AppError::Database(format!("提交删除模型别名事务失败: {error}")))?;
        Ok(deleted == 1)
    }

    pub fn create_route_target(
        &self,
        input: &CreateRouteTargetInput,
    ) -> Result<RouteTargetRecord, AppError> {
        let gateway_model_id = required_text(&input.gateway_model_id, "网关模型引用", MAX_ID_LEN)?;
        let upstream_id = required_text(&input.upstream_id, "上游引用", MAX_ID_LEN)?;
        let target_model = required_text(&input.target_model, "目标模型", MAX_ID_LEN)?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp_millis();
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|error| AppError::Database(format!("开始创建路由候选事务失败: {error}")))?;
        if !gateway_model_exists(&tx, &gateway_model_id)? {
            return Err(AppError::InvalidInput(format!(
                "网关模型不存在: {gateway_model_id}"
            )));
        }
        if !upstream_exists(&tx, &upstream_id)? {
            return Err(AppError::InvalidInput(format!("上游不存在: {upstream_id}")));
        }
        let duplicate = tx
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM route_targets
                     WHERE gateway_model_id = ?1 AND upstream_id = ?2 AND target_model = ?3
                 )",
                params![gateway_model_id, upstream_id, target_model],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| AppError::Database(format!("检查重复路由候选失败: {error}")))?
            != 0;
        if duplicate {
            return Err(AppError::InvalidInput(
                "相同网关模型、上游和目标模型的路由候选已存在".to_string(),
            ));
        }
        let position = tx
            .query_row(
                "SELECT COALESCE(MAX(position) + 1, 0)
                 FROM route_targets WHERE gateway_model_id = ?1",
                [gateway_model_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| AppError::Database(format!("计算路由候选位置失败: {error}")))?;
        tx.execute(
            "INSERT INTO route_targets
                (id, gateway_model_id, upstream_id, target_model, position, enabled,
                 created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?6)",
            params![
                id,
                gateway_model_id,
                upstream_id,
                target_model,
                position,
                now
            ],
        )
        .map_err(|error| AppError::Database(format!("创建路由候选失败: {error}")))?;
        let record = load_route_target(&tx, &id)?
            .ok_or_else(|| AppError::Database("路由候选创建后记录消失".to_string()))?;
        tx.commit()
            .map_err(|error| AppError::Database(format!("提交创建路由候选事务失败: {error}")))?;
        Ok(record)
    }

    pub fn update_route_target(
        &self,
        route_target_id: &str,
        input: &UpdateRouteTargetInput,
    ) -> Result<RouteTargetRecord, AppError> {
        let upstream_id = required_text(&input.upstream_id, "上游引用", MAX_ID_LEN)?;
        let target_model = required_text(&input.target_model, "目标模型", MAX_ID_LEN)?;
        let now = chrono::Utc::now().timestamp_millis();
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|error| AppError::Database(format!("开始更新路由候选事务失败: {error}")))?;
        let existing = load_route_target(&tx, route_target_id)?
            .ok_or_else(|| AppError::InvalidInput(format!("路由候选不存在: {route_target_id}")))?;
        if !upstream_exists(&tx, &upstream_id)? {
            return Err(AppError::InvalidInput(format!("上游不存在: {upstream_id}")));
        }
        let duplicate = tx
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM route_targets
                     WHERE gateway_model_id = ?1 AND upstream_id = ?2 AND target_model = ?3
                       AND id <> ?4
                 )",
                params![
                    existing.gateway_model_id,
                    upstream_id,
                    target_model,
                    route_target_id
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| AppError::Database(format!("检查重复路由候选失败: {error}")))?
            != 0;
        if duplicate {
            return Err(AppError::InvalidInput(
                "相同网关模型、上游和目标模型的路由候选已存在".to_string(),
            ));
        }
        if existing.enabled {
            let upstream_enabled = tx
                .query_row(
                    "SELECT enabled FROM upstreams WHERE id = ?1",
                    [upstream_id.as_str()],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| AppError::Database(format!("读取上游状态失败: {error}")))?
                != 0;
            let model_enabled = tx
                .query_row(
                    "SELECT enabled FROM gateway_models WHERE id = ?1",
                    [existing.gateway_model_id.as_str()],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| AppError::Database(format!("读取网关模型状态失败: {error}")))?
                != 0;
            if model_enabled && !upstream_enabled {
                return Err(AppError::InvalidInput(
                    "已启用网关模型的启用路由不能改指向已停用上游".to_string(),
                ));
            }
        }
        tx.execute(
            "UPDATE route_targets
             SET upstream_id = ?1, target_model = ?2, updated_at = ?3 WHERE id = ?4",
            params![upstream_id, target_model, now, route_target_id],
        )
        .map_err(|error| AppError::Database(format!("更新路由候选失败: {error}")))?;
        let record = load_route_target(&tx, route_target_id)?
            .ok_or_else(|| AppError::Database("路由候选更新后记录消失".to_string()))?;
        tx.commit()
            .map_err(|error| AppError::Database(format!("提交更新路由候选事务失败: {error}")))?;
        Ok(record)
    }

    pub fn delete_route_target(&self, route_target_id: &str) -> Result<bool, AppError> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|error| AppError::Database(format!("开始删除路由候选事务失败: {error}")))?;
        let Some(existing) = load_route_target(&tx, route_target_id)? else {
            return Ok(false);
        };
        let model_enabled = tx
            .query_row(
                "SELECT enabled FROM gateway_models WHERE id = ?1",
                [existing.gateway_model_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| AppError::Database(format!("读取网关模型状态失败: {error}")))?
            != 0;
        let upstream_enabled = tx
            .query_row(
                "SELECT enabled FROM upstreams WHERE id = ?1",
                [existing.upstream_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| AppError::Database(format!("读取上游状态失败: {error}")))?
            != 0;
        if model_enabled
            && existing.enabled
            && upstream_enabled
            && !available_route_exists(&tx, &existing.gateway_model_id, Some(route_target_id))?
        {
            return Err(AppError::InvalidInput(
                "不能删除已启用网关模型的最后一个可用路由候选".to_string(),
            ));
        }
        tx.execute("DELETE FROM route_targets WHERE id = ?1", [route_target_id])
            .map_err(|error| AppError::Database(format!("删除路由候选失败: {error}")))?;
        compact_route_order(&tx, &existing.gateway_model_id, now)?;
        tx.commit()
            .map_err(|error| AppError::Database(format!("提交删除路由候选事务失败: {error}")))?;
        Ok(true)
    }

    pub fn set_route_target_enabled_strict(
        &self,
        route_target_id: &str,
        enabled: bool,
    ) -> Result<bool, AppError> {
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|error| AppError::Database(format!("开始切换路由候选事务失败: {error}")))?;
        let Some(existing) = load_route_target(&tx, route_target_id)? else {
            return Ok(false);
        };
        let upstream_enabled = tx
            .query_row(
                "SELECT enabled FROM upstreams WHERE id = ?1",
                [existing.upstream_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| AppError::Database(format!("读取上游状态失败: {error}")))?
            != 0;
        if enabled && !upstream_enabled {
            return Err(AppError::InvalidInput(
                "不能启用指向已停用上游的路由候选".to_string(),
            ));
        }
        let model_enabled = tx
            .query_row(
                "SELECT enabled FROM gateway_models WHERE id = ?1",
                [existing.gateway_model_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| AppError::Database(format!("读取网关模型状态失败: {error}")))?
            != 0;
        if !enabled
            && existing.enabled
            && model_enabled
            && upstream_enabled
            && !available_route_exists(&tx, &existing.gateway_model_id, Some(route_target_id))?
        {
            return Err(AppError::InvalidInput(
                "不能停用已启用网关模型的最后一个可用路由候选".to_string(),
            ));
        }
        tx.execute(
            "UPDATE route_targets SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
            params![
                i64::from(enabled),
                chrono::Utc::now().timestamp_millis(),
                route_target_id
            ],
        )
        .map_err(|error| AppError::Database(format!("切换路由候选状态失败: {error}")))?;
        tx.commit()
            .map_err(|error| AppError::Database(format!("提交切换路由候选事务失败: {error}")))?;
        Ok(true)
    }

    pub fn reorder_route_targets_strict(
        &self,
        gateway_model_id: &str,
        ordered_ids: &[String],
    ) -> Result<(), AppError> {
        use std::collections::HashSet;

        if ordered_ids.iter().collect::<HashSet<_>>().len() != ordered_ids.len() {
            return Err(AppError::InvalidInput(
                "路由候选排序包含重复 ID".to_string(),
            ));
        }
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|error| AppError::Database(format!("开始重排路由候选事务失败: {error}")))?;
        if !gateway_model_exists(&tx, gateway_model_id)? {
            return Err(AppError::InvalidInput(format!(
                "网关模型不存在: {gateway_model_id}"
            )));
        }
        let existing_ids = {
            let mut stmt = tx
                .prepare("SELECT id FROM route_targets WHERE gateway_model_id = ?1")
                .map_err(|error| AppError::Database(format!("准备读取路由候选失败: {error}")))?;
            let rows = stmt
                .query_map([gateway_model_id], |row| row.get::<_, String>(0))
                .map_err(|error| AppError::Database(format!("读取路由候选失败: {error}")))?;
            rows.collect::<Result<std::collections::HashSet<_>, _>>()
                .map_err(|error| AppError::Database(format!("解析路由候选失败: {error}")))?
        };
        if existing_ids.len() != ordered_ids.len()
            || !ordered_ids.iter().all(|id| existing_ids.contains(id))
        {
            return Err(AppError::InvalidInput(
                "路由候选排序必须完整且仅包含该模型全部候选".to_string(),
            ));
        }
        write_route_order(
            &tx,
            gateway_model_id,
            ordered_ids,
            chrono::Utc::now().timestamp_millis(),
        )?;
        tx.commit()
            .map_err(|error| AppError::Database(format!("提交重排路由候选事务失败: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct TestProtector;

    impl CredentialProtector for TestProtector {
        fn scheme(&self) -> &'static str {
            "test-dpapi-v1"
        }

        fn protect(&self, plaintext: &[u8]) -> Result<Vec<u8>, AppError> {
            Ok(plaintext.iter().map(|byte| byte ^ 0xa5).collect())
        }

        fn unprotect(&self, ciphertext: &[u8]) -> Result<Vec<u8>, AppError> {
            self.protect(ciphertext)
        }
    }

    fn upstream_input(name: &str, url: &str) -> CreateGatewayUpstreamInput {
        CreateGatewayUpstreamInput {
            name: name.to_string(),
            enabled: true,
            base_url: url.to_string(),
            protocol: "openai_responses".to_string(),
            adapter_type: "codex".to_string(),
            config_json: json!({"requestHeaders": {"x-tenant": "one"}}),
            notes: None,
        }
    }

    fn create_upstream(db: &Database, name: &str, url: &str) -> GatewayUpstreamDto {
        db.create_gateway_upstream(&upstream_input(name, url))
            .expect("create upstream")
    }

    fn create_model(db: &Database, model_id: &str) -> GatewayModelRecord {
        db.create_gateway_model(&CreateGatewayModelInput {
            model_id: model_id.to_string(),
            display_name: model_id.to_string(),
            metadata_json: json!({}),
        })
        .expect("create model")
    }

    #[test]
    fn upstream_crud_validates_url_protocol_duplicates_and_redacts_output() {
        let db = Database::memory().expect("memory db");
        let created = create_upstream(&db, "Primary", "https://api.example.com/v1");
        assert_eq!(created.protocol, "openai_responses");
        assert!(!serde_json::to_value(&created)
            .expect("serialize dto")
            .as_object()
            .expect("dto object")
            .contains_key("configJson"));
        assert_eq!(db.list_gateway_upstream_dtos().expect("list").len(), 1);
        assert_eq!(
            db.get_gateway_upstream_dto(&created.id)
                .expect("get")
                .expect("found"),
            created
        );

        assert!(db
            .create_gateway_upstream(&upstream_input("primary", "https://api.example.com/v1"))
            .is_err());
        let mut invalid = upstream_input("Bad", "ftp://api.example.com");
        assert!(matches!(
            db.create_gateway_upstream(&invalid),
            Err(AppError::InvalidInput(_))
        ));
        invalid.base_url = "https://user:pass@api.example.com".to_string();
        assert!(db.create_gateway_upstream(&invalid).is_err());
        invalid.base_url = "https://api.example.com".to_string();
        invalid.adapter_type = "gemini".to_string();
        assert!(db.create_gateway_upstream(&invalid).is_err());
        invalid.adapter_type = "codex".to_string();
        invalid.config_json = json!({"apiKey": "must-not-live-here"});
        assert!(db.create_gateway_upstream(&invalid).is_err());

        let updated = db
            .update_gateway_upstream(
                &created.id,
                &UpdateGatewayUpstreamInput {
                    name: "Primary Updated".to_string(),
                    base_url: "https://api.example.com/v2".to_string(),
                    protocol: "openai_chat_completions".to_string(),
                    adapter_type: "module_openai".to_string(),
                    config_json: json!({}),
                    notes: Some("note".to_string()),
                },
            )
            .expect("update upstream");
        assert_eq!(updated.protocol, "openai_chat");
        assert_eq!(updated.notes.as_deref(), Some("note"));
        assert!(
            !db.set_gateway_upstream_enabled(&created.id, false)
                .expect("disable")
                .enabled
        );
        assert!(db.delete_gateway_upstream(&created.id).expect("delete"));
        assert!(!db
            .delete_gateway_upstream(&created.id)
            .expect("delete missing"));
    }

    #[test]
    fn credential_replace_is_encrypted_atomic_and_returns_hint_only() {
        let db = Database::memory().expect("memory db");
        let upstream = create_upstream(&db, "Credential", "https://credential.example.com");
        let first = db
            .replace_upstream_credential_with_protector(
                &upstream.id,
                "bearer_token",
                "sk-first-secret",
                &TestProtector,
            )
            .expect("replace credential");
        assert_eq!(first.encryption_scheme, "test-dpapi-v1");
        assert_eq!(first.key_hint.as_deref(), Some("sk-f...cret"));
        let (ciphertext, scheme): (Vec<u8>, String) = db
            .conn
            .lock()
            .expect("lock")
            .query_row(
                "SELECT encrypted_payload, encryption_scheme FROM upstream_credentials WHERE id = ?1",
                [first.id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("stored credential");
        assert_ne!(ciphertext, b"sk-first-secret");
        assert_eq!(scheme, "test-dpapi-v1");

        let second = db
            .replace_upstream_credential_with_protector(
                &upstream.id,
                "bearer_token",
                "sk-second-secret",
                &TestProtector,
            )
            .expect("replace same kind");
        assert_eq!(second.id, first.id);
        let hints = db
            .list_upstream_credential_hints(&upstream.id)
            .expect("list hints");
        assert_eq!(hints, vec![second]);
        assert!(db
            .delete_upstream_credential(&upstream.id, "bearer_token")
            .expect("delete credential"));
        assert!(db
            .list_upstream_credential_hints(&upstream.id)
            .expect("list empty")
            .is_empty());
        assert!(db
            .replace_upstream_credential_with_protector(
                "missing",
                "bearer_token",
                "secret-value",
                &TestProtector
            )
            .is_err());
    }

    #[test]
    fn model_alias_and_reference_validation_are_strict() {
        let db = Database::memory().expect("memory db");
        let first = create_model(&db, "stable-model");
        assert!(db
            .create_gateway_model(&CreateGatewayModelInput {
                model_id: "stable-model".to_string(),
                display_name: "Duplicate".to_string(),
                metadata_json: json!({}),
            })
            .is_err());
        let alias = db
            .upsert_gateway_model_alias("stable-alias", &first.id)
            .expect("upsert alias");
        assert_eq!(alias.gateway_model_id, first.id);
        assert!(db
            .create_gateway_model(&CreateGatewayModelInput {
                model_id: "stable-alias".to_string(),
                display_name: "Alias collision".to_string(),
                metadata_json: json!({}),
            })
            .is_err());
        assert!(db.delete_gateway_model(&first.id).is_err());
        assert!(db
            .delete_gateway_model_alias("stable-alias")
            .expect("delete alias"));
        let updated = db
            .update_gateway_model(
                &first.id,
                &UpdateGatewayModelInput {
                    model_id: "stable-model-v2".to_string(),
                    display_name: "Stable V2".to_string(),
                    metadata_json: json!({"family": "openai"}),
                },
            )
            .expect("update model");
        assert_eq!(updated.model_id, "stable-model-v2");
        assert!(db.delete_gateway_model(&first.id).expect("delete model"));
    }

    #[test]
    fn route_crud_state_and_reorder_preserve_references_atomically() {
        let db = Database::memory().expect("memory db");
        let upstream_a = create_upstream(&db, "A", "https://a.example.com");
        let upstream_b = create_upstream(&db, "B", "https://b.example.com");
        let model = create_model(&db, "routed-model");
        assert!(db
            .create_route_target(&CreateRouteTargetInput {
                gateway_model_id: "missing".to_string(),
                upstream_id: upstream_a.id.clone(),
                target_model: "vendor-a".to_string(),
            })
            .is_err());
        let route_a = db
            .create_route_target(&CreateRouteTargetInput {
                gateway_model_id: model.id.clone(),
                upstream_id: upstream_a.id.clone(),
                target_model: "vendor-a".to_string(),
            })
            .expect("create route a");
        let route_b = db
            .create_route_target(&CreateRouteTargetInput {
                gateway_model_id: model.id.clone(),
                upstream_id: upstream_b.id.clone(),
                target_model: "vendor-b".to_string(),
            })
            .expect("create route b");
        assert!(db
            .create_route_target(&CreateRouteTargetInput {
                gateway_model_id: model.id.clone(),
                upstream_id: upstream_b.id.clone(),
                target_model: "vendor-b".to_string(),
            })
            .is_err());
        assert!(db
            .set_gateway_model_state_strict(&model.id, true, "active")
            .is_err());
        assert!(db
            .set_route_target_enabled_strict(&route_a.id, true)
            .expect("enable route a"));
        assert!(db
            .set_gateway_model_state_strict(&model.id, true, "active")
            .expect("activate model"));
        assert!(db
            .set_route_target_enabled_strict(&route_a.id, false)
            .is_err());
        assert!(db
            .set_gateway_upstream_enabled(&upstream_a.id, false)
            .is_err());
        assert!(db
            .set_route_target_enabled_strict(&route_b.id, true)
            .expect("enable route b"));

        let before = db
            .list_route_targets()
            .expect("routes before invalid reorder");
        assert!(db
            .reorder_route_targets_strict(&model.id, std::slice::from_ref(&route_a.id))
            .is_err());
        assert_eq!(db.list_route_targets().expect("unchanged routes"), before);
        db.reorder_route_targets_strict(&model.id, &[route_b.id.clone(), route_a.id.clone()])
            .expect("reorder routes");
        let reordered = db.list_route_targets().expect("reordered");
        assert_eq!(reordered[0].id, route_b.id);
        assert_eq!(reordered[1].id, route_a.id);

        assert!(db.delete_gateway_upstream(&upstream_a.id).is_err());
        assert!(db.delete_gateway_model(&model.id).is_err());
        assert!(db.delete_route_target(&route_a.id).expect("delete route a"));
        assert!(db
            .set_gateway_model_state_strict(&model.id, false, "conflict")
            .expect("set conflict"));
        assert!(db.delete_route_target(&route_b.id).expect("delete route b"));
        assert!(db
            .delete_gateway_upstream(&upstream_a.id)
            .expect("delete a"));
        assert!(db
            .delete_gateway_upstream(&upstream_b.id)
            .expect("delete b"));
        assert!(db.delete_gateway_model(&model.id).expect("delete model"));
    }

    #[test]
    fn gateway_config_keeps_strict_loopback_boundary() {
        let db = Database::memory().expect("memory db");
        let mut config = db.get_gateway_config_record().expect("config");
        config.listen_address = "localhost".to_string();
        assert!(matches!(
            db.update_gateway_config_record(&config),
            Err(AppError::InvalidInput(_))
        ));
        config.listen_address = "::1".to_string();
        assert!(matches!(
            db.update_gateway_config_record(&config),
            Err(AppError::InvalidInput(_))
        ));
        config.listen_address = "127.0.0.1".to_string();
        db.update_gateway_config_record(&config)
            .expect("exact loopback accepted");
    }
}
