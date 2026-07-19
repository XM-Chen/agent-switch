//! 三个现有模块（Claude / Codex / Gemini）的版本化精确快照 adapter。
//!
//! 每个 adapter 实现 C1 定义的 `SnapshotModuleAdapter`：
//! - `capture_targets`：读取首次开启接管前的**原始文件字节**（不解析，不规整），
//!   目标存在则记录字节，不存在则 `existed=false`。
//! - `restore_manifest_transactional`：按 manifest 逐目标恢复——`existed=true`
//!   逐字节写回，`existed=false` 删除由 Agent-Switch 创建的目标；多目标先在内存
//!   备份当前状态，任一步失败即补偿回滚已写目标。
//! - `restore_legacy`：升级前遗留的无版本 JSON 备份走 best-effort 恢复，不伪称逐字节。
//!
//! 逐字节 round-trip 支持非 UTF-8（Gemini `.env`、Codex `config.toml` 的注释/格式），
//! 这是相对旧“解析 JSON 再序列化”备份最大的正确性提升。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::app_config::AppType;
use crate::config::{atomic_write, delete_file};
use crate::proxy::snapshot::{
    LegacySnapshot, SnapshotManifest, SnapshotModuleAdapter, SnapshotTarget,
};

/// 读取文件原始字节；不存在返回 `Ok(None)`，其它 IO 错误上抛。
fn read_optional_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("读取 {} 失败: {error}", path.display())),
    }
}

/// 按 target 的 `existed` 语义写回或删除单个文件目标。
/// `existed=true` 逐字节写回原始 payload；`existed=false` 删除 AGS 创建的目标。
fn apply_file_bytes_target(path: &Path, target: &SnapshotTarget) -> Result<(), String> {
    match target.file_payload()? {
        Some(bytes) => atomic_write(path, &bytes)
            .map_err(|error| format!("写回 {} 失败: {error}", path.display())),
        None => delete_file(path).map_err(|error| format!("删除 {} 失败: {error}", path.display())),
    }
}

/// 恢复前先在内存里备份目标当前字节，供补偿回滚使用。
struct FileTargetGuard {
    path: PathBuf,
    previous: Option<Vec<u8>>,
}

impl FileTargetGuard {
    fn capture(path: PathBuf) -> Result<Self, String> {
        let previous = read_optional_bytes(&path)?;
        Ok(Self { path, previous })
    }

    /// 把目标补偿回滚到 capture 时的状态。
    fn rollback(&self) -> Result<(), String> {
        match &self.previous {
            Some(bytes) => atomic_write(&self.path, bytes)
                .map_err(|error| format!("补偿写回 {} 失败: {error}", self.path.display())),
            None => delete_file(&self.path)
                .map_err(|error| format!("补偿删除 {} 失败: {error}", self.path.display())),
        }
    }
}

/// 从 manifest 中按 id 找到 file_bytes 目标。
fn find_target<'a>(manifest: &'a SnapshotManifest, id: &str) -> Result<&'a SnapshotTarget, String> {
    manifest
        .targets
        .iter()
        .find(|target| target.id() == id)
        .ok_or_else(|| format!("快照缺少目标 '{id}'"))
}

/// 校验 adapter 支持的目标集合，避免忽略未知 target 后误清所有权。
fn validate_file_targets(
    manifest: &SnapshotManifest,
    app_type: &AppType,
    expected_ids: &[&str],
) -> Result<(), String> {
    manifest.validate()?;
    if manifest.app_type != app_type.as_str() {
        return Err(format!(
            "快照 app_type 不匹配：期望 {}，实际 {}",
            app_type.as_str(),
            manifest.app_type
        ));
    }
    if manifest.targets.len() != expected_ids.len()
        || manifest
            .targets
            .iter()
            .any(|target| !expected_ids.contains(&target.id()))
    {
        return Err(format!("{} 快照包含不支持的目标集合", app_type.as_str()));
    }
    for id in expected_ids {
        find_target(manifest, id)?.file_payload()?;
    }
    Ok(())
}

