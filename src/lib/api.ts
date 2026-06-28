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
