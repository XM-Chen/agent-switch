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