// ==================== Claude ====================

/// Claude adapter：单目标 `settings`（settings.json）。
pub struct ClaudeSnapshotAdapter;

impl ClaudeSnapshotAdapter {
    const TARGET_SETTINGS: &'static str = "settings";

    fn settings_path() -> PathBuf {
        crate::config::get_claude_settings_path()
    }
}

impl SnapshotModuleAdapter for ClaudeSnapshotAdapter {
    fn app_type(&self) -> AppType {
        AppType::Claude
    }

    fn capture_targets(&self) -> Result<Vec<SnapshotTarget>, String> {
        let bytes = read_optional_bytes(&Self::settings_path())?;
        Ok(vec![SnapshotTarget::file_bytes(
            Self::TARGET_SETTINGS,
            bytes.as_deref(),
        )])
    }

    fn restore_manifest_transactional(&self, manifest: &SnapshotManifest) -> Result<(), String> {
        validate_file_targets(manifest, &AppType::Claude, &[Self::TARGET_SETTINGS])?;
        let target = find_target(manifest, Self::TARGET_SETTINGS)?;
        apply_file_bytes_target(&Self::settings_path(), target)
    }

    fn restore_legacy(&self, legacy: &LegacySnapshot) -> Result<(), String> {
        // 旧无版本备份是“解析后 JSON”，best-effort 经现有 sanitize + 写盘恢复。
        let config: Value = serde_json::from_str(&legacy.original_config)
            .map_err(|error| format!("解析 Claude 旧版备份失败: {error}"))?;
        let path = Self::settings_path();
        let settings = crate::services::provider::sanitize_claude_settings_for_live(&config);
        crate::config::write_json_file(&path, &settings)
            .map_err(|error| format!("写入 Claude 配置失败: {error}"))
    }
}

// ==================== Codex ====================

/// Codex adapter：多目标 `auth`（auth.json）/ `config`（config.toml）/
/// `model_catalog`（AGS 写的 model catalog 指针文件），事务式恢复 + 补偿回滚。
pub struct CodexSnapshotAdapter;

impl CodexSnapshotAdapter {
    const TARGET_AUTH: &'static str = "auth";
    const TARGET_CONFIG: &'static str = "config";
    const TARGET_MODEL_CATALOG: &'static str = "model_catalog";

    fn auth_path() -> PathBuf {
        crate::codex_config::get_codex_auth_path()
    }

    fn config_path() -> PathBuf {
        crate::codex_config::get_codex_config_path()
    }

    fn model_catalog_path() -> PathBuf {
        crate::codex_config::get_codex_model_catalog_path()
    }
}

impl SnapshotModuleAdapter for CodexSnapshotAdapter {
    fn app_type(&self) -> AppType {
        AppType::Codex
    }

    fn capture_targets(&self) -> Result<Vec<SnapshotTarget>, String> {
        let auth = read_optional_bytes(&Self::auth_path())?;
        let config = read_optional_bytes(&Self::config_path())?;
        // 首次开启前 model_catalog 文件通常不存在 → existed=false → 关闭时删除该 AGS 创建文件。
        let catalog = read_optional_bytes(&Self::model_catalog_path())?;
        Ok(vec![
            SnapshotTarget::file_bytes(Self::TARGET_AUTH, auth.as_deref()),
            SnapshotTarget::file_bytes(Self::TARGET_CONFIG, config.as_deref()),
            SnapshotTarget::file_bytes(Self::TARGET_MODEL_CATALOG, catalog.as_deref()),
        ])
    }

