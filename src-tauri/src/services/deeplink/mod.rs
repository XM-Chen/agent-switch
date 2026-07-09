//! Deep Link 一键导入（ccswitch://v1/import）。
//!
//! 对外链接只做解析/预览；只有显式 import 调用才写 DB 或触发 live 投影。

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine as _;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use url::Url;

use crate::db::dao::{endpoints, mcp_servers, prompts, providers};
use crate::services::crypto::CryptoService;
use crate::services::mcp::claude as mcp_claude;
use crate::services::mcp::validation::validate_server_spec;
use crate::services::prompts::claude as prompts_claude;
use crate::services::tool_takeover::{self, Tool};

const SENSITIVE_KEYS: &[&str] = &[
    "apiKey",
    "authToken",
    "accessToken",
    "token",
    "usageApiKey",
    "usageAccessToken",
    "content",
    "config",
    "usageScript",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeepLinkResource {
    Provider,
    Prompt,
    Mcp,
    Skill,
}

impl DeepLinkResource {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "provider" => Ok(Self::Provider),
            "prompt" => Ok(Self::Prompt),
            "mcp" => Ok(Self::Mcp),
            "skill" => Ok(Self::Skill),
            other => Err(format!("不支持的 resource: {}", other)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParsedDeepLink {
    resource: DeepLinkResource,
    params: HashMap<String, String>,
    redacted_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewItem {
    pub label: String,
    pub value: String,
    pub sensitive: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeepLinkPreview {
    pub resource: DeepLinkResource,
    pub resource_label: String,
    pub app: Option<String>,
    pub name: Option<String>,
    pub enabled: bool,
    pub blocked: bool,
    pub redacted_url: String,
    pub fields: Vec<PreviewItem>,
    pub actions: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportResultItem {
    pub kind: String,
    pub id: Option<String>,
    pub name: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeepLinkImportResult {
    pub resource: DeepLinkResource,
    pub created: Vec<ImportResultItem>,
    pub skipped: Vec<ImportResultItem>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
struct ProviderImport {
    app_type: String,
    name: String,
    endpoint: String,
    api_key: String,
    model: Option<String>,
    meta_env: serde_json::Map<String, Value>,
    notes: Option<String>,
    homepage: Option<String>,
    enabled: bool,
    warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct PromptImport {
    name: String,
    content: String,
    description: Option<String>,
    enabled: bool,
}

#[derive(Debug, Clone)]
struct McpImport {
    servers: Vec<(String, Value)>,
    enabled_claude: bool,
    warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct SkillImport {
    repo: String,
    directory: Option<String>,
    branch: Option<String>,
    subdir: Option<String>,
}

pub fn parse(raw: &str) -> Result<ParsedDeepLink, String> {
    let url = Url::parse(raw).map_err(|_| "Deep Link URL 格式无效".to_string())?;
    if url.scheme() != "ccswitch" {
        return Err("仅支持 ccswitch:// 协议".to_string());
    }
    if url.host_str() != Some("v1") {
        return Err("仅支持 ccswitch://v1/import".to_string());
    }
    if url.path() != "/import" {
        return Err("仅支持 ccswitch://v1/import".to_string());
    }

    let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
    let resource = params
        .get("resource")
        .ok_or_else(|| "缺少 resource 参数".to_string())
        .and_then(|v| DeepLinkResource::parse(v))?;

    Ok(ParsedDeepLink {
        resource,
        params,
        redacted_url: redact_url(raw),
    })
}

pub fn redact_url(raw: &str) -> String {
    let Ok(mut url) = Url::parse(raw) else {
        return "<invalid-url>".to_string();
    };
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| {
            let value = if is_sensitive_key(&k) {
                "[redacted]".to_string()
            } else {
                v.into_owned()
            };
            (k.into_owned(), value)
        })
        .collect();
    url.query_pairs_mut().clear().extend_pairs(pairs);
    url.to_string()
}

pub fn preview(raw: &str) -> Result<DeepLinkPreview, String> {
    let parsed = parse(raw)?;
    match parsed.resource {
        DeepLinkResource::Provider => preview_provider(parsed),
        DeepLinkResource::Prompt => preview_prompt(parsed),
        DeepLinkResource::Mcp => preview_mcp(parsed),
        DeepLinkResource::Skill => preview_skill(parsed),
    }
}

pub async fn import(
    db: &Mutex<Connection>,
    crypto: Option<&CryptoService>,
    data_dir: &Path,
    raw: &str,
) -> Result<DeepLinkImportResult, String> {
    let parsed = parse(raw)?;
    match parsed.resource {
        DeepLinkResource::Provider => import_provider(db, crypto, data_dir, parsed),
        DeepLinkResource::Prompt => import_prompt(db, parsed),
        DeepLinkResource::Mcp => import_mcp(db, parsed),
        DeepLinkResource::Skill => import_skill(db, data_dir, parsed).await,
    }
}

fn preview_provider(parsed: ParsedDeepLink) -> Result<DeepLinkPreview, String> {
    let mut warnings = Vec::new();
    let prepared = prepare_provider(&parsed, &mut warnings);
    let mut fields = Vec::new();
    let mut actions = Vec::new();
    let mut blocked = false;

    match &prepared {
        Ok(p) => {
            fields.push(item("目标应用", &p.app_type, false));
            fields.push(item("名称", &p.name, false));
            fields.push(item("主端点", &p.endpoint, false));
            fields.push(item("API Key", "已提供", true));
            if let Some(model) = &p.model {
                fields.push(item("模型", model, false));
            }
            if let Some(homepage) = &p.homepage {
                fields.push(item("主页", homepage, false));
            }
            actions.push("创建加密 endpoint".to_string());
            actions.push("创建 direct provider（settings_config 只保存 endpoint_id）".to_string());
            if p.enabled {
                actions.push("确认后立即切换到该 provider".to_string());
            }
            warnings.extend(p.warnings.clone());
        }
        Err(e) => {
            blocked = true;
            warnings.push(e.clone());
            if let Some(app) = param(&parsed, "app") {
                fields.push(item("目标应用", &app, false));
            }
            if let Some(name) = param(&parsed, "name") {
                fields.push(item("名称", &name, false));
            }
        }
    }

    if param(&parsed, "configUrl").is_some() {
        warnings.push("configUrl 需要外部网络获取，本版本不会自动拉取".to_string());
    }

    Ok(DeepLinkPreview {
        resource: DeepLinkResource::Provider,
        resource_label: "Provider".to_string(),
        app: param(&parsed, "app"),
        name: param(&parsed, "name"),
        enabled: bool_param(&parsed, "enabled"),
        blocked,
        redacted_url: parsed.redacted_url,
        fields,
        actions,
        warnings,
    })
}

fn preview_prompt(parsed: ParsedDeepLink) -> Result<DeepLinkPreview, String> {
    let mut warnings = Vec::new();
    let prepared = prepare_prompt(&parsed);
    let mut fields = Vec::new();
    let mut actions = Vec::new();
    let mut blocked = false;

    match &prepared {
        Ok(p) => {
            fields.push(item("目标应用", "claude-code", false));
            fields.push(item("名称", &p.name, false));
            if let Some(description) = &p.description {
                fields.push(item("描述", description, false));
            }
            fields.push(item("内容摘要", &summarize(&p.content, 160), true));
            actions.push("创建 Prompt".to_string());
            if p.enabled {
                actions.push("启用 Prompt，并复用 CLAUDE.md 回填保护".to_string());
            }
        }
        Err(e) => {
            blocked = true;
            warnings.push(e.clone());
        }
    }

    Ok(DeepLinkPreview {
        resource: DeepLinkResource::Prompt,
        resource_label: "Prompt".to_string(),
        app: param(&parsed, "app"),
        name: param(&parsed, "name"),
        enabled: bool_param(&parsed, "enabled"),
        blocked,
        redacted_url: parsed.redacted_url,
        fields,
        actions,
        warnings,
    })
}

fn preview_mcp(parsed: ParsedDeepLink) -> Result<DeepLinkPreview, String> {
    let mut warnings = Vec::new();
    let prepared = prepare_mcp(&parsed, &mut warnings);
    let mut fields = Vec::new();
    let mut actions = Vec::new();
    let mut blocked = false;

    match &prepared {
        Ok(mcp) => {
            fields.push(item("目标应用", "claude-code", false));
            fields.push(item("服务器数量", &mcp.servers.len().to_string(), false));
            if !mcp.servers.is_empty() {
                fields.push(item(
                    "服务器",
                    &mcp.servers
                        .iter()
                        .map(|(id, _)| id.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    false,
                ));
            }
            actions.push("导入 MCP server 清单".to_string());
            if mcp.enabled_claude {
                actions.push("启用到 Claude Code 并同步 ~/.claude.json".to_string());
            }
            warnings.extend(mcp.warnings.clone());
        }
        Err(e) => {
            blocked = true;
            warnings.push(e.clone());
        }
    }

    Ok(DeepLinkPreview {
        resource: DeepLinkResource::Mcp,
        resource_label: "MCP".to_string(),
        app: Some("claude-code".to_string()),
        name: None,
        enabled: bool_param(&parsed, "enabled"),
        blocked,
        redacted_url: parsed.redacted_url,
        fields,
        actions,
        warnings,
    })
}

fn preview_skill(parsed: ParsedDeepLink) -> Result<DeepLinkPreview, String> {
    let mut fields = Vec::new();
    let mut warnings = Vec::new();
    let mut blocked = false;
    let mut actions = vec!["从 GitHub 下载并安装到 Skills 库".to_string()];
    match prepare_skill(&parsed) {
        Ok(skill) => {
            fields.push(item("GitHub repo", &skill.repo, false));
            if let Some(directory) = skill.directory {
                fields.push(item("目录", &directory, false));
            }
            if let Some(subdir) = skill.subdir {
                fields.push(item("子目录", &subdir, false));
            }
            if let Some(branch) = skill.branch {
                fields.push(item("分支", &branch, false));
            }
            warnings.push("安装会从 GitHub 联网下载仓库内容，请确认来源可信".to_string());
        }
        Err(e) => {
            blocked = true;
            actions = vec!["修正链接参数后重试".to_string()];
            warnings.push(e);
        }
    }

    Ok(DeepLinkPreview {
        resource: DeepLinkResource::Skill,
        resource_label: "Skill".to_string(),
        app: None,
        name: param(&parsed, "repo"),
        enabled: bool_param(&parsed, "enabled"),
        blocked,
        redacted_url: parsed.redacted_url,
        fields,
        actions,
        warnings,
    })
}

fn import_provider(
    db: &Mutex<Connection>,
    crypto: Option<&CryptoService>,
    data_dir: &Path,
    parsed: ParsedDeepLink,
) -> Result<DeepLinkImportResult, String> {
    let mut warnings = Vec::new();
    let provider = prepare_provider(&parsed, &mut warnings)?;
    warnings.extend(provider.warnings.clone());

    let endpoint_id = uuid::Uuid::new_v4().to_string();
    let api_key_encrypted = encrypt_api_key(crypto, &endpoint_id, &provider.api_key)?;
    let endpoint = endpoints::create(
        db,
        endpoints::NewEndpoint {
            id: endpoint_id.clone(),
            account_id: None,
            name: provider.name.clone(),
            base_url: provider.endpoint.clone(),
            protocol_type: protocol_for_app(&provider.app_type).to_string(),
            api_key_encrypted: Some(api_key_encrypted),
            auth_mode: "api_key".to_string(),
            priority: 0,
            extra_json: None,
        },
    )?;

    let provider_id = uuid::Uuid::new_v4().to_string();
    let mut settings = json!({ "endpoint_id": endpoint.id });
    if let Some(model) = &provider.model {
        settings["model"] = json!(model);
    }
    if provider.app_type == "codex" {
        settings["wire_api"] = json!("responses");
        settings["requires_openai_auth"] = json!(true);
    }

    let mut meta = serde_json::Map::new();
    meta.insert("imported_from".to_string(), json!("deeplink"));
    meta.insert("source_protocol".to_string(), json!("ccswitch://v1/import"));
    if let Some(homepage) = &provider.homepage {
        meta.insert("homepage".to_string(), json!(homepage));
    }
    if !provider.meta_env.is_empty() {
        let mut snapshot = serde_json::Map::new();
        snapshot.insert("env".to_string(), Value::Object(provider.meta_env.clone()));
        meta.insert("snapshot".to_string(), Value::Object(snapshot));
    }

    let sort_index = providers::next_sort_index(db, &provider.app_type)?;
    let row = match providers::create(
        db,
        providers::NewProvider {
            id: provider_id.clone(),
            app_type: provider.app_type.clone(),
            name: unique_provider_name(db, &provider.app_type, &provider.name)?,
            mode: "direct".to_string(),
            settings_config: settings.to_string(),
            category: Some("custom".to_string()),
            sort_index: Some(sort_index),
            notes: provider.notes.clone(),
            meta: Value::Object(meta).to_string(),
        },
    ) {
        Ok(row) => row,
        Err(e) => {
            let _ = endpoints::delete(db, &endpoint_id);
            return Err(e);
        }
    };

    let mut errors = Vec::new();
    let mut switch_warnings = Vec::new();
    if provider.enabled {
        match switch_provider(db, data_dir, &row.id, crypto) {
            Ok(w) => switch_warnings.extend(w),
            Err(e) => errors.push(format!("provider 已创建，但切换失败: {}", e)),
        }
    }
    warnings.extend(switch_warnings);

    Ok(DeepLinkImportResult {
        resource: DeepLinkResource::Provider,
        created: vec![
            ImportResultItem {
                kind: "endpoint".to_string(),
                id: Some(endpoint_id),
                name: endpoint.name,
                message: Some("API Key 已加密保存".to_string()),
            },
            ImportResultItem {
                kind: "provider".to_string(),
                id: Some(row.id),
                name: row.name,
                message: Some("direct provider 已创建".to_string()),
            },
        ],
        skipped: Vec::new(),
        warnings,
        errors,
    })
}

fn import_prompt(
    db: &Mutex<Connection>,
    parsed: ParsedDeepLink,
) -> Result<DeepLinkImportResult, String> {
    let prompt = prepare_prompt(&parsed)?;
    let row = prompts::create(
        db,
        prompts::NewPrompt {
            id: uuid::Uuid::new_v4().to_string(),
            name: prompt.name,
            content: prompt.content,
            description: prompt.description,
            enabled_claude: false,
        },
    )?;

    let mut errors = Vec::new();
    if prompt.enabled {
        if let Err(e) = prompts_claude::enable_prompt(db, &row.id) {
            errors.push(format!("prompt 已创建，但启用失败: {}", e));
        }
    }

    Ok(DeepLinkImportResult {
        resource: DeepLinkResource::Prompt,
        created: vec![ImportResultItem {
            kind: "prompt".to_string(),
            id: Some(row.id),
            name: row.name,
            message: Some(
                if prompt.enabled {
                    "已请求启用"
                } else {
                    "已创建为未启用"
                }
                .to_string(),
            ),
        }],
        skipped: Vec::new(),
        warnings: Vec::new(),
        errors,
    })
}

fn import_mcp(
    db: &Mutex<Connection>,
    parsed: ParsedDeepLink,
) -> Result<DeepLinkImportResult, String> {
    let mut warnings = Vec::new();
    let mcp = prepare_mcp(&parsed, &mut warnings)?;
    warnings.extend(mcp.warnings.clone());

    let mut created = Vec::new();
    let mut skipped = Vec::new();
    let mut changed = false;

    for (id, spec) in mcp.servers {
        let server_config = spec.to_string();
        match mcp_servers::get(db, &id)? {
            Some(existing) => {
                if existing.server_config == server_config {
                    if mcp.enabled_claude && !existing.enabled_claude {
                        mcp_servers::update(
                            db,
                            &id,
                            mcp_servers::McpServerUpdate {
                                enabled_claude: Some(true),
                                ..Default::default()
                            },
                        )?;
                        changed = true;
                        created.push(ImportResultItem {
                            kind: "mcp".to_string(),
                            id: Some(id.clone()),
                            name: existing.name,
                            message: Some("已存在，已合并启用状态".to_string()),
                        });
                    } else {
                        skipped.push(ImportResultItem {
                            kind: "mcp".to_string(),
                            id: Some(id.clone()),
                            name: existing.name,
                            message: Some("已存在且配置相同".to_string()),
                        });
                    }
                } else {
                    skipped.push(ImportResultItem {
                        kind: "mcp".to_string(),
                        id: Some(id.clone()),
                        name: existing.name,
                        message: Some("同名 MCP 已存在且配置不同，未覆盖".to_string()),
                    });
                }
            }
            None => {
                let row = mcp_servers::create(
                    db,
                    mcp_servers::NewMcpServer {
                        id: id.clone(),
                        name: id.clone(),
                        server_config,
                        description: Some("通过 Deep Link 导入".to_string()),
                        homepage: None,
                        docs: None,
                        tags: "[]".to_string(),
                        enabled_claude: mcp.enabled_claude,
                    },
                )?;
                changed = true;
                created.push(ImportResultItem {
                    kind: "mcp".to_string(),
                    id: Some(row.id),
                    name: row.name,
                    message: Some(
                        if mcp.enabled_claude {
                            "已创建并启用"
                        } else {
                            "已创建为未启用"
                        }
                        .to_string(),
                    ),
                });
            }
        }
    }

    let mut errors = Vec::new();
    if changed {
        if let Err(e) = mcp_claude::sync_enabled_to_claude(db) {
            errors.push(format!("MCP 已写入 DB，但同步 ~/.claude.json 失败: {}", e));
        }
    }

    Ok(DeepLinkImportResult {
        resource: DeepLinkResource::Mcp,
        created,
        skipped,
        warnings,
        errors,
    })
}

async fn import_skill(
    db: &Mutex<Connection>,
    data_dir: &Path,
    parsed: ParsedDeepLink,
) -> Result<DeepLinkImportResult, String> {
    let skill = prepare_skill(&parsed)?;
    let repo = skill.repo.clone();

    // Deep Link 安装默认不启用任何 app 投影，交由用户在 Skills 页面显式启用。
    // GitHub token 由 Skills 服务端从 app_metadata 读取；此处不透传敏感参数。
    let input = crate::services::skills::InstallRepoInput {
        repo: skill.repo,
        branch: skill.branch,
        subdir: skill.subdir,
        directory: skill.directory,
        name: None,
        description: None,
        enabled_claude: false,
        enabled_codex: false,
        enabled_gemini: false,
        enabled_opencode: false,
        enabled_hermes: false,
    };

    match crate::services::skills::install_repo(db, data_dir, None, input).await {
        Ok(report) => Ok(DeepLinkImportResult {
            resource: DeepLinkResource::Skill,
            created: vec![ImportResultItem {
                kind: "skill".to_string(),
                id: Some(report.skill.id),
                name: report.skill.directory,
                message: Some("已从 GitHub 安装到 SSOT；请在 Skills 页面按需启用各应用投影".to_string()),
            }],
            skipped: Vec::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
        }),
        Err(e) => Ok(DeepLinkImportResult {
            resource: DeepLinkResource::Skill,
            created: Vec::new(),
            skipped: Vec::new(),
            warnings: Vec::new(),
            errors: vec![format!("安装 skill {} 失败: {}", repo, e)],
        }),
    }
}

fn prepare_provider(
    parsed: &ParsedDeepLink,
    warnings: &mut Vec<String>,
) -> Result<ProviderImport, String> {
    let app = param(parsed, "app").ok_or_else(|| "provider 缺少 app 参数".to_string())?;
    let app_type = match app.as_str() {
        "claude" | "claude-code" => "claude-code".to_string(),
        "codex" => "codex".to_string(),
        other => return Err(format!("暂不支持 app={} 的 provider 导入", other)),
    };

    let mut config = match param(parsed, "config") {
        Some(raw) => Some(decode_json_b64(&raw, "config")?),
        None => None,
    };
    if let Some(format) = param(parsed, "configFormat") {
        if format != "json" && format != "settings-json" {
            warnings.push(format!(
                "configFormat={} 暂未特殊处理，按 JSON 尝试解析",
                format
            ));
        }
    }

    let config_env = config
        .as_mut()
        .and_then(|v| v.get_mut("env"))
        .and_then(|v| v.as_object_mut());

    let endpoint = param(parsed, "endpoint")
        .or_else(|| env_string(config_env.as_deref(), "ANTHROPIC_BASE_URL"))
        .ok_or_else(|| "provider 缺少 endpoint 参数".to_string())?;
    let mut endpoints = endpoint.split(',').map(str::trim).filter(|s| !s.is_empty());
    let primary_endpoint = endpoints
        .next()
        .ok_or_else(|| "provider endpoint 为空".to_string())?
        .to_string();
    let extra_count = endpoints.count();
    if extra_count > 0 {
        warnings.push(format!(
            "endpoint 含 {} 个附加 URL，本版本仅导入第一个",
            extra_count
        ));
    }

    let api_key = param(parsed, "apiKey")
        .or_else(|| env_string(config_env.as_deref(), "ANTHROPIC_AUTH_TOKEN"))
        .ok_or_else(|| "provider 缺少 apiKey，无法创建可切换的 direct provider".to_string())?;

    if let Some(raw) = param(parsed, "usageScript") {
        let _ = decode_text_b64(&raw, "usageScript")?;
        warnings.push("usageScript 已校验但当前不会导入执行".to_string());
    }

    let name = param(parsed, "name").unwrap_or_else(|| "Deep Link Provider".to_string());
    let model =
        param(parsed, "model").or_else(|| env_string(config_env.as_deref(), "ANTHROPIC_MODEL"));
    let mut meta_env = serde_json::Map::new();
    if let Some(env) = config_env {
        for (key, value) in env.iter() {
            if key != "ANTHROPIC_BASE_URL" && key != "ANTHROPIC_AUTH_TOKEN" {
                meta_env.insert(key.clone(), value.clone());
            }
        }
    }
    put_env(&mut meta_env, "ANTHROPIC_MODEL", model.as_deref());
    put_env(
        &mut meta_env,
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        param(parsed, "haikuModel").as_deref(),
    );
    put_env(
        &mut meta_env,
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        param(parsed, "sonnetModel").as_deref(),
    );
    put_env(
        &mut meta_env,
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        param(parsed, "opusModel").as_deref(),
    );

    Ok(ProviderImport {
        app_type,
        name,
        endpoint: primary_endpoint,
        api_key,
        model,
        meta_env,
        notes: param(parsed, "notes"),
        homepage: param(parsed, "homepage"),
        enabled: bool_param(parsed, "enabled"),
        warnings: Vec::new(),
    })
}

fn prepare_prompt(parsed: &ParsedDeepLink) -> Result<PromptImport, String> {
    let app = param(parsed, "app").unwrap_or_else(|| "claude".to_string());
    if app != "claude" && app != "claude-code" {
        return Err(format!("暂不支持 app={} 的 prompt 导入", app));
    }
    let name = param(parsed, "name").unwrap_or_else(|| "Deep Link Prompt".to_string());
    let raw_content =
        param(parsed, "content").ok_or_else(|| "prompt 缺少 content 参数".to_string())?;
    let content = decode_text_b64(&raw_content, "content")?;
    Ok(PromptImport {
        name,
        content,
        description: param(parsed, "description"),
        enabled: bool_param(parsed, "enabled"),
    })
}

fn prepare_mcp(parsed: &ParsedDeepLink, warnings: &mut Vec<String>) -> Result<McpImport, String> {
    let apps = param(parsed, "apps").unwrap_or_else(|| "claude".to_string());
    let app_set: HashSet<&str> = apps
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if !app_set.is_empty() && !app_set.contains("claude") && !app_set.contains("claude-code") {
        return Err("MCP Deep Link 当前仅支持 apps=claude".to_string());
    }
    for app in app_set {
        if app != "claude" && app != "claude-code" {
            warnings.push(format!("MCP apps={} 暂不支持，已忽略", app));
        }
    }

    let raw_config = param(parsed, "config").ok_or_else(|| "mcp 缺少 config 参数".to_string())?;
    let root = decode_json_b64(&raw_config, "config")?;
    let map = root
        .get("mcpServers")
        .and_then(|v| v.as_object())
        .ok_or_else(|| "mcp config 必须包含 mcpServers 对象".to_string())?;

    let mut servers = Vec::new();
    for (id, spec) in map {
        validate_server_spec(spec).map_err(|e| format!("MCP server '{}' 无效: {}", id, e))?;
        servers.push((id.clone(), spec.clone()));
    }
    if servers.is_empty() {
        return Err("mcpServers 为空".to_string());
    }

    Ok(McpImport {
        servers,
        enabled_claude: bool_param(parsed, "enabled"),
        warnings: Vec::new(),
    })
}

fn prepare_skill(parsed: &ParsedDeepLink) -> Result<SkillImport, String> {
    let repo = param(parsed, "repo").ok_or_else(|| "skill 缺少 repo 参数".to_string())?;
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2 || parts.iter().any(|p| p.is_empty()) {
        return Err("skill repo 必须是 owner/name".to_string());
    }
    Ok(SkillImport {
        repo,
        directory: param(parsed, "directory"),
        branch: param(parsed, "branch"),
        subdir: param(parsed, "subdir"),
    })
}

fn encrypt_api_key(
    crypto: Option<&CryptoService>,
    endpoint_id: &str,
    api_key: &str,
) -> Result<Vec<u8>, String> {
    let crypto = crypto.ok_or_else(|| "系统凭据管理器不可用，无法保存凭据".to_string())?;
    let plaintext = serde_json::to_vec(&json!({ "api_key": api_key }))
        .map_err(|e| format!("序列化凭据失败: {}", e))?;
    crypto
        .encrypt(&plaintext, endpoint_id.as_bytes())
        .map_err(|e| format!("加密失败: {}", e))
}

fn switch_provider(
    db: &Mutex<Connection>,
    data_dir: &Path,
    provider_id: &str,
    crypto: Option<&CryptoService>,
) -> Result<Vec<String>, String> {
    let provider = providers::get(db, provider_id)?.ok_or_else(|| "provider 不存在".to_string())?;
    let tool = Tool::from_str(&provider.app_type)
        .filter(|tool| tool.supports_takeover())
        .ok_or_else(|| format!("app_type '{}' 不支持切换", provider.app_type))?;
    let prev_current = providers::get_current(db, &provider.app_type)?;
    let prev_for_backfill = prev_current.as_ref().filter(|p| p.id != provider.id);
    providers::set_current(db, provider_id)?;

    let result = match tool {
        Tool::ClaudeCode => {
            tool_takeover::switch_claude(db, data_dir, prev_for_backfill, &provider, crypto)
        }
        Tool::Codex => {
            tool_takeover::enable_direct(db, tool, data_dir, &provider, crypto).map(|_| Vec::new())
        }
        _ => unreachable!(),
    };

    match result {
        Ok(warnings) => Ok(warnings),
        Err(e) => {
            let rollback = match prev_current {
                Some(prev) => providers::set_current(db, &prev.id),
                None => providers::clear_current(db, &provider.app_type),
            };
            if let Err(re) = rollback {
                return Err(format!(
                    "切换失败且回滚失败: 切换错误={}; 回滚错误={}",
                    e, re
                ));
            }
            Err(e)
        }
    }
}

fn protocol_for_app(app_type: &str) -> &'static str {
    match app_type {
        "codex" => "openai-responses",
        _ => "anthropic",
    }
}

fn unique_provider_name(
    db: &Mutex<Connection>,
    app_type: &str,
    desired: &str,
) -> Result<String, String> {
    let existing: HashSet<String> = providers::list_by_app(db, app_type)?
        .into_iter()
        .map(|p| p.name)
        .collect();
    if !existing.contains(desired) {
        return Ok(desired.to_string());
    }
    let base = format!("{} (Deep Link)", desired);
    if !existing.contains(&base) {
        return Ok(base);
    }
    for n in 2..=1000 {
        let candidate = format!("{} (Deep Link {})", desired, n);
        if !existing.contains(&candidate) {
            return Ok(candidate);
        }
    }
    Ok(format!("{} ({})", desired, uuid::Uuid::new_v4()))
}

fn decode_json_b64(raw: &str, field: &str) -> Result<Value, String> {
    let text = decode_text_b64(raw, field)?;
    serde_json::from_str(&text).map_err(|e| format!("{} 不是合法 JSON: {}", field, e))
}

fn decode_text_b64(raw: &str, field: &str) -> Result<String, String> {
    let bytes = decode_b64(raw).map_err(|e| format!("{} Base64 解码失败: {}", field, e))?;
    String::from_utf8(bytes).map_err(|e| format!("{} 不是合法 UTF-8: {}", field, e))
}

fn decode_b64(raw: &str) -> Result<Vec<u8>, base64::DecodeError> {
    let normalized = raw.trim().replace(' ', "+");
    STANDARD
        .decode(&normalized)
        .or_else(|_| STANDARD_NO_PAD.decode(&normalized))
        .or_else(|_| URL_SAFE.decode(&normalized))
        .or_else(|_| URL_SAFE_NO_PAD.decode(&normalized))
}

fn param(parsed: &ParsedDeepLink, key: &str) -> Option<String> {
    parsed
        .params
        .get(key)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn bool_param(parsed: &ParsedDeepLink, key: &str) -> bool {
    param(parsed, key)
        .map(|v| matches!(v.as_str(), "true" | "1" | "yes" | "on"))
        .unwrap_or(false)
}

fn env_string(env: Option<&serde_json::Map<String, Value>>, key: &str) -> Option<String> {
    env.and_then(|m| m.get(key))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn put_env(map: &mut serde_json::Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) {
        map.insert(key.to_string(), Value::String(value.to_string()));
    }
}

fn is_sensitive_key(key: &str) -> bool {
    SENSITIVE_KEYS.iter().any(|k| key.eq_ignore_ascii_case(k))
}

fn item(label: &str, value: &str, sensitive: bool) -> PreviewItem {
    PreviewItem {
        label: label.to_string(),
        value: value.to_string(),
        sensitive,
    }
}

fn summarize(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in text.chars().take(max_chars) {
        out.push(ch);
    }
    if text.chars().count() > max_chars {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;
    use crate::services::crypto::{generate_master_key, CryptoService};

    fn setup_db() -> Mutex<Connection> {
        let conn = Connection::open_in_memory().expect("创建内存数据库失败");
        let db = Mutex::new(conn);
        run_migrations(&db).expect("迁移失败");
        db
    }

    fn crypto() -> CryptoService {
        CryptoService::new(generate_master_key())
    }

    #[test]
    fn parse_accepts_only_ccswitch_v1_import() {
        let ok = parse("ccswitch://v1/import?resource=provider&app=claude").unwrap();
        assert_eq!(ok.resource, DeepLinkResource::Provider);

        assert!(parse("agentswitch://v1/import?resource=provider").is_err());
        assert!(parse("ccswitch://v2/import?resource=provider").is_err());
        assert!(parse("ccswitch://v1/other?resource=provider").is_err());
        assert!(parse("ccswitch://v1/import?resource=unknown").is_err());
    }

    #[test]
    fn redaction_masks_sensitive_query_values() {
        let redacted = redact_url(
            "ccswitch://v1/import?resource=provider&apiKey=secret&content=abc&name=Visible",
        );
        assert!(redacted.contains("name=Visible"));
        assert!(!redacted.contains("secret"));
        assert!(!redacted.contains("content=abc"));
        assert!(redacted.contains("apiKey=%5Bredacted%5D"));
    }

    #[test]
    fn prompt_content_decodes_base64() {
        let raw = "ccswitch://v1/import?resource=prompt&app=claude&name=P&content=IyBIZWxsbwo=";
        let preview = preview(raw).unwrap();
        assert_eq!(preview.resource, DeepLinkResource::Prompt);
        assert!(!preview.blocked);
        assert!(preview
            .fields
            .iter()
            .any(|f| f.label == "内容摘要" && f.value.contains("# Hello")));
    }

    #[tokio::test]
    async fn provider_import_encrypts_key_and_keeps_settings_config_clean() {
        let db = setup_db();
        let crypto = crypto();
        let raw = "ccswitch://v1/import?resource=provider&app=claude&name=DL&endpoint=https%3A%2F%2Fapi.example.com&apiKey=sk-secret&model=claude-x";
        let result = import(&db, Some(&crypto), std::env::temp_dir().as_path(), raw)
            .await
            .unwrap();
        assert!(result.errors.is_empty());

        let providers = providers::list_by_app(&db, "claude-code").unwrap();
        assert_eq!(providers.len(), 1);
        let provider = &providers[0];
        assert!(!provider.settings_config.contains("sk-secret"));
        assert!(provider.settings_config.contains("endpoint_id"));
        assert!(provider.meta.contains("ANTHROPIC_MODEL"));

        let endpoints = endpoints::list(&db).unwrap();
        assert_eq!(endpoints.len(), 1);
        let endpoint = &endpoints[0];
        let plaintext = crypto
            .decrypt(
                endpoint.api_key_encrypted.as_ref().unwrap(),
                endpoint.id.as_bytes(),
            )
            .unwrap();
        let json: Value = serde_json::from_slice(&plaintext).unwrap();
        assert_eq!(json["api_key"], "sk-secret");
    }

    #[tokio::test]
    async fn mcp_existing_different_config_is_not_overwritten() {
        let db = setup_db();
        mcp_servers::create(
            &db,
            mcp_servers::NewMcpServer {
                id: "srv".to_string(),
                name: "srv".to_string(),
                server_config: json!({ "command": "old" }).to_string(),
                description: None,
                homepage: None,
                docs: None,
                tags: "[]".to_string(),
                enabled_claude: false,
            },
        )
        .unwrap();
        let config =
            STANDARD.encode(json!({ "mcpServers": { "srv": { "command": "new" } } }).to_string());
        let raw = format!(
            "ccswitch://v1/import?resource=mcp&apps=claude&enabled=true&config={}",
            config
        );
        let result = import(&db, None, std::env::temp_dir().as_path(), &raw)
            .await
            .unwrap();
        assert_eq!(result.skipped.len(), 1);
        let row = mcp_servers::get(&db, "srv").unwrap().unwrap();
        assert_eq!(row.server_config, json!({ "command": "old" }).to_string());
        assert!(!row.enabled_claude);
    }

    #[test]
    fn skill_preview_parses_repo_and_is_not_blocked() {
        // 阶段 C：Skills 后端安装已接入，skill Deep Link 不再 blocked。
        // 实际安装走网络（install_repo），单测只覆盖解析/预览，不触网。
        let raw =
            "ccswitch://v1/import?resource=skill&repo=owner%2Fname&branch=main&subdir=skills%2Ffoo";
        let preview = preview(raw).unwrap();
        assert_eq!(preview.resource, DeepLinkResource::Skill);
        assert!(!preview.blocked);
        // repo + 分支 + 子目录三个字段。
        assert_eq!(preview.fields.len(), 3);
        assert!(preview.fields.iter().any(|f| f.value == "owner/name"));
    }

    #[test]
    fn skill_preview_reports_missing_repo() {
        let raw = "ccswitch://v1/import?resource=skill";
        let preview = preview(raw).unwrap();
        assert_eq!(preview.resource, DeepLinkResource::Skill);
        assert!(!preview.warnings.is_empty());
    }
}
