use std::sync::Arc;

use tokio::sync::RwLock;

use crate::proxy::providers::codex_oauth_auth::CodexOAuthManager;
use crate::proxy::providers::copilot_auth::CopilotAuthManager;

/// GitHub Copilot 托管 OAuth 上游的进程内状态。
pub struct CopilotAuthState(pub Arc<RwLock<CopilotAuthManager>>);

/// ChatGPT Codex 托管 OAuth 上游的进程内状态。
pub struct CodexOAuthState(pub Arc<RwLock<CodexOAuthManager>>);