    fn restore_manifest_transactional(&self, manifest: &SnapshotManifest) -> Result<(), String> {
        let plan = [
            (Self::TARGET_AUTH, Self::auth_path()),
            (Self::TARGET_CONFIG, Self::config_path()),
            (Self::TARGET_MODEL_CATALOG, Self::model_catalog_path()),
        ];

        // 在任何文件写入前完成三件事：目标齐全、kind/payload 合法、当前三文件均可读取。
        // 否则后续目标的预捕获失败会让前面已写目标无法补偿，破坏多文件事务语义。
        validate_file_targets(
            manifest,
            &AppType::Codex,
            &[
                Self::TARGET_AUTH,
                Self::TARGET_CONFIG,
                Self::TARGET_MODEL_CATALOG,
            ],
        )?;
        let guards = plan
            .iter()
            .map(|(_, path)| FileTargetGuard::capture(path.clone()))
            .collect::<Result<Vec<_>, _>>()?;

        for (id, path) in &plan {
            let target = find_target(manifest, id)?;
            if let Err(error) = apply_file_bytes_target(path, target) {
                let rollback_errors = guards
                    .iter()
                    .rev()
                    .filter_map(|guard| guard.rollback().err())
                    .collect::<Vec<_>>();
                return if rollback_errors.is_empty() {
                    Err(error)
                } else {
                    Err(format!(
                        "{error}；补偿回滚失败: {}",
                        rollback_errors.join("；")
                    ))
                };
            }
        }
        Ok(())
    }

    fn restore_legacy(&self, legacy: &LegacySnapshot) -> Result<(), String> {
        // 旧无版本 Codex 备份是 `{ auth, config }` JSON，best-effort 走现有 verbatim 写盘。
        let config: Value = serde_json::from_str(&legacy.original_config)
            .map_err(|error| format!("解析 Codex 旧版备份失败: {error}"))?;
        crate::codex_config::write_codex_live_verbatim(&config)
            .map_err(|error| format!("写入 Codex 配置失败: {error}"))
    }
}

// ==================== Gemini ====================

/// Gemini adapter：单目标 `.env`（.env）。
pub struct GeminiSnapshotAdapter;

impl GeminiSnapshotAdapter {
    const TARGET_ENV: &'static str = ".env";

    fn env_path() -> PathBuf {
        crate::gemini_config::get_gemini_env_path()
    }
}

impl SnapshotModuleAdapter for GeminiSnapshotAdapter {
    fn app_type(&self) -> AppType {
        AppType::Gemini
    }

    fn capture_targets(&self) -> Result<Vec<SnapshotTarget>, String> {
        let bytes = read_optional_bytes(&Self::env_path())?;
        Ok(vec![SnapshotTarget::file_bytes(
            Self::TARGET_ENV,
            bytes.as_deref(),
        )])
    }

    fn restore_manifest_transactional(&self, manifest: &SnapshotManifest) -> Result<(), String> {
        validate_file_targets(manifest, &AppType::Gemini, &[Self::TARGET_ENV])?;
        let target = find_target(manifest, Self::TARGET_ENV)?;
        apply_file_bytes_target(&Self::env_path(), target)
    }

    fn restore_legacy(&self, legacy: &LegacySnapshot) -> Result<(), String> {
        // 旧无版本 Gemini 备份是 `{ env: {...} }` JSON，best-effort 经 json_to_env 写回。
        let config: Value = serde_json::from_str(&legacy.original_config)
            .map_err(|error| format!("解析 Gemini 旧版备份失败: {error}"))?;
        let env_map = crate::gemini_config::json_to_env(&config)
            .map_err(|error| format!("转换 Gemini 配置失败: {error}"))?;
        crate::gemini_config::write_gemini_env_atomic(&env_map)
            .map_err(|error| format!("写入 Gemini env 失败: {error}"))
    }
}

