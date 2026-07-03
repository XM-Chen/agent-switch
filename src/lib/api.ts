const API_BASE = 'http://127.0.0.1:42567/api';

export interface Account {
  id: string;
  name: string;
  account_type: string;
  platform: string;
  status: string;
  priority: number;
  has_credentials: boolean;
  last_login_at: string | null;
  last_error: string | null;
  last_error_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface Endpoint {
  id: string;
  account_id: string | null;
  name: string;
  base_url: string;
  protocol_type: string;
  auth_mode: string;
  enabled: boolean;
  priority: number;
  cooldown_until: string | null;
  last_success_at: string | null;
  last_failure_at: string | null;
  last_error_kind: string | null;
  has_api_key: boolean;
  created_at: string;
  updated_at: string;
}

export interface CodexLoginResponse {
  auth_url: string;
  message: string;
}

export interface CodexStatus {
  login_in_progress: boolean;
}

async function request<T>(path: string, options?: RequestInit): Promise<T> {
  const resp = await fetch(`${API_BASE}${path}`, {
    headers: { 'Content-Type': 'application/json' },
    ...options,
  });
  if (!resp.ok) {
    const text = await resp.text().catch(() => '');
    throw new Error(`${resp.status}: ${text || resp.statusText}`);
  }
  if (resp.status === 204) {
    return undefined as T;
  }
  return resp.json() as Promise<T>;
}

export const accountsApi = {
  list: () => request<Account[]>('/accounts'),
  get: (id: string) => request<Account>(`/accounts/${id}`),
  create: (data: {
    name: string;
    account_type: string;
    platform: string;
    api_key?: string;
    priority?: number;
  }) => request<Account>('/accounts', { method: 'POST', body: JSON.stringify(data) }),
  update: (id: string, data: {
    name?: string;
    status?: string;
    api_key?: string | null;
    priority?: number;
  }) => request<void>(`/accounts/${id}`, { method: 'PUT', body: JSON.stringify(data) }),
  delete: (id: string) => request<void>(`/accounts/${id}`, { method: 'DELETE' }),
};

export const endpointsApi = {
  list: () => request<Endpoint[]>('/endpoints'),
  get: (id: string) => request<Endpoint>(`/endpoints/${id}`),
  create: (data: {
    account_id?: string;
    name: string;
    base_url: string;
    protocol_type: string;
    auth_mode: string;
    api_key?: string;
    priority?: number;
  }) => request<Endpoint>('/endpoints', { method: 'POST', body: JSON.stringify(data) }),
  update: (id: string, data: {
    account_id?: string | null;
    name?: string;
    base_url?: string;
    protocol_type?: string;
    auth_mode?: string;
    api_key?: string | null;
    priority?: number;
  }) => request<void>(`/endpoints/${id}`, { method: 'PUT', body: JSON.stringify(data) }),
  toggle: (id: string, enabled: boolean) =>
    request<void>(`/endpoints/${id}/toggle`, {
      method: 'POST',
      body: JSON.stringify({ enabled }),
    }),
  delete: (id: string) => request<void>(`/endpoints/${id}`, { method: 'DELETE' }),
};

export const authApi = {
  startCodexLogin: () =>
    request<CodexLoginResponse>('/auth/codex/login', { method: 'POST' }),
  codexStatus: () => request<CodexStatus>('/auth/codex/status'),
};

export interface ModelItem {
  id: string;
  endpoint_id: string;
  model_name: string;
  display_name: string;
  source: string;
  capabilities: string[];
  context_window: number | null;
  is_available: boolean;
  last_seen_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface SyncReport {
  synced_at: string;
  succeeded: { endpoint_id: string; endpoint_name: string; model_count: number }[];
  failed: { endpoint_id: string; endpoint_name: string; model_count: number }[];
  errors: string[];
}

export interface AliasItem {
  id: string;
  scope_type: string;
  scope_id: string | null;
  alias_name: string;
  target_endpoint_id: string | null;
  target_model_name: string;
  priority: number;
  enabled: boolean;
  invalid_reason: string | null;
  created_at: string;
  updated_at: string;
}

export interface ResolvedAlias {
  alias_name: string;
  matched_scope: string;
  candidates: {
    endpoint_id: string | null;
    model_name: string;
    priority: number;
    is_valid: boolean;
    invalid_reason: string | null;
  }[];
}

export interface AutoRefreshState {
  enabled: boolean;
  last_sync_at: string | null;
  last_sync_error: string | null;
}

export const modelsApi = {
  list: (params?: { endpoint_id?: string; source?: string; capability?: string }) => {
    const qs = new URLSearchParams();
    if (params?.endpoint_id) qs.set('endpoint_id', params.endpoint_id);
    if (params?.source) qs.set('source', params.source);
    if (params?.capability) qs.set('capability', params.capability);
    const suffix = qs.toString() ? `?${qs.toString()}` : '';
    return request<ModelItem[]>(`/models${suffix}`);
  },
  sync: () => request<SyncReport>('/models/sync', { method: 'POST' }),
  createCustom: (data: {
    endpoint_id: string;
    model_name: string;
    display_name?: string;
    capabilities?: string[];
    context_window?: number;
  }) => request<ModelItem>('/models/custom', { method: 'POST', body: JSON.stringify(data) }),
  delete: (id: string) => request<void>(`/models/${id}`, { method: 'DELETE' }),
};

export const aliasesApi = {
  list: (params?: { scope_type?: string; scope_id?: string }) => {
    const qs = new URLSearchParams();
    if (params?.scope_type) qs.set('scope_type', params.scope_type);
    if (params?.scope_id) qs.set('scope_id', params.scope_id);
    const suffix = qs.toString() ? `?${qs.toString()}` : '';
    return request<AliasItem[]>(`/models/aliases${suffix}`);
  },
  create: (data: {
    scope_type: string;
    scope_id?: string | null;
    alias_name: string;
    target_endpoint_id?: string | null;
    target_model_name: string;
    priority?: number;
  }) => request<AliasItem>('/models/aliases', { method: 'POST', body: JSON.stringify(data) }),
  delete: (id: string) => request<void>(`/models/aliases/${id}`, { method: 'DELETE' }),
  resolve: (alias: string, params?: { tool?: string; route_id?: string; endpoint_id?: string }) => {
    const qs = new URLSearchParams();
    if (params?.tool) qs.set('tool', params.tool);
    if (params?.route_id) qs.set('route_id', params.route_id);
    if (params?.endpoint_id) qs.set('endpoint_id', params.endpoint_id);
    const suffix = qs.toString() ? `?${qs.toString()}` : '';
    return request<ResolvedAlias>(`/models/resolve/${encodeURIComponent(alias)}${suffix}`);
  },
};

export const settingsApi = {
  getAutoRefresh: () => request<AutoRefreshState>('/settings/auto-model-refresh'),
  setAutoRefresh: (enabled: boolean) =>
    request<void>('/settings/auto-model-refresh', {
      method: 'PUT',
      body: JSON.stringify({ enabled }),
    }),
};

// ── 配置导入导出 ──────────────────────────────────────────

export type PortabilityMode = 'full_backup' | 'portable';

/// 各表导入计数（与后端 ImportReport 字段一一对应）。
export interface ImportReport {
  accounts: number;
  endpoints: number;
  endpoint_models: number;
  model_aliases: number;
  route_settings: number;
  tool_takeover: number;
}

/// 导出结果：package 为导出包 JSON 文本（前端触发下载），warnings 含弱密码等提示。
export interface ExportResult {
  package: string;
  warnings: string[];
}

/// 导入结果：imported 为各表计数，pre_import_backup 仅 full_backup 导入前自动备份路径。
export interface ImportResult {
  imported: ImportReport;
  pre_import_backup?: string | null;
  warnings: string[];
}

export const portabilityApi = {
  /// 导出配置。full_backup 忽略 password（用主密钥），portable 必带 password。
  exportConfig: (mode: PortabilityMode, password?: string) =>
    request<ExportResult>('/settings/export', {
      method: 'POST',
      body: JSON.stringify({ mode, password }),
    }),
  /// 导入配置。package 为导出包 JSON 文本，portable 包需带 password。
  importConfig: (data: {
    package: string;
    password?: string;
    conflict_mode?: string;
  }) =>
    request<ImportResult>('/settings/import', {
      method: 'POST',
      body: JSON.stringify(data),
    }),
};

export interface ToolStatus {
  tool: string;
  supports_takeover: boolean;
  enabled: boolean;
  live_category: string;
  last_applied_at: string | null;
  last_target: string | null;
  last_error: string | null;
}

export interface ToolBackup {
  id: string;
  original_path: string;
  backup_path: string;
  original_existed: boolean;
  takeover_target: string | null;
  created_at: string;
}

export const toolsApi = {
  list: () => request<ToolStatus[]>('/tools'),
  get: (tool: string) => request<ToolStatus>(`/tools/${tool}`),
  setTakeover: (tool: string, enabled: boolean) =>
    request<void>(`/tools/${tool}/takeover`, {
      method: 'POST',
      body: JSON.stringify({ enabled }),
    }),
  reapply: (tool: string) =>
    request<void>(`/tools/${tool}/reapply`, { method: 'POST' }),
  backups: (tool: string) => request<ToolBackup[]>(`/tools/${tool}/backups`),
};

// ── 路由设置 ──────────────────────────────────────────────

export interface RouteCandidate {
  id: string;
  name: string;
  base_url: string;
  protocol_type: string;
  priority: number;
  enabled: boolean;
  cooldown_until: string | null;
  last_success_at: string | null;
  last_failure_at: string | null;
  last_error_kind: string | null;
}

export interface RouteSettings {
  id: string;
  label: string;
  strategy: string;
  protocol_type: string;
  failover_enabled: boolean;
  max_switches: number;
  same_account_retries: number;
  cooldown_multiplier: number;
  updated_at: string;
  candidates: RouteCandidate[];
}

export interface UpdateRouteRequest {
  strategy?: string;
  failover_enabled?: boolean;
  max_switches?: number;
  same_account_retries?: number;
  cooldown_multiplier?: number;
}

export const routesApi = {
  list: () => request<RouteSettings[]>('/routes'),
  get: (id: string) => request<RouteSettings>(`/routes/${id}`),
  update: (id: string, data: UpdateRouteRequest) =>
    request<void>(`/routes/${id}`, { method: 'PUT', body: JSON.stringify(data) }),
};

// ── 请求日志 ──────────────────────────────────────────────

export interface LogEntry {
  id: string;
  request_id: string;
  tool: string | null;
  inbound_endpoint: string;
  requested_model: string | null;
  resolved_alias: string | null;
  resolved_scope: string | null;
  target_endpoint_id: string | null;
  upstream_model: string | null;
  status: number | null;
  error_kind: string | null;
  fallback_chain: string | null;
  stream: boolean;
  duration_ms: number | null;
  first_token_ms: number | null;
  input_tokens: number | null;
  output_tokens: number | null;
  created_at: string;
}

export interface LogDetail extends LogEntry {
  upstream_endpoint: string | null;
  protocol_from: string | null;
  protocol_to: string | null;
  cache_creation_tokens: number | null;
  cache_read_tokens: number | null;
  request_body_hash: string | null;
}

export interface LogListResponse {
  items: LogEntry[];
  total: number;
}

export interface LogListParams {
  tool?: string;
  status?: number;
  from?: string;
  to?: string;
  limit?: number;
  offset?: number;
}

export const logsApi = {
  list: (params?: LogListParams) => {
    const qs = new URLSearchParams();
    if (params?.tool) qs.set('tool', params.tool);
    if (params?.status !== undefined) qs.set('status', String(params.status));
    if (params?.from) qs.set('from', params.from);
    if (params?.to) qs.set('to', params.to);
    if (params?.limit) qs.set('limit', String(params.limit));
    if (params?.offset) qs.set('offset', String(params.offset));
    const suffix = qs.toString() ? `?${qs.toString()}` : '';
    return request<LogListResponse>(`/logs${suffix}`);
  },
  get: (id: string) => request<LogDetail>(`/logs/${id}`),
};

// ── 链路测试 ─────────────────────────────────────────────

export interface TestRequest {
  route: string;
  path: string;
  model?: string;
  prompt: string;
  stream?: boolean;
}

export interface TestResponse {
  status: number;
  body: Record<string, unknown>;
  duration_ms: number;
  endpoint_id: string | null;
  error: string | null;
}

export interface TestStreamHandle {
  abort: () => void;
}

export interface TestStreamCallbacks {
  onChunk: (text: string) => void;
  onDone: (meta: { status: number; duration_ms: number; endpoint_id: string | null }) => void;
  onError: (err: Error) => void;
}

export const testsApi = {
  run: (data: TestRequest) => request<TestResponse>('/tests', {
    method: 'POST',
    body: JSON.stringify(data),
  }),

  /**
   * 流式测试：用 fetch + ReadableStream.getReader() 逐块读取 SSE 文本。
   *
   * 返回句柄包含 abort() 方法用于取消。
   */
  runStream: (data: TestRequest, callbacks: TestStreamCallbacks): TestStreamHandle => {
    const ac = new AbortController();

    (async () => {
      let resp: Response;
      try {
        resp = await fetch(`${API_BASE}/tests`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(data),
          signal: ac.signal,
        });
      } catch (e) {
        // abort 视为取消，不作为错误
        if (ac.signal.aborted) return;
        callbacks.onError(e instanceof Error ? e : new Error(String(e)));
        return;
      }

      // 读取响应头元数据
      const duration = Number(resp.headers.get('x-test-duration-ms') ?? 0) || 0;
      const endpoint_id = resp.headers.get('x-endpoint-id');

      if (!resp.ok || !resp.body) {
        // 非 2xx 或无响应体：读取错误文本
        const text = await resp.text().catch(() => '');
        callbacks.onError(new Error(`${resp.status}: ${text || resp.statusText}`));
        callbacks.onDone({ status: resp.status, duration_ms: duration, endpoint_id });
        return;
      }

      // 逐块读取流
      const reader = resp.body.getReader();
      const decoder = new TextDecoder();
      try {
        while (true) {
          const { value, done } = await reader.read();
          if (done) break;
          if (value) {
            callbacks.onChunk(decoder.decode(value, { stream: true }));
          }
        }
        // flush 剩余字节
        callbacks.onChunk(decoder.decode());
        callbacks.onDone({ status: resp.status, duration_ms: duration, endpoint_id });
      } catch (e) {
        if (ac.signal.aborted) {
          // 已取消：仍回传元数据
          callbacks.onDone({ status: resp.status, duration_ms: duration, endpoint_id });
          return;
        }
        callbacks.onError(e instanceof Error ? e : new Error(String(e)));
      }
    })();

    return {
      abort: () => ac.abort(),
    };
  },
};