/// C2a 三模块的 adapter 分发；其余四模块由 C2b 补齐，此处返回 `None`。
pub fn snapshot_adapter_for_app(app_type: &AppType) -> Option<Box<dyn SnapshotModuleAdapter>> {
    match app_type {
        AppType::Claude => Some(Box::new(ClaudeSnapshotAdapter)),
        AppType::Codex => Some(Box::new(CodexSnapshotAdapter)),
        AppType::Gemini => Some(Box::new(GeminiSnapshotAdapter)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::env;
    use tempfile::TempDir;

    struct TempHome {
        _dir: TempDir,
        original_home: Option<String>,
        original_userprofile: Option<String>,
        original_test_home: Option<String>,
    }

    impl TempHome {
        fn new() -> Self {
            let dir = TempDir::new().expect("failed to create temp home");
            let original_home = env::var("HOME").ok();
            let original_userprofile = env::var("USERPROFILE").ok();
            let original_test_home = env::var("AGENT_SWITCH_TEST_HOME").ok();

            env::set_var("HOME", dir.path());
            env::set_var("USERPROFILE", dir.path());
            env::set_var("AGENT_SWITCH_TEST_HOME", dir.path());

            Self {
                _dir: dir,
                original_home,
                original_userprofile,
                original_test_home,
            }
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            match &self.original_home {
                Some(value) => env::set_var("HOME", value),
                None => env::remove_var("HOME"),
            }
            match &self.original_userprofile {
                Some(value) => env::set_var("USERPROFILE", value),
                None => env::remove_var("USERPROFILE"),
            }
            match &self.original_test_home {
                Some(value) => env::set_var("AGENT_SWITCH_TEST_HOME", value),
                None => env::remove_var("AGENT_SWITCH_TEST_HOME"),
            }
        }
    }

    #[test]
    #[serial]
    fn claude_adapter_round_trips_existing_bytes() {
        let _home = TempHome::new();
        let path = ClaudeSnapshotAdapter::settings_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        // 非规整 JSON（含用户手写格式）：逐字节还原，不经 sanitize。
        let original = b"{\n  \"env\": { \"ANTHROPIC_API_KEY\": \"real\" },\n  \"custom\": 1\n}\n";
        fs::write(&path, original).unwrap();

        let adapter = ClaudeSnapshotAdapter;
        let manifest = SnapshotManifest::new(&AppType::Claude, adapter.capture_targets().unwrap())
            .expect("build manifest");

        // 模拟接管写入后 live 被改动。
        fs::write(
            &path,
            b"{\"env\":{\"ANTHROPIC_BASE_URL\":\"http://127.0.0.1:42567\"}}",
        )
        .unwrap();

        adapter
            .restore_manifest_transactional(&manifest)
            .expect("restore");
        assert_eq!(fs::read(&path).unwrap(), original.to_vec());
    }

    #[test]
    #[serial]
    fn claude_adapter_missing_target_deletes_on_restore() {
        let _home = TempHome::new();
        let path = ClaudeSnapshotAdapter::settings_path();
        // 首次开启前不存在。
        let adapter = ClaudeSnapshotAdapter;
        let manifest = SnapshotManifest::new(&AppType::Claude, adapter.capture_targets().unwrap())
            .expect("build manifest");

        // 接管创建了文件。
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{\"created\":true}").unwrap();

        adapter
            .restore_manifest_transactional(&manifest)
            .expect("restore");
        assert!(!path.exists(), "existed=false target must be deleted");
    }

    #[test]
    #[serial]
    fn gemini_adapter_round_trips_non_utf8() {
        let _home = TempHome::new();
        let path = GeminiSnapshotAdapter::env_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        // 含非 UTF-8 字节，验证 base64 逐字节 round-trip。
        let original = [b'A', b'=', 0x80, 0xFF, b'\n'];
        fs::write(&path, original).unwrap();

        let adapter = GeminiSnapshotAdapter;
        let manifest = SnapshotManifest::new(&AppType::Gemini, adapter.capture_targets().unwrap())
            .expect("build manifest");

        fs::write(&path, b"GEMINI_API_KEY=PROXY_MANAGED\n").unwrap();

        adapter
            .restore_manifest_transactional(&manifest)
            .expect("restore");
        assert_eq!(fs::read(&path).unwrap(), original.to_vec());
    }

    #[test]
    #[serial]
    fn codex_adapter_multi_target_round_trip_and_catalog_delete() {
        let _home = TempHome::new();
        let auth_path = CodexSnapshotAdapter::auth_path();
        let config_path = CodexSnapshotAdapter::config_path();
        let catalog_path = CodexSnapshotAdapter::model_catalog_path();
        fs::create_dir_all(auth_path.parent().unwrap()).unwrap();

        // auth + config 首次开启前存在；catalog 不存在。
        let original_auth = b"{\"OPENAI_API_KEY\":\"real-key\"}";
        // config.toml 保留注释与格式（逐字节）。
        let original_config = b"# user comment\nmodel = \"gpt-5\"\n";
        fs::write(&auth_path, original_auth).unwrap();
        fs::write(&config_path, original_config).unwrap();

        let adapter = CodexSnapshotAdapter;
        let manifest = SnapshotManifest::new(&AppType::Codex, adapter.capture_targets().unwrap())
            .expect("build manifest");

        // 接管写入：改 auth/config，创建 catalog。
        fs::write(&auth_path, b"{\"OPENAI_API_KEY\":\"PROXY_MANAGED\"}").unwrap();
        fs::write(&config_path, b"base_url = \"http://127.0.0.1:42567/v1\"\n").unwrap();
        fs::write(&catalog_path, b"{\"models\":[]}").unwrap();

        adapter
            .restore_manifest_transactional(&manifest)
            .expect("restore");

        assert_eq!(fs::read(&auth_path).unwrap(), original_auth.to_vec());
        assert_eq!(fs::read(&config_path).unwrap(), original_config.to_vec());
        assert!(
            !catalog_path.exists(),
            "AGS-created model_catalog must be deleted on restore"
        );
    }

    #[test]
    #[serial]
    fn codex_adapter_rolls_back_on_partial_failure() {
        let _home = TempHome::new();
        let auth_path = CodexSnapshotAdapter::auth_path();
        let config_path = CodexSnapshotAdapter::config_path();
        fs::create_dir_all(auth_path.parent().unwrap()).unwrap();

        let original_auth = b"{\"OPENAI_API_KEY\":\"real-key\"}";
        fs::write(&auth_path, original_auth).unwrap();
        fs::write(&config_path, b"model = \"gpt-5\"\n").unwrap();

        let adapter = CodexSnapshotAdapter;
        let good_manifest =
            SnapshotManifest::new(&AppType::Codex, adapter.capture_targets().unwrap())
                .expect("build manifest");

        let managed_auth = b"{\"OPENAI_API_KEY\":\"PROXY_MANAGED\"}";
        fs::write(&auth_path, managed_auth).unwrap();

        // 让 config_path 成为目录，使“预捕获所有目标”在任何写入前失败。
        // auth 必须保持恢复调用前的 managed 字节，不能被半恢复成首次快照。
        delete_file(&config_path).unwrap();
        fs::create_dir_all(&config_path).unwrap();

        let result = adapter.restore_manifest_transactional(&good_manifest);
        assert!(
            result.is_err(),
            "restore must fail when config cannot be captured"
        );
        assert_eq!(
            fs::read(&auth_path).unwrap(),
            managed_auth.to_vec(),
            "a later target capture failure must not partially restore earlier targets"
        );
    }

    #[test]
    fn dispatcher_returns_three_modules_only() {
        assert!(snapshot_adapter_for_app(&AppType::Claude).is_some());
        assert!(snapshot_adapter_for_app(&AppType::Codex).is_some());
        assert!(snapshot_adapter_for_app(&AppType::Gemini).is_some());
        assert!(snapshot_adapter_for_app(&AppType::ClaudeDesktop).is_none());
        assert!(snapshot_adapter_for_app(&AppType::OpenCode).is_none());
        assert!(snapshot_adapter_for_app(&AppType::OpenClaw).is_none());
        assert!(snapshot_adapter_for_app(&AppType::Hermes).is_none());
    }
}
